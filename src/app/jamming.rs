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
use gt_jam::dataset::JamDataset;
use gt_jam::day_selection::{DaySelection, EmptyReason};
use gt_jam::transport::{self, FetchOutcome, HttpTransport, Transport};
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
