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

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, Transport, TransportSource};
use gt_ionex::mirrors::{MirrorAttempt, MirrorBaseUrl, MirrorList, MirrorOutcome};
use gt_ionex::{IonexProduct, calendar, transport};
use gt_store::{IonexStore, IonexStoreError};
use gt_types::TimeRange;

use super::day_failures::DayFailure;

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
    queue: VecDeque<NaiveDate>,
    /// Every day queued this session, so a day is requested at most once even
    /// after it fails or comes back from the earlier product.
    seen: HashSet<NaiveDate>,
    in_flight: Option<NaiveDate>,
    failures: Vec<DayFailure>,
}

impl TecMapScheduler {
    pub fn new(
        ctx: Context,
        store: Option<Arc<IonexStore>>,
        mirrors: MirrorList,
        transport_source: TransportSource,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            ctx,
            tx,
            rx,
            mirrors,
            store,
            http: None,
            transport_source,
            queue: VecDeque::new(),
            seen: HashSet::new(),
            in_flight: None,
            failures: Vec::new(),
        }
    }

    /// Queue the days a recording spans.
    ///
    /// Days already archived in the settled product, outside the archives'
    /// coverage, or already queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    pub fn request_days_for(&mut self, range: TimeRange) {
        if self.store.is_none() {
            return;
        }
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
            match self.day_needs_fetch(day, today) {
                Ok(false) => {}
                Ok(true) => {
                    if self.seen.insert(day) {
                        self.queue.push_back(day);
                    }
                }
                Err(err) => {
                    if self.seen.insert(day) {
                        let detail = format!("reading the archive: {err}");
                        log::error!("Cannot determine whether {day} is archived: {detail}");
                        self.failures.push(DayFailure { day, detail });
                    }
                }
            }
        }
        self.start_next();
    }

    /// Days that could not be archived, in the order they were reported.
    pub fn failures(&self) -> &[DayFailure] {
        &self.failures
    }

    /// Whether `day` must be requested.
    ///
    /// Three conditions put a day on the queue: the archive holds no maps for
    /// it, the maps it holds came from [`IonexProduct::Rapid`] which JPL
    /// replaces with a final map about two days later, or the day is still
    /// running and so has no settled maps yet. A past day archived from
    /// [`IonexProduct::Final`] is never requested again.
    fn day_needs_fetch(
        &self,
        day: NaiveDate,
        today_utc: NaiveDate,
    ) -> Result<bool, IonexStoreError> {
        let Some(store) = self.store.as_deref() else {
            return Ok(false);
        };
        let Some(product) = store.archived_product(day)? else {
            return Ok(true);
        };
        Ok(product == IonexProduct::Rapid || day >= today_utc)
    }

    /// Apply finished fetches and start the next queued day.
    pub fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.in_flight = None;
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
                    log::info!("Archived {map_count} {product} TEC maps for {day} from {mirror}");
                }
                MapDayMessage::Failed { day, detail } => {
                    log::error!("No TEC maps archived for {day}: {detail}");
                    self.failures.push(DayFailure { day, detail });
                }
            }
        }
        self.start_next();
    }

    /// Point the scheduler at `mirrors`.
    ///
    /// A changed list drops the queue, `seen` and the failures: they belong to
    /// the old list. Archived days are kept - a day already archived does not
    /// depend on which mirror served it.
    pub fn set_mirrors(&mut self, mirrors: &MirrorList) {
        if self.mirrors == *mirrors {
            return;
        }
        self.mirrors = mirrors.clone();
        self.http = None;
        self.queue.clear();
        self.seen.clear();
        self.failures.clear();
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
    use std::cell::RefCell;
    use std::io::Write as _;

    use chrono::{DateTime, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use gt_fetch::{BytesResponse, HttpRequest, TransportError};
    use gt_ionex::DEFAULT_BASE_URL;
    use gt_store::Store;

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

    /// Serves a file for one mirror's requests, and a 404 for every other
    /// mirror.
    struct MirrorAwareTransport {
        serving: String,
        body: Vec<u8>,
        urls: RefCell<Vec<String>>,
    }

    impl Transport<Vec<u8>> for MirrorAwareTransport {
        fn send(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
            let url = request.url().to_owned();
            let serving = url.starts_with(&self.serving);
            self.urls.borrow_mut().push(url);
            Ok(BytesResponse {
                status: if serving { 200 } else { 404 },
                body: if serving {
                    self.body.clone()
                } else {
                    Vec::new()
                },
            })
        }
    }

    /// Serves one canned response to every request, recording the URLs.
    struct CannedTransport {
        status: u16,
        body: Vec<u8>,
        urls: RefCell<Vec<String>>,
    }

    impl CannedTransport {
        fn serving(body: Vec<u8>) -> Self {
            Self {
                status: 200,
                body,
                urls: RefCell::new(Vec::new()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.borrow().clone()
        }
    }

    impl Transport<Vec<u8>> for CannedTransport {
        fn send(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
            self.urls.borrow_mut().push(request.url().to_owned());
            Ok(BytesResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
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
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
        assert!(scheduler.failures().is_empty());
    }

    /// JPL published nothing before November 2008, so no request goes out for
    /// a recording older than that.
    #[test]
    fn a_day_before_coverage_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(1970, 1, 1, 0), at(1970, 1, 1, 1)));
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

    /// A past day archived from the settled product is settled, so nothing
    /// goes out for it.
    #[test]
    fn a_day_archived_from_the_final_product_is_not_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Final);

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        assert_eq!(scheduler.queued(), 0);
        assert!(!scheduler.is_fetching());
    }

    /// A rapid map is replaced by a final one about two days later, so the day
    /// goes out again.
    #[test]
    fn a_day_archived_from_the_rapid_product_is_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Rapid);

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        assert!(scheduler.is_fetching(), "the archived day is dispatched");
    }

    /// The current day has no settled maps yet, so an archived copy of it is
    /// never the last word.
    #[test]
    fn the_current_day_is_requested_even_when_archived() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let today = Utc::now().date_naive();
        archive_day(&store, today, IonexProduct::Final);

        scheduler.request_days_for(TimeRange::new(Utc::now(), Utc::now()));
        assert!(scheduler.is_fetching());
    }

    /// A recording is requested once. Loading it again requests nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let span = TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17));
        scheduler.request_days_for(span);
        let after_first = scheduler.seen.len();
        scheduler.request_days_for(span);
        assert_eq!(scheduler.seen.len(), after_first);
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is a
    /// backfill's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 4, 1, 0), at(2024, 5, 10, 0)));
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.seen.is_empty());
    }

    /// A changed mirror list drops what belonged to the old one, whether a
    /// host changed or the order did.
    #[rstest]
    #[case::another_host(&["https://mirror.example"])]
    #[case::the_same_hosts_in_another_order(&["https://mirror.example", DEFAULT_BASE_URL])]
    fn changing_the_mirror_list_drops_the_queue_and_its_failures(#[case] changed: &[&str]) {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.set_mirrors(&mirrors(&[DEFAULT_BASE_URL, "https://mirror.example"]));
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 11, 17)));
        scheduler.failures.push(DayFailure {
            day: day(2024, 5, 10),
            detail: "HTTP 500 Internal Server Error".to_owned(),
        });

        scheduler.set_mirrors(&mirrors(changed));

        assert!(scheduler.seen.is_empty(), "the old list's requests");
        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.failures().is_empty());
    }

    #[test]
    fn setting_the_same_mirror_list_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        let seen = scheduler.seen.len();

        scheduler.set_mirrors(&MirrorList::default());

        assert_eq!(scheduler.seen.len(), seen);
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
            scheduler.failures(),
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
        let transport = CannedTransport::serving(gzipped(&published_file()));

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
            transport.urls(),
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
        let transport = CannedTransport::serving(gzipped(&published_file()));

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
        let transport = CannedTransport::serving(gzipped(&published_file()));

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
        let transport = CannedTransport {
            status,
            body,
            urls: RefCell::new(Vec::new()),
        };

        let message = ingest(&transport, &store, &MirrorList::default(), day(2024, 5, 10));

        assert!(matches!(message, MapDayMessage::Failed { .. }));
        assert!(store.archived_days().expect("days").is_empty());
    }

    #[test]
    fn a_day_the_second_mirror_served_is_archived_under_that_mirror() {
        let (_dir, store) = archive();
        let ingested = day(2024, 5, 10);
        let transport = MirrorAwareTransport {
            serving: "https://second.example".to_owned(),
            body: gzipped(&published_file()),
            urls: RefCell::new(Vec::new()),
        };

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
        let transport = CannedTransport {
            status: 500,
            body: Vec::new(),
            urls: RefCell::new(Vec::new()),
        };

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

    /// Offline, a queued day is still dispatched: the transport declines the
    /// request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));

        assert_eq!(scheduler.queued(), 0);
        assert!(scheduler.is_fetching());
    }
}
