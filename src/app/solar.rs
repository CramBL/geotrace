//! Geomagnetic index fetch worker and archive ingest.
//!
//! Follows [`super::jamming`]: owned by the app, a background thread per
//! request reporting over an mpsc channel, `request_repaint` on every message.
//!
//! Loading a track queues the UTC days it spans, and one day's request covers
//! every index the service can have values for on it. A day the archive holds
//! in a form GFZ will not revise is never requested again. One request is in
//! flight at a time, and the transport spaces requests
//! [`transport::REQUEST_INTERVAL`] apart.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, Transport, TransportSource};
use gt_solar::series::{Hp30Series, KpSeries};
use gt_solar::{GeomagneticIndex, TimeWindow, calendar, transport, wire};
use gt_store::{SolarStore, SolarStoreError};
use gt_types::TimeRange;

use super::backfill::{BackfillProgress, PendingBackfill};
use super::geomagnetic_index_ui::GeomagneticIndexFetchStatus;

/// What one day's fetch produced.
enum IndexDayMessage {
    /// Every index the service could have values for was archived. Both
    /// counts are zero for a day it published nothing in.
    Stored {
        day: NaiveDate,
        kp_samples: usize,
        hp30_samples: usize,
    },
    Failed {
        day: NaiveDate,
        detail: String,
    },
}

impl IndexDayMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. } | Self::Failed { day, .. } => day,
        }
    }
}

/// A day that could not be added to the archive, for the settings section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFailure {
    pub day: NaiveDate,
    pub detail: String,
}

impl fmt::Display for DayFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.day, self.detail)
    }
}

/// What the archive holds for one UTC day a loaded recording spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingDayCoverage {
    /// Every index published for the day is archived.
    Archived,
    /// At least one index is still to be fetched.
    Awaited,
}

/// Queues geomagnetic index days and ingests them into the archive.
pub struct GeomagneticIndexScheduler {
    ctx: Context,
    tx: mpsc::Sender<IndexDayMessage>,
    rx: mpsc::Receiver<IndexDayMessage>,
    base_url: String,
    /// `None` disables fetching: no archive to write to.
    store: Option<Arc<SolarStore>>,
    /// Connected on the first request, and dropped when the host changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here decides whether requests may leave the machine.
    transport_source: TransportSource,
    queue: VecDeque<NaiveDate>,
    /// Every day queued this session, so a day is requested at most once even
    /// after it fails or comes back revisable.
    seen: HashSet<NaiveDate>,
    in_flight: Option<NaiveDate>,
    failures: Vec<DayFailure>,
    /// The UTC days the recordings loaded this session span, and what the
    /// archive holds for each. Read by the settings section, which reports
    /// how far the archive covers what is loaded.
    recording_days: BTreeMap<NaiveDate, RecordingDayCoverage>,
    /// Set while an explicit backfill is running.
    backfill: Option<PendingBackfill>,
}

impl GeomagneticIndexScheduler {
    pub fn new(
        ctx: Context,
        store: Option<Arc<SolarStore>>,
        base_url: String,
        transport_source: TransportSource,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
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
            recording_days: BTreeMap::new(),
            backfill: None,
        }
    }

    /// Queue the days a recording spans.
    ///
    /// Days already archived in final form, outside every index's coverage, or
    /// already queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    pub fn request_days_for(&mut self, range: TimeRange) {
        if self.store.is_none() {
            return;
        }
        let Some(days) = range.utc_days(calendar::MAX_DAYS_PER_TRACK) else {
            log::info!(
                "A recording spanning {} is past the {}-day limit; no geomagnetic index days queued",
                range.duration(),
                calendar::MAX_DAYS_PER_TRACK
            );
            return;
        };
        let today = Utc::now().date_naive();
        for day in days {
            if calendar::fetchable_indices(day, today).next().is_none() {
                continue;
            }
            let coverage = match self.day_needs_fetch(day, today) {
                Ok(false) => RecordingDayCoverage::Archived,
                Ok(true) => {
                    if self.seen.insert(day) {
                        self.queue.push_back(day);
                    }
                    RecordingDayCoverage::Awaited
                }
                Err(err) => {
                    if self.seen.insert(day) {
                        let detail = format!("reading the archive: {err}");
                        log::error!("Cannot tell whether {day} is archived: {detail}");
                        self.failures.push(DayFailure { day, detail });
                    }
                    RecordingDayCoverage::Awaited
                }
            };
            self.recording_days.insert(day, coverage);
        }
        self.start_next();
    }

    /// Queue every day in `from..=to` the archive does not already hold in a
    /// form GFZ will not revise.
    ///
    /// Re-running a backfill over the same range costs nothing: days already
    /// requested this session are skipped. Replaces a backfill already
    /// running.
    ///
    /// Returns how many days were queued, or [`None`] when there is no
    /// archive to write them to.
    pub fn backfill(&mut self, from: NaiveDate, to: NaiveDate) -> Option<usize> {
        self.store.as_ref()?;
        self.cancel_backfill();
        let today = Utc::now().date_naive();
        let mut pending = HashSet::new();
        for day in calendar::fetchable_days(from, to, today) {
            if !self.seen.insert(day) {
                continue;
            }
            match self.day_needs_fetch(day, today) {
                Ok(true) => {
                    self.queue.push_back(day);
                    pending.insert(day);
                }
                Ok(false) => {}
                Err(err) => {
                    let detail = format!("reading the archive: {err}");
                    log::error!("Cannot tell whether {day} is archived: {detail}");
                    self.failures.push(DayFailure { day, detail });
                }
            }
        }
        let total = pending.len();
        log::info!("Backfilling geomagnetic indices for {total} days between {from} and {to}");
        if total > 0 {
            self.backfill = Some(PendingBackfill::new(pending));
        }
        self.start_next();
        Some(total)
    }

    /// Drop a running backfill's queued days.
    ///
    /// A later backfill over the same range queues the cancelled days again:
    /// they leave `seen`. The day in flight is not one of them, and stays in
    /// `seen` until a response for it arrives: releasing it would let a second
    /// request go out for a day already being fetched.
    pub fn cancel_backfill(&mut self) {
        let Some(backfill) = self.backfill.take() else {
            return;
        };
        self.queue.retain(|day| !backfill.queued(*day));
        for day in backfill.into_pending_days() {
            if Some(day) != self.in_flight {
                self.seen.remove(&day);
            }
        }
    }

    /// Progress of the running backfill, or [`None`] when none is running.
    pub fn backfill_progress(&self) -> Option<BackfillProgress> {
        self.backfill.as_ref().map(PendingBackfill::progress)
    }

    /// Whether there is an archive to download into. Grays the backfill
    /// control when there is not.
    pub fn archive_available(&self) -> bool {
        self.store.is_some()
    }

    /// What the settings section reports about the queue and the archive's
    /// coverage of the loaded recordings.
    pub fn fetch_status(&self) -> GeomagneticIndexFetchStatus {
        GeomagneticIndexFetchStatus {
            fetching: self.in_flight,
            queued: self.queue.len(),
            recording_days: self.recording_days.len(),
            archived_recording_days: self
                .recording_days
                .values()
                .filter(|coverage| **coverage == RecordingDayCoverage::Archived)
                .count(),
        }
    }

    /// Days that could not be archived, in the order they were reported.
    pub fn failures(&self) -> &[DayFailure] {
        &self.failures
    }

    /// Whether `day` must be requested.
    ///
    /// Three conditions put a day back on the queue: an index that covers it
    /// has no archived samples, the archived Kp holds a nowcast value GFZ
    /// replaces with a definitive one later, or the day is still running and
    /// so has periods left to publish. A past day archived from definitive
    /// values is never requested again, an empty one included: the service
    /// published nothing for it and will not start.
    fn day_needs_fetch(
        &self,
        day: NaiveDate,
        today_utc: NaiveDate,
    ) -> Result<bool, SolarStoreError> {
        let Some(store) = self.store.as_deref() else {
            return Ok(false);
        };
        for index in calendar::fetchable_indices(day, today_utc) {
            if !store.contains(index, day)? {
                return Ok(true);
            }
        }
        if day >= today_utc {
            return Ok(true);
        }
        Ok(store
            .kp_series(day)?
            .is_some_and(|kp| kp.contains_nowcast_samples()))
    }

    /// Apply finished fetches and start the next queued day.
    pub fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.in_flight = None;
            if let Some(backfill) = self.backfill.as_mut() {
                backfill.retire(message.day());
                if backfill.is_finished() {
                    self.backfill = None;
                }
            }
            match message {
                IndexDayMessage::Stored {
                    day,
                    kp_samples,
                    hp30_samples,
                } => {
                    if let Some(coverage) = self.recording_days.get_mut(&day) {
                        *coverage = RecordingDayCoverage::Archived;
                    }
                    if kp_samples + hp30_samples == 0 {
                        log::info!("The service published no geomagnetic indices for {day}");
                    } else {
                        log::info!(
                            "Archived {kp_samples} Kp and {hp30_samples} Hp30 samples for {day}"
                        );
                    }
                }
                IndexDayMessage::Failed { day, detail } => {
                    log::error!("No geomagnetic indices archived for {day}: {detail}");
                    self.failures.push(DayFailure { day, detail });
                }
            }
        }
        self.start_next();
    }

    /// Point the scheduler at `base_url`.
    ///
    /// A changed host drops the queue, `seen`, the running backfill, and the
    /// failures: they belong to the old host. Archived days are kept - a day
    /// already archived does not depend on which host served it.
    pub fn set_base_url(&mut self, base_url: &str) {
        if self.base_url == base_url {
            return;
        }
        base_url.clone_into(&mut self.base_url);
        self.http = None;
        self.queue.clear();
        self.seen.clear();
        self.failures.clear();
        self.backfill = None;
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
        let (Some(store), Some(day)) = (
            self.store.as_ref().map(Arc::clone),
            self.queue.front().copied(),
        ) else {
            return;
        };
        let transport = self.transport();
        self.queue.pop_front();
        self.in_flight = Some(day);
        spawn_fetch(
            self.ctx.clone(),
            self.tx.clone(),
            transport,
            store,
            self.base_url.clone(),
            day,
        );
    }

    /// The transport to fetch on, opened once and kept until the host changes.
    ///
    /// A transport that cannot be opened stands in as the offline one for this
    /// dispatch only, and the day fails through the worker like any other
    /// failure. The stand-in is not cached, so the next dispatch tries to open
    /// a real one again.
    fn transport(&mut self) -> Arc<Connection> {
        if let Some(http) = self.http.as_ref() {
            return Arc::clone(http);
        }
        match self
            .transport_source
            .connect(Some(transport::REQUEST_INTERVAL))
        {
            Ok(connection) => {
                let http = Arc::new(connection);
                self.http = Some(Arc::clone(&http));
                http
            }
            Err(err) => {
                log::error!("Geomagnetic index transport unavailable: {err}");
                Arc::new(Connection::Offline(OfflineTransport))
            }
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "thread spawn can only fail under extreme system resource exhaustion"
)]
fn spawn_fetch(
    ctx: Context,
    tx: mpsc::Sender<IndexDayMessage>,
    transport: Arc<Connection>,
    store: Arc<SolarStore>,
    base_url: String,
    day: NaiveDate,
) {
    thread::Builder::new()
        .name(format!("solar-{day}"))
        .spawn(move || {
            let message = ingest(transport.as_ref(), &store, &base_url, day);
            tx.send(message).ok();
            ctx.request_repaint();
        })
        .expect("failed to spawn geomagnetic index worker thread");
}

/// The series of one day, as far as the fetch got.
#[derive(Default)]
struct FetchedDay {
    kp: Option<KpSeries>,
    hp30: Option<Hp30Series>,
}

/// Fetch every index covering `day`, parse them, and add them to the archive.
///
/// A day is archived whole. One index failing to arrive or to parse fails the
/// whole day, and a later session requests it again.
fn ingest(
    transport: &impl Transport,
    store: &SolarStore,
    base_url: &str,
    day: NaiveDate,
) -> IndexDayMessage {
    let window = TimeWindow::covering_utc_day(day);
    let mut fetched = FetchedDay::default();

    for index in calendar::fetchable_indices(day, Utc::now().date_naive()) {
        let body = match transport::fetch_index_window(transport, base_url, index, window) {
            Ok(body) => body,
            Err(failure) => {
                return IndexDayMessage::Failed {
                    day,
                    detail: format!("{index}: {failure}"),
                };
            }
        };
        match index {
            GeomagneticIndex::Kp => match wire::parse_kp_series(&body) {
                Ok(series) => fetched.kp = Some(series),
                Err(err) => {
                    return IndexDayMessage::Failed {
                        day,
                        detail: format!("{index}: {err}"),
                    };
                }
            },
            GeomagneticIndex::Hp30 => match wire::parse_hp30_series(&body) {
                Ok(series) => fetched.hp30 = Some(series),
                Err(err) => {
                    return IndexDayMessage::Failed {
                        day,
                        detail: format!("{index}: {err}"),
                    };
                }
            },
        }
    }

    let fetched_at = Utc::now();
    let mut kp_samples = 0;
    let mut hp30_samples = 0;
    if let Some(series) = fetched.kp.as_ref() {
        if let Err(err) = store.insert_or_replace_kp_day(day, base_url, fetched_at, series) {
            return IndexDayMessage::Failed {
                day,
                detail: err.to_string(),
            };
        }
        kp_samples = series.samples.len();
    }
    if let Some(series) = fetched.hp30.as_ref() {
        if let Err(err) = store.insert_or_replace_hp30_day(day, base_url, fetched_at, series) {
            return IndexDayMessage::Failed {
                day,
                detail: err.to_string(),
            };
        }
        hp30_samples = series.samples.len();
    }
    IndexDayMessage::Stored {
        day,
        kp_samples,
        hp30_samples,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use chrono::{DateTime, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use gt_fetch::{HttpRequest, HttpResponse, TransportError};
    use gt_solar::DEFAULT_BASE_URL;
    use gt_solar::series::{KpSample, KpStatus};
    use gt_store::Store;

    use super::*;

    /// One period of each index, so the same body parses as either.
    const ONE_PERIOD_OF_BOTH_INDICES: &str = r#"{"Kp":[2.667],"Hp30":[3.0],
        "datetime":["2026-07-20T00:00:00Z"],"status":["def"]}"#;

    /// A window the service has no values for.
    const NO_VALUES: &str = r#"{"Kp":[],"Hp30":[],"datetime":[],"status":[]}"#;

    fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .map(|naive| naive.and_utc())
            .unwrap_or_default()
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn archive() -> (TempDir, Arc<SolarStore>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(dir.path())
            .open_geomagnetic_indices()
            .expect("archive");
        (dir, store)
    }

    /// Archive-backed, and wired so no request leaves the machine.
    fn scheduler_with_archive() -> (TempDir, Arc<SolarStore>, GeomagneticIndexScheduler) {
        let (dir, store) = archive();
        let scheduler = GeomagneticIndexScheduler::new(
            Context::default(),
            Some(Arc::clone(&store)),
            DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
        );
        (dir, store, scheduler)
    }

    /// A scheduler with no archive to write to, so it fetches nothing.
    fn scheduler_without_archive() -> GeomagneticIndexScheduler {
        GeomagneticIndexScheduler::new(
            Context::default(),
            None,
            DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
        )
    }

    /// Answers every request with one canned response, recording the URLs.
    struct CannedTransport {
        status: u16,
        body: String,
        urls: RefCell<Vec<String>>,
    }

    impl CannedTransport {
        fn serving(body: &str) -> Self {
            Self {
                status: 200,
                body: body.to_owned(),
                urls: RefCell::new(Vec::new()),
            }
        }

        fn requested_indices(&self) -> Vec<String> {
            self.urls
                .borrow()
                .iter()
                .filter_map(|url| url.split("index=").nth(1).map(str::to_owned))
                .collect()
        }
    }

    impl Transport for CannedTransport {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
            self.urls.borrow_mut().push(request.url().to_owned());
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    fn kp_day(status: KpStatus) -> KpSeries {
        KpSeries {
            samples: vec![KpSample {
                period_start: at(2026, 7, 20, 0),
                activity: gt_solar::activity::GeomagneticActivity::from_published_value(
                    GeomagneticIndex::Kp,
                    2.667,
                ),
                status,
            }],
        }
    }

    /// Archive `day` as a whole definitive day, the way a finished ingest
    /// leaves it.
    fn archive_definitive_day(store: &SolarStore, archived: NaiveDate) {
        store
            .insert_or_replace_kp_day(archived, "host", Utc::now(), &kp_day(KpStatus::Definitive))
            .expect("insert kp");
        store
            .insert_or_replace_hp30_day(
                archived,
                "host",
                Utc::now(),
                &Hp30Series { samples: vec![] },
            )
            .expect("insert hp30");
    }

    #[test]
    fn a_scheduler_without_an_archive_queues_nothing() {
        let mut scheduler = scheduler_without_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
        assert!(scheduler.failures().is_empty());
    }

    #[test]
    fn a_day_before_every_index_begins_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(1900, 1, 1, 0), at(1900, 1, 1, 1)));
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
    }

    #[test]
    fn a_day_in_the_future_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let ahead = Utc::now() + TimeDelta::days(3);
        scheduler.request_days_for(TimeRange::new(ahead, ahead));
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
    }

    /// A past day whose archived Kp is definitive is settled, so nothing goes
    /// out for it.
    #[test]
    fn a_definitive_archived_day_is_not_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_definitive_day(&store, day(2026, 7, 20));

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
    }

    /// A day the service published nothing for is archived empty, and an
    /// empty past day is as final as a valued one.
    #[test]
    fn an_archived_day_without_values_is_not_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        store
            .insert_or_replace_kp_day(archived, "host", Utc::now(), &KpSeries { samples: vec![] })
            .expect("insert kp");
        store
            .insert_or_replace_hp30_day(
                archived,
                "host",
                Utc::now(),
                &Hp30Series { samples: vec![] },
            )
            .expect("insert hp30");

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.queued(), 0);
    }

    /// A nowcast Kp value is replaced by a definitive one later, so the day
    /// goes out again.
    #[test]
    fn a_day_archived_from_nowcast_values_is_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        store
            .insert_or_replace_kp_day(archived, "host", Utc::now(), &kp_day(KpStatus::Nowcast))
            .expect("insert kp");
        store
            .insert_or_replace_hp30_day(
                archived,
                "host",
                Utc::now(),
                &Hp30Series { samples: vec![] },
            )
            .expect("insert hp30");

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert!(scheduler.is_fetching(), "the archived day is dispatched");
    }

    /// The current day has periods left to publish, so an archived copy of it
    /// is never the final one.
    #[test]
    fn the_current_day_is_requested_even_when_archived() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let today = Utc::now().date_naive();
        archive_definitive_day(&store, today);

        scheduler.request_days_for(TimeRange::new(Utc::now(), Utc::now()));
        assert!(scheduler.is_fetching());
    }

    /// One index archived without the other is a half-written day, so it goes
    /// out again.
    #[test]
    fn a_day_missing_one_index_is_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        store
            .insert_or_replace_kp_day(
                day(2026, 7, 20),
                "host",
                Utc::now(),
                &kp_day(KpStatus::Definitive),
            )
            .expect("insert kp");

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert!(scheduler.is_fetching());
    }

    /// A recording is requested once. Loading it again asks for nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let span = TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17));
        scheduler.request_days_for(span);
        let after_first = scheduler.seen.len();
        scheduler.request_days_for(span);
        assert_eq!(scheduler.seen.len(), after_first);
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is the
    /// backfill feature's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 6, 1, 0), at(2026, 7, 20, 0)));
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.seen.is_empty());
    }

    /// A queued day is always dispatched, offline included: the transport
    /// declines the request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.is_fetching());
    }

    /// A changed host drops what belonged to the old one.
    #[test]
    fn changing_the_host_drops_the_queue() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        scheduler.set_base_url("https://mirror.example");

        assert!(scheduler.seen.is_empty(), "the old host's requests");
        assert_eq!(scheduler.queued(), 0);
    }

    #[test]
    fn setting_the_same_host_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        let seen = scheduler.seen.len();

        scheduler.set_base_url(DEFAULT_BASE_URL);

        assert_eq!(scheduler.seen.len(), seen);
    }

    /// A failure reaches the settings section's list.
    #[test]
    fn a_failed_day_is_reported() {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .tx
            .send(IndexDayMessage::Failed {
                day: day(2026, 7, 20),
                detail: "HTTP 500 Internal Server Error".to_owned(),
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(
            scheduler.failures(),
            [DayFailure {
                day: day(2026, 7, 20),
                detail: "HTTP 500 Internal Server Error".to_owned(),
            }]
        );
    }

    /// Days the archive already holds in final form are not queued.
    #[test]
    fn a_backfill_queues_only_the_days_the_archive_lacks() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for archived in [day(2026, 7, 21), day(2026, 7, 22)] {
            archive_definitive_day(&store, archived);
        }

        let queued = scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26));
        assert_eq!(queued, Some(5), "seven days in range, two already held");
    }

    /// Re-running a backfill over a range already downloaded costs nothing.
    #[test]
    fn a_fully_archived_range_queues_nothing() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for offset in 20..=26 {
            archive_definitive_day(&store, day(2026, 7, offset));
        }

        assert_eq!(
            scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26)),
            Some(0)
        );
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// Nothing is requested for a range before Kp begins.
    #[test]
    fn a_backfill_before_coverage_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(
            scheduler.backfill(day(1900, 1, 1), day(1931, 12, 31)),
            Some(0)
        );
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// A missing archive reports [`None`], which the control words differently
    /// from the `Some(0)` of a range that is already downloaded.
    #[test]
    fn a_backfill_without_an_archive_reports_no_archive() {
        let mut scheduler = scheduler_without_archive();
        assert!(!scheduler.archive_available());
        assert_eq!(scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26)), None);
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// Fill the queue and the backfill without dispatching, so the tests
    /// below do not depend on whether a transport can be built.
    fn queued_backfill(scheduler: &mut GeomagneticIndexScheduler, days: &[NaiveDate]) {
        for day in days {
            scheduler.seen.insert(*day);
            scheduler.queue.push_back(*day);
        }
        scheduler.backfill = Some(PendingBackfill::new(days.iter().copied().collect()));
    }

    /// A range of failing days still reaches its total: every outcome retires
    /// its day.
    #[rstest]
    #[case::stored(IndexDayMessage::Stored { day: day(2026, 7, 20), kp_samples: 8, hp30_samples: 48 })]
    #[case::failed(IndexDayMessage::Failed { day: day(2026, 7, 20), detail: "boom".to_owned() })]
    fn progress_advances_on_every_outcome(#[case] message: IndexDayMessage) {
        let mut scheduler = scheduler_without_archive();
        queued_backfill(&mut scheduler, &[day(2026, 7, 20), day(2026, 7, 21)]);
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

    /// The section stops showing a bar once the last day retires the
    /// backfill.
    #[test]
    fn the_last_day_ends_the_backfill() {
        let mut scheduler = scheduler_without_archive();
        queued_backfill(&mut scheduler, &[day(2026, 7, 20)]);

        scheduler
            .tx
            .send(IndexDayMessage::Failed {
                day: day(2026, 7, 20),
                detail: "boom".to_owned(),
            })
            .expect("send");
        scheduler.poll();
        assert_eq!(scheduler.backfill_progress(), None);
    }

    /// Cancelling drops the queued days and lets a later backfill request
    /// them again.
    #[test]
    fn cancelling_releases_the_queued_days() {
        let mut scheduler = scheduler_without_archive();
        queued_backfill(&mut scheduler, &[day(2026, 7, 20), day(2026, 7, 21)]);

        scheduler.cancel_backfill();
        assert_eq!(scheduler.backfill_progress(), None);
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.seen.is_empty(), "cancelled days can be re-queued");
    }

    /// Cancelling must not release the day already being fetched: releasing
    /// it lets a later request go out for a day still in flight.
    #[test]
    fn cancelling_keeps_the_in_flight_day() {
        let mut scheduler = scheduler_without_archive();
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
        let mut scheduler = scheduler_without_archive();
        let track_day = day(2026, 7, 19);
        scheduler.seen.insert(track_day);
        scheduler.queue.push_back(track_day);
        queued_backfill(&mut scheduler, &[day(2026, 7, 20)]);

        scheduler.cancel_backfill();
        assert_eq!(scheduler.queued(), 1);
        assert!(scheduler.seen.contains(&track_day));
    }

    /// Changing the host abandons a backfill and the failures the old host
    /// produced.
    #[test]
    fn changing_the_host_abandons_the_backfill_and_its_failures() {
        let mut scheduler = scheduler_without_archive();
        queued_backfill(&mut scheduler, &[day(2026, 7, 20)]);
        scheduler.failures.push(DayFailure {
            day: day(2026, 7, 20),
            detail: "HTTP 500 Internal Server Error".to_owned(),
        });

        scheduler.set_base_url("https://mirror.example");

        assert_eq!(scheduler.backfill_progress(), None);
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.failures().is_empty());
    }

    /// The status reports the day in flight, the queue behind it, and how much
    /// of what is loaded the archive holds.
    #[test]
    fn the_status_reports_the_queue_and_the_archived_recording_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_definitive_day(&store, day(2026, 7, 20));

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 21, 17)));

        assert_eq!(
            scheduler.fetch_status(),
            GeomagneticIndexFetchStatus {
                fetching: Some(day(2026, 7, 21)),
                queued: 0,
                recording_days: 2,
                archived_recording_days: 1,
            }
        );
    }

    /// An archived day moves the loaded recording's coverage up.
    #[test]
    fn archiving_a_day_covers_the_recording_day_it_belongs_to() {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .recording_days
            .insert(day(2026, 7, 20), RecordingDayCoverage::Awaited);

        scheduler
            .tx
            .send(IndexDayMessage::Stored {
                day: day(2026, 7, 20),
                kp_samples: 8,
                hp30_samples: 48,
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(scheduler.fetch_status().archived_recording_days, 1);
    }

    /// Both indices are requested for one day, and the archive records the
    /// host that served them.
    #[test]
    fn an_ingested_day_archives_every_index_and_the_host() {
        let (_dir, store) = archive();
        let ingested = day(2026, 7, 20);
        let transport = CannedTransport::serving(ONE_PERIOD_OF_BOTH_INDICES);

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, ingested);

        assert!(matches!(
            message,
            IndexDayMessage::Stored {
                kp_samples: 1,
                hp30_samples: 1,
                ..
            }
        ));
        assert_eq!(transport.requested_indices(), ["Kp", "Hp30"]);
        for index in [GeomagneticIndex::Kp, GeomagneticIndex::Hp30] {
            let archived = store.archived_days(index).expect("days");
            assert_eq!(
                archived.first().map(|entry| entry.host.as_str()),
                Some(DEFAULT_BASE_URL),
                "{index}"
            );
        }
    }

    /// Before Hp30 begins, only Kp is asked for, and the day is archived from
    /// what the service does publish.
    #[test]
    fn a_day_before_hp30_begins_archives_kp_alone() {
        let (_dir, store) = archive();
        let ingested = day(1970, 1, 1);
        let transport = CannedTransport::serving(
            r#"{"Kp":[2.667],"datetime":["1970-01-01T00:00:00Z"],"status":["def"]}"#,
        );

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, ingested);

        assert!(matches!(
            message,
            IndexDayMessage::Stored {
                kp_samples: 1,
                hp30_samples: 0,
                ..
            }
        ));
        assert_eq!(transport.requested_indices(), ["Kp"]);
        assert!(
            store
                .archived_days(GeomagneticIndex::Hp30)
                .expect("days")
                .is_empty()
        );
    }

    /// Empty arrays are a published answer, not a failure: the day is archived
    /// with no samples, which is what keeps it from being asked for again.
    #[test]
    fn a_day_without_published_values_is_archived_empty() {
        let (_dir, store) = archive();
        let ingested = day(2026, 7, 20);
        let transport = CannedTransport::serving(NO_VALUES);

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, ingested);

        assert!(matches!(
            message,
            IndexDayMessage::Stored {
                kp_samples: 0,
                hp30_samples: 0,
                ..
            }
        ));
        assert_eq!(
            store.kp_series(ingested).expect("kp"),
            Some(KpSeries { samples: vec![] })
        );
        assert_eq!(
            store.hp30_series(ingested).expect("hp30"),
            Some(Hp30Series { samples: vec![] })
        );
    }

    /// A revised day replaces the archived one instead of appending to it.
    #[test]
    fn ingesting_a_day_twice_replaces_what_was_archived() {
        let (_dir, store) = archive();
        let ingested = day(2026, 7, 20);
        let transport = CannedTransport::serving(ONE_PERIOD_OF_BOTH_INDICES);

        ingest(&transport, &store, DEFAULT_BASE_URL, ingested);
        ingest(&transport, &store, DEFAULT_BASE_URL, ingested);

        assert_eq!(
            store
                .kp_series(ingested)
                .expect("kp")
                .map(|series| series.samples.len()),
            Some(1)
        );
    }

    #[rstest]
    #[case::a_body_that_is_not_a_series(200, "<html>captive portal</html>")]
    #[case::a_server_error(500, "")]
    #[case::a_refused_request(403, "")]
    fn a_day_that_cannot_be_read_archives_nothing(#[case] status: u16, #[case] body: &str) {
        let (_dir, store) = archive();
        let transport = CannedTransport {
            status,
            body: body.to_owned(),
            urls: RefCell::new(Vec::new()),
        };

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, day(2026, 7, 20));

        assert!(matches!(message, IndexDayMessage::Failed { .. }));
        for index in [GeomagneticIndex::Kp, GeomagneticIndex::Hp30] {
            assert!(
                store.archived_days(index).expect("days").is_empty(),
                "{index}"
            );
        }
    }

    /// One index failing leaves the whole day unarchived, so the next session
    /// asks for both again.
    #[test]
    fn a_day_whose_second_index_fails_archives_neither() {
        let (_dir, store) = archive();
        let transport = CannedTransport::serving(
            r#"{"Kp":[2.667],"datetime":["2026-07-20T00:00:00Z"],"status":["def"]}"#,
        );

        let message = ingest(&transport, &store, DEFAULT_BASE_URL, day(2026, 7, 20));

        assert!(
            matches!(message, IndexDayMessage::Failed { .. }),
            "the body carries no Hp30 array"
        );
        assert!(
            store
                .archived_days(GeomagneticIndex::Kp)
                .expect("days")
                .is_empty()
        );
    }
}
