//! Interference fetch worker and archive ingest.
//!
//! Follows [`super::snap`]: owned by the app, a background thread per
//! request reporting over an mpsc channel, `request_repaint` on every
//! message.
//!
//! Loading a track queues the UTC days it spans. A day already in the
//! archive is never requested, so the queue shrinks to nothing as the
//! archive fills. One request is in flight at a time.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_jam::calendar::{self, DayOutlook};
use gt_jam::dataset::JamDataset;
use gt_jam::day_selection::{DaySelection, EmptyReason};
use gt_jam::transport::{
    self, Connection, FetchOutcome, REQUEST_INTERVAL, Transport, TransportSource,
};
use gt_jam::wire::{self, ParseWarningReporter};
use gt_store::JamStore;
#[cfg(test)]
use gt_store::Store;
use gt_types::TimeRange;
use gt_types::{LoadedFile, TrackRef};
use gt_ui_types::{JammingPoint, JammingSeries};

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
}

impl JamMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. } | Self::Missing { day, .. } | Self::Failed { day, .. } => day,
        }
    }
}

/// A day that could not be added to the archive, for the side panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFailure {
    pub day: NaiveDate,
    pub detail: String,
}

/// A backfill's progress, for the panel's bar and count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillProgress {
    /// Days that have reported back, whether archived, missing, or failed.
    pub done: usize,
    /// Days the backfill queued. Days already archived or already requested
    /// this session are not among them.
    pub total: usize,
}

impl BackfillProgress {
    pub fn fraction(self) -> f32 {
        if self.total == 0 {
            return 1.0;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a backfill spans at most a few thousand days"
        )]
        {
            self.done as f32 / self.total as f32
        }
    }
}

/// A backfill in flight.
struct Backfill {
    /// Queued days still to report back.
    pending: HashSet<NaiveDate>,
    total: usize,
}

/// Queues interference days and ingests them into the archive.
pub struct JammingScheduler {
    ctx: Context,
    tx: mpsc::Sender<JamMessage>,
    rx: mpsc::Receiver<JamMessage>,
    base_url: String,
    /// `None` disables fetching: no archive to write to.
    store: Option<JamStore>,
    /// Connected on the first request, and dropped when the host changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here decides whether requests may leave the machine.
    transport_source: TransportSource,
    queue: VecDeque<NaiveDate>,
    /// Every day queued this session, so a day is requested at most once
    /// even after it fails.
    seen: HashSet<NaiveDate>,
    in_flight: Option<NaiveDate>,
    failures: Vec<DayFailure>,
    /// Cells archived per day, read once at startup and updated on ingest,
    /// so the display toggle does not open the archive per frame. Assumes
    /// this process is the archive's only writer.
    archived_cells: HashMap<NaiveDate, u32>,
    /// The day the overlay draws, and its cells, loaded from the archive on
    /// demand and kept until the shown day changes. A day is never
    /// re-ingested - `insert_day` refuses one already stored - so a loaded
    /// day cannot go out of date.
    shown: Option<(NaiveDate, JamDataset)>,
    /// Which day the overlay shows, and the stepper's bounds.
    selection: DaySelection,
    /// Days the host answered it has no dataset for, which the legend
    /// distinguishes from a day nothing was downloaded for.
    refused: HashSet<NaiveDate>,
    /// Per-track plot points, keyed by the days they were resolved from, so
    /// the `Arc` identity the plot caches on only changes when the archive
    /// gained a day the track needs.
    plot_points: HashMap<TrackRef, (Vec<NaiveDate>, Arc<Vec<JammingPoint>>)>,
    /// Set while an explicit backfill is running.
    backfill: Option<Backfill>,
    /// When the last request was handed to a worker, so [`REQUEST_INTERVAL`]
    /// is honoured across days.
    last_request: Option<Instant>,
}

impl JammingScheduler {
    pub fn new(
        ctx: Context,
        store: Option<JamStore>,
        base_url: String,
        transport_source: TransportSource,
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
            plot_points: HashMap::new(),
            ctx,
            tx,
            rx,
            base_url,
            store,
            http: None,
            transport_source,
            queue: VecDeque::new(),
            seen: HashSet::new(),
            in_flight: None,
            failures: Vec::new(),
            backfill: None,
            last_request: None,
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
        )
    }

    /// Queue the days a recording spans.
    ///
    /// Days outside the coverage window, already archived, or already
    /// queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    pub fn request_days_for(&mut self, range: TimeRange) {
        let Some(store) = self.store.as_ref() else {
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
            if !self.seen.insert(day) {
                continue;
            }
            if calendar::day_outlook(day, today) != DayOutlook::Fetchable {
                continue;
            }
            match store.contains(day) {
                Ok(true) => {}
                Ok(false) => self.queue.push_back(day),
                Err(err) => {
                    let detail = format!("reading the archive: {err}");
                    log::error!("Cannot tell whether {day} is archived: {detail}");
                    self.failures.push(DayFailure { day, detail });
                }
            }
        }
        self.start_next();
    }

    /// Queue every unarchived day in `from..=to`.
    ///
    /// Days outside the coverage window, already archived, or already
    /// requested this session are skipped, so re-running a backfill over the
    /// same range costs nothing. Replaces a backfill already running.
    ///
    /// Returns how many days were queued, or [`None`] when there is no
    /// archive to write them to.
    pub fn backfill(&mut self, from: NaiveDate, to: NaiveDate) -> Option<usize> {
        let store = self.store.clone()?;
        self.cancel_backfill();
        let mut pending = HashSet::new();
        for day in calendar::fetchable_days(from, to, calendar::today_utc()) {
            if !self.seen.insert(day) {
                continue;
            }
            match store.contains(day) {
                Ok(true) => {}
                Ok(false) => {
                    self.queue.push_back(day);
                    pending.insert(day);
                }
                Err(err) => {
                    let detail = format!("reading the archive: {err}");
                    log::error!("Cannot tell whether {day} is archived: {detail}");
                    self.failures.push(DayFailure { day, detail });
                }
            }
        }
        let total = pending.len();
        log::info!("Backfilling interference for {total} days between {from} and {to}");
        if total > 0 {
            self.backfill = Some(Backfill { pending, total });
        }
        self.start_next();
        Some(total)
    }

    /// Whether there is an archive to download into. Grays the backfill
    /// control when there is not.
    pub fn archive_available(&self) -> bool {
        self.store.is_some()
    }

    /// Drop a running backfill's queued days.
    ///
    /// Cancelled days leave `seen`, so a later backfill over the same range
    /// queues them again. The day in flight is not one of them: it stays in
    /// `seen` until its own request reports back, or a second request would
    /// go out for a day already being fetched.
    pub fn cancel_backfill(&mut self) {
        let Some(backfill) = self.backfill.take() else {
            return;
        };
        self.queue.retain(|day| !backfill.pending.contains(day));
        for day in backfill.pending {
            if Some(day) != self.in_flight {
                self.seen.remove(&day);
            }
        }
    }

    /// Progress of the running backfill, or [`None`] when none is running.
    pub fn backfill_progress(&self) -> Option<BackfillProgress> {
        self.backfill.as_ref().map(|backfill| BackfillProgress {
            done: backfill.total.saturating_sub(backfill.pending.len()),
            total: backfill.total,
        })
    }

    /// Apply finished fetches and start the next queued day.
    pub fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.in_flight = None;
            if let Some(backfill) = self.backfill.as_mut() {
                backfill.pending.remove(&message.day());
                if backfill.pending.is_empty() {
                    self.backfill = None;
                }
            }
            match message {
                JamMessage::Stored { day, cells } => {
                    log::info!("Archived {cells} interference cells for {day}");
                    self.archived_cells
                        .insert(day, u32::try_from(cells).unwrap_or(u32::MAX));
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
                    self.failures.push(DayFailure { day, detail });
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

                let days = self.days_available_for(track);
                let cached = self
                    .plot_points
                    .get(&track_ref)
                    .filter(|(resolved, _)| *resolved == days);
                let points = match cached {
                    Some((_, points)) => Arc::clone(points),
                    None => {
                        let points = Arc::new(Self::resolve_points(
                            self.store.as_ref(),
                            &mut datasets,
                            track,
                        ));
                        self.plot_points
                            .insert(track_ref, (days, Arc::clone(&points)));
                        points
                    }
                };
                if points.iter().any(|point| point.percent.is_some()) {
                    series.points_by_track.insert(track_ref, points);
                }
            }
        }
        self.plot_points.retain(|track, _| live.contains(track));
        series
    }

    /// Dense per-fix interference percentages, for the query providers.
    /// Shaped like the snap-error values so both reach the provider the same
    /// way.
    pub fn query_values(series: &JammingSeries) -> HashMap<TrackRef, Arc<Vec<Option<f64>>>> {
        series
            .points_by_track
            .iter()
            .map(|(&track, points)| {
                let values = points.iter().map(|point| point.percent).collect();
                (track, Arc::new(values))
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

    /// One point per fix, valued from the dataset of the fix's own day.
    fn resolve_points(
        store: Option<&JamStore>,
        datasets: &mut HashMap<NaiveDate, Option<JamDataset>>,
        track: &gt_types::LoadedTrack,
    ) -> Vec<JammingPoint> {
        track
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
            .as_ref()?
            .dataset(day)
            .inspect_err(|err| log::error!("Reading interference cells for {day}: {err}"))
            .ok()
            .flatten()
    }

    /// Point the scheduler at `base_url`.
    ///
    /// A changed host drops the queue, `seen`, and `refused`: those requests
    /// and refusals belong to the old host. `archived_cells` is kept - a day
    /// already archived does not depend on which host served it.
    pub fn set_base_url(&mut self, base_url: &str) {
        if self.base_url == base_url {
            return;
        }
        base_url.clone_into(&mut self.base_url);
        self.http = None;
        self.queue.clear();
        self.seen.clear();
        self.refused.clear();
        self.backfill = None;
    }

    /// Days that could not be archived, oldest first.
    #[cfg(test)]
    fn failures(&self) -> Vec<DayFailure> {
        let mut failures = self.failures.clone();
        failures.sort_by_key(|failure| failure.day);
        failures
    }

    #[cfg(test)]
    fn is_fetching(&self) -> bool {
        self.in_flight.is_some()
    }

    #[cfg(test)]
    fn queued(&self) -> usize {
        self.queue.len()
    }

    fn start_next(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let (Some(store), Some(day)) = (self.store.clone(), self.queue.front().copied()) else {
            return;
        };
        let transport = self.transport();
        let delay = dispatch_delay(self.last_request, Instant::now());
        self.queue.pop_front();
        self.in_flight = Some(day);
        self.last_request = Some(Instant::now() + delay);
        spawn_fetch(
            self.ctx.clone(),
            self.tx.clone(),
            transport,
            store,
            self.base_url.clone(),
            day,
            delay,
        );
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
        match self.transport_source.connect() {
            Ok(connection) => {
                let http = Arc::new(connection);
                self.http = Some(Arc::clone(&http));
                http
            }
            Err(err) => {
                log::error!("Interference transport unavailable: {err}");
                Arc::new(Connection::Offline(gt_jam::transport::OfflineTransport))
            }
        }
    }
}

/// How long the next request waits, so requests to the host stay
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

#[expect(
    clippy::expect_used,
    reason = "thread spawn can only fail under extreme system resource exhaustion"
)]
fn spawn_fetch(
    ctx: Context,
    tx: mpsc::Sender<JamMessage>,
    transport: Arc<Connection>,
    store: JamStore,
    base_url: String,
    day: NaiveDate,
    delay: Duration,
) {
    thread::Builder::new()
        .name(format!("jam-{day}"))
        .spawn(move || {
            thread::sleep(delay);
            let message = ingest(transport.as_ref(), &store, &base_url, day);
            tx.send(message).ok();
            ctx.request_repaint();
        })
        .expect("failed to spawn interference worker thread");
}

/// Fetch `day`, parse it, and add it to the archive.
fn ingest(
    transport: &impl Transport,
    store: &JamStore,
    base_url: &str,
    day: NaiveDate,
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

    use gt_jam::DEFAULT_BASE_URL;
    use gt_jam::transport::{HttpResponse, TransportError};

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

    fn archive() -> (TempDir, JamStore) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(dir.path())
            .open_interference()
            .expect("archive");
        (dir, store)
    }

    /// Archive-backed, and wired so no request leaves the machine.
    fn scheduler_with_archive() -> (TempDir, JamStore, JammingScheduler) {
        let (dir, store) = archive();
        let scheduler = JammingScheduler::new(
            Context::default(),
            Some(store.clone()),
            DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
        );
        (dir, store, scheduler)
    }

    /// Answers every request with one canned response.
    struct CannedTransport {
        status: u16,
        body: String,
    }

    impl Transport for CannedTransport {
        fn get(&self, _url: &str) -> Result<HttpResponse, TransportError> {
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
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
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// A range entirely outside the coverage window asks for nothing.
    #[test]
    fn a_backfill_outside_coverage_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(
            scheduler.backfill(day(2019, 1, 1), day(2020, 1, 1)),
            Some(0)
        );
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// No archive is distinct from an empty range: the control says so
    /// instead of claiming the range is already downloaded.
    #[test]
    fn a_backfill_without_an_archive_reports_no_archive() {
        let mut scheduler = scheduler();
        assert!(!scheduler.archive_available());
        assert_eq!(scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26)), None);
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// Fill the queue and the backfill without dispatching, so the tests
    /// below do not depend on whether a transport can be built.
    fn queued_backfill(scheduler: &mut JammingScheduler, days: &[NaiveDate]) {
        for day in days {
            scheduler.seen.insert(*day);
            scheduler.queue.push_back(*day);
        }
        scheduler.backfill = Some(Backfill {
            pending: days.iter().copied().collect(),
            total: days.len(),
        });
    }

    /// Every outcome retires its day, so a range of missing or failing days
    /// still reaches its total.
    #[rstest]
    #[case::stored(JamMessage::Stored { day: day(2026, 7, 20), cells: 1 })]
    #[case::missing(JamMessage::Missing { day: day(2026, 7, 20), pending: false })]
    #[case::failed(JamMessage::Failed { day: day(2026, 7, 20), detail: "boom".to_owned() })]
    fn progress_advances_on_every_outcome(#[case] message: JamMessage) {
        let mut scheduler = scheduler();
        let days = [day(2026, 7, 20), day(2026, 7, 21)];
        queued_backfill(&mut scheduler, &days);
        assert_eq!(
            scheduler.backfill_progress(),
            Some(BackfillProgress { done: 0, total: 2 })
        );

        scheduler.tx.send(message).expect("send");
        scheduler.poll();
        assert_eq!(
            scheduler.backfill_progress(),
            Some(BackfillProgress { done: 1, total: 2 })
        );
    }

    /// The last day retires the backfill, so the panel stops showing a bar.
    #[test]
    fn the_last_day_ends_the_backfill() {
        let mut scheduler = scheduler();
        let days = [day(2026, 7, 20)];
        queued_backfill(&mut scheduler, &days);

        scheduler
            .tx
            .send(JamMessage::Stored {
                day: day(2026, 7, 20),
                cells: 1,
            })
            .expect("send");
        scheduler.poll();
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// Cancelling drops the queued days and lets a later backfill ask for
    /// them again.
    #[test]
    fn cancelling_releases_the_queued_days() {
        let mut scheduler = scheduler();
        let days = [day(2026, 7, 20), day(2026, 7, 21)];
        queued_backfill(&mut scheduler, &days);

        scheduler.cancel_backfill();
        assert_eq!(scheduler.backfill_progress(), None);
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.seen.is_empty(), "cancelled days can be re-queued");
    }

    /// Cancelling must not release the day already being fetched: releasing
    /// it lets a later request go out for a day still in flight.
    #[test]
    fn cancelling_keeps_the_in_flight_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let (in_flight, queued) = (day(2026, 7, 20), day(2026, 7, 21));
        queued_backfill(&mut scheduler, &[in_flight, queued]);
        scheduler.in_flight = Some(in_flight);
        scheduler.queue.retain(|day| *day != in_flight);

        scheduler.cancel_backfill();
        assert!(
            scheduler.seen.contains(&in_flight),
            "the day being fetched stays claimed"
        );
        assert!(
            !scheduler.seen.contains(&queued),
            "a day that never went out can be asked for again"
        );
    }

    /// A day queued by a track load is not cancelled with the backfill.
    #[test]
    fn cancelling_leaves_track_requested_days_alone() {
        let mut scheduler = scheduler();
        let track_day = day(2026, 7, 19);
        scheduler.seen.insert(track_day);
        scheduler.queue.push_back(track_day);
        queued_backfill(&mut scheduler, &[day(2026, 7, 20)]);

        scheduler.cancel_backfill();
        assert_eq!(scheduler.queued(), 1);
        assert!(scheduler.seen.contains(&track_day));
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
        queued_backfill(&mut scheduler, &[day(2026, 7, 20), day(2026, 7, 21)]);
        scheduler.start_next();

        assert!(scheduler.is_fetching());
        assert_eq!(
            scheduler.backfill_progress(),
            Some(BackfillProgress { done: 0, total: 2 })
        );
    }

    /// Changing the host abandons a backfill: its remaining days belong to
    /// the old host.
    #[test]
    fn changing_the_host_abandons_the_backfill() {
        let mut scheduler = scheduler();
        queued_backfill(&mut scheduler, &[day(2026, 7, 20)]);
        scheduler.set_base_url("https://mirror.example");
        assert_eq!(scheduler.backfill_progress(), None);
        assert_eq!(scheduler.queued(), 0);
    }

    #[test]
    fn a_scheduler_without_an_archive_queues_nothing() {
        let mut scheduler = scheduler();
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
        assert!(scheduler.failures().is_empty());
    }

    /// A store is needed to reach the queue at all, so this covers the
    /// day-selection rules through the archive-backed path.
    #[test]
    fn only_fetchable_unarchived_days_are_queued() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();

        // Before coverage: refused by the calendar, never requested.
        scheduler.request_days_for(range(at(2020, 1, 1, 0), at(2020, 1, 1, 1)));
        assert_eq!(scheduler.queued(), 0);

        // In the future: same.
        let ahead = Utc::now() + TimeDelta::days(3);
        scheduler.request_days_for(range(ahead, ahead));
        assert_eq!(scheduler.queued(), 0);

        // Already archived: skipped.
        let archived = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        store
            .insert_day(archived, "host", Utc::now(), &[])
            .expect("insert");
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.queued(), 0);
    }

    /// A queued day is always dispatched, offline included: the transport
    /// declines the request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.is_fetching());
    }

    /// A recording is requested once; loading it again asks for nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let span = range(at(2026, 7, 20, 8), at(2026, 7, 20, 17));
        scheduler.request_days_for(span);
        let after_first = scheduler.seen.len();
        scheduler.request_days_for(span);
        assert_eq!(scheduler.seen.len(), after_first);
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is
    /// the backfill feature's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(range(at(2026, 6, 1, 0), at(2026, 7, 20, 0)));
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.seen.is_empty());
    }

    /// The archive records the host, not the day's own URL: a per-day string
    /// would make the column useless for spotting a mirror change.
    #[test]
    fn an_ingested_day_records_the_host_it_came_from() {
        let (_dir, store) = archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let transport = CannedTransport {
            status: 200,
            body: "hex,count_good_aircraft,count_bad_aircraft\n84005c7ffffffff,412,3\n".to_owned(),
        };

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, day);
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
        let transport = CannedTransport {
            status: 404,
            body: r#"{"message":"File not found"}"#.to_owned(),
        };

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, day);
        assert!(matches!(message, JamMessage::Missing { .. }));
        assert!(store.days().expect("days").is_empty());
    }

    /// A body that is not a dataset is reported, not archived.
    #[test]
    fn an_unparsable_body_is_a_failure() {
        let (_dir, store) = archive();
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let transport = CannedTransport {
            status: 200,
            body: "<html>captive portal</html>".to_owned(),
        };

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, day);
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

    /// A track whose fixes fall in an archived cell gets a value per fix;
    /// one whose day is not archived breaks the line instead.
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

        let files = vec![gt_types::LoadedFile {
            metadata: gt_types::FileMetadata::default(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(std::sync::Arc::from(Vec::<u8>::new())),
        }];

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

    /// With nothing archived, the track contributes no series at all.
    #[test]
    fn a_track_with_no_archived_day_has_no_plot_series() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let track = track_with_time_range();
        let files = vec![gt_types::LoadedFile {
            metadata: gt_types::FileMetadata::default(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(std::sync::Arc::from(Vec::<u8>::new())),
        }];

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

        assert!(scheduler.seen.is_empty(), "the old host's requests");
        assert!(scheduler.refused.is_empty(), "the old host's refusals");
        assert_eq!(scheduler.queued(), 0);
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
        let seen = scheduler.seen.len();

        scheduler.set_base_url(DEFAULT_BASE_URL);

        assert_eq!(scheduler.seen.len(), seen);
        assert!(scheduler.refused.contains(&day));
    }
}
