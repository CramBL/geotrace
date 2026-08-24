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

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use chrono::{NaiveDate, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, Transport, TransportSource};
use gt_pending_writes::{PendingWrites, WriteRefusal};
use gt_solar::activity::GeomagneticActivity;
use gt_solar::series::{Hp30Series, IndexSample, IndexSeries, KpSeries};
use gt_solar::{GeomagneticIndex, TimeWindow, calendar, transport, wire};
use gt_store::{
    ArchiveUsage, DayArchiveError as _, GeomagneticIndexArchive, ReadOnlySolarStore, SolarStore,
    SolarStoreError,
};
use gt_types::{LoadedFile, LoadedTrack, TimeRange, TrackRef};
use gt_ui_types::{
    GeomagneticContextLines, GeomagneticPoint, GeomagneticSeries, IndexContextSample,
};
use rustc_hash::{FxHashMap, FxHashSet};
use strum::IntoEnumIterator as _;

use super::background_thread;
use super::context_line::{ContextSampleCache, ContextSource, ContextSpan, midnight_secs};
use super::day_fetch_queue::DayFetchQueue;
use super::day_index_read_retry::DayIndexReadRetry;
use super::environment_storage::{EnvironmentArchive, PrunedDays};
use super::track_day_values::TrackValuesByArchivedDays;

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
    /// The day was downloaded, then discarded unarchived because the write
    /// registry turned it away.
    NotArchived {
        day: NaiveDate,
        refusal: WriteRefusal,
    },
}

impl IndexDayMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. } | Self::Failed { day, .. } | Self::NotArchived { day, .. } => {
                day
            }
        }
    }
}

/// Queues geomagnetic index days and ingests them into the archive.
pub struct GeomagneticIndexScheduler {
    ctx: Context,
    tx: mpsc::Sender<IndexDayMessage>,
    rx: mpsc::Receiver<IndexDayMessage>,
    base_url: String,
    /// `None` disables fetching: no archive was opened. A read-only session
    /// has one here, and [`Self::writable_archive`] is [`None`].
    store: Option<GeomagneticIndexArchive>,
    /// Connected on the first request, and dropped when the host changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here determines whether requests may leave the machine.
    transport_source: TransportSource,
    days: DayFetchQueue,
    /// UTC days the archive holds samples for, read from its day index and
    /// extended on ingest, so resolving a fix's value never reads the day
    /// index per frame. Ordered so the days a plot span holds are a range
    /// query. Assumes this process is the archive's only writer.
    archived_days: BTreeSet<NaiveDate>,
    day_index_read: DayIndexReadRetry,
    plot_points: TrackValuesByArchivedDays<Vec<GeomagneticPoint>>,
    /// The Hp30 line drawn across the plot's whole span, one sample per
    /// archived period.
    hp30_context: ContextSampleCache<IndexContextSample>,
    /// The Kp line, sampled like [`Self::hp30_context`] at its own cadence.
    kp_context: ContextSampleCache<IndexContextSample>,
    /// Registers every archive insert, and refuses the ones that would start
    /// after shutdown began.
    pending_writes: PendingWrites,
}

impl GeomagneticIndexScheduler {
    pub fn new(
        ctx: Context,
        store: Option<GeomagneticIndexArchive>,
        base_url: String,
        transport_source: TransportSource,
        pending_writes: PendingWrites,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut scheduler = Self {
            archived_days: BTreeSet::new(),
            day_index_read: DayIndexReadRetry::for_archive(EnvironmentArchive::GeomagneticIndices),
            plot_points: TrackValuesByArchivedDays::default(),
            hp30_context: ContextSampleCache::default(),
            kp_context: ContextSampleCache::default(),
            ctx,
            tx,
            rx,
            base_url,
            store: None,
            http: None,
            transport_source,
            days: DayFetchQueue::default(),
            pending_writes,
        };
        scheduler.adopt_store(store);
        scheduler
    }

    /// Take an opened archive, reading the days it already holds.
    pub fn adopt_store(&mut self, store: Option<GeomagneticIndexArchive>) {
        self.store = store;
        self.archived_days = BTreeSet::new();
        self.read_the_day_index();
    }

    /// Reads the days the archive holds, keeping the days already read when
    /// another process has the file open.
    fn read_the_day_index(&mut self) {
        let Some(store) = self.store.clone() else {
            self.day_index_read.forget_the_due_reread();
            return;
        };
        if let Some(days) = self
            .day_index_read
            .record_read(&self.ctx, archived_days_of(store.read()))
        {
            self.archived_days = days;
        }
    }

    fn reread_the_day_index_when_due(&mut self, now: Instant) {
        if self.day_index_read.is_due(now) {
            self.read_the_day_index();
        }
    }

    /// Queue the days a recording spans.
    ///
    /// Days already archived in final form, outside every index's coverage, or
    /// already queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    pub fn request_days_for(&mut self, range: TimeRange) {
        let Some(store) = self.store.clone() else {
            return;
        };
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
            self.days
                .request_recording_day(day, day_needs_fetch(store.read(), day, today));
        }
        self.start_next();
    }

    /// Queue every day in `from..=to` the archive does not already hold in a
    /// form GFZ will not revise, as one backfill.
    ///
    /// Returns how many days were queued, or [`None`] when there is no archive
    /// to write them to.
    pub fn backfill(&mut self, from: NaiveDate, to: NaiveDate) -> Option<usize> {
        let store = self.store.clone()?;
        let today = Utc::now().date_naive();
        let total = self
            .days
            .start_backfill(calendar::fetchable_days(from, to, today), |day| {
                day_needs_fetch(store.read(), day, today)
            });
        log::info!("Backfilling geomagnetic indices for {total} days between {from} and {to}");
        self.start_next();
        Some(total)
    }

    /// Whether there is an archive to download into. Grays the backfill
    /// control when there is not.
    pub fn archive_available(&self) -> bool {
        self.store.is_some()
    }

    /// The archive to delete days from, for the settings page. [`None`] in a
    /// read-only session, where the delete controls are grayed.
    pub fn writable_archive(&self) -> Option<Arc<SolarStore>> {
        self.store.as_ref()?.writer()
    }

    /// What the archive holds, as the environment storage rows show it.
    pub fn archive_usage(&self) -> Option<ArchiveUsage> {
        let store = self.store.as_ref()?;
        Some(ArchiveUsage::measure(
            store.read().path(),
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
        self.plot_points.forget_pruned_days(pruned);
        self.hp30_context.forget_pruned_days(pruned);
        self.kp_context.forget_pruned_days(pruned);
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
        self.reread_the_day_index_when_due(Instant::now());
        while let Ok(message) = self.rx.try_recv() {
            self.days.finish_day(message.day());
            match message {
                IndexDayMessage::Stored {
                    day,
                    kp_samples,
                    hp30_samples,
                } => {
                    self.archived_days.insert(day);
                    self.days.mark_archived(day);
                    self.hp30_context.forget(day);
                    self.kp_context.forget(day);
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
                    self.days.report_failure(day, detail);
                }
                IndexDayMessage::NotArchived { day, refusal } => {
                    log::debug!("No geomagnetic indices archived for {day}: {refusal}");
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

    /// Geomagnetic index values for the plot: one point per fix of every
    /// loaded track, from the archived period covering that fix's own UTC
    /// time.
    ///
    /// A track's points are rebuilt only when the archive gains one of the
    /// days it spans, so the `Arc` the plot caches on stays stable.
    pub fn plot_series(&mut self, files: &[LoadedFile]) -> GeomagneticSeries {
        let mut series = GeomagneticSeries::default();
        let mut live: FxHashSet<TrackRef> = FxHashSet::default();
        // Shared across tracks: a batch of recordings from one drive all read
        // the same day.
        let mut archived: FxHashMap<NaiveDate, ArchivedDay> = FxHashMap::default();

        for (fi, file) in files.iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref =
                    TrackRef::new(gt_types::FileIdx::new(fi), gt_types::TrackIdx::new(ti));
                live.insert(track_ref);

                let archived_days = self.archived_days_spanned_by(track);
                let points = self.plot_points.resolve(track_ref, archived_days, || {
                    Self::resolve_points(
                        self.store.as_ref().map(GeomagneticIndexArchive::read),
                        &mut archived,
                        track,
                    )
                });
                if points
                    .iter()
                    .any(|point| point.hp30.is_some() || point.kp.is_some())
                {
                    series.points_by_track.insert(track_ref, points);
                }
            }
        }
        self.plot_points.retain_loaded_tracks(&live);
        series
    }

    /// The index lines across `span`, each sampled at its own published
    /// cadence: one sample per archived Kp period of three hours, and per
    /// archived Hp30 period of half an hour.
    ///
    /// Both indices are planetary, so neither depends on where the receiver
    /// was. Days the archive holds nothing for break the lines.
    pub fn context_lines(&mut self, span: ContextSpan) -> GeomagneticContextLines {
        let source = ContextSource {
            span,
            archived_days: self.archived_days.range(span.days()).copied().collect(),
            positions: None,
        };
        let store = self.store.clone();
        let gap_at = |day| {
            Some(IndexContextSample {
                start_secs: midnight_secs(day),
                value: None,
            })
        };
        GeomagneticContextLines {
            hp30: self.hp30_context.resolve(
                source.clone(),
                |day| {
                    context_periods(store.as_ref().and_then(|store| {
                        read_archived_series(
                            store.read().hp30_series(day),
                            GeomagneticIndex::Hp30,
                            day,
                        )
                    }))
                },
                gap_at,
            ),
            kp: self.kp_context.resolve(
                source,
                |day| {
                    context_periods(store.as_ref().and_then(|store| {
                        read_archived_series(store.read().kp_series(day), GeomagneticIndex::Kp, day)
                    }))
                },
                gap_at,
            ),
        }
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

    /// One point per fix, valued from the archived periods of the fix's own
    /// UTC day. Both indices publish periods that start on the hour or the
    /// half hour, so no period a fix falls in begins on the day before.
    fn resolve_points(
        store: Option<&ReadOnlySolarStore>,
        archived: &mut FxHashMap<NaiveDate, ArchivedDay>,
        track: &LoadedTrack,
    ) -> Vec<GeomagneticPoint> {
        track
            .points
            .iter()
            .map(|point| {
                let time = point.tpv.time().utc();
                let day = time.date_naive();
                let day_series = archived
                    .entry(day)
                    .or_insert_with(|| ArchivedDay::read(store, day));
                GeomagneticPoint {
                    x_secs: time.timestamp() as f64,
                    hp30: day_series
                        .hp30
                        .as_ref()
                        .and_then(|series| series.activity_at(time))
                        .map(GeomagneticActivity::value),
                    kp: day_series
                        .kp
                        .as_ref()
                        .and_then(|series| series.activity_at(time))
                        .map(GeomagneticActivity::value),
                }
            })
            .collect()
    }

    fn start_next(&mut self) {
        let Some(store) = self.writable_archive() else {
            return;
        };
        if self.pending_writes.refusal().is_some() {
            return;
        }
        let Some(day) = self.days.take_next_day() else {
            return;
        };
        let transport = self.transport();
        self.spawn_fetch(transport, store, day);
    }

    fn spawn_fetch(&self, transport: Arc<Connection>, store: Arc<SolarStore>, day: NaiveDate) {
        let ctx = self.ctx.clone();
        let tx = self.tx.clone();
        let base_url = self.base_url.clone();
        let pending_writes = self.pending_writes.clone();
        background_thread::spawn_or_panic(format!("solar-{day}"), move || {
            let message = ingest(transport.as_ref(), &store, &base_url, day, &pending_writes);
            tx.send(message).ok();
            ctx.request_repaint();
        });
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

/// Whether `day` must be requested.
///
/// Three conditions put a day back on the queue: an index that covers it
/// has no archived samples, the archived Kp holds a nowcast value GFZ
/// replaces with a definitive one later, or the day is still running and
/// so has periods left to publish. A past day archived from definitive
/// values is never requested again, an empty one included: the service
/// published nothing for it and will not start.
fn day_needs_fetch(
    store: &ReadOnlySolarStore,
    day: NaiveDate,
    today_utc: NaiveDate,
) -> Result<bool, SolarStoreError> {
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

/// One day's archived series, read once and shared by every fix falling in
/// that day.
#[derive(Default)]
struct ArchivedDay {
    kp: Option<KpSeries>,
    hp30: Option<Hp30Series>,
}

impl ArchivedDay {
    fn read(store: Option<&ReadOnlySolarStore>, day: NaiveDate) -> Self {
        let Some(store) = store else {
            return Self::default();
        };
        Self {
            kp: read_archived_series(store.kp_series(day), GeomagneticIndex::Kp, day),
            hp30: read_archived_series(store.hp30_series(day), GeomagneticIndex::Hp30, day),
        }
    }
}

/// One day's archived periods as context line samples, oldest first.
fn context_periods<S: IndexSample>(series: Option<IndexSeries<S>>) -> Vec<IndexContextSample> {
    series
        .into_iter()
        .flat_map(|series| series.samples)
        .map(|sample| IndexContextSample {
            start_secs: sample.period_start().timestamp() as f64,
            value: sample.activity().map(GeomagneticActivity::value),
        })
        .collect()
}

/// The archived series, reporting a read that failed and treating it as an
/// unarchived day.
fn read_archived_series<S>(
    read: Result<Option<S>, SolarStoreError>,
    index: GeomagneticIndex,
    day: NaiveDate,
) -> Option<S> {
    read.inspect_err(|err| log::error!("Reading archived {index} for {day}: {err}"))
        .ok()
        .flatten()
}

/// Every day the archive holds samples of any index for.
///
/// The days of the indices that were read still resolve when one index's own
/// read failed: that failure is reported and its index left out. A failure
/// reporting that another process has the file open is handed to
/// [`DayIndexReadRetry`] instead, which reads every index again.
fn archived_days_of(store: &ReadOnlySolarStore) -> Result<BTreeSet<NaiveDate>, SolarStoreError> {
    let mut days = BTreeSet::new();
    for index in GeomagneticIndex::iter() {
        match store.archived_days(index) {
            Ok(archived) => days.extend(archived.into_iter().map(|archived| archived.day)),
            Err(err) if err.is_held_by_another_process() => return Err(err),
            Err(err) => log::error!("Reading the {index} archive index: {err}"),
        }
    }
    Ok(days)
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
    pending_writes: &PendingWrites,
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

    let _write =
        match EnvironmentArchive::GeomagneticIndices.try_begin_day_insert(pending_writes, day) {
            Ok(write) => write,
            Err(refusal) => return IndexDayMessage::NotArchived { day, refusal },
        };
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
    use std::time::Duration;

    use chrono::{DateTime, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use gt_fetch::HttpResponse;
    use gt_pending_writes::WriteAccess;
    use gt_solar::DEFAULT_BASE_URL;
    use gt_solar::series::{Hp30Sample, KpSample, KpStatus};
    use gt_store::{ArchiveHandle, Store};
    use gt_test_utils::{ScriptedTransport, pending_writes};

    use crate::app::backfill::BackfillProgress;
    use crate::app::day_failures::DayFailure;
    use crate::app::day_fetch_status::{ArchivedDayCount, DayFetchStatus};

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
            .expect("archive")
            .writer()
            .expect("an owner session opens the archive writable");
        (dir, store)
    }

    /// Archive-backed, and wired so no request leaves the machine.
    fn scheduler_with_archive() -> (TempDir, Arc<SolarStore>, GeomagneticIndexScheduler) {
        let (dir, store) = archive();
        let scheduler = GeomagneticIndexScheduler::new(
            Context::default(),
            Some(ArchiveHandle::Owner(Arc::clone(&store))),
            DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
            PendingWrites::default(),
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
            PendingWrites::default(),
        )
    }

    fn serving(body: &str) -> ScriptedTransport<String> {
        ScriptedTransport::always(Ok(HttpResponse {
            status: 200,
            body: body.to_owned(),
        }))
    }

    /// The indices the requests fetched, in order, read off their query
    /// strings.
    fn requested_indices(transport: &ScriptedTransport<String>) -> Vec<String> {
        transport
            .requested_urls()
            .iter()
            .filter_map(|url| url.split("index=").nth(1).map(str::to_owned))
            .collect()
    }

    fn kp_sample(period_start: DateTime<Utc>, value: f64) -> KpSample {
        KpSample {
            period_start,
            activity: GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, value),
            status: KpStatus::Definitive,
        }
    }

    fn kp_day(status: KpStatus) -> KpSeries {
        KpSeries {
            samples: vec![KpSample {
                status,
                ..kp_sample(at(2026, 7, 20, 0), 2.667)
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

    /// A read-only session reads the day index beside the instance that owns
    /// the data directory, so the read at
    /// [`GeomagneticIndexScheduler::adopt_store`] can fail on that instance's
    /// open. Without a re-read the session plots no Kp or Hp30 for the rest of
    /// its run.
    #[test]
    fn a_day_index_read_that_failed_on_another_process_is_run_again_and_finds_the_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_definitive_day(&store, day(2026, 7, 20));
        let failed = scheduler.day_index_read.record_read(
            &scheduler.ctx,
            Err::<BTreeSet<NaiveDate>, _>(SolarStoreError::HeldByAnotherProcess),
        );
        assert_eq!(failed, None);
        assert_eq!(scheduler.archived_days_covered(PrunedDays::All), 0);

        scheduler.reread_the_day_index_when_due(Instant::now() + Duration::from_secs(60));

        assert_eq!(scheduler.archived_days_covered(PrunedDays::All), 1);
    }

    #[test]
    fn a_scheduler_without_an_archive_queues_nothing() {
        let mut scheduler = scheduler_without_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
        assert!(scheduler.days.failures().is_empty());
    }

    #[test]
    fn a_day_before_every_index_begins_is_never_queued() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(1900, 1, 1, 0), at(1900, 1, 1, 1)));
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

    /// A past day whose archived Kp is definitive is settled, so nothing goes
    /// out for it.
    #[test]
    fn a_definitive_archived_day_is_not_requested_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_definitive_day(&store, day(2026, 7, 20));

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(!scheduler.days.is_fetching());
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
        assert_eq!(scheduler.days.queued(), 0);
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
        assert!(
            scheduler.days.is_fetching(),
            "the archived day is dispatched"
        );
    }

    /// The current day has periods left to publish, so an archived copy of it
    /// is never the final one.
    #[test]
    fn the_current_day_is_requested_even_when_archived() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let today = Utc::now().date_naive();
        archive_definitive_day(&store, today);

        scheduler.request_days_for(TimeRange::new(Utc::now(), Utc::now()));
        assert!(scheduler.days.is_fetching());
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
        assert!(scheduler.days.is_fetching());
    }

    /// A recording is requested once. Loading it again requests nothing.
    #[test]
    fn a_day_is_queued_at_most_once() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let span = TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17));
        scheduler.request_days_for(span);
        let after_first = scheduler.days.requested_days().len();
        scheduler.request_days_for(span);
        assert_eq!(scheduler.days.requested_days().len(), after_first);
    }

    /// A track spanning more than the cap queues nothing: bulk fetching is the
    /// backfill feature's job.
    #[test]
    fn an_overlong_recording_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 6, 1, 0), at(2026, 7, 20, 0)));
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.requested_days().is_empty());
    }

    /// A queued day is always dispatched, offline included: the transport
    /// declines the request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.is_fetching());
    }

    /// A changed host drops what belonged to the old one.
    #[test]
    fn changing_the_host_drops_the_queue() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        scheduler.set_base_url("https://mirror.example");

        assert!(
            scheduler.days.requested_days().is_empty(),
            "the old host's requests"
        );
        assert_eq!(scheduler.days.queued(), 0);
    }

    #[test]
    fn setting_the_same_host_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));
        let seen = scheduler.days.requested_days().len();

        scheduler.set_base_url(DEFAULT_BASE_URL);

        assert_eq!(scheduler.days.requested_days().len(), seen);
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
            scheduler.days.failures(),
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
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// Nothing is requested for a range before Kp begins.
    #[test]
    fn a_backfill_before_coverage_queues_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        assert_eq!(
            scheduler.backfill(day(1900, 1, 1), day(1931, 12, 31)),
            Some(0)
        );
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// A missing archive reports [`None`], which the control words differently
    /// from the `Some(0)` of a range that is already downloaded.
    #[test]
    fn a_backfill_without_an_archive_reports_no_archive() {
        let mut scheduler = scheduler_without_archive();
        assert!(!scheduler.archive_available());
        assert_eq!(scheduler.backfill(day(2026, 7, 20), day(2026, 7, 26)), None);
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// A range of failing days still reaches its total: every outcome retires
    /// its day.
    #[rstest]
    #[case::stored(IndexDayMessage::Stored { day: day(2026, 7, 20), kp_samples: 8, hp30_samples: 48 })]
    #[case::failed(IndexDayMessage::Failed { day: day(2026, 7, 20), detail: "boom".to_owned() })]
    #[case::not_archived(IndexDayMessage::NotArchived {
        day: day(2026, 7, 20),
        refusal: WriteRefusal::ShuttingDown,
    })]
    fn progress_advances_on_every_outcome(#[case] message: IndexDayMessage) {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .days
            .queue_backfill_of(&[day(2026, 7, 20), day(2026, 7, 21)]);
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

    /// The section stops showing a bar once the last day retires the
    /// backfill.
    #[test]
    fn the_last_day_ends_the_backfill() {
        let mut scheduler = scheduler_without_archive();
        scheduler.days.queue_backfill_of(&[day(2026, 7, 20)]);

        scheduler
            .tx
            .send(IndexDayMessage::Failed {
                day: day(2026, 7, 20),
                detail: "boom".to_owned(),
            })
            .expect("send");
        scheduler.poll();
        assert_eq!(scheduler.days.backfill_progress(), None);
    }

    /// Cancelling drops the queued days and lets a later backfill request
    /// them again.
    #[test]
    fn cancelling_releases_the_queued_days() {
        let mut scheduler = scheduler_without_archive();
        scheduler
            .days
            .queue_backfill_of(&[day(2026, 7, 20), day(2026, 7, 21)]);

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
        let mut scheduler = scheduler_without_archive();
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
        let mut scheduler = scheduler_without_archive();
        let track_day = day(2026, 7, 19);
        scheduler.days.queue_track_day(track_day);
        scheduler.days.queue_backfill_of(&[day(2026, 7, 20)]);

        scheduler.days.cancel_backfill();
        assert_eq!(scheduler.days.queued(), 1);
        assert!(scheduler.days.requested_days().contains(&track_day));
    }

    /// Changing the host abandons a backfill and the failures the old host
    /// produced.
    #[test]
    fn changing_the_host_abandons_the_backfill_and_its_failures() {
        let mut scheduler = scheduler_without_archive();
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

    /// The status reports the day in flight, the queue behind it, and how much
    /// of what is loaded the archive holds.
    #[test]
    fn the_status_reports_the_queue_and_the_archived_recording_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_definitive_day(&store, day(2026, 7, 20));

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 21, 17)));

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

    /// An archived day moves the loaded recording's coverage up.
    #[test]
    fn archiving_a_day_covers_the_recording_day_it_belongs_to() {
        let mut scheduler = scheduler_without_archive();
        scheduler.days.await_recording_day(day(2026, 7, 20));

        scheduler
            .tx
            .send(IndexDayMessage::Stored {
                day: day(2026, 7, 20),
                kp_samples: 8,
                hp30_samples: 48,
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(scheduler.days.fetch_status().recording_days.archived, 1);
    }

    /// Both indices are requested for one day, and the archive records the
    /// host that served them.
    #[test]
    fn an_ingested_day_archives_every_index_and_the_host() {
        let (_dir, store) = archive();
        let ingested = day(2026, 7, 20);
        let transport = serving(ONE_PERIOD_OF_BOTH_INDICES);

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            ingested,
            &PendingWrites::default(),
        );

        assert!(matches!(
            message,
            IndexDayMessage::Stored {
                kp_samples: 1,
                hp30_samples: 1,
                ..
            }
        ));
        assert_eq!(requested_indices(&transport), ["Kp", "Hp30"]);
        for index in [GeomagneticIndex::Kp, GeomagneticIndex::Hp30] {
            let archived = store.archived_days(index).expect("days");
            assert_eq!(
                archived.first().map(|entry| entry.host.as_str()),
                Some(DEFAULT_BASE_URL),
                "{index}"
            );
        }
    }

    /// A day queued while the registry refuses writes stays queued: no worker
    /// starts a download whose archive insert would be refused.
    #[rstest]
    #[case::shutting_down(pending_writes::shutting_down_registry())]
    #[case::read_only_session(PendingWrites::new(WriteAccess::ReadOnly))]
    fn no_day_is_dispatched_while_writes_are_refused(#[case] pending_writes: PendingWrites) {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.pending_writes = pending_writes;

        scheduler.request_days_for(TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17)));

        assert_eq!(scheduler.days.queued(), 1);
        assert!(!scheduler.days.is_fetching());
    }

    /// One guard covers both index inserts, so a day downloaded where the
    /// insert is refused archives neither, and says which refusal discarded
    /// it.
    #[rstest]
    #[case::shutting_down(pending_writes::shutting_down_registry(), WriteRefusal::ShuttingDown)]
    #[case::read_only_session(
        PendingWrites::new(WriteAccess::ReadOnly),
        WriteRefusal::ReadOnlySession
    )]
    fn a_day_downloaded_where_the_insert_is_refused_archives_no_index(
        #[case] pending_writes: PendingWrites,
        #[case] expected: WriteRefusal,
    ) {
        let (_dir, store) = archive();
        let ingested = day(2026, 7, 20);
        let transport = serving(ONE_PERIOD_OF_BOTH_INDICES);

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            ingested,
            &pending_writes,
        );

        let IndexDayMessage::NotArchived { refusal, .. } = message else {
            panic!("the day was archived where the insert is refused");
        };
        assert_eq!(refusal, expected);
        for index in [GeomagneticIndex::Kp, GeomagneticIndex::Hp30] {
            assert!(
                store.archived_days(index).expect("days").is_empty(),
                "{index}"
            );
        }
    }

    /// Before Hp30 begins, only Kp is requested, and the day is archived from
    /// what the service does publish.
    #[test]
    fn a_day_before_hp30_begins_archives_kp_alone() {
        let (_dir, store) = archive();
        let ingested = day(1970, 1, 1);
        let transport =
            serving(r#"{"Kp":[2.667],"datetime":["1970-01-01T00:00:00Z"],"status":["def"]}"#);

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            ingested,
            &PendingWrites::default(),
        );

        assert!(matches!(
            message,
            IndexDayMessage::Stored {
                kp_samples: 1,
                hp30_samples: 0,
                ..
            }
        ));
        assert_eq!(requested_indices(&transport), ["Kp"]);
        assert!(
            store
                .archived_days(GeomagneticIndex::Hp30)
                .expect("days")
                .is_empty()
        );
    }

    /// Empty arrays are a published result, not a failure: the day is archived
    /// with no samples, which is what keeps it from being requested again.
    #[test]
    fn a_day_without_published_values_is_archived_empty() {
        let (_dir, store) = archive();
        let ingested = day(2026, 7, 20);
        let transport = serving(NO_VALUES);

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            ingested,
            &PendingWrites::default(),
        );

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
        let transport = serving(ONE_PERIOD_OF_BOTH_INDICES);

        ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            ingested,
            &PendingWrites::default(),
        );
        ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            ingested,
            &PendingWrites::default(),
        );

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
        let transport = ScriptedTransport::always(Ok(HttpResponse {
            status,
            body: body.to_owned(),
        }));

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            day(2026, 7, 20),
            &PendingWrites::default(),
        );

        assert!(matches!(message, IndexDayMessage::Failed { .. }));
        for index in [GeomagneticIndex::Kp, GeomagneticIndex::Hp30] {
            assert!(
                store.archived_days(index).expect("days").is_empty(),
                "{index}"
            );
        }
    }

    /// The fixes of one recording, `step_secs` apart, with the time range a
    /// real load derives from its points.
    fn track_over(start: DateTime<Utc>, count: usize, step_secs: i64) -> gt_types::LoadedTrack {
        let mut track = gt_test_utils::fixtures::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_from(start, count, step_secs),
        );
        track.metadata.time_range = TimeRange::new(
            start,
            start + chrono::TimeDelta::seconds(step_secs * count.saturating_sub(1) as i64),
        );
        track
    }

    fn loaded_files_of(track: gt_types::LoadedTrack) -> Vec<LoadedFile> {
        vec![LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![track],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        }]
    }

    fn hp30_sample(period_start: DateTime<Utc>, value: f64) -> Hp30Sample {
        Hp30Sample {
            period_start,
            activity: GeomagneticActivity::from_published_value(GeomagneticIndex::Hp30, value),
        }
    }

    /// The first two Hp30 periods and the first Kp period of `day`, archived
    /// as the scheduler's own ingest leaves them.
    fn archive_first_periods_of(store: &SolarStore, day: NaiveDate) {
        let midnight = day.and_time(chrono::NaiveTime::MIN).and_utc();
        store
            .insert_or_replace_hp30_day(
                day,
                "host",
                Utc::now(),
                &Hp30Series {
                    samples: vec![
                        hp30_sample(midnight, 4.667),
                        hp30_sample(midnight + chrono::TimeDelta::minutes(30), 6.333),
                    ],
                },
            )
            .expect("insert hp30");
        store
            .insert_or_replace_kp_day(
                day,
                "host",
                Utc::now(),
                &KpSeries {
                    samples: vec![kp_sample(midnight, 5.0)],
                },
            )
            .expect("insert kp");
    }

    /// The value of the period a fix falls in holds for the whole period, so
    /// fixes either side of a period boundary read different Hp30 values while
    /// the six-times longer Kp period covers all of them.
    #[test]
    fn a_fix_takes_the_value_of_the_period_it_falls_in() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        archive_first_periods_of(&store, archived);
        scheduler.archived_days.insert(archived);
        let files = loaded_files_of(track_over(
            at(2026, 7, 20, 0) + chrono::TimeDelta::minutes(29),
            4,
            30,
        ));

        let series = scheduler.plot_series(&files);
        let points = series
            .points_by_track
            .values()
            .next()
            .expect("the track has values");

        let hp30: Vec<Option<f64>> = points.iter().map(|point| point.hp30).collect();
        assert_eq!(
            hp30,
            [Some(4.667), Some(4.667), Some(6.333), Some(6.333)],
            "the boundary falls between the second and third fix"
        );
        let kp: Vec<Option<f64>> = points.iter().map(|point| point.kp).collect();
        assert_eq!(kp, [Some(5.0); 4]);
    }

    /// A fix past the last archived period of its day has no value, and its
    /// track offers no series when no fix has one.
    #[test]
    fn a_track_past_every_archived_period_has_no_plot_series() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        archive_first_periods_of(&store, archived);
        scheduler.archived_days.insert(archived);
        let files = loaded_files_of(track_over(at(2026, 7, 20, 12), 4, 30));

        assert!(scheduler.plot_series(&files).is_empty());
    }

    /// A recording from before Hp30 begins draws the Kp line alone.
    #[test]
    fn a_day_archived_for_one_index_values_only_that_line() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(1970, 1, 1);
        let midnight = archived.and_time(chrono::NaiveTime::MIN).and_utc();
        store
            .insert_or_replace_kp_day(
                archived,
                "host",
                Utc::now(),
                &KpSeries {
                    samples: vec![kp_sample(midnight, 2.667)],
                },
            )
            .expect("insert kp");
        scheduler.archived_days.insert(archived);
        let files = loaded_files_of(track_over(midnight, 2, 30));

        let series = scheduler.plot_series(&files);
        let points = series
            .points_by_track
            .values()
            .next()
            .expect("the track has values");
        assert!(points.iter().all(|point| point.hp30.is_none()));
        assert!(points.iter().all(|point| point.kp == Some(2.667)));
    }

    /// Until the day arrives the track has no points to draw.
    #[test]
    fn a_track_with_no_archived_day_has_no_plot_series() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let files = loaded_files_of(track_over(at(2026, 7, 20, 0), 4, 30));

        assert!(scheduler.plot_series(&files).is_empty());
    }

    /// A day the fetch worker archives is resolved into the loaded track's
    /// points on the next frame, without reloading the recording.
    #[test]
    fn archiving_a_day_gives_the_loaded_track_its_values() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        let files = loaded_files_of(track_over(at(2026, 7, 20, 0), 4, 30));
        assert!(scheduler.plot_series(&files).is_empty());

        archive_first_periods_of(&store, archived);
        scheduler
            .tx
            .send(IndexDayMessage::Stored {
                day: archived,
                kp_samples: 1,
                hp30_samples: 2,
            })
            .expect("send");
        scheduler.poll();

        let series = scheduler.plot_series(&files);
        let points = series
            .points_by_track
            .values()
            .next()
            .expect("the archived day reached the track");
        assert!(points.iter().all(|point| point.hp30 == Some(4.667)));
    }

    /// The lines a context span holds, resolved from the archive.
    fn context_lines_over(
        scheduler: &mut GeomagneticIndexScheduler,
        days: std::ops::RangeInclusive<NaiveDate>,
    ) -> GeomagneticContextLines {
        let midnight =
            |day: NaiveDate| day.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as f64;
        scheduler.context_lines(ContextSpan::covering(
            midnight(*days.start())..=midnight(*days.end()),
        ))
    }

    /// Each index draws every archived period of its own cadence, whether or
    /// not a recording covers it.
    #[test]
    fn the_context_lines_carry_every_archived_period() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        archive_first_periods_of(&store, archived);
        scheduler.archived_days.insert(archived);

        let lines = context_lines_over(&mut scheduler, archived..=archived);

        let midnight = at(2026, 7, 20, 0).timestamp() as f64;
        assert_eq!(
            lines
                .hp30
                .iter()
                .map(|sample| (sample.start_secs - midnight, sample.value))
                .collect::<Vec<_>>(),
            [(0.0, Some(4.667)), (1800.0, Some(6.333))]
        );
        assert_eq!(
            lines
                .kp
                .iter()
                .map(|sample| (sample.start_secs - midnight, sample.value))
                .collect::<Vec<_>>(),
            [(0.0, Some(5.0))]
        );
    }

    /// A day the archive does not hold breaks both lines, so what is drawn is
    /// what was downloaded.
    #[test]
    fn an_unarchived_day_between_two_archived_ones_breaks_the_lines() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for archived in [day(2026, 7, 20), day(2026, 7, 22)] {
            archive_first_periods_of(&store, archived);
            scheduler.archived_days.insert(archived);
        }

        let lines = context_lines_over(&mut scheduler, day(2026, 7, 20)..=day(2026, 7, 22));

        assert_eq!(
            lines
                .kp
                .iter()
                .map(|sample| sample.value)
                .collect::<Vec<_>>(),
            [Some(5.0), None, Some(5.0)]
        );
    }

    /// A day the fetch worker revised reaches the line, so a definitive value
    /// replaces the nowcast one it was drawn from.
    #[test]
    fn revising_an_archived_day_redraws_the_context_lines() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2026, 7, 20);
        archive_first_periods_of(&store, archived);
        scheduler.archived_days.insert(archived);
        assert_eq!(
            context_lines_over(&mut scheduler, archived..=archived)
                .kp
                .first()
                .and_then(|sample| sample.value),
            Some(5.0)
        );

        store
            .insert_or_replace_kp_day(
                archived,
                "host",
                Utc::now(),
                &KpSeries {
                    samples: vec![kp_sample(at(2026, 7, 20, 0), 7.667)],
                },
            )
            .expect("insert kp");
        scheduler
            .tx
            .send(IndexDayMessage::Stored {
                day: archived,
                kp_samples: 1,
                hp30_samples: 2,
            })
            .expect("send");
        scheduler.poll();

        assert_eq!(
            context_lines_over(&mut scheduler, archived..=archived)
                .kp
                .first()
                .and_then(|sample| sample.value),
            Some(7.667)
        );
    }

    /// One index failing leaves the whole day unarchived, so the next session
    /// requests both again.
    #[test]
    fn a_day_whose_second_index_fails_archives_neither() {
        let (_dir, store) = archive();
        let transport =
            serving(r#"{"Kp":[2.667],"datetime":["2026-07-20T00:00:00Z"],"status":["def"]}"#);

        let message = ingest(
            &transport,
            &store,
            DEFAULT_BASE_URL,
            day(2026, 7, 20),
            &PendingWrites::default(),
        );

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
