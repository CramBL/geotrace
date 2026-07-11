//! Snap-to-road worker and per-track state store.
//!
//! Follows the file loader's channel shape ([`super::loader`]): owned by the
//! app, background threads reporting over an mpsc channel, `request_repaint`
//! on every message so results appear without user input.
//!
//! Snapping is manual-only for now: tracks enter a FIFO queue via
//! [`SnapScheduler::request_snap`], and one run is in flight at a time so the
//! server's fair-use budget is shared globally (the transport also paces
//! individual requests). An automatic mode with a visibility-driven priority
//! queue is planned future work.
//!
//! Results are cached per [`SnapCacheKey`] - track content fingerprint plus
//! request parameters - so a result survives file removals and index shifts,
//! and re-requesting a snapped track is a cache hit, never a request.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use egui::Context;

use gt_snap::request_plan::{self, RequestPlan};
use gt_snap::stitch::{self, SnapResult, SnapWarning, SnapWarningReporter};
use gt_snap::transport::HttpTransport;
use gt_snap::wire::Costing;
use gt_snap::{DEFAULT_SERVER_URL, transport};
use gt_types::{LoadedTrack, TrackRef, TravelMode};

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

/// Content fingerprint of a track plus the request parameters, identifying a
/// snap result independent of file/track indices (which shift on removal).
/// Tracks are immutable once loaded, so time range + point count pin the
/// content for session-cache purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapCacheKey {
    start_us: i64,
    end_us: i64,
    tpv_count: usize,
    costing: Costing,
}

impl SnapCacheKey {
    pub fn new(track: &LoadedTrack, costing: Costing) -> Self {
        let range = track.metadata.time_range;
        Self {
            start_us: range.start.timestamp_micros(),
            end_us: range.end.timestamp_micros(),
            tpv_count: track.metadata.tpv_count,
            costing,
        }
    }
}

/// A completed snap run: the stitched result plus the run's warnings.
#[derive(Debug)]
pub struct SnapRun {
    pub result: SnapResult,
    /// Consumed once the snapped-track renderer and the snap error plot land.
    #[expect(
        dead_code,
        reason = "consumers land with the snapped-track renderer and the snap error plot"
    )]
    pub warnings: Vec<SnapWarning>,
}

/// Transient per-track state while a run is queued or in flight, or after a
/// failure. Completed runs live in the cache instead ([`SnapScheduler::run_for`]).
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

/// One queued request, carrying everything the worker thread needs.
struct PendingRun {
    track: TrackRef,
    key: SnapCacheKey,
    plan: RequestPlan,
}

/// Schedules snap runs - FIFO queue, one in flight at a time so the server's
/// fair-use budget stays global - and owns the per-track activity states and
/// the session result cache they resolve into.
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
    in_flight: Option<TrackRef>,
    activity: HashMap<TrackRef, SnapActivity>,
    cache: HashMap<SnapCacheKey, Arc<SnapRun>>,
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
            in_flight: None,
            activity: HashMap::new(),
            cache: HashMap::new(),
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

    /// The cached run for a track under the given costing, if one completed
    /// this session.
    pub fn run_for(&self, track: &LoadedTrack, costing: Costing) -> Option<Arc<SnapRun>> {
        self.cache.get(&SnapCacheKey::new(track, costing)).cloned()
    }

    /// The transient activity for a track (queued, in flight, or failed).
    pub fn activity_for(&self, track: TrackRef) -> Option<&SnapActivity> {
        self.activity.get(&track)
    }

    /// Whether requesting is disabled entirely (`GEOTRACE_OFFLINE`).
    pub fn offline() -> bool {
        gt_types::env::offline()
    }

    /// Queue a snap run for a track. No-ops when offline, when the result is
    /// already cached, or when the track is already queued or in flight.
    pub fn request_snap(&mut self, track_ref: TrackRef, track: &LoadedTrack, costing: Costing) {
        if Self::offline() {
            return;
        }
        let key = SnapCacheKey::new(track, costing);
        if self.cache.contains_key(&key) || self.activity.contains_key(&track_ref) {
            return;
        }
        let plan = request_plan::plan(&track.points);
        if plan.chunks.is_empty() {
            return;
        }
        self.activity.insert(track_ref, SnapActivity::Queued);
        self.queue.push_back(PendingRun {
            track: track_ref,
            key,
            plan,
        });
        self.start_next_if_idle();
    }

    /// Forget all transient per-track activity. Call after file/track
    /// removals shift indices; the content-keyed cache is unaffected, and an
    /// in-flight run finishes into the cache under its stable key.
    pub fn reset_track_states(&mut self) {
        self.queue.clear();
        self.activity.clear();
        // The in-flight run's messages must still be routed to the cache,
        // but its TrackRef-keyed activity is gone (stale by definition).
        self.in_flight = None;
    }

    /// Drain worker messages; returns `true` when anything changed (the
    /// caller repaints). Also starts the next queued run when idle.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
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
                    self.cache.insert(key, Arc::from(run));
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
        changed
    }

    fn start_next_if_idle(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let Some(pending) = self.queue.pop_front() else {
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
    let PendingRun { track, key, plan } = pending;
    let costing = key.costing;
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
                costing,
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
            let result = stitch::stitch(&plan, costing, &outcomes, &reporter);
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
                    run: Box::new(SnapRun {
                        result,
                        warnings: reporter.warnings(),
                    }),
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
        let points = fixtures::nav_points_from(
            chrono::DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default(),
            points,
            1,
        );
        LoadedTrack {
            metadata: gt_types::track::TrackMetadata {
                time_range: gt_types::track::TimeRange::new(
                    points
                        .first()
                        .map(|p| p.tpv.time().utc())
                        .unwrap_or_default(),
                    points
                        .last()
                        .map(|p| p.tpv.time().utc())
                        .unwrap_or_default(),
                ),
                tpv_count: points.len(),
                ..gt_types::track::TrackMetadata::default()
            },
            points,
            lod: gt_types::track::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: Vec::new(),
            generated_markers: Vec::new(),
            event_markers: Vec::new(),
            channels: Vec::new(),
        }
    }

    fn scheduler() -> SnapScheduler {
        SnapScheduler::new(Context::default())
    }

    fn done_message(track: TrackRef, key: SnapCacheKey) -> SnapMessage {
        SnapMessage::Done {
            track,
            key,
            run: Box::new(SnapRun {
                result: stitch::stitch(
                    &request_plan::plan(&[]),
                    Costing::Auto,
                    &[],
                    &SnapWarningReporter::default(),
                ),
                warnings: Vec::new(),
            }),
        }
    }

    /// Tests drive the manager through its message channel instead of real
    /// worker threads; `GEOTRACE_OFFLINE=1` (set by `just test`) would make
    /// `request_snap` refuse, so these tests inject messages directly.
    #[test]
    fn done_message_moves_run_into_cache_and_clears_activity() {
        let mut scheduler = scheduler();
        let track = track(10);
        let key = SnapCacheKey::new(&track, Costing::Auto);
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);
        scheduler.in_flight = Some(track_ref());

        scheduler.tx.send(done_message(track_ref(), key)).ok();
        assert!(scheduler.poll());

        assert!(scheduler.run_for(&track, Costing::Auto).is_some());
        assert_eq!(scheduler.activity_for(track_ref()), None);
        assert_eq!(scheduler.in_flight, None);
    }

    #[test]
    fn cache_key_distinguishes_costing_but_not_indices() {
        let track = track(10);
        assert_eq!(
            SnapCacheKey::new(&track, Costing::Auto),
            SnapCacheKey::new(&track, Costing::Auto),
        );
        assert_ne!(
            SnapCacheKey::new(&track, Costing::Auto),
            SnapCacheKey::new(&track, Costing::Bicycle),
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
        let key = SnapCacheKey::new(&track, Costing::Auto);
        scheduler.activity.insert(track_ref(), SnapActivity::Queued);
        scheduler.in_flight = Some(track_ref());
        scheduler.reset_track_states();

        scheduler.tx.send(done_message(track_ref(), key)).ok();
        scheduler.poll();

        assert!(
            scheduler.run_for(&track, Costing::Auto).is_some(),
            "content-keyed cache survives index resets"
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
}
