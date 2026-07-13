//! Snap-to-road worker and per-track state store.
//!
//! Follows the file loader's channel shape ([`super::loader`]): owned by the
//! app, background threads reporting over an mpsc channel, `request_repaint`
//! on every message so results appear without user input.
//!
//! Tracks enter a priority queue via [`SnapScheduler::request_snap`], and
//! one run is in flight at a time so the server's fair-use budget is shared
//! globally (the transport also paces individual requests). Manual entries
//! run first (FIFO among themselves); automatic entries run only while
//! their track is shown on the map, and stay parked while hidden - server
//! load is bounded by what the user actually inspects.
//!
//! Two content-keyed stores hold completed runs, so both survive file
//! removals and index shifts:
//!
//! - The **cache** ([`SnapCacheKey`]: content fingerprint + parameters +
//!   server host) deduplicates requests - re-requesting a known combination
//!   is a cache hit, never a request.
//! - The **latest-run store** ([`TrackContentKey`]) holds the run each track
//!   currently displays. A cache hit promotes the cached run to latest, so
//!   switching parameters back and forth redisplays instantly.
//!
//! A latest run whose parameters or server host differ from what a fresh
//! run would use *now* is **stale**: still shown (results are never
//! silently dropped), but marked, with the difference spelled out and a
//! re-run offered ([`stale_reasons`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use egui::Context;

use gt_snap::request_plan::{self, RequestPlan, SnapParams};
use gt_snap::stitch::{self, SnapResult, SnapWarning, SnapWarningReporter};
use gt_snap::transport::HttpTransport;
use gt_snap::wire::Costing;
use gt_snap::{DEFAULT_SERVER_URL, server_host, transport};
use gt_types::mercator::{self};
use gt_types::{Latitude, LoadedTrack, Longitude, TrackRef, TravelMode};
use gt_ui_types::{
    SnappedEdgeInfo, SnappedEdgeSpan, SnappedSegment, SnappedTrackGeometry, TrackDataVisibility,
};

/// The costing a track snaps with: the file's declared travel mode beats the
/// configured default costing, and declarations without a road-network
/// counterpart (boat, rail, aircraft) make the track unsnappable (`None`).
/// Unknown declarations fall back to the configured default - an
/// unrecognized platform is no reason to refuse a manual snap.
pub fn resolve_costing(declared: Option<&TravelMode>, configured: Costing) -> Option<Costing> {
    match declared {
        None | Some(TravelMode::Unknown(_)) => Some(configured),
        Some(TravelMode::Car | TravelMode::Motorcycle) => Some(Costing::Auto),
        Some(TravelMode::Bicycle) => Some(Costing::Bicycle),
        Some(TravelMode::Pedestrian) => Some(Costing::Pedestrian),
        Some(TravelMode::Boat | TravelMode::Rail | TravelMode::Aircraft) => None,
    }
}

/// Content fingerprint of a track, identifying it independent of file/track
/// indices (which shift on removal). Tracks are immutable once loaded, so
/// time range + point count pin the content for session-store purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackContentKey {
    pub(crate) start_us: i64,
    pub(crate) end_us: i64,
    pub(crate) tpv_count: usize,
}

impl TrackContentKey {
    pub fn new(track: &LoadedTrack) -> Self {
        let range = track.metadata.time_range;
        Self {
            start_us: range.start.timestamp_micros(),
            end_us: range.end.timestamp_micros(),
            tpv_count: track.metadata.tpv_count,
        }
    }
}

/// [`SnapParams`] in hashable form: the float options as their bit
/// patterns. Bit equality is the right identity for cache purposes - the
/// values come from settings, not arithmetic, so equal settings produce
/// equal bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SnapParamsKey {
    costing: Costing,
    search_radius_bits: Option<u64>,
    turn_penalty_bits: Option<u64>,
    gps_accuracy_bits: Option<u64>,
}

impl From<SnapParams> for SnapParamsKey {
    fn from(params: SnapParams) -> Self {
        Self {
            costing: params.costing,
            search_radius_bits: params.search_radius_m.map(f64::to_bits),
            turn_penalty_bits: params.turn_penalty_factor.map(f64::to_bits),
            gps_accuracy_bits: params.gps_accuracy_override_m.map(f64::to_bits),
        }
    }
}

/// Identity of one snap result in the dedupe cache: track content, request
/// parameters, and the server host the run would go to. Host included so a
/// server change never masquerades old results as current ones - the same
/// parameters against a different server are a different run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapCacheKey {
    content: TrackContentKey,
    params: SnapParamsKey,
    host: Option<String>,
}

impl SnapCacheKey {
    pub fn new(track: &LoadedTrack, params: SnapParams, host: Option<String>) -> Self {
        Self {
            content: TrackContentKey::new(track),
            params: params.into(),
            host,
        }
    }
}

/// A completed snap run: the stitched result, the run's warnings, the host
/// it ran against, and the map-ready projection of the snapped-track
/// segments.
#[derive(Debug)]
pub struct SnapRun {
    pub result: SnapResult,
    /// Collected per run, shown in the snap status hover via
    /// [`warning_line`]; a run that produced no result at all instead
    /// surfaces through [`SnapActivity::Failed`]'s summary.
    pub warnings: Vec<SnapWarning>,
    /// Host of the server the run was sent to, for staleness against the
    /// current server setting.
    pub server_host: Option<String>,
    /// The result's map-ready geometry - projected segments plus the edge
    /// hover rows - built once at completion (on the worker thread): the
    /// map redraws every frame but a cached run never changes. Shared with
    /// the map via [`gt_ui_types::SnappedTracks`].
    pub geometry: Arc<SnappedTrackGeometry>,
}

impl SnapRun {
    pub(crate) fn new(
        result: SnapResult,
        warnings: Vec<SnapWarning>,
        server_host: Option<String>,
    ) -> Self {
        let segments = result
            .segments
            .iter()
            .map(|segment| SnappedSegment {
                points: segment
                    .positions
                    .iter()
                    .map(|p| mercator::normalize(Latitude::new(p.lat), Longitude::new(p.lon)))
                    .collect(),
                edge_spans: segment
                    .edge_spans
                    .iter()
                    .map(|span| SnappedEdgeSpan {
                        start: span.start,
                        end: span.end,
                        edge: span.edge,
                    })
                    .collect(),
            })
            .collect();
        let edges = result
            .edges
            .iter()
            .map(|edge| SnappedEdgeInfo {
                name: (!edge.names.is_empty()).then(|| edge.names.join(", ")),
                road_class: edge.road_class.map(|c| c.display_name().to_owned()),
                speed_limit_kmh: edge.speed_limit,
                surface: edge.surface.map(|s| s.display_name().to_owned()),
            })
            .collect();
        Self {
            result,
            warnings,
            server_host,
            geometry: Arc::new(SnappedTrackGeometry { segments, edges }),
        }
    }
}

/// One [`SnapWarning`] as the snap status hover shows it.
///
/// Lives app-side so the panel needs no gt-snap dependency; chunk indices
/// are shown 1-based, matching the progress display ("completed 2 of 5").
pub fn warning_line(warning: &SnapWarning) -> String {
    match warning {
        SnapWarning::ChunkFailed {
            chunk_index,
            detail,
        } => format!(
            "Chunk {} failed - its points carry no snap data ({detail})",
            chunk_index + 1
        ),
        SnapWarning::PointCountMismatch {
            chunk_index,
            sent,
            received,
        } => format!(
            "Chunk {} returned {received} points for {sent} sent - its results were discarded",
            chunk_index + 1
        ),
        SnapWarning::Geometry {
            chunk_index,
            detail,
        } => format!(
            "Chunk {} contributed no snapped-track geometry ({detail})",
            chunk_index + 1
        ),
        SnapWarning::OsmChangesetMismatch { first, later } => {
            format!("The map data updated mid-run (OSM changeset {first} to {later})")
        }
        SnapWarning::Server {
            chunk_index,
            warnings,
        } => format!(
            "The server attached {} {} to chunk {}",
            warnings.len(),
            gt_fmt::pluralize(warnings.len(), "warning", "warnings"),
            chunk_index + 1
        ),
    }
}

/// Why a run is stale: each line names one difference between how the run
/// was produced and how a fresh run would be produced now. Empty = fresh.
///
/// Compares the *configured* parameters (the gps-accuracy override, not the
/// eph-derived value actually sent) plus the server host, per the design:
/// a settings change never invalidates or re-runs anything, but the user
/// must always be able to see that the shown result predates the settings.
pub fn stale_reasons(
    run: &SnapRun,
    effective: SnapParams,
    current_host: Option<&str>,
) -> Vec<String> {
    let stored = run.result.params;
    let mut reasons = Vec::new();
    if stored.costing != effective.costing {
        reasons.push(format!(
            "Snapped as {} - would now snap as {}",
            stored.costing.display_name(),
            effective.costing.display_name()
        ));
    }
    let meters = |v: Option<f64>| match v {
        Some(v) => format!("{v} m"),
        None => "unset".to_owned(),
    };
    let plain = |v: Option<f64>| match v {
        Some(v) => v.to_string(),
        None => "unset".to_owned(),
    };
    if stored.search_radius_m != effective.search_radius_m {
        reasons.push(format!(
            "Search radius was {} - the setting is now {}",
            meters(stored.search_radius_m),
            meters(effective.search_radius_m)
        ));
    }
    if stored.turn_penalty_factor != effective.turn_penalty_factor {
        reasons.push(format!(
            "Turn penalty factor was {} - the setting is now {}",
            plain(stored.turn_penalty_factor),
            plain(effective.turn_penalty_factor)
        ));
    }
    if stored.gps_accuracy_override_m != effective.gps_accuracy_override_m {
        reasons.push(format!(
            "GPS accuracy override was {} - the setting is now {}",
            meters(stored.gps_accuracy_override_m),
            meters(effective.gps_accuracy_override_m)
        ));
    }
    if run.server_host.as_deref() != current_host {
        let name = |host: Option<&str>| host.unwrap_or("an unknown server").to_owned();
        reasons.push(format!(
            "Snapped against {} - the server is now {}",
            name(run.server_host.as_deref()),
            name(current_host)
        ));
    }
    reasons
}

/// Transient per-track state while a run is queued or in flight, or after a
/// failure. Completed runs live in the content-keyed stores instead
/// ([`SnapScheduler::latest_run_for`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapActivity {
    Queued,
    InFlight {
        completed_chunks: usize,
        total_chunks: usize,
    },
    /// The run produced no result at all (e.g. every chunk failed).
    Failed {
        error: String,
    },
}

/// A worker-to-app message.
enum SnapMessage {
    Progress {
        track: TrackRef,
        completed_chunks: usize,
        total_chunks: usize,
    },
    Done {
        track: TrackRef,
        key: SnapCacheKey,
        run: Box<SnapRun>,
    },
    Failed {
        track: TrackRef,
        error: String,
    },
}

/// Why a run entered the queue: manual triggers outrank automatic entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapPriority {
    /// A user trigger: served before any automatic entry, FIFO among
    /// manuals, and never gated on visibility - the user asked for exactly
    /// this track.
    Manual,
    /// Enqueued by auto mode: served only while its track is shown on the
    /// map, parked (not dropped) while hidden.
    Auto,
}

/// One queued request, carrying everything the worker thread needs.
struct PendingRun {
    track: TrackRef,
    priority: SnapPriority,
    key: SnapCacheKey,
    params: SnapParams,
    plan: RequestPlan,
    /// Host the run will go to, recorded on the completed [`SnapRun`] for
    /// staleness against later server changes.
    server_host: Option<String>,
}

/// Schedules snap runs - a priority queue ([`next_eligible`]) with one run
/// in flight at a time so the server's fair-use budget stays global - and
/// owns the per-track activity states and the session result stores they
/// resolve into.
pub struct SnapScheduler {
    ctx: Context,
    tx: mpsc::Sender<SnapMessage>,
    rx: mpsc::Receiver<SnapMessage>,
    /// Base URL of the matching server the next transport is built against.
    server_url: String,
    /// Shared across runs so request pacing carries over run boundaries.
    /// Lazily built; `None` until the first run (or after a build failure or
    /// a server-URL change).
    http: Option<Arc<HttpTransport>>,
    queue: VecDeque<PendingRun>,
    /// The queued tracks currently shown on the map, per
    /// [`Self::set_visibility`]. Gates which [`SnapPriority::Auto`] entries
    /// may dequeue; scoped to the queue so frame-to-frame comparison stays
    /// proportional to pending work, not to the loaded data.
    visible: HashSet<TrackRef>,
    in_flight: Option<TrackRef>,
    activity: HashMap<TrackRef, SnapActivity>,
    /// Dedupe store: every completed run this session, by content +
    /// parameters + host. Never displayed from directly.
    cache: HashMap<SnapCacheKey, Arc<SnapRun>>,
    /// Display store: the run each track currently shows. Content-keyed,
    /// so it survives index shifts like the cache does.
    latest: HashMap<TrackContentKey, Arc<SnapRun>>,
}

impl SnapScheduler {
    pub fn new(ctx: Context) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            ctx,
            tx,
            rx,
            server_url: DEFAULT_SERVER_URL.to_owned(),
            http: None,
            queue: VecDeque::new(),
            visible: HashSet::new(),
            in_flight: None,
            activity: HashMap::new(),
            cache: HashMap::new(),
            latest: HashMap::new(),
        }
    }

    /// Point future runs at a different matching server. Drops the cached
    /// transport so the next run builds against the new URL; a run already in
    /// flight finishes against the old server.
    pub fn set_server_url(&mut self, url: &str) {
        if self.server_url == url {
            return;
        }
        url.clone_into(&mut self.server_url);
        self.http = None;
    }

    /// The run a track currently displays, if any completed this session.
    pub fn latest_run_for(&self, track: &LoadedTrack) -> Option<Arc<SnapRun>> {
        self.latest.get(&TrackContentKey::new(track)).cloned()
    }

    /// The host the next run would go to - the staleness comparison side of
    /// [`SnapRun::server_host`].
    pub fn current_host(&self) -> Option<String> {
        server_host(&self.server_url)
    }

    /// The transient activity for a track (queued, in flight, or failed).
    pub fn activity_for(&self, track: TrackRef) -> Option<&SnapActivity> {
        self.activity.get(&track)
    }

    /// Whether requesting is disabled entirely (`GEOTRACE_OFFLINE`).
    pub fn offline() -> bool {
        gt_types::env::offline()
    }

    /// Queue a snap run for a track. No-ops when offline or when the track
    /// is already queued or in flight - except that a manual request for a
    /// track queued automatically promotes it to the front of the queue. A
    /// cache hit for the same content, parameters, and host promotes the
    /// cached run to the track's displayed run instead of re-requesting -
    /// switching parameters back to a known combination is instant and
    /// costs no server budget.
    pub fn request_snap(
        &mut self,
        track_ref: TrackRef,
        track: &LoadedTrack,
        params: SnapParams,
        priority: SnapPriority,
    ) {
        // The cache lookup comes before the offline gate: promotion is
        // local, and cached results stay fully usable offline.
        let key = SnapCacheKey::new(track, params, self.current_host());
        if let Some(run) = self.cache.get(&key) {
            self.latest.insert(key.content, Arc::clone(run));
            return;
        }
        if let Some(position) = self.queue.iter().position(|p| p.track == track_ref) {
            if priority == SnapPriority::Manual {
                self.promote_to_front(position);
                self.start_next_if_idle();
            }
            return;
        }
        if Self::offline() || self.activity.contains_key(&track_ref) {
            return;
        }
        let plan = request_plan::plan(&track.points);
        if plan.chunks.is_empty() {
            return;
        }
        let server_host = key.host.clone();
        self.activity.insert(track_ref, SnapActivity::Queued);
        self.queue.push_back(PendingRun {
            track: track_ref,
            priority,
            key,
            params,
            plan,
            server_host,
        });
        self.start_next_if_idle();
    }

    /// Move the queue entry at `position` to the front as a manual entry -
    /// the user singled it out, so it outranks everything still waiting.
    fn promote_to_front(&mut self, position: usize) {
        if let Some(mut entry) = self.queue.remove(position) {
            entry.priority = SnapPriority::Manual;
            self.queue.push_front(entry);
        }
    }

    /// Update which tracks are shown on the map. Called every frame; when
    /// the set changes, a parked automatic entry may have become eligible,
    /// so the worker gets a start poke.
    pub fn set_visibility(&mut self, visibility: &TrackDataVisibility) {
        let visible: HashSet<TrackRef> = self
            .queue
            .iter()
            .map(|p| p.track)
            .filter(|&t| visibility.track_shown(t))
            .collect();
        if visible != self.visible {
            self.visible = visible;
            self.start_next_if_idle();
        }
    }

    /// Seed a run restored from the recording history database into the
    /// session stores. A run completed this session wins over the stored
    /// one (it is strictly newer), so restoration never clobbers; a queued
    /// automatic entry for the track is cancelled - its answer just
    /// arrived from disk. Manual entries proceed: the user explicitly
    /// asked for a fresh run.
    pub fn restore_run(&mut self, track_ref: TrackRef, track: &LoadedTrack, run: SnapRun) {
        // Cancel first: a pending automatic fetch is answered by this run.
        if let Some(position) = self
            .queue
            .iter()
            .position(|p| p.track == track_ref && p.priority == SnapPriority::Auto)
        {
            self.queue.remove(position);
            self.activity.remove(&track_ref);
        }
        let content = TrackContentKey::new(track);
        if self.latest.contains_key(&content) {
            return;
        }
        let key = SnapCacheKey {
            content,
            params: run.result.params.into(),
            host: run.server_host.clone(),
        };
        let run = Arc::new(run);
        self.latest.insert(content, Arc::clone(&run));
        self.cache.insert(key, run);
    }

    /// Insert a completed run directly into the cache and the display
    /// store, bypassing the worker. Tests only - production runs arrive via
    /// the message channel.
    #[cfg(test)]
    pub fn insert_run(&mut self, key: SnapCacheKey, run: SnapRun) {
        let run = Arc::new(run);
        self.latest.insert(key.content, Arc::clone(&run));
        self.cache.insert(key, run);
    }

    /// Forget all transient per-track activity. Call after file/track
    /// removals shift indices; the content-keyed cache and display stores
    /// are unaffected, and an in-flight run finishes into them under its
    /// stable key.
    pub fn reset_track_states(&mut self) {
        self.queue.clear();
        self.activity.clear();
        // The in-flight run's messages must still be routed to the cache,
        // but its TrackRef-keyed activity is gone (stale by definition).
        self.in_flight = None;
    }

    /// Drain worker messages, returning the content keys of runs that
    /// completed - the caller persists their files' runs to the history
    /// database. Also starts the next queued run when idle.
    pub fn poll(&mut self) -> Vec<TrackContentKey> {
        let mut changed = false;
        let mut completed = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            changed = true;
            match message {
                SnapMessage::Progress {
                    track,
                    completed_chunks,
                    total_chunks,
                } => {
                    // Only meaningful while this track is still known; after
                    // a reset the entry is gone and progress is dropped.
                    if self.activity.contains_key(&track) {
                        self.activity.insert(
                            track,
                            SnapActivity::InFlight {
                                completed_chunks,
                                total_chunks,
                            },
                        );
                    }
                }
                SnapMessage::Done { track, key, run } => {
                    let run: Arc<SnapRun> = Arc::from(run);
                    self.latest.insert(key.content, Arc::clone(&run));
                    completed.push(key.content);
                    self.cache.insert(key, run);
                    self.activity.remove(&track);
                    if self.in_flight == Some(track) {
                        self.in_flight = None;
                    }
                }
                SnapMessage::Failed { track, error } => {
                    if self.activity.contains_key(&track) {
                        self.activity.insert(track, SnapActivity::Failed { error });
                    }
                    if self.in_flight == Some(track) {
                        self.in_flight = None;
                    }
                }
            }
        }
        if changed {
            self.start_next_if_idle();
        }
        completed
    }

    fn start_next_if_idle(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let Some(position) = next_eligible(&self.queue, &self.visible) else {
            return;
        };
        let Some(pending) = self.queue.remove(position) else {
            return;
        };
        let transport = match self.transport() {
            Ok(transport) => transport,
            Err(error) => {
                log::error!("Snap transport unavailable: {error}");
                self.activity
                    .insert(pending.track, SnapActivity::Failed { error });
                return;
            }
        };
        self.in_flight = Some(pending.track);
        self.activity.insert(
            pending.track,
            SnapActivity::InFlight {
                completed_chunks: 0,
                total_chunks: pending.plan.chunks.len(),
            },
        );
        spawn_run(self.ctx.clone(), self.tx.clone(), transport, pending);
    }

    fn transport(&mut self) -> Result<Arc<HttpTransport>, String> {
        if let Some(http) = &self.http {
            return Ok(Arc::clone(http));
        }
        let http = HttpTransport::new(&self.server_url)
            .map(Arc::new)
            .map_err(|err| format!("{err:#}"))?;
        self.http = Some(Arc::clone(&http));
        Ok(http)
    }
}

/// The queue position to run next: the oldest manual entry, else the
/// oldest automatic entry whose track is currently shown. Automatic
/// entries of hidden tracks stay parked - they neither run nor block the
/// entries behind them.
fn next_eligible(queue: &VecDeque<PendingRun>, visible: &HashSet<TrackRef>) -> Option<usize> {
    let mut first_shown_auto = None;
    for (position, pending) in queue.iter().enumerate() {
        match pending.priority {
            SnapPriority::Manual => return Some(position),
            SnapPriority::Auto => {
                if first_shown_auto.is_none() && visible.contains(&pending.track) {
                    first_shown_auto = Some(position);
                }
            }
        }
    }
    first_shown_auto
}

/// Run one snap on a worker thread, reporting progress and the final result.
#[expect(
    clippy::expect_used,
    reason = "thread spawn can only fail under extreme system resource exhaustion"
)]
fn spawn_run(
    ctx: Context,
    tx: mpsc::Sender<SnapMessage>,
    transport: Arc<HttpTransport>,
    pending: PendingRun,
) {
    let PendingRun {
        track,
        priority: _,
        key,
        params,
        plan,
        server_host,
    } = pending;
    thread::Builder::new()
        .name(format!(
            "snap-{}-{}",
            track.fi.as_usize(),
            track.index.as_usize()
        ))
        .spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let outcomes = transport::send_plan(
                transport.as_ref(),
                &plan,
                &params,
                move |completed_chunks, total_chunks| {
                    progress_tx
                        .send(SnapMessage::Progress {
                            track,
                            completed_chunks,
                            total_chunks,
                        })
                        .ok();
                    progress_ctx.request_repaint();
                },
            );
            let reporter = SnapWarningReporter::default();
            let result = stitch::stitch(&plan, params, &outcomes, &reporter);
            let message = if result.points.is_empty() && result.partial {
                // Nothing usable came back: every chunk failed.
                SnapMessage::Failed {
                    track,
                    error: run_failure_summary(&reporter),
                }
            } else {
                SnapMessage::Done {
                    track,
                    key,
                    run: Box::new(SnapRun::new(result, reporter.warnings(), server_host)),
                }
            };
            tx.send(message).ok();
            ctx.request_repaint();
        })
        .expect("failed to spawn snap worker thread");
}

/// A one-line failure summary from the run's warnings.
fn run_failure_summary(reporter: &SnapWarningReporter) -> String {
    reporter
        .warnings()
        .iter()
        .find_map(|warning| match warning {
            SnapWarning::ChunkFailed { detail, .. } => Some(detail.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "no chunk produced a result".to_owned())
}

#[cfg(test)]
mod tests {
    use gt_test_utils::fixtures;
    use gt_types::{FileIdx, TrackIdx};

    use super::*;

    fn track_ref() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    fn track(points: usize) -> LoadedTrack {
        fixtures::loaded_track_from(
            chrono::DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default(),
            points,
            1,
        )
    }

    fn scheduler() -> SnapScheduler {
        SnapScheduler::new(Context::default())
    }

    fn key(track: &LoadedTrack, params: SnapParams) -> SnapCacheKey {
        SnapCacheKey::new(track, params, server_host(DEFAULT_SERVER_URL))
    }

    fn empty_run(params: SnapParams) -> SnapRun {
        SnapRun::new(
            stitch::stitch(
                &request_plan::plan(&[]),
                params,
                &[],
                &SnapWarningReporter::default(),
            ),
            Vec::new(),
            server_host(DEFAULT_SERVER_URL),
        )
    }

    fn done_message(track: TrackRef, key: SnapCacheKey) -> SnapMessage {
        SnapMessage::Done {
            track,
            key,
            run: Box::new(empty_run(SnapParams::new(Costing::Auto))),
        }
    }

    /// Tests drive the manager through its message channel instead of real
    /// worker threads; `GEOTRACE_OFFLINE=1` (set by `just test`) would make
    /// `request_snap` refuse, so these tests inject messages directly.
    #[test]
    fn done_message_moves_run_into_cache_and_clears_activity() {
        let mut scheduler = scheduler();
        let track = track(10);
        let key = key(&track, SnapParams::new(Costing::Auto));
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);
        scheduler.in_flight = Some(track_ref());

        scheduler.tx.send(done_message(track_ref(), key)).ok();
        assert_eq!(scheduler.poll().len(), 1, "one completion reported");

        assert!(scheduler.latest_run_for(&track).is_some());
        assert_eq!(scheduler.activity_for(track_ref()), None);
        assert_eq!(scheduler.in_flight, None);
    }

    #[test]
    fn cache_key_distinguishes_params_and_host_but_not_indices() {
        let track = track(10);
        let auto = SnapParams::new(Costing::Auto);
        assert_eq!(key(&track, auto), key(&track, auto));
        assert_ne!(
            key(&track, auto),
            key(&track, SnapParams::new(Costing::Bicycle))
        );
        let tuned = SnapParams {
            search_radius_m: Some(25.0),
            ..auto
        };
        assert_ne!(key(&track, auto), key(&track, tuned));
        assert_ne!(
            key(&track, auto),
            SnapCacheKey::new(&track, auto, server_host("http://localhost:8002")),
        );
    }

    #[test]
    fn progress_after_reset_is_dropped() {
        let mut scheduler = scheduler();
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);
        scheduler.reset_track_states();

        scheduler
            .tx
            .send(SnapMessage::Progress {
                track: track_ref(),
                completed_chunks: 1,
                total_chunks: 2,
            })
            .ok();
        scheduler.poll();

        assert_eq!(scheduler.activity_for(track_ref()), None);
    }

    #[test]
    fn done_after_reset_still_lands_in_cache() {
        let mut scheduler = scheduler();
        let track = track(10);
        let key = key(&track, SnapParams::new(Costing::Auto));
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);
        scheduler.in_flight = Some(track_ref());
        scheduler.reset_track_states();

        scheduler.tx.send(done_message(track_ref(), key)).ok();
        scheduler.poll();

        assert!(
            scheduler.latest_run_for(&track).is_some(),
            "content-keyed stores survive index resets"
        );
    }

    /// The mapping from the design doc's travel-mode table: declared platform
    /// beats the configured default, road-less platforms are unsnappable, and
    /// absent or unknown declarations fall back to the configured default.
    /// Exhaustive over [`TravelMode`]: a new variant fails to compile in
    /// `resolve_costing`'s match before it can silently resolve wrongly here.
    #[rstest::rstest]
    #[case(None, Some(Costing::Pedestrian))]
    #[case(Some(TravelMode::Car), Some(Costing::Auto))]
    #[case(Some(TravelMode::Motorcycle), Some(Costing::Auto))]
    #[case(Some(TravelMode::Bicycle), Some(Costing::Bicycle))]
    #[case(Some(TravelMode::Pedestrian), Some(Costing::Pedestrian))]
    #[case(Some(TravelMode::Boat), None)]
    #[case(Some(TravelMode::Rail), None)]
    #[case(Some(TravelMode::Aircraft), None)]
    #[case(Some(TravelMode::Unknown("hovercraft".to_owned())), Some(Costing::Pedestrian))]
    fn resolve_costing_follows_the_travel_mode_table(
        #[case] declared: Option<TravelMode>,
        #[case] expected: Option<Costing>,
    ) {
        // Pedestrian as the configured default so the fallback cases are
        // distinguishable from the Car/Motorcycle mapping to Auto.
        assert_eq!(
            resolve_costing(declared.as_ref(), Costing::Pedestrian),
            expected
        );
    }

    #[test]
    fn changing_the_server_url_drops_the_cached_transport() {
        let mut scheduler = scheduler();
        scheduler.http = HttpTransport::new(DEFAULT_SERVER_URL).map(Arc::new).ok();
        assert!(scheduler.http.is_some());

        // Same URL: the shared transport (and its request pacing) is kept.
        scheduler.set_server_url(DEFAULT_SERVER_URL);
        assert!(scheduler.http.is_some());

        scheduler.set_server_url("http://localhost:8002");
        assert!(scheduler.http.is_none());
        assert_eq!(scheduler.server_url, "http://localhost:8002");
    }

    #[test]
    fn failure_message_records_failed_activity() {
        let mut scheduler = scheduler();
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);
        scheduler.in_flight = Some(track_ref());

        scheduler
            .tx
            .send(SnapMessage::Failed {
                track: track_ref(),
                error: "all chunks failed".to_owned(),
            })
            .ok();
        scheduler.poll();

        assert_eq!(
            scheduler.activity_for(track_ref()),
            Some(&SnapActivity::Failed {
                error: "all chunks failed".to_owned()
            })
        );
        assert_eq!(scheduler.in_flight, None);
    }
    /// A cache hit promotes the cached run to the track's displayed run
    /// without any queue or network activity - switching parameters back to
    /// a known combination is instant.
    #[test]
    fn cache_hit_promotes_cached_run_to_latest() {
        let mut scheduler = scheduler();
        let track = track(10);
        let auto = SnapParams::new(Costing::Auto);
        let bicycle = SnapParams::new(Costing::Bicycle);

        // Two completed runs for the same track under different params.
        scheduler.insert_run(key(&track, auto), empty_run(auto));
        scheduler.insert_run(key(&track, bicycle), empty_run(bicycle));
        assert_eq!(
            scheduler
                .latest_run_for(&track)
                .map(|r| r.result.params.costing),
            Some(Costing::Bicycle),
            "the most recently inserted run displays"
        );

        // Requesting auto again: cache hit, promoted, nothing queued.
        // (request_snap's offline gate sits after the cache lookup, so this
        // path is exercised even under GEOTRACE_OFFLINE.)
        scheduler.request_snap(track_ref(), &track, auto, SnapPriority::Manual);
        assert_eq!(
            scheduler
                .latest_run_for(&track)
                .map(|r| r.result.params.costing),
            Some(Costing::Auto),
        );
        assert!(scheduler.queue.is_empty());
        assert_eq!(scheduler.activity_for(track_ref()), None);
    }

    /// Staleness reasons: each differing component contributes one line
    /// naming the stored and the current value; identical runs are fresh.
    #[rstest::rstest]
    #[case::fresh(SnapParams::new(Costing::Auto), server_host(DEFAULT_SERVER_URL), &[])]
    #[case::costing_differs(
        SnapParams::new(Costing::Bicycle),
        server_host(DEFAULT_SERVER_URL),
        &["Snapped as Auto - would now snap as Bicycle"],
    )]
    #[case::search_radius_differs(
        SnapParams { search_radius_m: Some(25.0), ..SnapParams::new(Costing::Auto) },
        server_host(DEFAULT_SERVER_URL),
        &["Search radius was unset - the setting is now 25 m"],
    )]
    #[case::turn_penalty_differs(
        SnapParams { turn_penalty_factor: Some(300.0), ..SnapParams::new(Costing::Auto) },
        server_host(DEFAULT_SERVER_URL),
        &["Turn penalty factor was unset - the setting is now 300"],
    )]
    #[case::gps_accuracy_differs(
        SnapParams { gps_accuracy_override_m: Some(10.0), ..SnapParams::new(Costing::Auto) },
        server_host(DEFAULT_SERVER_URL),
        &["GPS accuracy override was unset - the setting is now 10 m"],
    )]
    #[case::host_differs(
        SnapParams::new(Costing::Auto),
        server_host("http://localhost:8002"),
        &["Snapped against valhalla1.openstreetmap.de - the server is now localhost"],
    )]
    #[case::multiple_differences(
        SnapParams { search_radius_m: Some(25.0), ..SnapParams::new(Costing::Bicycle) },
        server_host(DEFAULT_SERVER_URL),
        &[
            "Snapped as Auto - would now snap as Bicycle",
            "Search radius was unset - the setting is now 25 m",
        ],
    )]
    fn stale_reasons_name_each_difference(
        #[case] effective: SnapParams,
        #[case] current_host: Option<String>,
        #[case] expected: &[&str],
    ) {
        let run = empty_run(SnapParams::new(Costing::Auto));
        assert_eq!(
            stale_reasons(&run, effective, current_host.as_deref()),
            expected,
        );
    }

    fn pending(track: &LoadedTrack, track_ref: TrackRef, priority: SnapPriority) -> PendingRun {
        PendingRun {
            track: track_ref,
            priority,
            key: key(track, SnapParams::new(Costing::Auto)),
            params: SnapParams::new(Costing::Auto),
            plan: request_plan::plan(&[]),
            server_host: None,
        }
    }

    fn nth_track_ref(n: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(n), TrackIdx::new(0))
    }

    /// Dequeue order over (priority, shown-on-map) queue configurations:
    /// the oldest manual wins regardless of visibility, hidden automatic
    /// entries park without blocking the entries behind them.
    #[rstest::rstest]
    #[case::empty(&[], None)]
    #[case::manual_fifo(&[(SnapPriority::Manual, true), (SnapPriority::Manual, true)], Some(0))]
    #[case::manual_outranks_earlier_auto(&[(SnapPriority::Auto, true), (SnapPriority::Manual, false)], Some(1))]
    #[case::hidden_auto_parks(&[(SnapPriority::Auto, false)], None)]
    #[case::hidden_auto_does_not_block(&[(SnapPriority::Auto, false), (SnapPriority::Auto, true)], Some(1))]
    #[case::shown_auto_fifo(&[(SnapPriority::Auto, true), (SnapPriority::Auto, true)], Some(0))]
    fn next_eligible_orders_by_priority_and_visibility(
        #[case] entries: &[(SnapPriority, bool)],
        #[case] expected: Option<usize>,
    ) {
        let track = track(10);
        let mut queue = VecDeque::new();
        let mut visible = HashSet::new();
        for (n, &(priority, shown)) in entries.iter().enumerate() {
            let track_ref = nth_track_ref(n);
            if shown {
                visible.insert(track_ref);
            }
            queue.push_back(pending(&track, track_ref, priority));
        }
        assert_eq!(next_eligible(&queue, &visible), expected);
    }

    /// A manual request for a track already queued - automatically or
    /// manually - promotes it to the front as a manual entry; an automatic
    /// re-request leaves the queue untouched. Nothing duplicates entries.
    #[test]
    fn manual_request_promotes_queued_auto_entry() {
        let mut scheduler = scheduler();
        // A sentinel in-flight run keeps the test from spawning a worker.
        scheduler.in_flight = Some(nth_track_ref(9));
        let track = track(10);
        let (a, b) = (nth_track_ref(0), nth_track_ref(1));
        scheduler
            .queue
            .push_back(pending(&track, a, SnapPriority::Auto));
        scheduler
            .queue
            .push_back(pending(&track, b, SnapPriority::Auto));

        scheduler.request_snap(
            b,
            &track,
            SnapParams::new(Costing::Auto),
            SnapPriority::Auto,
        );
        assert_eq!(
            scheduler.queue.front().map(|p| p.track),
            Some(a),
            "an automatic re-request must not reorder the queue"
        );

        scheduler.request_snap(
            b,
            &track,
            SnapParams::new(Costing::Auto),
            SnapPriority::Manual,
        );
        assert_eq!(
            scheduler.queue.front().map(|p| (p.track, p.priority)),
            Some((b, SnapPriority::Manual)),
        );
        assert_eq!(scheduler.queue.len(), 2, "promotion must not duplicate");

        // Promote `a` past the already-manual `b`, leaving `b` manual and
        // non-front; a second manual click on `b` still moves it to the
        // front without duplicating - the last click wins the queue order.
        scheduler.request_snap(
            a,
            &track,
            SnapParams::new(Costing::Auto),
            SnapPriority::Manual,
        );
        assert_eq!(
            scheduler.queue.front().map(|p| (p.track, p.priority)),
            Some((a, SnapPriority::Manual)),
        );
        scheduler.request_snap(
            b,
            &track,
            SnapParams::new(Costing::Auto),
            SnapPriority::Manual,
        );
        assert_eq!(
            scheduler.queue.front().map(|p| (p.track, p.priority)),
            Some((b, SnapPriority::Manual)),
        );
        assert_eq!(scheduler.queue.len(), 2, "re-promotion must not duplicate");
    }

    /// Hiding a queued track parks its automatic entry; showing it again
    /// makes it eligible - visibility gates dequeueing, never drops work.
    #[test]
    fn visibility_change_parks_and_unparks_auto_entries() {
        use gt_ui_types::{FileVisibility, TrackVisibility};

        let mut scheduler = scheduler();
        scheduler.in_flight = Some(nth_track_ref(9));
        let track = track(10);
        scheduler
            .queue
            .push_back(pending(&track, nth_track_ref(0), SnapPriority::Auto));

        let mut vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![TrackVisibility::all_visible()],
            }],
        };
        vis.files[0].tracks[0].track_visible = false;
        scheduler.set_visibility(&vis);
        assert_eq!(next_eligible(&scheduler.queue, &scheduler.visible), None);

        vis.files[0].tracks[0].track_visible = true;
        scheduler.set_visibility(&vis);
        assert_eq!(next_eligible(&scheduler.queue, &scheduler.visible), Some(0));
    }

    /// A restored run seeds the display and dedupe stores and cancels a
    /// pending automatic entry (its answer just arrived from disk); manual
    /// entries survive, and a session run is never clobbered.
    #[test]
    fn restored_run_seeds_stores_and_cancels_auto_entry() {
        let mut scheduler = scheduler();
        scheduler.in_flight = Some(nth_track_ref(9));
        let track = track(10);

        scheduler
            .queue
            .push_back(pending(&track, track_ref(), SnapPriority::Auto));
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);

        scheduler.restore_run(
            track_ref(),
            &track,
            empty_run(SnapParams::new(Costing::Auto)),
        );
        assert!(scheduler.latest_run_for(&track).is_some());
        assert!(scheduler.queue.is_empty(), "the parked answer arrived");
        assert_eq!(scheduler.activity_for(track_ref()), None);

        // A second restore (or an older stored run) never clobbers.
        scheduler.restore_run(
            track_ref(),
            &track,
            empty_run(SnapParams::new(Costing::Bicycle)),
        );
        assert_eq!(
            scheduler
                .latest_run_for(&track)
                .map(|r| r.result.params.costing),
            Some(Costing::Auto),
        );
    }

    /// A manual entry outlives restoration: the user explicitly asked for
    /// a fresh run, so the stored one only fills the display until then.
    #[test]
    fn restore_keeps_manual_entries_queued() {
        let mut scheduler = scheduler();
        scheduler.in_flight = Some(nth_track_ref(9));
        let track = track(10);
        scheduler
            .queue
            .push_back(pending(&track, track_ref(), SnapPriority::Manual));

        scheduler.restore_run(
            track_ref(),
            &track,
            empty_run(SnapParams::new(Costing::Auto)),
        );

        assert!(scheduler.latest_run_for(&track).is_some());
        assert_eq!(scheduler.queue.len(), 1, "the manual request still runs");
    }

    /// Every warning variant renders to a hover line carrying its key facts
    /// (1-based chunk numbers, counts, the failure detail). The table length
    /// is pinned to the enum so a new variant fails here instead of
    /// silently shipping without a rendering.
    #[test]
    fn warning_lines_cover_every_variant() {
        use strum::EnumCount;

        let cases = [
            (
                SnapWarning::ChunkFailed {
                    chunk_index: 2,
                    detail: "HTTP 502".to_owned(),
                },
                "Chunk 3 failed - its points carry no snap data (HTTP 502)",
            ),
            (
                SnapWarning::PointCountMismatch {
                    chunk_index: 0,
                    sent: 1000,
                    received: 998,
                },
                "Chunk 1 returned 998 points for 1000 sent - its results were discarded",
            ),
            (
                SnapWarning::Geometry {
                    chunk_index: 1,
                    detail: "edge 7 carries no shape index range".to_owned(),
                },
                "Chunk 2 contributed no snapped-track geometry (edge 7 carries no shape index range)",
            ),
            (
                SnapWarning::OsmChangesetMismatch {
                    first: 100,
                    later: 200,
                },
                "The map data updated mid-run (OSM changeset 100 to 200)",
            ),
            (
                SnapWarning::Server {
                    chunk_index: 4,
                    warnings: vec![serde_json::json!({"code": 1})],
                },
                "The server attached 1 warning to chunk 5",
            ),
        ];
        assert_eq!(cases.len(), SnapWarning::COUNT);
        for (warning, expected) in cases {
            assert_eq!(warning_line(&warning), expected);
        }
    }
}
