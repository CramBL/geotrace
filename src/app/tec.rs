//! TEC map fetch worker and archive ingest.
//!
//! Follows [`super::solar`]: owned by the app, a background thread per
//! request reporting over an mpsc channel, `request_repaint` on every message.
//!
//! Loading a track queues the UTC days it spans, and one day's request walks
//! the configured mirrors and JPL's products until one has a file. A day the
//! archive holds in the settled product is never requested again. One request
//! is in flight at a time, and the transport spaces requests
//! [`transport::REQUEST_INTERVAL`] apart.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, Transport, TransportSource};
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::mirrors::{MirrorAttempt, MirrorBaseUrl, MirrorList, MirrorOutcome};
use gt_ionex::tec::TotalElectronContent;
use gt_ionex::{IonexProduct, calendar, transport};
use gt_store::{IonexStore, IonexStoreError};
use gt_types::{LoadedFile, LoadedTrack, TimeRange, TrackRef};
use gt_ui_types::{TecPoint, TecSeries};

use super::day_fetch_queue::DayFetchQueue;

/// What one day's fetch produced.
enum MapDayMessage {
    Stored {
        day: NaiveDate,
        mirror: MirrorBaseUrl,
        product: IonexProduct,
        map_count: usize,
        /// The mirrors tried before the one that served, and what each
        /// returned.
        skipped: Vec<MirrorAttempt>,
    },
    Failed {
        day: NaiveDate,
        detail: String,
    },
}

impl MapDayMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. } | Self::Failed { day, .. } => day,
        }
    }
}

/// Queues TEC map days and ingests them into the archive.
pub struct TecMapScheduler {
    ctx: Context,
    tx: mpsc::Sender<MapDayMessage>,
    rx: mpsc::Receiver<MapDayMessage>,
    /// The hosts a day is requested from, in the order they are tried.
    mirrors: MirrorList,
    /// `None` disables fetching: no archive to write to.
    store: Option<Arc<IonexStore>>,
    /// Connected on the first request, and dropped when the mirror list
    /// changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here determines whether requests may leave the machine.
    transport_source: TransportSource,
    days: DayFetchQueue,
    /// UTC days the archive holds maps for, read once at startup and extended
    /// on ingest, so resolving a fix's value never reads the day index per
    /// frame. Assumes this process is the archive's only writer.
    archived_days: HashSet<NaiveDate>,
    /// Per-track plot points, keyed by the days they were resolved from, so
    /// the `Arc` identity the plot caches on only changes when the archive
    /// gained a day the track needs.
    plot_points: HashMap<TrackRef, (Vec<NaiveDate>, Arc<Vec<TecPoint>>)>,
}

impl TecMapScheduler {
    pub fn new(
        ctx: Context,
        store: Option<Arc<IonexStore>>,
        mirrors: MirrorList,
        transport_source: TransportSource,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let archived_days = store
            .as_ref()
            .map(|store| archived_days_of(store))
            .unwrap_or_default();
        Self {
            ctx,
            tx,
            rx,
            mirrors,
            store,
            http: None,
            transport_source,
            days: DayFetchQueue::default(),
            archived_days,
            plot_points: HashMap::new(),
        }
    }

    /// Queue the days a recording spans.
    ///
    /// Days already archived in the settled product, outside the archives'
    /// coverage, or already queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    pub fn request_days_for(&mut self, range: TimeRange) {
        let Some(store) = self.store.as_ref().map(Arc::clone) else {
            return;
        };
        let Some(days) = range.utc_days(calendar::MAX_DAYS_PER_TRACK) else {
            log::info!(
                "A recording spanning {} is past the {}-day limit; no TEC map days queued",
                range.duration(),
                calendar::MAX_DAYS_PER_TRACK
            );
            return;
        };
        let today = Utc::now().date_naive();
        for day in days {
            if calendar::fetchable_products(day, today).is_empty() {
                continue;
            }
            self.days
                .request_recording_day(day, day_needs_fetch(&store, day, today));
        }
        self.start_next();
    }

    /// Queue every day in `from..=to` the archive does not already hold in the
    /// settled product, as one backfill.
    ///
    /// Returns how many days were queued, or [`None`] when there is no archive
    /// to write them to.
    pub fn backfill(&mut self, from: NaiveDate, to: NaiveDate) -> Option<usize> {
        let store = self.store.as_ref().map(Arc::clone)?;
        let today = Utc::now().date_naive();
        let total = self
            .days
            .start_backfill(calendar::fetchable_days(from, to, today), |day| {
                day_needs_fetch(&store, day, today)
            });
        log::info!("Backfilling TEC maps for {total} days between {from} and {to}");
        self.start_next();
        Some(total)
    }

    /// Whether there is an archive to download into. Grays the backfill
    /// control when there is not.
    pub fn archive_available(&self) -> bool {
        self.store.is_some()
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
                MapDayMessage::Stored {
                    day,
                    mirror,
                    product,
                    map_count,
                    skipped,
                } => {
                    for attempt in skipped {
                        match attempt.outcome {
                            MirrorOutcome::NoFile => log::debug!(
                                "{} holds no {} TEC map for {day}",
                                attempt.mirror,
                                attempt.product
                            ),
                            MirrorOutcome::Failed(detail) => log::warn!(
                                "No {} TEC map for {day} from {}: {detail}",
                                attempt.product,
                                attempt.mirror
                            ),
                        }
                    }
                    self.archived_days.insert(day);
                    self.days.mark_archived(day);
                    log::info!("Archived {map_count} {product} TEC maps for {day} from {mirror}");
                }
                MapDayMessage::Failed { day, detail } => {
                    log::error!("No TEC maps archived for {day}: {detail}");
                    self.days.report_failure(day, detail);
                }
            }
        }
        self.start_next();
    }

    /// Point the scheduler at `mirrors`.
    ///
    /// A changed list drops the queue, the days requested of the old list, its
    /// failures and the running backfill. Archived days are kept - a day
    /// already archived does not depend on which mirror served it.
    pub fn set_mirrors(&mut self, mirrors: &MirrorList) {
        if self.mirrors == *mirrors {
            return;
        }
        self.mirrors = mirrors.clone();
        self.http = None;
        self.days.forget_host();
    }

    /// TEC values for the plot: one point per fix of every loaded track,
    /// interpolated from the archived maps over that fix's own position and
    /// time.
    ///
    /// A track's points are rebuilt only when the archive gains one of the
    /// days it spans, so the `Arc` the plot caches on stays stable.
    pub fn plot_series(&mut self, files: &[LoadedFile]) -> TecSeries {
        let mut series = TecSeries::default();
        let mut live: HashSet<TrackRef> = HashSet::new();
        // Shared across tracks: a batch of recordings from one drive all read
        // the same day.
        let mut archived: HashMap<NaiveDate, Option<GlobalIonosphereMaps>> = HashMap::new();

        for (fi, file) in files.iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref =
                    TrackRef::new(gt_types::FileIdx::new(fi), gt_types::TrackIdx::new(ti));
                live.insert(track_ref);

                let resolved_from = self.archived_days_spanned_by(track);
                let cached = self
                    .plot_points
                    .get(&track_ref)
                    .filter(|(days, _)| *days == resolved_from);
                let points = match cached {
                    Some((_, points)) => Arc::clone(points),
                    None => {
                        let points = Arc::new(Self::resolve_points(
                            self.store.as_deref(),
                            &mut archived,
                            track,
                        ));
                        self.plot_points
                            .insert(track_ref, (resolved_from, Arc::clone(&points)));
                        points
                    }
                };
                if points.iter().any(|point| point.tecu.is_some()) {
                    series.points_by_track.insert(track_ref, points);
                }
            }
        }
        self.plot_points.retain(|track, _| live.contains(track));
        series
    }

    /// The archived days the track's fixes fall in, as the cache key: a
    /// track's points change exactly when this set does.
    fn archived_days_spanned_by(&self, track: &LoadedTrack) -> Vec<NaiveDate> {
        let range = track.metadata.time_range;
        gt_types::utc_days::days_in_range(
            range.start.date_naive()..=range.end.date_naive(),
            |day| self.archived_days.contains(&day),
        )
    }

    /// One point per fix, interpolated from the maps archived for the fix's
    /// own UTC day. Both maps bracketing any instant inside the day sit in
    /// that day's file, because a published file carries a map at each end of
    /// its day (the last epoch is the next day's midnight). A fix whose time
    /// falls outside the epochs its day was archived with has no value.
    fn resolve_points(
        store: Option<&IonexStore>,
        archived: &mut HashMap<NaiveDate, Option<GlobalIonosphereMaps>>,
        track: &LoadedTrack,
    ) -> Vec<TecPoint> {
        track
            .points
            .iter()
            .map(|point| {
                let time = point.tpv.time().utc();
                let day = time.date_naive();
                let maps = archived
                    .entry(day)
                    .or_insert_with(|| read_archived_maps(store, day));
                TecPoint {
                    x_secs: time.timestamp() as f64,
                    tecu: maps
                        .as_ref()
                        .and_then(|maps| {
                            maps.total_electron_content_at(point.tpv.lat(), point.tpv.lon(), time)
                        })
                        .map(TotalElectronContent::tecu),
                }
            })
            .collect()
    }

    fn start_next(&mut self) {
        let Some(store) = self.store.as_ref().map(Arc::clone) else {
            return;
        };
        let Some(day) = self.days.take_next_day() else {
            return;
        };
        let transport = self.transport();
        spawn_fetch(
            self.ctx.clone(),
            self.tx.clone(),
            transport,
            store,
            self.mirrors.clone(),
            day,
        );
    }

    /// The transport to fetch on, opened once and kept until the mirror list
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
                log::error!("TEC map transport unavailable: {err}");
                Arc::new(Connection::Offline(OfflineTransport))
            }
        }
    }
}

/// Whether `day` must be requested.
///
/// Three conditions put a day on the queue: the archive holds no maps for it,
/// the maps it holds came from [`IonexProduct::Rapid`] which JPL replaces with
/// a final map about two days later, or the day is still running and so has no
/// settled maps yet. A past day archived from [`IonexProduct::Final`] is never
/// requested again.
fn day_needs_fetch(
    store: &IonexStore,
    day: NaiveDate,
    today_utc: NaiveDate,
) -> Result<bool, IonexStoreError> {
    let Some(product) = store.archived_product(day)? else {
        return Ok(true);
    };
    Ok(product == IonexProduct::Rapid || day >= today_utc)
}

/// The maps archived for `day`, reporting a read that failed and treating it
/// as an unarchived day.
fn read_archived_maps(store: Option<&IonexStore>, day: NaiveDate) -> Option<GlobalIonosphereMaps> {
    store?
        .day_maps(day)
        .inspect_err(|err| log::error!("Reading the archived TEC maps for {day}: {err}"))
        .ok()
        .flatten()
}

/// Every day the archive holds maps for.
fn archived_days_of(store: &IonexStore) -> HashSet<NaiveDate> {
    store
        .archived_days()
        .inspect_err(|err| log::error!("Reading the TEC map archive index: {err}"))
        .unwrap_or_default()
        .into_iter()
        .map(|archived| archived.day)
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "thread spawn can only fail under extreme system resource exhaustion"
)]
fn spawn_fetch(
    ctx: Context,
    tx: mpsc::Sender<MapDayMessage>,
    transport: Arc<Connection>,
    store: Arc<IonexStore>,
    mirrors: MirrorList,
    day: NaiveDate,
) {
    thread::Builder::new()
        .name(format!("tec-{day}"))
        .spawn(move || {
            let message = ingest(transport.as_ref(), &store, &mirrors, day);
            tx.send(message).ok();
            ctx.request_repaint();
        })
        .expect("failed to spawn TEC map worker thread");
}

/// Fetch `day` from the first mirror that has it, parse the file, and add its
/// maps to the archive.
///
/// A day no mirror has a file for fails like a request that could not be made:
/// nothing is known about it, so a later session requests it again.
fn ingest(
    transport: &impl Transport<Vec<u8>>,
    store: &IonexStore,
    mirrors: &MirrorList,
    day: NaiveDate,
) -> MapDayMessage {
    let today = Utc::now().date_naive();
    let (mirror, product, maps, skipped) =
        match transport::fetch_day_maps(transport, mirrors, day, today) {
            transport::DayFetch::Fetched {
                mirror,
                product,
                maps,
                skipped,
            } => (mirror, product, maps, skipped),
            transport::DayFetch::Missing => {
                return MapDayMessage::Failed {
                    day,
                    detail: "no map published by any mirror".to_owned(),
                };
            }
            transport::DayFetch::Failed(failure) => {
                return MapDayMessage::Failed {
                    day,
                    detail: failure.to_string(),
                };
            }
        };

    if let Err(err) = store.insert_or_replace_day(day, mirror.as_ref(), Utc::now(), product, &maps)
    {
        return MapDayMessage::Failed {
            day,
            detail: err.to_string(),
        };
    }
    MapDayMessage::Stored {
        day,
        mirror,
        product,
        map_count: maps.maps().len(),
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use chrono::{DateTime, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use crate::app::backfill::BackfillProgress;
    use crate::app::day_failures::DayFailure;
    use crate::app::day_fetch_status::DayFetchStatus;
    use gt_fetch::BytesResponse;
    use gt_ionex::DEFAULT_BASE_URL;
    use gt_store::Store;
    use gt_test_utils::{ScriptedTransport, UrlPrefixAnswers};

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .map(|naive| naive.and_utc())
            .unwrap_or_default()
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    /// Column the record label starts at.
    const LABEL_COLUMN: usize = 60;

    fn record(values: &str, label: &str) -> String {
        format!("{values:<LABEL_COLUMN$}{label}\n")
    }

    /// A file of one map on a grid of two latitudes and two longitudes, dated
    /// the day the ingest tests archive.
    fn published_file() -> String {
        let epoch = "  2024     5    10     0     0     0";
        [
            record(
                "     1.0            IONOSPHERE MAPS     GPS",
                "IONEX VERSION / TYPE",
            ),
            record(epoch, "EPOCH OF FIRST MAP"),
            record(epoch, "EPOCH OF LAST MAP"),
            record("  7200", "INTERVAL"),
            record("     1", "# OF MAPS IN FILE"),
            record("   450.0 450.0   0.0", "HGT1 / HGT2 / DHGT"),
            record("    87.5  85.0  -2.5", "LAT1 / LAT2 / DLAT"),
            record("  -180.0-175.0   5.0", "LON1 / LON2 / DLON"),
            record("    -1", "EXPONENT"),
            record("", "END OF HEADER"),
            record("     1", "START OF TEC MAP"),
            record(epoch, "EPOCH OF CURRENT MAP"),
            record("    87.5-180.0-175.0   5.0 450.0", "LAT/LON1/LON2/DLON/H"),
            record("  100  200", ""),
            record("    85.0-180.0-175.0   5.0 450.0", "LAT/LON1/LON2/DLON/H"),
            record("  300  400", ""),
            record("     1", "END OF TEC MAP"),
            record("", "END OF FILE"),
        ]
        .concat()
    }

    fn gzipped(text: &str) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(text.as_bytes()).expect("compress");
        encoder.finish().expect("finish")
    }

    fn archive() -> (TempDir, Arc<IonexStore>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(dir.path()).open_tec_maps().expect("archive");
        (dir, store)
    }

    /// Archive-backed, and wired so no request leaves the machine.
    fn scheduler_with_archive() -> (TempDir, Arc<IonexStore>, TecMapScheduler) {
        let (dir, store) = archive();
        let scheduler = TecMapScheduler::new(
            Context::default(),
            Some(Arc::clone(&store)),
            MirrorList::default(),
            TransportSource::Offline,
        );
        (dir, store, scheduler)
    }

    /// A scheduler with no archive to write to, so it fetches nothing.
    fn scheduler_without_archive() -> TecMapScheduler {
        TecMapScheduler::new(
            Context::default(),
            None,
            MirrorList::default(),
            TransportSource::Offline,
        )
    }

    fn mirrors(hosts: &[&str]) -> MirrorList {
        MirrorList::new(hosts.iter().copied().map(MirrorBaseUrl::new).collect())
            .expect("a named host")
    }

    /// Archive `archived` as a whole day from `product`, the way a finished
    /// ingest leaves it.
    fn archive_day(store: &IonexStore, archived: NaiveDate, product: IonexProduct) {
        let maps = gt_ionex::parse::global_ionosphere_maps(&published_file()).expect("parse");
        store
            .insert_or_replace_day(archived, "host", Utc::now(), product, &maps)
            .expect("insert");
    }

    #[test]
    fn a_scheduler_without_an_archive_queues_nothing() {
        let mut scheduler = scheduler_without_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
        assert!(scheduler.days.failures().is_empty());
    }

    /// JPL published nothing before November 2008, so no request goes out for
    /// a recording older than that.
    #[test]
    fn a_day_before_coverage_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(1970, 1, 1, 0), at(1970, 1, 1, 1)));
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

    /// A past day archived from the settled product is settled, so nothing
    /// goes out for it.
    #[test]
    fn a_day_archived_from_the_final_product_is_not_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Final);

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
    }

    /// A rapid map is replaced by a final one about two days later, so the day
    /// goes out again.
    #[test]
    fn a_day_archived_from_the_rapid_product_is_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Rapid);

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        assert!(
            scheduler.days.is_fetching(),
            "the archived day is dispatched"
        );
    }

    /// The current day has no settled maps yet, so an archived copy of it is
    /// never the last word.
    #[test]
    fn the_current_day_is_requested_even_when_archived() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let today = Utc::now().date_naive();
        archive_day(&store, today, IonexProduct::Final);

        scheduler.request_days_for(TimeRange::new(Utc::now(), Utc::now()));
        assert!(scheduler.days.is_fetching());
    }

    /// A recording is requested once. Loading it again requests nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let span = TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17));
        scheduler.request_days_for(span);
        let after_first = scheduler.days.requested_days().len();
        scheduler.request_days_for(span);
        assert_eq!(scheduler.days.requested_days().len(), after_first);
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is a
    /// backfill's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 4, 1, 0), at(2024, 5, 10, 0)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.requested_days().is_empty());
    }

    /// A changed mirror list drops what belonged to the old one, whether a
    /// host changed or the order did, the running backfill included.
    #[rstest]
    #[case::another_host(&["https://mirror.example"])]
    #[case::the_same_hosts_in_another_order(&["https://mirror.example", DEFAULT_BASE_URL])]
    fn changing_the_mirror_list_drops_the_queue_and_its_failures(#[case] changed: &[&str]) {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.set_mirrors(&mirrors(&[DEFAULT_BASE_URL, "https://mirror.example"]));
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 11, 17)));
        scheduler.days.report_failure(
            day(2024, 5, 10),
            "HTTP 500 Internal Server Error".to_owned(),
        );

        scheduler.set_mirrors(&mirrors(changed));

        assert!(
            scheduler.days.requested_days().is_empty(),
            "the old list's requests"
        );
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.failures().is_empty());
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    #[test]
    fn setting_the_same_mirror_list_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        let seen = scheduler.days.requested_days().len();

        scheduler.set_mirrors(&MirrorList::default());

        assert_eq!(scheduler.days.requested_days().len(), seen);
    }

    /// A failure reaches the settings section's list.
    #[test]
    fn a_failed_day_is_reported() {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .tx
            .send(MapDayMessage::Failed {
                day: day(2024, 5, 10),
                detail: "final: HTTP 500 Internal Server Error".to_owned(),
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(
            scheduler.days.failures(),
            [DayFailure {
                day: day(2024, 5, 10),
                detail: "final: HTTP 500 Internal Server Error".to_owned(),
            }]
        );
    }

    /// The settled product is requested first, and the archive records the
    /// mirror that served it along with the product it came from.
    #[test]
    fn an_ingested_day_archives_its_maps_the_product_and_the_mirror() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 10);
        let transport = ScriptedTransport::always(Ok(BytesResponse {
            status: 200,
            body: gzipped(&published_file()),
        }));

        let message = ingest(&transport, &store, &MirrorList::default(), ingested);

        assert!(matches!(
            message,
            MapDayMessage::Stored {
                product: IonexProduct::Final,
                map_count: 1,
                ..
            }
        ));
        assert_eq!(
            transport.requested_urls(),
            ["https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz"]
        );
        let archived = store.archived_days().expect("days");
        assert_eq!(
            archived.first().map(|entry| entry.host.as_str()),
            Some(DEFAULT_BASE_URL)
        );
        assert_eq!(
            archived.first().map(|entry| entry.product),
            Some(IonexProduct::Final)
        );
    }

    /// The archived maps are the ones the file held, so a later lookup reads
    /// what the producer published.
    #[test]
    fn an_ingested_day_reads_back_as_the_file_it_came_from() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 10);
        let transport = ScriptedTransport::always(Ok(BytesResponse {
            status: 200,
            body: gzipped(&published_file()),
        }));

        ingest(&transport, &store, &MirrorList::default(), ingested);

        let maps = store
            .day_maps(ingested)
            .expect("read")
            .expect("the day is archived");
        assert_eq!(
            maps.total_electron_content_at(
                gt_types::Latitude::new(87.5),
                gt_types::Longitude::new(-180.0),
                at(2024, 5, 10, 0),
            )
            .map(gt_ionex::tec::TotalElectronContent::tecu),
            Some(10.0),
            "the stored integer 100 at exponent -1"
        );
    }

    /// A revised day replaces the archived one instead of appending to it.
    #[test]
    fn ingesting_a_day_twice_replaces_what_was_archived() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 10);
        let transport = ScriptedTransport::always(Ok(BytesResponse {
            status: 200,
            body: gzipped(&published_file()),
        }));

        ingest(&transport, &store, &MirrorList::default(), ingested);
        ingest(&transport, &store, &MirrorList::default(), ingested);

        assert_eq!(store.archived_days().expect("days").len(), 1);
    }

    #[rstest]
    #[case::a_body_that_is_not_a_file(200, b"<html>captive portal</html>".to_vec())]
    #[case::a_server_error(500, Vec::new())]
    #[case::a_refused_request(403, Vec::new())]
    #[case::no_file_at_either_product(404, Vec::new())]
    fn a_day_that_cannot_be_read_archives_nothing(#[case] status: u16, #[case] body: Vec<u8>) {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Ok(BytesResponse { status, body }));

        let message = ingest(&transport, &store, &MirrorList::default(), day(2024, 5, 10));

        assert!(matches!(message, MapDayMessage::Failed { .. }));
        assert!(store.archived_days().expect("days").is_empty());
    }

    #[test]
    fn a_day_the_second_mirror_served_is_archived_under_that_mirror() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 10);
        let transport = ScriptedTransport::by_url_prefix(UrlPrefixAnswers {
            prefix: "https://second.example".to_owned(),
            matching: Ok(BytesResponse {
                status: 200,
                body: gzipped(&published_file()),
            }),
            other: Ok(BytesResponse {
                status: 404,
                body: Vec::new(),
            }),
        });

        let message = ingest(
            &transport,
            &store,
            &mirrors(&["https://first.example", "https://second.example"]),
            ingested,
        );

        match message {
            MapDayMessage::Stored {
                mirror, skipped, ..
            } => {
                assert_eq!(mirror, MirrorBaseUrl::new("https://second.example"));
                assert_eq!(skipped.len(), 1, "the first mirror holds no file");
            }
            MapDayMessage::Failed { detail, .. } => panic!("{detail}"),
        }
        assert_eq!(
            store
                .archived_days()
                .expect("days")
                .first()
                .map(|entry| entry.host.clone()),
            Some("https://second.example".to_owned())
        );
    }

    /// A day every mirror failed on names each of them and why.
    #[test]
    fn a_day_every_mirror_failed_on_reports_each_failure() {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Ok(BytesResponse {
            status: 500,
            body: Vec::new(),
        }));

        let message = ingest(
            &transport,
            &store,
            &mirrors(&["https://first.example", "https://second.example"]),
            day(2024, 5, 10),
        );

        match message {
            MapDayMessage::Failed { day, detail } => {
                assert_eq!(
                    DayFailure { day, detail }.to_string(),
                    "2024-05-10 - final: https://first.example: HTTP 500 Internal Server Error, \
                     https://second.example: HTTP 500 Internal Server Error"
                );
            }
            MapDayMessage::Stored { .. } => panic!("no mirror served a file"),
        }
    }

    /// A backfill puts every day the refresh rule wants on the queue: an
    /// unarchived day, and one archived from the revisable product.
    #[test]
    fn a_backfill_queues_the_days_the_refresh_rule_wants() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Final);
        archive_day(&store, day(2024, 5, 11), IonexProduct::Rapid);

        let queued = scheduler.backfill(day(2024, 5, 10), day(2024, 5, 12));

        assert_eq!(queued, Some(2), "the rapid day and the unarchived one");
        assert_eq!(
            scheduler.days.fetch_status().fetching,
            Some(day(2024, 5, 11)),
            "the day archived from the settled product is never requested"
        );
    }

    /// Re-running a backfill over a range already archived from the settled
    /// product costs nothing.
    #[test]
    fn a_fully_archived_range_queues_nothing() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for offset in 10..=12 {
            archive_day(&store, day(2024, 5, offset), IonexProduct::Final);
        }

        assert_eq!(
            scheduler.backfill(day(2024, 5, 10), day(2024, 5, 12)),
            Some(0)
        );
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// A range before JPL's first published day requests nothing.
    #[test]
    fn a_backfill_before_coverage_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(
            scheduler.backfill(day(1970, 1, 1), day(2008, 11, 18)),
            Some(0)
        );
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// No archive is distinct from an empty range: the control says so instead
    /// of claiming the range is already downloaded.
    #[test]
    fn a_backfill_without_an_archive_reports_no_archive() {
        let mut scheduler = scheduler_without_archive();
        assert!(!scheduler.archive_available());
        assert_eq!(scheduler.backfill(day(2024, 5, 10), day(2024, 5, 12)), None);
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// Every outcome retires its day, and the last one ends the backfill.
    #[rstest]
    #[case::stored(MapDayMessage::Stored {
        day: day(2024, 5, 10),
        mirror: MirrorBaseUrl::new(DEFAULT_BASE_URL),
        product: IonexProduct::Final,
        map_count: 13,
        skipped: Vec::new(),
    })]
    #[case::failed(MapDayMessage::Failed {
        day: day(2024, 5, 10),
        detail: "final: HTTP 500 Internal Server Error".to_owned(),
    })]
    fn progress_advances_on_every_outcome(#[case] message: MapDayMessage) {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .days
            .queue_backfill_of(&[day(2024, 5, 10), day(2024, 5, 11)]);
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

    /// Cancelling releases the queued days for a later backfill, and keeps the
    /// day already being fetched claimed.
    #[test]
    fn cancelling_releases_every_queued_day_but_the_one_in_flight() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let (in_flight, queued) = (day(2024, 5, 10), day(2024, 5, 11));
        scheduler.days.queue_backfill_of(&[in_flight, queued]);
        assert_eq!(scheduler.days.take_next_day(), Some(in_flight));

        scheduler.days.cancel_backfill();

        assert_eq!(scheduler.days.backfill_progress(), None);
        assert_eq!(scheduler.days.queued(), 0);
        assert!(
            scheduler.days.requested_days().contains(&in_flight),
            "the day being fetched stays claimed"
        );
        assert!(
            !scheduler.days.requested_days().contains(&queued),
            "a day that never went out can be requested again"
        );
    }

    /// The status reports the day in flight, the queue behind it, and how much
    /// of what is loaded the archive holds.
    #[test]
    fn the_status_reports_the_queue_and_the_archived_recording_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Final);

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 11, 17)));

        assert_eq!(
            scheduler.days.fetch_status(),
            DayFetchStatus {
                fetching: Some(day(2024, 5, 11)),
                queued: 0,
                recording_days: 2,
                archived_recording_days: 1,
            }
        );
    }

    /// A recording made before JPL's coverage begins leaves the count empty,
    /// which the settings page shows as an absent value.
    #[test]
    fn a_day_outside_coverage_is_no_recording_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(1970, 1, 1, 0), at(1970, 1, 1, 1)));

        assert_eq!(scheduler.days.fetch_status().recording_days, 0);
    }

    /// An archived day moves the loaded recording's coverage up.
    #[test]
    fn archiving_a_day_covers_the_recording_day_it_belongs_to() {
        let mut scheduler = scheduler_without_archive();
        scheduler.days.await_recording_day(day(2024, 5, 10));

        scheduler
            .tx
            .send(MapDayMessage::Stored {
                day: day(2024, 5, 10),
                mirror: MirrorBaseUrl::new(DEFAULT_BASE_URL),
                product: IonexProduct::Final,
                map_count: 13,
                skipped: Vec::new(),
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(scheduler.days.fetch_status().archived_recording_days, 1);
    }

    /// Offline, a queued day is still dispatched: the transport declines the
    /// request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));

        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.is_fetching());
    }

    /// The fixes of one recording, `step_secs` apart, with the time range a
    /// real load derives from its points. The fixture points sit at 55 N,
    /// 12 E, inside the grid [`uniform_maps`] declares.
    fn track_over(start: DateTime<Utc>, count: usize, step_secs: i64) -> gt_types::LoadedTrack {
        let mut track = gt_test_utils::fixtures::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_from(start, count, step_secs),
        );
        track.metadata.time_range = TimeRange::new(
            start,
            start + TimeDelta::seconds(step_secs * count.saturating_sub(1) as i64),
        );
        track
    }

    fn loaded_files_of(track: gt_types::LoadedTrack) -> Vec<gt_types::LoadedFile> {
        vec![gt_types::LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        }]
    }

    /// Maps of `day` whose every node carries one value, at the given whole
    /// hours from that day's midnight. Hour 24 is the map at the end of the
    /// day, which a published file dates to the next day's midnight.
    fn uniform_maps(day: NaiveDate, samples: &[(i64, f64)]) -> GlobalIonosphereMaps {
        let midnight = day.and_time(chrono::NaiveTime::MIN).and_utc();
        let axis = |first_degrees: f64, last_degrees: f64, step_degrees: f64| {
            gt_ionex::grid::GridAxis::new(gt_ionex::grid::AxisDeclaration {
                first_degrees,
                last_degrees,
                step_degrees,
            })
            .expect("axis")
        };
        let grid = gt_ionex::grid::MapGrid {
            latitudes: gt_ionex::grid::LatitudeAxis::new(axis(57.5, 52.5, -2.5)),
            longitudes: gt_ionex::grid::LongitudeAxis::new(axis(10.0, 15.0, 5.0)),
            shell_height_km: 450.0,
        };
        let maps = samples
            .iter()
            .map(|&(hours, tecu)| {
                gt_ionex::maps::TecMap::new(
                    midnight + TimeDelta::hours(hours),
                    vec![vec![Some(TotalElectronContent::from_tecu(tecu)); 2]; 3],
                )
            })
            .collect();
        GlobalIonosphereMaps::new(grid, TimeDelta::hours(2), maps)
    }

    /// Archive `archived` with the last two maps a published day carries: one
    /// at 22:00 and the one dated to the next day's midnight.
    fn archive_last_maps_of(store: &IonexStore, archived: NaiveDate) {
        store
            .insert_or_replace_day(
                archived,
                "host",
                Utc::now(),
                IonexProduct::Final,
                &uniform_maps(archived, &[(22, 10.0), (24, 20.0)]),
            )
            .expect("insert");
    }

    /// A fix takes the value interpolated at its own time, and one after the
    /// last full hour reads the map at the end of its day - the map a
    /// published file dates to the next day's midnight and archives under the
    /// day it belongs to.
    #[test]
    fn a_fix_is_valued_between_the_maps_bracketing_its_own_time() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        archive_last_maps_of(&store, archived);
        scheduler.archived_days.insert(archived);
        let files = loaded_files_of(track_over(at(2024, 5, 10, 22), 4, 1800));

        let series = scheduler.plot_series(&files);
        let points = series
            .points_by_track
            .values()
            .next()
            .expect("the track has values");

        assert_eq!(
            points.iter().map(|point| point.tecu).collect::<Vec<_>>(),
            [Some(10.0), Some(12.5), Some(15.0), Some(17.5)]
        );
    }

    /// A fix outside the epochs its day was archived with has no value, and
    /// its track offers no series when no fix has one.
    #[test]
    fn a_track_outside_every_archived_epoch_has_no_plot_series() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        archive_last_maps_of(&store, archived);
        scheduler.archived_days.insert(archived);
        let files = loaded_files_of(track_over(at(2024, 5, 10, 12), 4, 1800));

        assert!(scheduler.plot_series(&files).is_empty());
    }

    /// Until the day arrives the track has no points to draw.
    #[test]
    fn a_track_with_no_archived_day_has_no_plot_series() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let files = loaded_files_of(track_over(at(2024, 5, 10, 22), 4, 1800));

        assert!(scheduler.plot_series(&files).is_empty());
    }

    /// A day the fetch worker archives is resolved into the loaded track's
    /// points on the next frame, without reloading the recording.
    #[test]
    fn archiving_a_day_gives_the_loaded_track_its_values() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        let files = loaded_files_of(track_over(at(2024, 5, 10, 22), 4, 1800));
        assert!(scheduler.plot_series(&files).is_empty());

        archive_last_maps_of(&store, archived);
        scheduler
            .tx
            .send(MapDayMessage::Stored {
                day: archived,
                mirror: MirrorBaseUrl::new(DEFAULT_BASE_URL),
                product: IonexProduct::Final,
                map_count: 2,
                skipped: Vec::new(),
            })
            .expect("send");
        scheduler.poll();

        let series = scheduler.plot_series(&files);
        let points = series
            .points_by_track
            .values()
            .next()
            .expect("the archived day reached the track");
        assert_eq!(points.first().map(|point| point.tecu), Some(Some(10.0)));
    }
}
