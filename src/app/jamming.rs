//! Interference fetch worker and archive ingest.
//!
//! Follows [`super::snap`]: owned by the app, a background thread per
//! request reporting over an mpsc channel, `request_repaint` on every
//! message.
//!
//! Loading a track queues the UTC days it spans. A day already in the
//! archive is never requested, so the queue shrinks to nothing as the
//! archive fills. One request is in flight at a time.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{NaiveDate, NaiveTime, TimeDelta, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, Transport, TransportSource};
use gt_jam::calendar::{self, DayOutlook};
use gt_jam::dataset::JamDataset;
use gt_jam::day_selection::{DaySelection, EmptyReason};
use gt_jam::transport::{self, FetchOutcome, REQUEST_INTERVAL};
use gt_jam::wire::{self, ParseWarningReporter};
use gt_pending_writes::PendingWrites;
use gt_query_run::JammingValues;
#[cfg(test)]
use gt_store::Store;
use gt_store::{ArchiveUsage, JamStore};
use gt_types::TimeRange;
use gt_types::{LoadedFile, TrackRef};
use gt_ui_types::{ArcIdentity, JammingContextSample, JammingPoint, JammingSeries};

use super::context_line::{ContextSampleCache, ContextSource, ContextSpan, midnight_secs};
use super::day_fetch_queue::DayFetchQueue;
use super::environment_storage::{EnvironmentArchive, PrunedDays};
use super::fix_positions::FixPositionTimeline;

/// What one day's fetch produced.
enum JamMessage {
    Stored {
        day: NaiveDate,
        cells: usize,
    },
    /// The host has no dataset for the day.
    Missing {
        day: NaiveDate,
        pending: bool,
    },
    Failed {
        day: NaiveDate,
        detail: String,
    },
    /// The day was downloaded, then discarded unarchived because the process
    /// is shutting down.
    NotArchivedDuringShutdown {
        day: NaiveDate,
    },
}

impl JamMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. }
            | Self::Missing { day, .. }
            | Self::Failed { day, .. }
            | Self::NotArchivedDuringShutdown { day } => day,
        }
    }
}

/// One track's interference, resolved from a set of archived days.
struct ResolvedTrackInterference {
    /// The archived days these values came from, as the cache key: a track's
    /// values change exactly when this set does.
    archived_days: Vec<NaiveDate>,
    plot_points: Arc<Vec<JammingPoint>>,
    /// Absent when the archive valued none of the track's fixes, which is
    /// also when the plot draws no line for the track.
    query_values: Option<Arc<Vec<Option<f64>>>>,
}

impl ResolvedTrackInterference {
    /// The points the plot draws for the track, absent when the archive
    /// valued none of its fixes.
    fn valued_plot_points(&self) -> Option<Arc<Vec<JammingPoint>>> {
        self.query_values
            .is_some()
            .then(|| Arc::clone(&self.plot_points))
    }

    /// One point per fix, valued from the dataset of the fix's own day.
    fn resolve(
        archived_days: Vec<NaiveDate>,
        store: Option<&JamStore>,
        datasets: &mut HashMap<NaiveDate, Option<JamDataset>>,
        track: &gt_types::LoadedTrack,
    ) -> Self {
        let plot_points: Vec<JammingPoint> = track
            .points
            .iter()
            .map(|point| {
                let time = point.tpv.time().utc();
                let day = time.date_naive();
                let dataset = datasets.entry(day).or_insert_with(|| {
                    store.and_then(|store| {
                        store
                            .dataset(day)
                            .inspect_err(|err| {
                                log::error!("Reading interference cells for {day}: {err}");
                            })
                            .ok()
                            .flatten()
                    })
                });
                let observation = dataset.as_ref().and_then(|dataset| {
                    let (lat, lon) = (point.tpv.lat(), point.tpv.lon());
                    dataset.observation_at(lat, lon)
                });
                let rate = observation.and_then(gt_jam::wire::HexObservation::rate);
                JammingPoint {
                    x_secs: time.timestamp() as f64,
                    percent: rate.map(gt_jam::wire::InterferenceRate::percent),
                    aircraft: rate.map_or(0, |rate| rate.aircraft),
                    bad: observation.map_or(0, |observation| observation.bad),
                }
            })
            .collect();
        let query_values = plot_points
            .iter()
            .any(|point| point.percent.is_some())
            .then(|| {
                Arc::new(
                    plot_points
                        .iter()
                        .map(|point| point.percent)
                        .collect::<Vec<_>>(),
                )
            });
        Self {
            archived_days,
            plot_points: Arc::new(plot_points),
            query_values,
        }
    }
}

/// Queues interference days and ingests them into the archive.
pub struct JammingScheduler {
    ctx: Context,
    tx: mpsc::Sender<JamMessage>,
    rx: mpsc::Receiver<JamMessage>,
    base_url: String,
    /// `None` disables fetching: no archive to write to.
    store: Option<Arc<JamStore>>,
    /// Connected on the first request, and dropped when the host changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here determines whether requests may leave the machine.
    transport_source: TransportSource,
    days: DayFetchQueue,
    /// Cells archived per day, read once at startup and updated on ingest,
    /// so the display toggle does not open the archive per frame. Ordered so
    /// the days a plot span holds are a range query. Assumes this process is
    /// the archive's only writer.
    archived_cells: BTreeMap<NaiveDate, u32>,
    /// The day the overlay draws, and its cells, loaded from the archive on
    /// demand and kept until the shown day changes. A day is never
    /// re-ingested - `insert_day` refuses one already stored - so a loaded
    /// day cannot go out of date.
    shown: Option<(NaiveDate, JamDataset)>,
    /// Which day the overlay shows, and the stepper's bounds.
    selection: DaySelection,
    /// Days the host has no dataset for, which the legend distinguishes from
    /// a day nothing was downloaded for.
    refused: HashSet<NaiveDate>,
    /// Per-track resolved interference. The `Arc` identities the plot and the
    /// query fingerprint cache on hold across frames: a track's values are
    /// rebuilt only when the archive gains a day it spans.
    interference_by_track: HashMap<TrackRef, ResolvedTrackInterference>,
    /// The line drawn across the plot's whole span, one sample per archived
    /// day.
    context: ContextSampleCache<JammingContextSample>,
    /// When the last request was handed to a worker, so [`REQUEST_INTERVAL`]
    /// is honoured across days.
    last_request: Option<Instant>,
    /// Registers every archive insert, and refuses the ones that would start
    /// after shutdown began.
    pending_writes: PendingWrites,
}

impl JammingScheduler {
    pub fn new(
        ctx: Context,
        store: Option<Arc<JamStore>>,
        base_url: String,
        transport_source: TransportSource,
        pending_writes: PendingWrites,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let archived_cells = store
            .as_ref()
            .map(|store| store.days())
            .transpose()
            .inspect_err(|err| log::error!("Reading the interference archive index: {err}"))
            .ok()
            .flatten()
            .into_iter()
            .flatten()
            .map(|stored| (stored.day, stored.cells))
            .collect();
        Self {
            archived_cells,
            shown: None,
            selection: DaySelection::new(None, calendar::today_utc()),
            refused: HashSet::new(),
            interference_by_track: HashMap::new(),
            context: ContextSampleCache::default(),
            ctx,
            tx,
            rx,
            base_url,
            store,
            http: None,
            transport_source,
            days: DayFetchQueue::default(),
            last_request: None,
            pending_writes,
        }
    }

    /// A scheduler with no archive to write to, so it fetches nothing.
    #[cfg(test)]
    fn disabled(ctx: Context) -> Self {
        Self::new(
            ctx,
            None,
            gt_jam::DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
            PendingWrites::default(),
        )
    }

    /// Queue the days a recording spans.
    ///
    /// Days outside the coverage window, already archived, or already
    /// queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    pub fn request_days_for(&mut self, range: TimeRange) {
        let Some(store) = self.store.as_ref().map(Arc::clone) else {
            return;
        };
        let Some(days) = calendar::days_spanned(range.start, range.end) else {
            log::info!(
                "A recording spanning {} is past the {}-day limit; no interference days queued",
                range.duration(),
                calendar::MAX_DAYS_PER_TRACK
            );
            return;
        };
        let today = calendar::today_utc();
        if let Some(first) = days.first() {
            self.selection.adopt_default(*first);
        }
        for day in days {
            if calendar::day_outlook(day, today) != DayOutlook::Fetchable {
                continue;
            }
            let needs_fetch = store.contains(day).map(|archived| !archived);
            self.days.request_recording_day(day, needs_fetch);
        }
        self.start_next();
    }

    /// Queue every unarchived day in `from..=to`, as one backfill.
    ///
    /// Returns how many days were queued, or [`None`] when there is no
    /// archive to write them to.
    pub fn backfill(&mut self, from: NaiveDate, to: NaiveDate) -> Option<usize> {
        let store = self.store.as_ref().map(Arc::clone)?;
        let total = self.days.start_backfill(
            calendar::fetchable_days(from, to, calendar::today_utc()),
            |day| store.contains(day).map(|archived| !archived),
        );
        log::info!("Backfilling interference for {total} days between {from} and {to}");
        self.start_next();
        Some(total)
    }

    /// Whether there is an archive to download into. Grays the backfill
    /// control when there is not.
    pub fn archive_available(&self) -> bool {
        self.store.is_some()
    }

    /// The archive, for the settings page to report and delete from.
    pub fn archive(&self) -> Option<Arc<JamStore>> {
        self.store.as_ref().map(Arc::clone)
    }

    /// What the archive holds, as the environment storage rows show it.
    pub fn archive_usage(&self) -> Option<ArchiveUsage> {
        let store = self.store.as_ref()?;
        Some(ArchiveUsage::measure(
            store.path(),
            self.archived_cells.keys().copied(),
        ))
    }

    /// How many archived days a delete of `pruned` would remove.
    pub fn archived_days_covered(&self, pruned: PrunedDays) -> usize {
        pruned.count_covered(self.archived_cells.keys().copied())
    }

    /// Drop what this scheduler holds for the days a delete removed from the
    /// archive.
    pub fn forget_pruned_days(&mut self, pruned: PrunedDays) {
        self.archived_cells.retain(|day, _| !pruned.covers(*day));
        self.interference_by_track
            .retain(|_, resolved| !resolved.archived_days.iter().any(|day| pruned.covers(*day)));
        self.refused.retain(|day| !pruned.covers(*day));
        self.context.forget_pruned_days(pruned);
        self.days.forget_pruned_days(pruned);
    }

    /// The queued, in-flight and failed days, as the settings page reports
    /// them and the download control drives them.
    pub fn fetch_queue(&self) -> &DayFetchQueue {
        &self.days
    }

    pub fn fetch_queue_mut(&mut self) -> &mut DayFetchQueue {
        &mut self.days
    }

    /// Apply finished fetches and start the next queued day.
    pub fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.days.finish_day(message.day());
            match message {
                JamMessage::Stored { day, cells } => {
                    log::info!("Archived {cells} interference cells for {day}");
                    self.archived_cells
                        .insert(day, u32::try_from(cells).unwrap_or(u32::MAX));
                    self.days.mark_archived(day);
                    self.context.forget(day);
                }
                JamMessage::Missing { day, pending } => {
                    log::info!(
                        "No interference data published for {day}{}",
                        if pending { " yet" } else { "" }
                    );
                    self.refused.insert(day);
                }
                JamMessage::Failed { day, detail } => {
                    log::error!("No interference data archived for {day}: {detail}");
                    self.days.report_failure(day, detail);
                }
                JamMessage::NotArchivedDuringShutdown { day } => {
                    log::debug!("No interference data archived for {day}: shutting down");
                }
            }
        }
        self.start_next();
    }

    /// Why the overlay is drawing nothing, or [`None`] when it has cells.
    pub fn empty_reason(&self) -> Option<EmptyReason> {
        let archived = self
            .selection
            .day()
            .map_or(0, |day| self.archived_cells(day));
        let refused = self
            .selection
            .day()
            .is_some_and(|day| self.refused.contains(&day));
        self.selection.empty_reason(archived, refused)
    }

    /// Cells archived for `day`.
    fn archived_cells(&self, day: NaiveDate) -> usize {
        self.archived_cells
            .get(&day)
            .map_or(0, |&cells| cells as usize)
    }

    /// Interference values for the plot: one point per fix of every loaded
    /// track, from that fix's own UTC day.
    ///
    /// A track's points are rebuilt only when the archive gains one of the
    /// days it spans, so the `Arc` the plot caches on stays stable.
    pub fn plot_series(&mut self, files: &[LoadedFile]) -> JammingSeries {
        let mut series = JammingSeries::default();
        let mut live: HashSet<TrackRef> = HashSet::new();
        // Shared across tracks: a batch of recordings from one trip all read
        // the same day.
        let mut datasets: HashMap<NaiveDate, Option<JamDataset>> = HashMap::new();

        for (fi, file) in files.iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref =
                    TrackRef::new(gt_types::FileIdx::new(fi), gt_types::TrackIdx::new(ti));
                live.insert(track_ref);

                let archived_days = self.days_available_for(track);
                let cached = self
                    .interference_by_track
                    .get(&track_ref)
                    .filter(|resolved| resolved.archived_days == archived_days);
                let points = match cached {
                    Some(resolved) => resolved.valued_plot_points(),
                    None => {
                        let resolved = ResolvedTrackInterference::resolve(
                            archived_days,
                            self.store.as_deref(),
                            &mut datasets,
                            track,
                        );
                        let points = resolved.valued_plot_points();
                        self.interference_by_track.insert(track_ref, resolved);
                        points
                    }
                };
                if let Some(points) = points {
                    series.points_by_track.insert(track_ref, points);
                }
            }
        }
        self.interference_by_track
            .retain(|track, _| live.contains(track));
        series
    }

    /// The interference line across `span`: one sample per archived UTC day,
    /// read at the cell the receiver was in nearest that day's midpoint in
    /// time.
    ///
    /// Days the archive holds nothing for break the line, so what it draws is
    /// what has been downloaded.
    pub fn context_line(
        &mut self,
        span: ContextSpan,
        positions: &Arc<FixPositionTimeline>,
    ) -> Arc<Vec<JammingContextSample>> {
        let source = ContextSource {
            span,
            archived_days: self.archived_days_in(span),
            positions: Some(ArcIdentity::of(positions)),
        };
        let store = self.store.as_ref().map(Arc::clone);
        let positions = Arc::clone(positions);
        self.context.resolve(
            source,
            |day| context_day(store.as_deref(), &positions, day),
            |day| {
                Some(JammingContextSample {
                    start_secs: midnight_secs(day),
                    percent: None,
                    aircraft: 0,
                    bad: 0,
                })
            },
        )
    }

    /// The days inside `span` the archive holds cells for, oldest first.
    fn archived_days_in(&self, span: ContextSpan) -> Vec<NaiveDate> {
        self.archived_cells
            .range(span.days())
            .filter(|&(_, &cells)| cells > 0)
            .map(|(&day, _)| day)
            .collect()
    }

    /// Dense per-fix interference percentages for the query providers, one
    /// entry per track the archive valued when [`Self::plot_series`] last
    /// resolved it.
    ///
    /// The `Arc` identities hold until the archive gains a day a track spans:
    /// the query fingerprint compares them to tell whether a run's results
    /// still describe the data on display. Shaped like the snap-error values
    /// so both reach the provider the same way.
    pub fn query_values(&self) -> JammingValues {
        self.interference_by_track
            .iter()
            .filter_map(|(&track, resolved)| {
                Some((track, Arc::clone(resolved.query_values.as_ref()?)))
            })
            .collect()
    }

    /// The archived days the track's fixes fall in, as the cache key: a
    /// track's points change exactly when this set does.
    fn days_available_for(&self, track: &gt_types::LoadedTrack) -> Vec<NaiveDate> {
        let range = track.metadata.time_range;
        calendar::days_spanned(range.start, range.end)
            .unwrap_or_default()
            .into_iter()
            .filter(|day| self.archived_cells(*day) > 0)
            .collect()
    }

    /// The selected day's cells and the day selection, borrowed apart so the
    /// map can draw one while the stepper mutates the other. Loads the day
    /// from the archive the first time it is shown.
    pub fn overlay_state(&mut self) -> (Option<&JamDataset>, &mut DaySelection) {
        let day = self.selection.day();
        if self
            .shown
            .as_ref()
            .is_none_or(|(shown, _)| Some(*shown) != day)
        {
            self.shown = day.and_then(|day| self.load_dataset(day).map(|dataset| (day, dataset)));
        }
        let dataset = self
            .shown
            .as_ref()
            .filter(|(shown, _)| Some(*shown) == day)
            .map(|(_, dataset)| dataset);
        (dataset, &mut self.selection)
    }

    fn load_dataset(&self, day: NaiveDate) -> Option<JamDataset> {
        if self
            .archived_cells
            .get(&day)
            .is_none_or(|&cells| cells == 0)
        {
            return None;
        }
        self.store
            .as_deref()?
            .dataset(day)
            .inspect_err(|err| log::error!("Reading interference cells for {day}: {err}"))
            .ok()
            .flatten()
    }

    /// Point the scheduler at `base_url`.
    ///
    /// A changed host drops the queue, the days requested of the old host,
    /// its refusals, its failures and the running backfill. `archived_cells`
    /// is kept - a day already archived does not depend on which host served
    /// it.
    pub fn set_base_url(&mut self, base_url: &str) {
        if self.base_url == base_url {
            return;
        }
        base_url.clone_into(&mut self.base_url);
        self.http = None;
        self.refused.clear();
        self.days.forget_host();
    }

    fn start_next(&mut self) {
        let Some(store) = self.store.as_ref().map(Arc::clone) else {
            return;
        };
        if self.pending_writes.is_shutting_down() {
            return;
        }
        let Some(day) = self.days.take_next_day() else {
            return;
        };
        let transport = self.transport();
        let delay = dispatch_delay(self.last_request, Instant::now());
        self.last_request = Some(Instant::now() + delay);
        self.spawn_fetch(transport, store, day, delay);
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_fetch(
        &self,
        transport: Arc<Connection>,
        store: Arc<JamStore>,
        day: NaiveDate,
        delay: Duration,
    ) {
        let ctx = self.ctx.clone();
        let tx = self.tx.clone();
        let base_url = self.base_url.clone();
        let pending_writes = self.pending_writes.clone();
        thread::Builder::new()
            .name(format!("jam-{day}"))
            .spawn(move || {
                thread::sleep(delay);
                let message = ingest(transport.as_ref(), &store, &base_url, day, &pending_writes);
                tx.send(message).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn interference worker thread");
    }

    /// The transport to fetch on, opened once and kept until the host
    /// changes.
    ///
    /// A transport that cannot be opened stands in as the offline one for
    /// this dispatch only, and the day fails through the worker like any
    /// other failure. The stand-in is not cached, so the next dispatch tries
    /// to open a real one again.
    ///
    /// [`super::snap::SnapScheduler`] reports its equivalent failure per
    /// track instead, because a snap run has a place in the UI to show it.
    fn transport(&mut self) -> Arc<Connection> {
        if let Some(http) = self.http.as_ref() {
            return Arc::clone(http);
        }
        match self.transport_source.connect(None) {
            Ok(connection) => {
                let http = Arc::new(connection);
                self.http = Some(Arc::clone(&http));
                http
            }
            Err(err) => {
                log::error!("Interference transport unavailable: {err}");
                Arc::new(Connection::Offline(OfflineTransport))
            }
        }
    }
}

/// One archived day's sample of the context line: the share over the cell the
/// receiver was in nearest the day's midpoint in time.
///
/// A day with no recording to place it at contributes nothing, and so breaks
/// the line like a day the archive does not hold.
fn context_day(
    store: Option<&JamStore>,
    positions: &FixPositionTimeline,
    day: NaiveDate,
) -> Vec<JammingContextSample> {
    let start = day.and_time(NaiveTime::MIN).and_utc();
    let Some((latitude, longitude)) = positions.nearest_position(start + TimeDelta::hours(12))
    else {
        return Vec::new();
    };
    let Some(dataset) = store.and_then(|store| {
        store
            .dataset(day)
            .inspect_err(|err| log::error!("Reading interference cells for {day}: {err}"))
            .ok()
            .flatten()
    }) else {
        return Vec::new();
    };
    let observation = dataset.observation_at(latitude, longitude);
    let rate = observation.and_then(gt_jam::wire::HexObservation::rate);
    vec![JammingContextSample {
        start_secs: start.timestamp() as f64,
        percent: rate.map(gt_jam::wire::InterferenceRate::percent),
        aircraft: rate.map_or(0, |rate| rate.aircraft),
        bad: observation.map_or(0, |observation| observation.bad),
    }]
}

/// Delay before dispatching the next request, keeping requests to the host
/// [`REQUEST_INTERVAL`] apart.
///
/// `last_request` is when the previous request was scheduled to go out, which
/// is already in the future when that one is itself still waiting.
fn dispatch_delay(last_request: Option<Instant>, now: Instant) -> Duration {
    let Some(last) = last_request else {
        return Duration::ZERO;
    };
    (last + REQUEST_INTERVAL).saturating_duration_since(now)
}

/// Fetch `day`, parse it, and add it to the archive.
fn ingest(
    transport: &impl Transport,
    store: &JamStore,
    base_url: &str,
    day: NaiveDate,
    pending_writes: &PendingWrites,
) -> JamMessage {
    match transport::fetch_day(transport, base_url, day) {
        FetchOutcome::Served(csv) => {
            let reporter = ParseWarningReporter::default();
            let observations = match wire::parse_dataset(&csv, &reporter) {
                Ok(observations) => observations,
                Err(err) => {
                    return JamMessage::Failed {
                        day,
                        detail: err.to_string(),
                    };
                }
            };
            let unusable = reporter.warnings().len() + reporter.suppressed();
            if unusable > 0 {
                log::warn!(
                    "{day}: {unusable} unusable interference rows: {:?}",
                    reporter.warnings()
                );
            }
            let Some(_write) =
                EnvironmentArchive::AircraftInterference.try_begin_day_insert(pending_writes, day)
            else {
                return JamMessage::NotArchivedDuringShutdown { day };
            };
            match store.insert_day(day, base_url, Utc::now(), &observations) {
                Ok(()) => JamMessage::Stored {
                    day,
                    cells: observations.len(),
                },
                Err(err) => JamMessage::Failed {
                    day,
                    detail: err.to_string(),
                },
            }
        }
        FetchOutcome::Missing => JamMessage::Missing {
            day,
            pending: calendar::awaiting_publication(day, calendar::today_utc()),
        },
        FetchOutcome::Failed(detail) => JamMessage::Failed { day, detail },
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use gt_fetch::HttpResponse;
    use gt_jam::DEFAULT_BASE_URL;
    use gt_test_utils::ScriptedTransport;

    use crate::app::backfill::BackfillProgress;
    use crate::app::day_failures::DayFailure;
    use crate::app::day_fetch_status::{ArchivedDayCount, DayFetchStatus};
    use crate::app::fix_positions::FixPositions;

    use super::*;

    fn range(start: DateTime<Utc>, end: DateTime<Utc>) -> TimeRange {
        TimeRange::new(start, end)
    }

    fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .map(|naive| naive.and_utc())
            .unwrap_or_default()
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn scheduler() -> JammingScheduler {
        JammingScheduler::disabled(Context::default())
    }

    fn archive() -> (TempDir, Arc<JamStore>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(dir.path())
            .open_interference()
            .expect("archive");
        (dir, store)
    }

    /// Archive-backed, and wired so no request leaves the machine.
    fn scheduler_with_archive() -> (TempDir, Arc<JamStore>, JammingScheduler) {
        let (dir, store) = archive();
        let scheduler = JammingScheduler::new(
            Context::default(),
            Some(Arc::clone(&store)),
            DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
            PendingWrites::default(),
        );
        (dir, store, scheduler)
    }

    /// Days the archive already holds are not queued.
    #[test]
    fn a_backfill_queues_only_the_unarchived_days_in_range() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for archived in [day(2026, 7, 21), day(2026, 7, 22)] {
            store
                .insert_day(archived, "host", Utc::now(), &[])
                .expect("insert");
        }

        let queued = scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26));
        assert_eq!(queued, Some(5), "seven days in range, two already held");
    }

    /// Re-running a backfill over a range already downloaded costs nothing.
    #[test]
    fn a_fully_archived_range_queues_nothing() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for offset in 20..=26 {
            store
                .insert_day(day(2026, 7, offset), "host", Utc::now(), &[])
                .expect("insert");
        }

        assert_eq!(
            scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26)),
            Some(0)
        );
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// A range entirely outside the coverage window requests nothing.
    #[test]
    fn a_backfill_outside_coverage_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(
            scheduler.backfill(day(2019, 1, 1), day(2020, 1, 1)),
            Some(0)
        );
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// No archive is distinct from an empty range: the control says so
    /// instead of claiming the range is already downloaded.
    #[test]
    fn a_backfill_without_an_archive_reports_no_archive() {
        let mut scheduler = scheduler();
        assert!(!scheduler.archive_available());
        assert_eq!(scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26)), None);
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// Every outcome retires its day, so a range of missing or failing days
    /// still reaches its total.
    #[rstest]
    #[case::stored(JamMessage::Stored { day: day(2026, 7, 20), cells: 1 })]
    #[case::missing(JamMessage::Missing { day: day(2026, 7, 20), pending: false })]
    #[case::failed(JamMessage::Failed { day: day(2026, 7, 20), detail: "boom".to_owned() })]
    #[case::shutting_down(JamMessage::NotArchivedDuringShutdown { day: day(2026, 7, 20) })]
    fn progress_advances_on_every_outcome(#[case] message: JamMessage) {
        let mut scheduler = scheduler();
        let days = [day(2026, 7, 20), day(2026, 7, 21)];
        scheduler.days.queue_backfill_of(&days);
        assert_eq!(
            scheduler.days.backfill_progress(),
            Some(BackfillProgress { done: 0, total: 2 })
        );

        scheduler.tx.send(message).expect("send");
        scheduler.poll();
        assert_eq!(
            scheduler.days.backfill_progress(),
            Some(BackfillProgress { done: 1, total: 2 })
        );
    }

    /// The last day retires the backfill, so the panel stops showing a bar.
    #[test]
    fn the_last_day_ends_the_backfill() {
        let mut scheduler = scheduler();
        let days = [day(2026, 7, 20)];
        scheduler.days.queue_backfill_of(&days);

        scheduler
            .tx
            .send(JamMessage::Stored {
                day: day(2026, 7, 20),
                cells: 1,
            })
            .expect("send");
        scheduler.poll();
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// Cancelling drops the queued days and lets a later backfill request
    /// them again.
    #[test]
    fn cancelling_releases_the_queued_days() {
        let mut scheduler = scheduler();
        let days = [day(2026, 7, 20), day(2026, 7, 21)];
        scheduler.days.queue_backfill_of(&days);

        scheduler.days.cancel_backfill();
        assert_eq!(scheduler.days.backfill_progress(), None);
        assert_eq!(scheduler.days.queued(), 0);
        assert!(
            scheduler.days.requested_days().is_empty(),
            "cancelled days can be re-queued"
        );
    }

    /// Cancelling must not release the day already being fetched: releasing
    /// it lets a later request go out for a day still in flight.
    #[test]
    fn cancelling_keeps_the_in_flight_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let (in_flight, queued) = (day(2026, 7, 20), day(2026, 7, 21));
        scheduler.days.queue_backfill_of(&[in_flight, queued]);
        assert_eq!(scheduler.days.take_next_day(), Some(in_flight));

        scheduler.days.cancel_backfill();
        assert!(
            scheduler.days.requested_days().contains(&in_flight),
            "the day being fetched stays claimed"
        );
        assert!(
            !scheduler.days.requested_days().contains(&queued),
            "a day that never went out can be requested again"
        );
    }

    /// A day queued by a track load is not cancelled with the backfill.
    #[test]
    fn cancelling_leaves_track_requested_days_alone() {
        let mut scheduler = scheduler();
        let track_day = day(2026, 7, 19);
        scheduler.days.queue_track_day(track_day);
        scheduler.days.queue_backfill_of(&[day(2026, 7, 20)]);

        scheduler.days.cancel_backfill();
        assert_eq!(scheduler.days.queued(), 1);
        assert!(scheduler.days.requested_days().contains(&track_day));
    }

    #[rstest]
    #[case::the_first_request(None, Duration::ZERO)]
    #[case::right_after_one(Some(Duration::ZERO), REQUEST_INTERVAL)]
    #[case::part_way_through(Some(REQUEST_INTERVAL), Duration::ZERO)]
    #[case::long_after_one(Some(REQUEST_INTERVAL * 4), Duration::ZERO)]
    fn requests_are_spaced_by_the_host_interval(
        #[case] since_last: Option<Duration>,
        #[case] expected: Duration,
    ) {
        let now = Instant::now() + REQUEST_INTERVAL * 8;
        let last = since_last.and_then(|elapsed| now.checked_sub(elapsed));
        assert_eq!(dispatch_delay(last, now), expected);
    }

    /// A request still waiting pushes the one behind it out by a further
    /// interval, so a queue of days does not all fire at once.
    #[test]
    fn a_pending_request_delays_the_next_one_further() {
        let now = Instant::now();
        let scheduled = now + REQUEST_INTERVAL;
        assert_eq!(dispatch_delay(Some(scheduled), now), REQUEST_INTERVAL * 2);
    }

    /// With one, the day goes out and the backfill keeps running.
    #[test]
    fn a_backfill_dispatches_its_first_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler
            .days
            .queue_backfill_of(&[day(2026, 7, 20), day(2026, 7, 21)]);
        scheduler.start_next();

        assert!(scheduler.days.is_fetching());
        assert_eq!(
            scheduler.days.backfill_progress(),
            Some(BackfillProgress { done: 0, total: 2 })
        );
    }

    /// Changing the host abandons a backfill and the failures the old host
    /// produced.
    #[test]
    fn changing_the_host_abandons_the_backfill_and_its_failures() {
        let mut scheduler = scheduler();
        scheduler.days.queue_backfill_of(&[day(2026, 7, 20)]);
        scheduler.days.report_failure(
            day(2026, 7, 20),
            "HTTP 500 Internal Server Error".to_owned(),
        );

        scheduler.set_base_url("https://mirror.example");

        assert_eq!(scheduler.days.backfill_progress(), None);
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.failures().is_empty());
    }

    /// A failure reaches the settings page's list.
    #[test]
    fn a_failed_day_is_reported() {
        let mut scheduler = scheduler();
        scheduler
            .tx
            .send(JamMessage::Failed {
                day: day(2026, 7, 20),
                detail: "HTTP 500 Internal Server Error".to_owned(),
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(
            scheduler.days.failures(),
            [DayFailure {
                day: day(2026, 7, 20),
                detail: "HTTP 500 Internal Server Error".to_owned(),
            }]
        );
    }

    /// Shutting down is not a fetch failure: the day was downloaded and
    /// discarded, and the settings page has nothing to report about it.
    #[test]
    fn a_day_discarded_during_shutdown_is_not_reported_as_a_failure() {
        let mut scheduler = scheduler();
        scheduler
            .tx
            .send(JamMessage::NotArchivedDuringShutdown {
                day: day(2026, 7, 20),
            })
            .expect("send");

        scheduler.poll();

        assert!(scheduler.days.failures().is_empty());
    }

    /// A day queued once shutdown began stays queued: no worker starts a
    /// download whose archive insert would be refused.
    #[test]
    fn no_day_is_dispatched_once_shutting_down() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.pending_writes.begin_shutdown();

        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.days.queued(), 1);
        assert!(!scheduler.days.is_fetching());
    }

    /// A download that finishes after shutdown began is discarded, not
    /// archived.
    #[test]
    fn a_day_downloaded_during_shutdown_is_not_archived() {
        let (_dir, store) = archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let transport = ScriptedTransport::always(Ok(HttpResponse {
            status: 200,
            body: "hex,count_good_aircraft,count_bad_aircraft\n84005c7ffffffff,412,3\n".to_owned(),
        }));
        let pending_writes = PendingWrites::default();
        pending_writes.begin_shutdown();

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, day, &pending_writes);

        assert!(matches!(
            message,
            JamMessage::NotArchivedDuringShutdown { .. }
        ));
        assert!(store.days().expect("days").is_empty());
    }

    /// The status reports the day in flight, the queue behind it, and how much
    /// of what is loaded the archive holds.
    #[test]
    fn the_status_reports_the_queue_and_the_archived_recording_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        store
            .insert_day(day(2026, 7, 20), "host", Utc::now(), &[])
            .expect("insert");

        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 21, 17)));

        assert_eq!(
            scheduler.days.fetch_status(),
            DayFetchStatus {
                fetching: Some(day(2026, 7, 21)),
                queued: 0,
                recording_days: ArchivedDayCount {
                    days: 2,
                    archived: 1,
                },
            }
        );
    }

    /// A recording made before the host's coverage begins leaves the count
    /// empty, which the settings page shows as an absent value.
    #[test]
    fn a_day_outside_coverage_is_no_recording_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2020, 1, 1, 0), at(2020, 1, 1, 1)));

        assert_eq!(scheduler.days.fetch_status().recording_days.days, 0);
    }

    /// An archived day moves the loaded recording's coverage up.
    #[test]
    fn archiving_a_day_covers_the_recording_day_it_belongs_to() {
        let mut scheduler = scheduler();
        scheduler.days.await_recording_day(day(2026, 7, 20));

        scheduler
            .tx
            .send(JamMessage::Stored {
                day: day(2026, 7, 20),
                cells: 1,
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(scheduler.days.fetch_status().recording_days.archived, 1);
    }

    #[test]
    fn a_scheduler_without_an_archive_queues_nothing() {
        let mut scheduler = scheduler();
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
        assert!(scheduler.days.failures().is_empty());
    }

    /// A store is needed to reach the queue at all, so this covers the
    /// day-selection rules through the archive-backed path.
    #[test]
    fn only_fetchable_unarchived_days_are_queued() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();

        // Before coverage: refused by the calendar, never requested.
        scheduler.request_days_for(range(at(2020, 1, 1, 0), at(2020, 1, 1, 1)));
        assert_eq!(scheduler.days.queued(), 0);

        // In the future: same.
        let ahead = Utc::now() + TimeDelta::days(3);
        scheduler.request_days_for(range(ahead, ahead));
        assert_eq!(scheduler.days.queued(), 0);

        // Already archived: skipped.
        let archived = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        store
            .insert_day(archived, "host", Utc::now(), &[])
            .expect("insert");
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.days.queued(), 0);
    }

    /// A day the archive lost goes back on the queue for the recording that
    /// spans it, which is what a delete leaves the scheduler to do.
    #[test]
    fn a_deleted_day_a_recording_spans_is_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        store
            .insert_day(archived, "host", Utc::now(), &[])
            .expect("insert");
        scheduler.archived_cells.insert(archived, 0);
        let recording = range(at(2026, 7, 20, 8), at(2026, 7, 20, 17));
        scheduler.request_days_for(recording);
        assert_eq!(scheduler.days.queued(), 0, "an archived day is not fetched");
        assert_eq!(scheduler.days.fetch_status().recording_days.archived, 1);

        store.delete_all_days().expect("delete");
        scheduler.forget_pruned_days(PrunedDays::All);
        scheduler.request_days_for(recording);

        assert_eq!(scheduler.archived_days_covered(PrunedDays::All), 0);
        assert!(
            scheduler.days.is_fetching() || scheduler.days.queued() == 1,
            "the deleted day was not requested again"
        );
        assert_eq!(scheduler.days.fetch_status().recording_days.archived, 0);
    }

    /// A queued day is always dispatched, offline included: the transport
    /// declines the request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.is_fetching());
    }

    /// A recording is requested once. Loading it again requests nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let span = range(at(2026, 7, 20, 8), at(2026, 7, 20, 17));
        scheduler.request_days_for(span);
        let after_first = scheduler.days.requested_days().len();
        scheduler.request_days_for(span);
        assert_eq!(scheduler.days.requested_days().len(), after_first);
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is
    /// the backfill feature's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2026, 6, 1, 0), at(2026, 7, 20, 0)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.requested_days().is_empty());
    }

    /// The archive records the host, not the day's own URL: a per-day string
    /// would make the column useless for spotting a mirror change.
    #[test]
    fn an_ingested_day_records_the_host_it_came_from() {
        let (_dir, store) = archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let transport = ScriptedTransport::always(Ok(HttpResponse {
            status: 200,
            body: "hex,count_good_aircraft,count_bad_aircraft\n84005c7ffffffff,412,3\n".to_owned(),
        }));

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            day,
            &PendingWrites::default(),
        );
        assert!(matches!(message, JamMessage::Stored { cells: 1, .. }));

        let stored = store.days().expect("days");
        assert_eq!(
            stored.first().map(|entry| entry.host.as_str()),
            Some(DEFAULT_BASE_URL)
        );
    }

    #[test]
    fn a_day_the_host_does_not_have_is_not_a_failure() {
        let (_dir, store) = archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let transport = ScriptedTransport::always(Ok(HttpResponse {
            status: 404,
            body: r#"{"message":"File not found"}"#.to_owned(),
        }));

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            day,
            &PendingWrites::default(),
        );
        assert!(matches!(message, JamMessage::Missing { .. }));
        assert!(store.days().expect("days").is_empty());
    }

    /// A body that is not a dataset is reported, not archived.
    #[test]
    fn an_unparsable_body_is_a_failure() {
        let (_dir, store) = archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let transport = ScriptedTransport::always(Ok(HttpResponse {
            status: 200,
            body: "<html>captive portal</html>".to_owned(),
        }));

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            day,
            &PendingWrites::default(),
        );
        assert!(matches!(message, JamMessage::Failed { .. }));
        assert!(store.days().expect("days").is_empty());
    }

    /// The overlay adopts the earliest day of the loaded tracks, whichever
    /// order the loads finish in.
    #[test]
    fn the_earliest_loaded_day_is_shown() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2026, 7, 25, 8), at(2026, 7, 25, 9)));
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 9)));

        let (_, selection) = scheduler.overlay_state();
        assert_eq!(selection.day(), NaiveDate::from_ymd_opt(2026, 7, 20));
    }

    /// Once stepped, a later load does not move the overlay.
    #[test]
    fn a_later_load_does_not_move_a_stepped_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2026, 7, 25, 8), at(2026, 7, 25, 9)));
        scheduler.overlay_state().1.step_back();

        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 9)));
        let (_, selection) = scheduler.overlay_state();
        assert_eq!(selection.day(), NaiveDate::from_ymd_opt(2026, 7, 24));
    }

    /// A day the host refused reads differently from one never downloaded.
    #[test]
    fn a_refused_day_is_not_reported_as_undownloaded() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 9)));
        assert_eq!(scheduler.empty_reason(), Some(EmptyReason::NotFetched));

        scheduler.refused.insert(day);
        assert_eq!(scheduler.empty_reason(), Some(EmptyReason::NotPublished));
    }

    /// With no track loaded the legend says so, rather than showing a day.
    #[test]
    fn no_loaded_track_means_no_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(scheduler.empty_reason(), Some(EmptyReason::NoTrack));
        assert_eq!(scheduler.overlay_state().1.day(), None);
    }

    /// The fixture track, with the time range a real load derives from its
    /// points. `days_spanned` reads that range, not the points.
    fn track_with_time_range() -> gt_types::LoadedTrack {
        use gt_test_utils::fixtures;

        let mut track = fixtures::loaded_track_with_points(fixtures::nav_test_data());
        let times: Vec<_> = track
            .points
            .iter()
            .map(|point| point.tpv.time().utc())
            .collect();
        if let (Some(&start), Some(&end)) = (times.iter().min(), times.iter().max()) {
            track.metadata.time_range = gt_types::TimeRange::new(start, end);
        }
        track
    }

    /// The one loaded file the plot series is resolved over.
    fn files_holding(track: gt_types::LoadedTrack) -> Vec<gt_types::LoadedFile> {
        vec![gt_types::LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(std::sync::Arc::from(Vec::<u8>::new())),
        }]
    }

    /// A track whose fixes fall in an archived cell gets a value per fix.
    /// One whose day is not archived breaks the line instead.
    #[test]
    fn plot_points_come_from_the_fixs_own_day() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let track = track_with_time_range();
        let day = track.metadata.time_range.start.date_naive();
        let first = track.points.first().expect("a fix");
        let cell = gt_jam::dataset::cell_at(first.tpv.lat(), first.tpv.lon()).expect("cell");
        store
            .insert_day(
                day,
                "host",
                Utc::now(),
                &[gt_jam::wire::HexObservation {
                    cell,
                    good: 90,
                    bad: 10,
                }],
            )
            .expect("insert");
        scheduler.archived_cells.insert(day, 1);

        let files = files_holding(track);

        let series = scheduler.plot_series(&files);
        let points = series
            .points_by_track
            .values()
            .next()
            .expect("the track has values");
        let valued: Vec<_> = points.iter().filter(|p| p.percent.is_some()).collect();
        assert!(
            !valued.is_empty(),
            "fixes in the archived cell carry a value"
        );
        for point in &valued {
            // The share is computed in f32 and widened, so 10 % is not exact.
            let percent = point.percent.unwrap_or_default();
            assert!((percent - 10.0).abs() < 1e-6, "{percent}");
            assert_eq!(point.aircraft, 100, "the count behind the share");
        }
        // The fixture track is about a kilometre across, well inside one
        // 22 km cell, so every fix takes that cell's value.
        assert_eq!(valued.len(), points.len());
    }

    /// An unchanged archive hands out the same `Arc` for a track's values,
    /// and archiving a day the track spans produces a new one. The query
    /// fingerprint compares those identities every frame.
    #[test]
    fn query_values_keep_their_identity_until_an_archived_day_changes_them() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let mut track = track_with_time_range();
        let recorded = track.metadata.time_range.start.date_naive();
        let next = recorded
            .checked_add_days(chrono::Days::new(1))
            .expect("the day after the recording");
        track.metadata.time_range = TimeRange::new(
            track.metadata.time_range.start,
            next.and_time(NaiveTime::MIN).and_utc(),
        );
        let fix = track.points.first().expect("a fix");
        let cell = gt_jam::dataset::cell_at(fix.tpv.lat(), fix.tpv.lon()).expect("cell");
        let observations = [gt_jam::wire::HexObservation {
            cell,
            good: 90,
            bad: 10,
        }];
        store
            .insert_day(recorded, "host", Utc::now(), &observations)
            .expect("insert");
        scheduler.archived_cells.insert(recorded, 1);
        let files = files_holding(track);
        let track_ref = TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
        let values_of = |values: &JammingValues| {
            values
                .get(&track_ref)
                .map(Arc::clone)
                .expect("the archived track has values")
        };

        scheduler.plot_series(&files);
        let first_frame = values_of(&scheduler.query_values());
        scheduler.plot_series(&files);
        let second_frame = values_of(&scheduler.query_values());

        assert!(
            Arc::ptr_eq(&first_frame, &second_frame),
            "an unchanged archive hands out the values it already resolved"
        );

        store
            .insert_day(next, "host", Utc::now(), &observations)
            .expect("insert");
        scheduler.archived_cells.insert(next, 1);
        scheduler.plot_series(&files);
        let after_archiving = values_of(&scheduler.query_values());

        assert!(
            !Arc::ptr_eq(&second_frame, &after_archiving),
            "archiving a day the track spans re-resolves its values"
        );
    }

    /// The context line over the UTC days `from..=to`.
    fn context_line_over(
        scheduler: &mut JammingScheduler,
        positions: &Arc<FixPositionTimeline>,
        days: std::ops::RangeInclusive<NaiveDate>,
    ) -> Arc<Vec<JammingContextSample>> {
        let midnight = |day: NaiveDate| day.and_time(NaiveTime::MIN).and_utc().timestamp() as f64;
        scheduler.context_line(
            ContextSpan::covering(midnight(*days.start())..=midnight(*days.end())),
            positions,
        )
    }

    /// A day outside every recording is still drawn, read at the cell of the
    /// fix nearest that day in time.
    #[test]
    fn the_context_line_values_a_day_no_recording_covers() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let track = track_with_time_range();
        let first = track.points.first().expect("a fix");
        let cell = gt_jam::dataset::cell_at(first.tpv.lat(), first.tpv.lon()).expect("cell");
        let recorded = track.metadata.time_range.start.date_naive();
        let later = recorded
            .checked_add_days(chrono::Days::new(3))
            .expect("a later day");
        for archived in [recorded, later] {
            store
                .insert_day(
                    archived,
                    "host",
                    Utc::now(),
                    &[gt_jam::wire::HexObservation {
                        cell,
                        good: 90,
                        bad: 10,
                    }],
                )
                .expect("insert");
            scheduler.archived_cells.insert(archived, 1);
        }
        let mut positions = FixPositions::default();
        let files = files_holding(track);
        let timeline = Arc::clone(positions.timeline(&files));

        let line = context_line_over(&mut scheduler, &timeline, recorded..=later);

        let percents: Vec<Option<f64>> = line.iter().map(|sample| sample.percent).collect();
        assert_eq!(percents.len(), 3, "two archived days and the gap between");
        assert!(
            percents
                .first()
                .copied()
                .flatten()
                .is_some_and(|percent| (percent - 10.0).abs() < 1e-6),
            "{percents:?}"
        );
        assert_eq!(percents.get(1), Some(&None), "the days between are a break");
        assert!(
            percents
                .get(2)
                .copied()
                .flatten()
                .is_some_and(|percent| (percent - 10.0).abs() < 1e-6),
            "the day after the recording reads the nearest fix's cell"
        );
    }

    /// With no recording loaded there is no position to read a day at, so the
    /// line stays empty.
    #[test]
    fn the_context_line_needs_a_recording_to_place_a_day_at() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        store
            .insert_day(archived, "host", Utc::now(), &[])
            .expect("insert");
        scheduler.archived_cells.insert(archived, 1);

        let timeline = Arc::new(FixPositionTimeline::default());
        let line = context_line_over(&mut scheduler, &timeline, archived..=archived);

        assert!(line.is_empty());
    }

    /// With nothing archived, the track contributes no series at all.
    #[test]
    fn a_track_with_no_archived_day_has_no_plot_series() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let track = track_with_time_range();
        let files = files_holding(track);

        assert!(scheduler.plot_series(&files).is_empty());
    }

    /// A changed host drops what belonged to the old one, and keeps what
    /// does not depend on it.
    #[test]
    fn changing_the_host_drops_only_the_hosts_own_state() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        scheduler.refused.insert(day);
        scheduler.archived_cells.insert(day, 44_546);

        scheduler.set_base_url("https://mirror.example");

        assert!(
            scheduler.days.requested_days().is_empty(),
            "the old host's requests"
        );
        assert!(scheduler.refused.is_empty(), "the old host's refusals");
        assert_eq!(scheduler.days.queued(), 0);
        assert_eq!(
            scheduler.archived_cells.get(&day),
            Some(&44_546),
            "archived days do not depend on the host"
        );
    }

    #[test]
    fn setting_the_same_host_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        scheduler.refused.insert(day);
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        let seen = scheduler.days.requested_days().len();

        scheduler.set_base_url(DEFAULT_BASE_URL);

        assert_eq!(scheduler.days.requested_days().len(), seen);
        assert!(scheduler.refused.contains(&day));
    }
}
