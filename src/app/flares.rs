//! Solar flare fetch worker and archive ingest.
//!
//! Follows [`super::solar`]: owned by the app, a background thread per request
//! reporting over an mpsc channel, `request_repaint` on every message.
//!
//! Every request carries the user's own API key, so without one nothing is
//! queued at all. Loading a track queues the UTC days it spans, and a past day
//! the archive holds is never requested again. One request is in flight at a
//! time, and the transport spaces requests
//! [`transport::REQUEST_INTERVAL`] apart.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, Transport, TransportSource};
use gt_flare::{ApiKey, DateWindow, MarkedFlare, SolarFlare, calendar, transport, wire};
use gt_pending_writes::PendingWrites;
use gt_store::{ArchiveUsage, FlareStore, FlareStoreError};
use gt_types::{SunlitSide, TimeRange};
use gt_ui_types::ArcIdentity;

use super::context_line::{ContextSampleCache, ContextSource, ContextSpan};
use super::day_fetch_queue::DayFetchQueue;
use super::environment_storage::{EnvironmentArchive, PrunedDays};
use super::fix_positions::FixPositionTimeline;

/// What one day's fetch produced.
enum FlareDayMessage {
    /// The day was archived. `flares` is zero for a day the catalog lists
    /// none for.
    Stored {
        day: NaiveDate,
        flares: usize,
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

impl FlareDayMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. }
            | Self::Failed { day, .. }
            | Self::NotArchivedDuringShutdown { day } => day,
        }
    }
}

/// Queues solar flare days and ingests them into the archive.
pub struct SolarFlareScheduler {
    ctx: Context,
    tx: mpsc::Sender<FlareDayMessage>,
    rx: mpsc::Receiver<FlareDayMessage>,
    base_url: String,
    /// `None` disables fetching: the endpoint answers no request without a
    /// key.
    api_key: Option<ApiKey>,
    /// `None` disables fetching: no archive to write to.
    store: Option<Arc<FlareStore>>,
    /// Connected on the first request, and dropped when the endpoint changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here determines whether requests may leave the machine.
    transport_source: TransportSource,
    days: DayFetchQueue,
    /// UTC days the archive holds events for, read once at startup and
    /// extended on ingest, so resolving the markers never reads the day index
    /// per frame. Assumes this process is the archive's only writer.
    archived_days: BTreeSet<NaiveDate>,
    /// The flares of the archived days the plot shows, read once per day.
    markers: ContextSampleCache<MarkedFlare>,
    /// Registers every archive insert, and refuses the ones that would start
    /// after shutdown began.
    pending_writes: PendingWrites,
}

impl SolarFlareScheduler {
    pub fn new(
        ctx: Context,
        store: Option<Arc<FlareStore>>,
        base_url: String,
        api_key: Option<ApiKey>,
        transport_source: TransportSource,
        pending_writes: PendingWrites,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let archived_days = store
            .as_ref()
            .map(|store| archived_days_of(store))
            .unwrap_or_default();
        Self {
            archived_days,
            markers: ContextSampleCache::default(),
            ctx,
            tx,
            rx,
            base_url,
            api_key,
            store,
            http: None,
            transport_source,
            days: DayFetchQueue::default(),
            pending_writes,
        }
    }

    /// Queue the days a recording spans.
    ///
    /// Days already archived, outside the catalog's coverage, or already
    /// queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing, and so does a session
    /// without an API key.
    pub fn request_days_for(&mut self, range: TimeRange) {
        let Some(store) = self.fetchable_archive() else {
            return;
        };
        let Some(days) = range.utc_days(calendar::MAX_DAYS_PER_TRACK) else {
            log::info!(
                "A recording spanning {} is past the {}-day limit; no solar flare days queued",
                range.duration(),
                calendar::MAX_DAYS_PER_TRACK
            );
            return;
        };
        let today = Utc::now().date_naive();
        for day in days {
            if calendar::day_outlook(day, today) != calendar::DayOutlook::Fetchable {
                continue;
            }
            self.days
                .request_recording_day(day, day_needs_fetch(&store, day, today));
        }
        self.start_next();
    }

    /// Queue every day in `from..=to` the archive does not already hold, as
    /// one backfill.
    ///
    /// Returns how many days were queued, or [`None`] when there is nothing to
    /// fetch with: no archive to write to, or no API key to request with.
    pub fn backfill(&mut self, from: NaiveDate, to: NaiveDate) -> Option<usize> {
        let store = self.fetchable_archive()?;
        let today = Utc::now().date_naive();
        let total = self
            .days
            .start_backfill(calendar::fetchable_days(from, to, today), |day| {
                day_needs_fetch(&store, day, today)
            });
        log::info!("Backfilling solar flares for {total} days between {from} and {to}");
        self.start_next();
        Some(total)
    }

    /// Whether there is an archive to download into. Grays the backfill
    /// control when there is not.
    pub fn archive_available(&self) -> bool {
        self.store.is_some()
    }

    /// The archive, for the settings page to report and delete from.
    pub fn archive(&self) -> Option<Arc<FlareStore>> {
        self.store.as_ref().map(Arc::clone)
    }

    /// What the archive holds, as the environment storage rows show it.
    pub fn archive_usage(&self) -> Option<ArchiveUsage> {
        let store = self.store.as_ref()?;
        Some(ArchiveUsage::measure(
            store.path(),
            self.archived_days.iter().copied(),
        ))
    }

    /// How many archived days a delete of `pruned` would remove.
    pub fn archived_days_covered(&self, pruned: PrunedDays) -> usize {
        pruned.count_covered(self.archived_days.iter().copied())
    }

    /// Drop what this scheduler holds for the days a delete removed from the
    /// archive.
    pub fn forget_pruned_days(&mut self, pruned: PrunedDays) {
        self.archived_days.retain(|day| !pruned.covers(*day));
        self.markers.forget_pruned_days(pruned);
        self.days.forget_pruned_days(pruned);
    }

    /// Whether a key has been entered. Grays every control that would send a
    /// request when it has not.
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
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
                FlareDayMessage::Stored { day, flares } => {
                    self.archived_days.insert(day);
                    self.days.mark_archived(day);
                    self.markers.forget(day);
                    log::info!(
                        "Archived {flares} {} for {day}",
                        gt_fmt::pluralize(flares, "solar flare", "solar flares")
                    );
                }
                FlareDayMessage::Failed { day, detail } => {
                    log::error!("No solar flares archived for {day}: {detail}");
                    self.days.report_failure(day, detail);
                }
                FlareDayMessage::NotArchivedDuringShutdown { day } => {
                    log::debug!("No solar flares archived for {day}: shutting down");
                }
            }
        }
        self.start_next();
    }

    /// Point the scheduler at `base_url`.
    ///
    /// A changed host drops the queue, the days requested of the old host, its
    /// failures and the running backfill. Archived days are kept - a day
    /// already archived does not depend on which host served it.
    pub fn set_base_url(&mut self, base_url: &str) {
        if self.base_url == base_url {
            return;
        }
        base_url.clone_into(&mut self.base_url);
        self.http = None;
        self.days.forget_host();
    }

    /// Fetch with `api_key` from now on.
    ///
    /// A changed key drops the queue and the failures the old key produced: a
    /// day refused as unauthorized is worth requesting again under the new
    /// one.
    pub fn set_api_key(&mut self, api_key: Option<ApiKey>) {
        if self.api_key == api_key {
            return;
        }
        self.api_key = api_key;
        self.days.forget_host();
    }

    /// The flares to mark across `span`, from the archived days it covers,
    /// each with the side of Earth the receiver was on when it peaked.
    pub fn markers(
        &mut self,
        span: ContextSpan,
        positions: &Arc<FixPositionTimeline>,
    ) -> Arc<Vec<MarkedFlare>> {
        let source = ContextSource {
            span,
            archived_days: self.archived_days.range(span.days()).copied().collect(),
            positions: Some(ArcIdentity::of(positions)),
        };
        let store = self.store.as_ref().map(Arc::clone);
        let positions = Arc::clone(positions);
        self.markers.resolve(
            source,
            |day| {
                store
                    .as_deref()
                    .and_then(|store| read_archived_flares(store, day))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|flare| mark_with_receiver_side(flare, &positions))
                    .collect()
            },
            |_uncovered| None,
        )
    }

    /// The UTC days a flare peaking inside `range` can be archived under,
    /// oldest first.
    ///
    /// The catalog files a flare under the day it began, so a flare peaking
    /// just after midnight belongs to the day before its peak.
    pub fn archived_days_for(&self, range: TimeRange) -> Vec<NaiveDate> {
        let first = range
            .start
            .date_naive()
            .pred_opt()
            .unwrap_or(NaiveDate::MIN);
        self.archived_days
            .range(first..=range.end.date_naive())
            .copied()
            .collect()
    }

    /// The archived flares peaking inside `range`, each with the side of
    /// Earth the receiver was on at that peak.
    pub fn flares_peaking_in(
        &self,
        range: TimeRange,
        positions: &FixPositionTimeline,
    ) -> Vec<MarkedFlare> {
        let Some(store) = self.store.as_deref() else {
            return Vec::new();
        };
        self.archived_days_for(range)
            .into_iter()
            .filter_map(|day| read_archived_flares(store, day))
            .flatten()
            .filter(|flare| (range.start..=range.end).contains(&flare.peak))
            .map(|flare| mark_with_receiver_side(flare, positions))
            .collect()
    }

    /// The archive to fetch into, or [`None`] when nothing may be requested.
    fn fetchable_archive(&self) -> Option<Arc<FlareStore>> {
        self.api_key.as_ref()?;
        self.store.as_ref().map(Arc::clone)
    }

    fn start_next(&mut self) {
        let Some(store) = self.fetchable_archive() else {
            return;
        };
        let Some(key) = self.api_key.clone() else {
            return;
        };
        if self.pending_writes.is_shutting_down() {
            return;
        }
        let Some(day) = self.days.take_next_day() else {
            return;
        };
        let transport = self.transport();
        let endpoint = Endpoint {
            base_url: self.base_url.clone(),
            key,
        };
        self.spawn_fetch(transport, store, endpoint, day);
    }

    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_fetch(
        &self,
        transport: Arc<Connection>,
        store: Arc<FlareStore>,
        endpoint: Endpoint,
        day: NaiveDate,
    ) {
        let ctx = self.ctx.clone();
        let tx = self.tx.clone();
        let pending_writes = self.pending_writes.clone();
        thread::Builder::new()
            .name(format!("flares-{day}"))
            .spawn(move || {
                let message = ingest(transport.as_ref(), &store, &endpoint, day, &pending_writes);
                tx.send(message).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn solar flare worker thread");
    }

    /// The transport to fetch on, opened once and kept until the endpoint
    /// changes.
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
                log::error!("Solar flare transport unavailable: {err}");
                Arc::new(Connection::Offline(OfflineTransport))
            }
        }
    }
}

/// Where one day's request goes, and with which key.
struct Endpoint {
    base_url: String,
    key: ApiKey,
}

/// Whether `day` must be requested.
///
/// A day the archive does not hold is fetched, and so is the current day,
/// which the catalog is still submitting events for. A past day is final, a
/// day without flares included: DONKI back-publishes rarely enough that a
/// second request for a settled day costs more than it finds.
fn day_needs_fetch(
    store: &FlareStore,
    day: NaiveDate,
    today_utc: NaiveDate,
) -> Result<bool, FlareStoreError> {
    Ok(day >= today_utc || !store.contains(day)?)
}

/// One flare with the side of Earth the receiver was on at its peak, read at
/// the position of the fix nearest that instant. A flare no loaded recording
/// places the receiver at is marked without a side.
fn mark_with_receiver_side(flare: SolarFlare, positions: &FixPositionTimeline) -> MarkedFlare {
    let receiver_side = positions
        .nearest_position(flare.peak)
        .map(|(latitude, longitude)| SunlitSide::at_position(latitude, longitude, flare.peak));
    MarkedFlare {
        flare,
        receiver_side,
    }
}

/// The archived flares of one day, reporting a read that failed and treating
/// it as an unarchived day.
fn read_archived_flares(store: &FlareStore, day: NaiveDate) -> Option<Vec<SolarFlare>> {
    store
        .flares(day)
        .inspect_err(|err| log::error!("Reading archived solar flares for {day}: {err}"))
        .ok()
        .flatten()
}

/// Every day the archive holds a fetched catalog day for.
fn archived_days_of(store: &FlareStore) -> BTreeSet<NaiveDate> {
    store
        .archived_days()
        .inspect_err(|err| log::error!("Reading the solar flare archive index: {err}"))
        .into_iter()
        .flatten()
        .map(|archived| archived.day)
        .collect()
}

/// Fetch `day`, parse it, and add it to the archive.
///
/// Only the flares beginning on `day` are stored, which is the day the
/// catalog lists them under: an event answered outside the requested day
/// would be archived twice once its own day is fetched.
fn ingest(
    transport: &impl Transport,
    store: &FlareStore,
    endpoint: &Endpoint,
    day: NaiveDate,
    pending_writes: &PendingWrites,
) -> FlareDayMessage {
    let window = DateWindow::covering_utc_day(day);
    let body =
        match transport::fetch_flare_window(transport, &endpoint.base_url, &endpoint.key, window) {
            Ok(body) => body,
            Err(failure) => {
                return FlareDayMessage::Failed {
                    day,
                    detail: failure.to_string(),
                };
            }
        };
    let flares: Vec<SolarFlare> = match wire::parse_flares(&body) {
        Ok(flares) => flares
            .into_iter()
            .filter(|flare| flare.begin_day() == day)
            .collect(),
        Err(err) => {
            return FlareDayMessage::Failed {
                day,
                detail: err.to_string(),
            };
        }
    };

    let Some(_write) = EnvironmentArchive::SolarFlares.try_begin_day_insert(pending_writes, day)
    else {
        return FlareDayMessage::NotArchivedDuringShutdown { day };
    };
    // The key is never part of what the archive records.
    match store.insert_or_replace_day(day, &endpoint.base_url, Utc::now(), &flares) {
        Ok(()) => FlareDayMessage::Stored {
            day,
            flares: flares.len(),
        },
        Err(err) => FlareDayMessage::Failed {
            day,
            detail: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use gt_fetch::HttpResponse;
    use gt_flare::DEFAULT_BASE_URL;
    use gt_store::Store;
    use gt_test_utils::ScriptedTransport;
    use gt_types::{Latitude, Longitude};

    use crate::app::day_failures::DayFailure;
    use crate::app::day_fetch_status::{ArchivedDayCount, DayFetchStatus};
    use crate::app::fix_positions::FixPositions;

    use super::*;

    /// One day of the May 2024 storm, as the catalog answers it.
    const ONE_FLARE: &str = r#"[{"flrID":"2024-05-09T08:45:00-FLR-001",
        "beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
        "endTime":"2024-05-09T09:36Z","classType":"X2.2",
        "sourceLocation":"S20W25","activeRegionNum":13664}]"#;

    /// A day the catalog lists nothing for.
    const NO_FLARES: &str = "[]";

    const TEST_KEY: &str = "test-key";

    fn key() -> Option<ApiKey> {
        ApiKey::new(TEST_KEY)
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

    fn archive() -> (TempDir, Arc<FlareStore>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(dir.path())
            .open_solar_flares()
            .expect("archive");
        (dir, store)
    }

    /// Archive-backed and keyed, and wired so no request leaves the machine.
    fn scheduler_with_archive() -> (TempDir, Arc<FlareStore>, SolarFlareScheduler) {
        let (dir, store) = archive();
        let scheduler = SolarFlareScheduler::new(
            Context::default(),
            Some(Arc::clone(&store)),
            DEFAULT_BASE_URL.to_owned(),
            key(),
            TransportSource::Offline,
            PendingWrites::default(),
        );
        (dir, store, scheduler)
    }

    /// A scheduler with no archive to write to, so it fetches nothing.
    fn scheduler_without_archive() -> SolarFlareScheduler {
        SolarFlareScheduler::new(
            Context::default(),
            None,
            DEFAULT_BASE_URL.to_owned(),
            key(),
            TransportSource::Offline,
            PendingWrites::default(),
        )
    }

    fn serving(body: &str) -> ScriptedTransport<String> {
        ScriptedTransport::always(Ok(HttpResponse {
            status: 200,
            body: body.to_owned(),
        }))
    }

    fn endpoint() -> Endpoint {
        Endpoint {
            base_url: DEFAULT_BASE_URL.to_owned(),
            key: ApiKey::new(TEST_KEY).expect("a key"),
        }
    }

    fn a_recording_day() -> TimeRange {
        TimeRange::new(at(2024, 5, 9, 8), at(2024, 5, 9, 17))
    }

    /// Without a key the endpoint answers nothing, so nothing is requested and
    /// no failure is reported for a day that never went out.
    #[test]
    fn a_scheduler_without_a_key_queues_nothing() {
        let (_dir, store) = archive();
        let mut scheduler = SolarFlareScheduler::new(
            Context::default(),
            Some(store),
            DEFAULT_BASE_URL.to_owned(),
            None,
            TransportSource::Offline,
            PendingWrites::default(),
        );

        scheduler.request_days_for(a_recording_day());

        assert!(!scheduler.has_api_key());
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
        assert!(scheduler.days.failures().is_empty());
        assert_eq!(scheduler.backfill(day(2024, 5, 1), day(2024, 5, 9)), None);
    }

    /// Entering a key is what starts the fetching, so the day a loaded
    /// recording spans goes out once one is set.
    #[test]
    fn a_key_entered_later_lets_the_next_recording_queue_its_days() {
        let (_dir, store) = archive();
        let mut scheduler = SolarFlareScheduler::new(
            Context::default(),
            Some(store),
            DEFAULT_BASE_URL.to_owned(),
            None,
            TransportSource::Offline,
            PendingWrites::default(),
        );
        scheduler.request_days_for(a_recording_day());
        assert!(!scheduler.days.is_fetching());

        scheduler.set_api_key(key());
        scheduler.request_days_for(a_recording_day());

        assert!(scheduler.has_api_key());
        assert!(scheduler.days.is_fetching());
    }

    #[test]
    fn a_scheduler_without_an_archive_queues_nothing() {
        let mut scheduler = scheduler_without_archive();
        scheduler.request_days_for(a_recording_day());
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
    }

    #[test]
    fn a_day_before_the_catalog_begins_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2009, 1, 1, 0), at(2009, 1, 1, 1)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
    }

    #[test]
    fn a_day_in_the_future_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let ahead = Utc::now() + TimeDelta::days(3);
        scheduler.request_days_for(TimeRange::new(ahead, ahead));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
    }

    /// A past day the archive holds is settled, so nothing goes out for it.
    #[test]
    fn an_archived_day_is_not_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        store
            .insert_or_replace_day(day(2024, 5, 9), "host", Utc::now(), &[])
            .expect("archive the day");

        scheduler.request_days_for(a_recording_day());

        assert_eq!(scheduler.days.queued(), 0);
        assert!(
            !scheduler.days.is_fetching(),
            "a day the catalog listed no flare for is as final as one with flares"
        );
    }

    /// The current day is still being submitted to, so an archived copy of it
    /// is never the final one.
    #[test]
    fn the_current_day_is_requested_even_when_archived() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let today = Utc::now().date_naive();
        store
            .insert_or_replace_day(today, "host", Utc::now(), &[])
            .expect("archive today");

        scheduler.request_days_for(TimeRange::new(Utc::now(), Utc::now()));
        assert!(scheduler.days.is_fetching());
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is the
    /// backfill feature's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 4, 1, 0), at(2024, 5, 9, 0)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.requested_days().is_empty());
    }

    /// A changed endpoint drops what belonged to the old one.
    #[rstest]
    #[case::a_changed_host(|scheduler: &mut SolarFlareScheduler| {
        scheduler.set_base_url("https://proxy.example");
    })]
    #[case::a_changed_key(|scheduler: &mut SolarFlareScheduler| {
        scheduler.set_api_key(ApiKey::new("another-key"));
    })]
    fn changing_the_endpoint_drops_the_queue_and_its_failures(
        #[case] change: fn(&mut SolarFlareScheduler),
    ) {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(a_recording_day());
        scheduler
            .days
            .report_failure(day(2024, 5, 9), "HTTP 403 Forbidden".to_owned());

        change(&mut scheduler);

        assert!(
            scheduler.days.requested_days().is_empty(),
            "the old endpoint's requests"
        );
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.failures().is_empty());
    }

    #[test]
    fn setting_the_same_endpoint_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(a_recording_day());
        let seen = scheduler.days.requested_days().len();

        scheduler.set_base_url(DEFAULT_BASE_URL);
        scheduler.set_api_key(key());

        assert_eq!(scheduler.days.requested_days().len(), seen);
    }

    /// A failure reaches the settings page's list.
    #[test]
    fn a_failed_day_is_reported() {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .tx
            .send(FlareDayMessage::Failed {
                day: day(2024, 5, 9),
                detail: "HTTP 403 Forbidden".to_owned(),
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(
            scheduler.days.failures(),
            [DayFailure {
                day: day(2024, 5, 9),
                detail: "HTTP 403 Forbidden".to_owned(),
            }]
        );
    }

    /// Days the archive already holds are not queued.
    #[test]
    fn a_backfill_queues_only_the_days_the_archive_lacks() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for archived in [day(2024, 5, 10), day(2024, 5, 11)] {
            store
                .insert_or_replace_day(archived, "host", Utc::now(), &[])
                .expect("archive");
        }

        let queued = scheduler.backfill(day(2024, 5, 9), day(2024, 5, 15));
        assert_eq!(queued, Some(5), "seven days in range, two already held");
    }

    /// Nothing is requested for a range before the catalog begins.
    #[test]
    fn a_backfill_before_coverage_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(
            scheduler.backfill(day(2009, 1, 1), day(2010, 4, 2)),
            Some(0)
        );
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// The status reports the day in flight and how much of what is loaded the
    /// archive holds.
    #[test]
    fn the_status_reports_the_queue_and_the_archived_recording_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        store
            .insert_or_replace_day(day(2024, 5, 9), "host", Utc::now(), &[])
            .expect("archive");

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 9, 8), at(2024, 5, 10, 17)));

        assert_eq!(
            scheduler.days.fetch_status(),
            DayFetchStatus {
                fetching: Some(day(2024, 5, 10)),
                queued: 0,
                recording_days: ArchivedDayCount {
                    days: 2,
                    archived: 1,
                },
            }
        );
    }

    /// A day is archived with the flares the catalog listed, under the host
    /// that served them.
    #[test]
    fn an_ingested_day_archives_its_flares_and_the_host() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 9);
        let transport = serving(ONE_FLARE);

        let message = ingest(
            &transport,
            &store,
            &endpoint(),
            ingested,
            &PendingWrites::default(),
        );

        assert!(matches!(message, FlareDayMessage::Stored { flares: 1, .. }));
        let archived = store.archived_days().expect("days");
        assert_eq!(
            archived.first().map(|entry| entry.host.as_str()),
            Some(DEFAULT_BASE_URL)
        );
        assert_eq!(
            store
                .flares(ingested)
                .expect("flares")
                .and_then(|flares| flares.first().map(|flare| flare.classification.to_string())),
            Some("X2.2".to_owned())
        );
    }

    /// The request carries the key, and the archive records the host without
    /// it.
    #[test]
    fn the_key_reaches_the_request_and_not_the_archive() {
        let (_dir, store) = archive();
        let transport = serving(NO_FLARES);

        ingest(
            &transport,
            &store,
            &endpoint(),
            day(2024, 5, 9),
            &PendingWrites::default(),
        );

        assert!(
            transport
                .requested_urls()
                .iter()
                .all(|url| url.contains(TEST_KEY)),
            "the endpoint needs the key on every request"
        );
        assert!(
            store
                .archived_days()
                .expect("days")
                .iter()
                .all(|entry| !entry.host.contains(TEST_KEY))
        );
    }

    /// An empty array is a published result, not a failure: the day is
    /// archived with no flares, which is what keeps it from being requested
    /// again.
    #[test]
    fn a_day_without_flares_is_archived_empty() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 9);
        let transport = serving(NO_FLARES);

        let message = ingest(
            &transport,
            &store,
            &endpoint(),
            ingested,
            &PendingWrites::default(),
        );

        assert!(matches!(message, FlareDayMessage::Stored { flares: 0, .. }));
        assert_eq!(store.flares(ingested).expect("flares"), Some(vec![]));
    }

    /// The catalog answers a window, and a window's ends can hold events of
    /// the neighbouring days. Only the requested day's are archived under it.
    #[test]
    fn an_event_of_another_day_is_not_archived_under_this_one() {
        let (_dir, store) = archive();
        let transport = serving(
            r#"[{"flrID":"a","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
                "classType":"X2.2"},
                {"flrID":"b","beginTime":"2024-05-10T00:10Z","peakTime":"2024-05-10T00:13Z",
                "classType":"M1.3"}]"#,
        );

        let message = ingest(
            &transport,
            &store,
            &endpoint(),
            day(2024, 5, 9),
            &PendingWrites::default(),
        );

        assert!(matches!(message, FlareDayMessage::Stored { flares: 1, .. }));
        assert_eq!(
            store
                .flares(day(2024, 5, 9))
                .expect("flares")
                .map(|flares| flares.iter().map(|flare| flare.id.clone()).collect()),
            Some(vec!["a".to_owned()])
        );
    }

    #[rstest]
    #[case::a_body_that_is_not_the_catalog(200, "<html>captive portal</html>")]
    #[case::a_server_error(500, "")]
    #[case::a_rejected_key(403, "")]
    fn a_day_that_cannot_be_read_archives_nothing(#[case] status: u16, #[case] body: &str) {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Ok(HttpResponse {
            status,
            body: body.to_owned(),
        }));

        let message = ingest(
            &transport,
            &store,
            &endpoint(),
            day(2024, 5, 9),
            &PendingWrites::default(),
        );

        assert!(matches!(message, FlareDayMessage::Failed { .. }));
        assert!(store.archived_days().expect("days").is_empty());
    }

    /// A failure a rejected key produced must not carry the key into the
    /// settings page's list.
    #[test]
    fn a_failed_day_reports_no_key() {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Err(gt_fetch::TransportError {
            detail: format!(
                "error sending request for url ({})",
                gt_flare::flare_url(
                    DEFAULT_BASE_URL,
                    DateWindow::covering_utc_day(day(2024, 5, 9)),
                    &ApiKey::new(TEST_KEY).expect("a key"),
                )
            ),
        }));

        let message = ingest(
            &transport,
            &store,
            &endpoint(),
            day(2024, 5, 9),
            &PendingWrites::default(),
        );

        let FlareDayMessage::Failed { detail, .. } = message else {
            panic!("the transport refused every attempt");
        };
        assert!(!detail.contains(TEST_KEY), "{detail}");
        assert!(detail.contains(gt_flare::REDACTED_KEY), "{detail}");
    }

    /// The markers a context span holds, resolved from the archive.
    fn markers_over(
        scheduler: &mut SolarFlareScheduler,
        positions: &Arc<FixPositionTimeline>,
        days: std::ops::RangeInclusive<NaiveDate>,
    ) -> Arc<Vec<MarkedFlare>> {
        let midnight =
            |day: NaiveDate| day.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as f64;
        scheduler.markers(
            ContextSpan::covering(midnight(*days.start())..=midnight(*days.end())),
            positions,
        )
    }

    fn no_recording_loaded() -> Arc<FixPositionTimeline> {
        Arc::new(FixPositionTimeline::default())
    }

    /// A recording of four hourly fixes from 08:00 on the archived day, at the
    /// position the caller places the receiver at.
    fn timeline_at(latitude: Latitude, longitude: Longitude) -> Arc<FixPositionTimeline> {
        let start = at(2024, 5, 9, 8);
        let mut track = gt_test_utils::fixtures::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_walking_from(start, 4, 3600, latitude, longitude),
        );
        track.metadata.time_range = TimeRange::new(start, start + TimeDelta::hours(3));
        let files = vec![gt_types::LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        }];
        let mut positions = FixPositions::default();
        Arc::clone(positions.timeline(&files))
    }

    fn archive_one_flare(store: &FlareStore, day: NaiveDate, class_type: &str) {
        let flare = SolarFlare {
            id: format!("{day}-FLR-001"),
            begin: day.and_hms_opt(8, 45, 0).unwrap_or_default().and_utc(),
            peak: day.and_hms_opt(9, 13, 0).unwrap_or_default().and_utc(),
            end: None,
            classification: class_type.parse().expect("a published class"),
            source_location: None,
            active_region: None,
        };
        store
            .insert_or_replace_day(day, "host", Utc::now(), &[flare])
            .expect("archive");
    }

    /// The side is read at the position of the fix nearest the peak, and stays
    /// absent while no recording places the receiver. The 09:13 peak is
    /// mid-morning over Denmark and late evening over the mid-Pacific.
    #[rstest]
    #[case::recorded_in_daylight(
        Some((Latitude::new(55.0), Longitude::new(12.0))),
        Some(SunlitSide::Sunlit)
    )]
    #[case::recorded_on_the_night_side(
        Some((Latitude::new(0.0), Longitude::new(-170.0))),
        Some(SunlitSide::Night)
    )]
    #[case::no_recording_loaded(None, None)]
    fn a_marker_states_which_side_of_earth_the_receiver_was_on(
        #[case] receiver: Option<(Latitude, Longitude)>,
        #[case] expected: Option<SunlitSide>,
    ) {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 9);
        archive_one_flare(&store, archived, "X2.2");
        scheduler.archived_days.insert(archived);
        let positions = receiver.map_or_else(no_recording_loaded, |(latitude, longitude)| {
            timeline_at(latitude, longitude)
        });

        let markers = markers_over(&mut scheduler, &positions, archived..=archived);

        assert_eq!(
            markers.first().map(|marked| marked.receiver_side),
            Some(expected)
        );
    }

    /// Every archived day in the span contributes its flares, and a day the
    /// archive lacks contributes none.
    #[test]
    fn the_markers_carry_every_archived_flare_of_the_span() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for (archived, class_type) in [(day(2024, 5, 9), "X2.2"), (day(2024, 5, 11), "X5.8")] {
            archive_one_flare(&store, archived, class_type);
            scheduler.archived_days.insert(archived);
        }

        let markers = markers_over(
            &mut scheduler,
            &no_recording_loaded(),
            day(2024, 5, 9)..=day(2024, 5, 11),
        );

        assert_eq!(
            markers
                .iter()
                .map(|marked| marked.flare.classification.to_string())
                .collect::<Vec<_>>(),
            ["X2.2", "X5.8"]
        );
    }

    /// A flare peaking outside the recording is not evidence about it, and
    /// one peaking inside it is read at the receiver's position.
    #[test]
    fn only_the_flares_peaking_over_the_recording_are_read() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 9);
        archive_one_flare(&store, archived, "X2.2");
        scheduler.archived_days.insert(archived);
        let positions = timeline_at(Latitude::new(55.0), Longitude::new(12.0));

        let peaking = scheduler.flares_peaking_in(
            TimeRange::new(at(2024, 5, 9, 9), at(2024, 5, 9, 10)),
            &positions,
        );
        assert_eq!(
            peaking
                .iter()
                .map(|marked| marked.receiver_side)
                .collect::<Vec<_>>(),
            [Some(SunlitSide::Sunlit)],
            "the 09:13 peak falls inside the range"
        );

        assert!(
            scheduler
                .flares_peaking_in(
                    TimeRange::new(at(2024, 5, 9, 10), at(2024, 5, 9, 12)),
                    &positions,
                )
                .is_empty()
        );
    }

    /// The catalog files a flare under the day it began, so the day before a
    /// range is read too: a flare that began before midnight can peak inside
    /// it.
    #[test]
    fn the_day_before_the_range_is_read_for_a_flare_that_crossed_midnight() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        for archived in [day(2024, 5, 9), day(2024, 5, 10)] {
            scheduler.archived_days.insert(archived);
        }

        assert_eq!(
            scheduler.archived_days_for(TimeRange::new(at(2024, 5, 10, 0), at(2024, 5, 10, 6))),
            [day(2024, 5, 9), day(2024, 5, 10)]
        );
    }

    /// A day the fetch worker archives reaches the markers on the next frame,
    /// without reloading the recording.
    #[test]
    fn archiving_a_day_gives_the_plot_its_markers() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 9);
        assert!(
            markers_over(&mut scheduler, &no_recording_loaded(), archived..=archived).is_empty()
        );

        archive_one_flare(&store, archived, "X2.2");
        scheduler
            .tx
            .send(FlareDayMessage::Stored {
                day: archived,
                flares: 1,
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(
            markers_over(&mut scheduler, &no_recording_loaded(), archived..=archived).len(),
            1
        );
    }
}
