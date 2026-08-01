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

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_jam::calendar::{self, DayOutlook};
use gt_jam::transport::{self, FetchOutcome, HttpTransport, Transport};
use gt_jam::wire::{self, ParseWarningReporter};
use gt_store::JamStore;
#[cfg(test)]
use gt_store::Store;
use gt_types::TimeRange;

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

/// A day that could not be added to the archive, for the side panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFailure {
    pub day: NaiveDate,
    pub detail: String,
}

/// Queues interference days and ingests them into the archive.
pub struct JammingScheduler {
    ctx: Context,
    tx: mpsc::Sender<JamMessage>,
    rx: mpsc::Receiver<JamMessage>,
    base_url: String,
    /// `None` disables fetching: no archive to write to.
    store: Option<JamStore>,
    /// Built on the first request; `None` while offline or after a build
    /// failure.
    http: Option<Arc<HttpTransport>>,
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
}

impl JammingScheduler {
    pub fn new(ctx: Context, store: Option<JamStore>, base_url: String) -> Self {
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
            ctx,
            tx,
            rx,
            base_url,
            store,
            http: None,
            queue: VecDeque::new(),
            seen: HashSet::new(),
            in_flight: None,
            failures: Vec::new(),
        }
    }

    /// A scheduler with no archive to write to, so it fetches nothing.
    #[cfg(test)]
    fn disabled(ctx: Context) -> Self {
        Self::new(ctx, None, gt_jam::DEFAULT_BASE_URL.to_owned())
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

    /// Apply finished fetches and start the next queued day.
    pub fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.in_flight = None;
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
                }
                JamMessage::Failed { day, detail } => {
                    log::error!("No interference data archived for {day}: {detail}");
                    self.failures.push(DayFailure { day, detail });
                }
            }
        }
        self.start_next();
    }

    /// Cells archived for `day`, or zero if the day is not archived.
    pub fn archived_cells(&self, day: NaiveDate) -> usize {
        self.archived_cells
            .get(&day)
            .map_or(0, |&cells| cells as usize)
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
        let transport = match self.transport() {
            Ok(transport) => transport,
            Err(detail) => {
                // No transport means no day can be fetched, so the queue is
                // dropped rather than retried per day.
                log::info!("Interference fetching is unavailable: {detail}");
                self.queue.clear();
                return;
            }
        };
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

    fn transport(&mut self) -> Result<Arc<HttpTransport>, String> {
        if let Some(http) = self.http.as_ref() {
            return Ok(Arc::clone(http));
        }
        let http = Arc::new(HttpTransport::new().map_err(|err| err.to_string())?);
        self.http = Some(Arc::clone(&http));
        Ok(http)
    }
}

#[expect(
    clippy::expect_used,
    reason = "thread spawn can only fail under extreme system resource exhaustion"
)]
fn spawn_fetch(
    ctx: Context,
    tx: mpsc::Sender<JamMessage>,
    transport: Arc<HttpTransport>,
    store: JamStore,
    base_url: String,
    day: NaiveDate,
) {
    thread::Builder::new()
        .name(format!("jam-{day}"))
        .spawn(move || {
            let message = ingest(transport.as_ref(), &store, &base_url, day);
            tx.send(message).ok();
            ctx.request_repaint();
        })
        .expect("failed to spawn interference worker thread");
}

/// Fetch `day`, parse it, and add it to the archive.
fn ingest<T: Transport>(
    transport: &T,
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

    fn scheduler_with_archive() -> (TempDir, JamStore, JammingScheduler) {
        let (dir, store) = archive();
        let scheduler = JammingScheduler::new(
            Context::default(),
            Some(store.clone()),
            DEFAULT_BASE_URL.to_owned(),
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

    /// A queued day never stays queued: offline no transport can be built
    /// and the queue is dropped, otherwise the day is dispatched.
    #[test]
    fn a_queued_day_is_never_left_pending() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = JamStore::open_or_create(&dir.path().join("jamming.h5")).expect("archive");
        let mut scheduler =
            JammingScheduler::new(Context::default(), Some(store), DEFAULT_BASE_URL.to_owned());
        scheduler.request_days_for(range(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.queued(), 0);
        assert_eq!(scheduler.is_fetching(), !gt_types::env::offline());
    }

    /// A recording is requested once; loading it again asks for nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = JamStore::open_or_create(&dir.path().join("jamming.h5")).expect("archive");
        let mut scheduler =
            JammingScheduler::new(Context::default(), Some(store), DEFAULT_BASE_URL.to_owned());
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
        let dir = tempfile::tempdir().expect("temp dir");
        let store = JamStore::open_or_create(&dir.path().join("jamming.h5")).expect("archive");
        let mut scheduler =
            JammingScheduler::new(Context::default(), Some(store), DEFAULT_BASE_URL.to_owned());
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
}
