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

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use egui::Context;

use gt_fetch::{Connection, OfflineTransport, SecretToken, Transport, TransportSource};
use gt_ionex::instant_selection::TecInstantSelection;
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::mirrors::{MirrorAttempt, MirrorBaseUrl, MirrorList, MirrorOutcome};
use gt_ionex::quiet_time::{self, QuietTimeDeviation};
use gt_ionex::tec::TotalElectronContent;
use gt_ionex::text;
use gt_ionex::{IonexProduct, calendar, transport};
use gt_map::{TecHeatmapSnapshot, TecLayer};
use gt_pending_writes::{PendingWrites, WriteRefusal};
use gt_store::{ArchiveUsage, IonexStore, IonexStoreError};
use gt_types::{LoadedFile, LoadedTrack, TimeRange, TrackRef};
use gt_ui_types::{ArcIdentity, TecContextSample, TecPoint, TecSeries};
use rustc_hash::{FxHashMap, FxHashSet};

use super::background_thread;
use super::context_line::{ContextSampleCache, ContextSource, ContextSpan, midnight_secs};
use super::day_fetch_queue::DayFetchQueue;
use super::day_fetch_status::ArchivedDayCount;
use super::day_index_read_retry::DayIndexReadRetry;
use super::environment_storage::{EnvironmentArchive, PrunedDays};
use super::fix_positions::FixPositionTimeline;
use super::tec_quiet_time::QuietTimeDeviationCache;
use super::track_day_values::TrackValuesByArchivedDays;

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
    /// The day was downloaded, then discarded unarchived because the write
    /// registry turned it away.
    NotArchived {
        day: NaiveDate,
        refusal: WriteRefusal,
    },
}

impl MapDayMessage {
    fn day(&self) -> NaiveDate {
        match *self {
            Self::Stored { day, .. } | Self::Failed { day, .. } | Self::NotArchived { day, .. } => {
                day
            }
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
    /// Authenticates the mirrors serving an archive that needs it. [`None`]
    /// leaves those mirrors unrequested.
    earthdata_token: Option<SecretToken>,
    /// `None` disables fetching: no archive to write to.
    store: Option<Arc<IonexStore>>,
    /// Connected on the first request, and dropped when the mirror list
    /// changes.
    http: Option<Arc<Connection>>,
    /// Where that transport comes from. Supplied by the application, so
    /// nothing here determines whether requests may leave the machine.
    transport_source: TransportSource,
    days: DayFetchQueue,
    /// UTC days the archive holds maps for, read from its day index and
    /// extended on ingest, so resolving a fix's value never reads the day
    /// index per frame. Ordered so the days a plot span holds are a range
    /// query. Assumes this process is the archive's only writer.
    archived_days: BTreeSet<NaiveDate>,
    day_index_read: DayIndexReadRetry,
    plot_points: TrackValuesByArchivedDays<Vec<TecPoint>>,
    /// The line drawn across the plot's whole span, one sample per archived
    /// map epoch.
    context: ContextSampleCache<TecContextSample>,
    /// How far each track's TEC stands from the quiet-time background of the
    /// days before it.
    quiet_time: QuietTimeDeviationCache,
    /// Which instant the heatmap draws, and the stepper's bounds.
    selection: TecInstantSelection,
    /// The day the heatmap draws and its maps, read from the archive on demand
    /// and kept until the shown day changes or that day is archived again.
    shown: Option<(NaiveDate, GlobalIonosphereMaps)>,
    /// Registers every archive insert, and refuses the ones that would start
    /// after shutdown began.
    pending_writes: PendingWrites,
}

impl TecMapScheduler {
    pub fn new(
        ctx: Context,
        store: Option<Arc<IonexStore>>,
        mirrors: MirrorList,
        earthdata_token: Option<SecretToken>,
        transport_source: TransportSource,
        pending_writes: PendingWrites,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut scheduler = Self {
            ctx,
            tx,
            rx,
            mirrors,
            earthdata_token,
            store: None,
            http: None,
            transport_source,
            days: DayFetchQueue::default(),
            archived_days: BTreeSet::new(),
            day_index_read: DayIndexReadRetry::for_archive(EnvironmentArchive::IonosphericTec),
            plot_points: TrackValuesByArchivedDays::default(),
            context: ContextSampleCache::default(),
            quiet_time: QuietTimeDeviationCache::default(),
            selection: TecInstantSelection::new(None, Utc::now().date_naive()),
            shown: None,
            pending_writes,
        };
        scheduler.adopt_store(store);
        scheduler
    }

    /// Take an opened archive, reading the days it already holds.
    pub fn adopt_store(&mut self, store: Option<Arc<IonexStore>>) {
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
            .record_read(&self.ctx, archived_days_of(&store))
        {
            self.archived_days = days;
        }
    }

    fn reread_the_day_index_when_due(&mut self, now: Instant) {
        if self.day_index_read.is_due(now) {
            self.read_the_day_index();
        }
    }

    /// Queue the days a recording spans, and the quiet-time window before each
    /// of them.
    ///
    /// Days already archived in the settled product, outside the archives'
    /// coverage, or already queued are dropped. A recording spanning more than
    /// [`calendar::MAX_DAYS_PER_TRACK`] queues nothing.
    ///
    /// The window is what the TEC deviation warning measures against, so one
    /// recording day pulls in the 27 days before it as well: about 3.4 MB per
    /// recording day at the 125 KiB a day's file costs.
    pub fn request_days_for(&mut self, range: TimeRange) {
        let Some(store) = self.store.as_ref().map(Arc::clone) else {
            return;
        };
        let Some(days) = range.utc_days(calendar::MAX_DAYS_PER_TRACK) else {
            log::info!(
                "A recording spanning {} is past the {}-day limit. No TEC map days queued.",
                range.duration(),
                calendar::MAX_DAYS_PER_TRACK
            );
            return;
        };
        let today = Utc::now().date_naive();
        self.selection.adopt_default(range.start);
        let recording_days: Vec<NaiveDate> = days
            .into_iter()
            .filter(|day| !calendar::fetchable_products(*day, today).is_empty())
            .collect();
        let mut background: BTreeSet<NaiveDate> = recording_days
            .iter()
            .flat_map(|day| quiet_time::background_days(*day))
            .collect();
        for day in &recording_days {
            background.remove(day);
            self.days
                .request_recording_day(*day, day_needs_fetch(&store, *day, today));
        }
        for day in background {
            if calendar::fetchable_products(day, today).is_empty() {
                continue;
            }
            self.days
                .request_background_day(day, day_needs_fetch(&store, day, today));
        }
        self.start_next();
    }

    /// How far the archive covers the quiet-time windows of the loaded
    /// recordings, as the settings page reports it.
    pub fn background_day_coverage(&self) -> ArchivedDayCount {
        self.days.background_day_coverage()
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

    /// The archive, for the settings page to report and delete from.
    pub fn archive(&self) -> Option<Arc<IonexStore>> {
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
        self.plot_points.forget_pruned_days(pruned);
        self.context.forget_pruned_days(pruned);
        self.quiet_time.forget_pruned_days(pruned);
        if self
            .shown
            .as_ref()
            .is_some_and(|(day, _)| pruned.covers(*day))
        {
            self.shown = None;
        }
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
                            MirrorOutcome::SkippedWithoutToken => log::warn!(
                                "No {} TEC map for {day} from {}: {}",
                                attempt.product,
                                attempt.mirror,
                                text::MIRROR_SKIPPED_WITHOUT_TOKEN
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
                    self.context.forget(day);
                    self.quiet_time.forget(day);
                    if self.shown.as_ref().is_some_and(|(shown, _)| *shown == day) {
                        self.shown = None;
                    }
                    log::info!("Archived {map_count} {product} TEC maps for {day} from {mirror}");
                }
                MapDayMessage::Failed { day, detail } => {
                    log::error!("No TEC maps archived for {day}: {detail}");
                    self.days.report_failure(day, detail);
                }
                MapDayMessage::NotArchived { day, refusal } => {
                    log::debug!("No TEC maps archived for {day}: {refusal}");
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

    /// Authenticate the mirrors that need a token with `earthdata_token`.
    ///
    /// A changed token drops the queue and its failures the same way a changed
    /// mirror list does: the days passed over for want of one are worth
    /// requesting again.
    pub fn set_earthdata_token(&mut self, earthdata_token: Option<SecretToken>) {
        if self.earthdata_token == earthdata_token {
            return;
        }
        self.earthdata_token = earthdata_token;
        self.days.forget_host();
    }

    /// Show the ionosphere at `instant` for as long as the fix at that time
    /// stays hovered or selected. [`None`] hands the heatmap back to the
    /// display toggle's stepper.
    pub fn follow_instant(&mut self, instant: Option<DateTime<Utc>>) {
        self.selection.follow(instant);
    }

    /// What the map heatmap draws this frame: the shown instant's maps, the
    /// instant selection the stepper moves, and why there is nothing to draw.
    ///
    /// The archived day is read the first time it is shown and kept until the
    /// shown day changes.
    pub fn overlay_layer(&mut self) -> TecLayer<'_> {
        let day = self.selection.instant().map(|instant| instant.date_naive());
        if self
            .shown
            .as_ref()
            .is_none_or(|(shown, _)| Some(*shown) != day)
        {
            self.shown = day
                .filter(|day| self.archived_days.contains(day))
                .and_then(|day| {
                    read_archived_maps(self.store.as_deref(), day).map(|maps| (day, maps))
                });
        }
        if let Some((_, maps)) = self.shown.as_ref() {
            let interval = maps.interval();
            self.selection.set_map_interval(interval);
        }
        let snapshot = self
            .shown
            .as_ref()
            .filter(|(shown, _)| Some(*shown) == day)
            .zip(self.selection.instant())
            .map(|((_, maps), instant)| TecHeatmapSnapshot { maps, instant });
        let empty_reason = self
            .selection
            .empty_reason(snapshot.as_ref().map_or(0, TecHeatmapSnapshot::node_count));
        TecLayer {
            snapshot,
            instant: &mut self.selection,
            empty_reason,
        }
    }

    /// TEC values for the plot: one point per fix of every loaded track,
    /// interpolated from the archived maps over that fix's own position and
    /// time.
    ///
    /// A track's points are rebuilt only when the archive gains one of the
    /// days it spans, so the `Arc` the plot caches on stays stable.
    pub fn plot_series(&mut self, files: &[LoadedFile]) -> TecSeries {
        let mut series = TecSeries::default();
        let mut live: FxHashSet<TrackRef> = FxHashSet::default();
        // Shared across tracks: a batch of recordings from one drive all read
        // the same day.
        let mut archived: FxHashMap<NaiveDate, Option<GlobalIonosphereMaps>> = FxHashMap::default();

        for (fi, file) in files.iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref =
                    TrackRef::new(gt_types::FileIdx::new(fi), gt_types::TrackIdx::new(ti));
                live.insert(track_ref);

                let archived_days = self.archived_days_spanned_by(track);
                let points = self.plot_points.resolve(track_ref, archived_days, || {
                    Self::resolve_points(self.store.as_deref(), &mut archived, track)
                });
                if points.iter().any(|point| point.tecu.is_some()) {
                    series.points_by_track.insert(track_ref, points);
                }
            }
        }
        self.plot_points.retain_loaded_tracks(&live);
        series
    }

    /// The peak deviation of each loaded track's TEC from the quiet-time
    /// background of the same grid node and time of day.
    ///
    /// A track without a value carries no entry: a window the archive holds
    /// too little of yields no background at all.
    pub fn quiet_time_deviations(
        &mut self,
        files: &[LoadedFile],
    ) -> FxHashMap<TrackRef, QuietTimeDeviation> {
        self.quiet_time
            .resolve(self.store.as_deref(), &self.archived_days, files)
    }

    /// The TEC line across `span`: one sample per archived map epoch, read at
    /// the position the receiver was in nearest that epoch in time.
    ///
    /// The epochs are the ones the producer published, two hours apart in a
    /// final file and one in a rapid one. Days the archive holds no maps for
    /// break the line.
    pub fn context_line(
        &mut self,
        span: ContextSpan,
        positions: &Arc<FixPositionTimeline>,
    ) -> Arc<Vec<TecContextSample>> {
        let source = ContextSource {
            span,
            archived_days: self.archived_days.range(span.days()).copied().collect(),
            positions: Some(ArcIdentity::of(positions)),
        };
        let store = self.store.as_ref().map(Arc::clone);
        let positions = Arc::clone(positions);
        self.context.resolve(
            source,
            |day| context_day(store.as_deref(), &positions, day),
            |day| {
                Some(TecContextSample {
                    x_secs: midnight_secs(day),
                    tecu: None,
                })
            },
        )
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
        archived: &mut FxHashMap<NaiveDate, Option<GlobalIonosphereMaps>>,
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
        if self.pending_writes.refusal().is_some() {
            return;
        }
        let Some(day) = self.days.take_next_day() else {
            return;
        };
        let transport = self.transport();
        self.spawn_fetch(transport, store, day);
    }

    fn spawn_fetch(&self, transport: Arc<Connection>, store: Arc<IonexStore>, day: NaiveDate) {
        let ctx = self.ctx.clone();
        let tx = self.tx.clone();
        let mirrors = self.mirrors.clone();
        let earthdata_token = self.earthdata_token.clone();
        let pending_writes = self.pending_writes.clone();
        background_thread::spawn_or_panic(format!("tec-{day}"), move || {
            let message = ingest(
                transport.as_ref(),
                &store,
                &mirrors,
                earthdata_token.as_ref(),
                day,
                &pending_writes,
            );
            tx.send(message).ok();
            ctx.request_repaint();
        });
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

/// One archived day's samples of the context line: the value over the
/// receiver's nearest position in time, at each epoch the day was archived
/// with.
///
/// A day with no recording to place its epochs at contributes nothing, and so
/// breaks the line like a day the archive does not hold.
fn context_day(
    store: Option<&IonexStore>,
    positions: &FixPositionTimeline,
    day: NaiveDate,
) -> Vec<TecContextSample> {
    let Some(maps) = read_archived_maps(store, day) else {
        return Vec::new();
    };
    maps.maps()
        .iter()
        .map(|map| {
            let epoch = map.epoch();
            TecContextSample {
                x_secs: epoch.timestamp() as f64,
                tecu: positions
                    .nearest_position(epoch)
                    .and_then(|(latitude, longitude)| {
                        maps.total_electron_content_at(latitude, longitude, epoch)
                    })
                    .map(TotalElectronContent::tecu),
            }
        })
        .collect()
}

/// The maps archived for `day`, reporting a read that failed and treating it
/// as an unarchived day.
pub(super) fn read_archived_maps(
    store: Option<&IonexStore>,
    day: NaiveDate,
) -> Option<GlobalIonosphereMaps> {
    store?
        .day_maps(day)
        .inspect_err(|err| log::error!("Reading the archived TEC maps for {day}: {err}"))
        .ok()
        .flatten()
}

/// Every day the archive holds maps for.
fn archived_days_of(store: &IonexStore) -> Result<BTreeSet<NaiveDate>, IonexStoreError> {
    Ok(store
        .archived_days()?
        .into_iter()
        .map(|archived| archived.day)
        .collect())
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
    earthdata_token: Option<&SecretToken>,
    day: NaiveDate,
    pending_writes: &PendingWrites,
) -> MapDayMessage {
    let today = Utc::now().date_naive();
    let (mirror, product, maps, skipped) =
        match transport::fetch_day_maps(transport, mirrors, earthdata_token, day, today) {
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

    let _write = match EnvironmentArchive::IonosphericTec.try_begin_day_insert(pending_writes, day)
    {
        Ok(write) => write,
        Err(refusal) => return MapDayMessage::NotArchived { day, refusal },
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
    use std::time::Duration;

    use chrono::{DateTime, Datelike as _, TimeDelta};
    use rstest::rstest;
    use tempfile::TempDir;

    use crate::app::backfill::BackfillProgress;
    use crate::app::day_failures::DayFailure;
    use crate::app::day_fetch_status::{ArchivedDayCount, DayFetchStatus};
    use crate::app::fix_positions::FixPositions;
    use gt_fetch::BytesResponse;
    use gt_ionex::quiet_time::IonosphericStormGrade;
    use gt_ionex::{DEFAULT_BASE_URL, MirrorLayout};
    use gt_pending_writes::WriteAccess;
    use gt_store::Store;
    use gt_test_utils::{ScriptedTransport, UrlPrefixAnswers, ionex_fixtures, pending_writes};

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
            None,
            TransportSource::Offline,
            PendingWrites::default(),
        );
        (dir, store, scheduler)
    }

    /// A scheduler with no archive to write to, so it fetches nothing.
    fn scheduler_without_archive() -> TecMapScheduler {
        TecMapScheduler::new(
            Context::default(),
            None,
            MirrorList::default(),
            None,
            TransportSource::Offline,
            PendingWrites::default(),
        )
    }

    fn mirrors(hosts: &[&str]) -> MirrorList {
        MirrorList::new(
            hosts
                .iter()
                .map(|host| gt_ionex::Mirror::new(MirrorBaseUrl::new(*host), MirrorLayout::Jpl))
                .collect(),
        )
        .expect("a named host")
    }

    /// The publishing host alone, which every ingest test fetches from.
    fn publishing_host() -> MirrorList {
        mirrors(&[DEFAULT_BASE_URL])
    }

    /// Archive `archived` as a whole day from `product`, the way a finished
    /// ingest leaves it.
    fn archive_day(store: &IonexStore, archived: NaiveDate, product: IonexProduct) {
        let maps = gt_ionex::parse::global_ionosphere_maps(&published_file()).expect("parse");
        store
            .insert_or_replace_day(archived, "host", Utc::now(), product, &maps)
            .expect("insert");
    }

    /// A read-only session reads the day index beside the instance that owns
    /// the data directory, so the read at [`TecMapScheduler::adopt_store`] can
    /// fail on that instance's open. Without a re-read the session draws no
    /// TEC for the rest of its run.
    #[test]
    fn a_day_index_read_that_failed_on_another_process_is_run_again_and_finds_the_days() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 10), IonexProduct::Final);
        let failed = scheduler.day_index_read.record_read(
            &scheduler.ctx,
            Err::<BTreeSet<NaiveDate>, _>(IonexStoreError::HeldByAnotherProcess),
        );
        assert_eq!(failed, None);
        assert_eq!(scheduler.archived_days_covered(PrunedDays::All), 0);

        scheduler.reread_the_day_index_when_due(Instant::now() + Duration::from_secs(60));

        assert_eq!(scheduler.archived_days_covered(PrunedDays::All), 1);
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

        assert!(
            !scheduler.days.requested_days().contains(&day(2024, 5, 10)),
            "the archived recording day stayed off the queue"
        );
    }

    /// A recording day is read against the quiet-time window before it, so
    /// the 27 days preceding it go out too and no later day does. A day the
    /// archive already holds is counted without being requested.
    #[test]
    fn a_recording_day_queues_the_quiet_time_window_before_it() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_day(&store, day(2024, 5, 3), IonexProduct::Final);

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));

        let requested = scheduler.days.requested_days();
        assert!(
            requested.contains(&day(2024, 4, 13)),
            "the first day of the window"
        );
        assert!(
            requested.contains(&day(2024, 5, 9)),
            "the last day of the window"
        );
        assert!(
            !requested.contains(&day(2024, 4, 12)),
            "a day before the window"
        );
        assert!(
            !requested.contains(&day(2024, 5, 11)),
            "a day after the recording"
        );
        assert!(
            !requested.contains(&day(2024, 5, 3)),
            "a day the archive already holds"
        );
        assert_eq!(
            scheduler.background_day_coverage(),
            ArchivedDayCount {
                days: 27,
                archived: 1,
            }
        );
    }

    /// The window is clipped to what JPL publishes: a recording five days
    /// into the published record queues those five days alone.
    #[test]
    fn a_window_reaching_before_coverage_stops_at_the_first_published_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let recorded = calendar::COVERAGE_START + TimeDelta::days(5);

        scheduler.request_days_for(TimeRange::new(
            at(recorded.year(), recorded.month(), recorded.day(), 8),
            at(recorded.year(), recorded.month(), recorded.day(), 17),
        ));

        assert_eq!(
            scheduler.background_day_coverage(),
            ArchivedDayCount {
                days: 5,
                archived: 0,
            }
        );
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

    /// A token entered after days were passed over releases them, and their
    /// failures, for another request.
    #[test]
    fn changing_the_earthdata_token_drops_the_queue_and_its_failures() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 11, 17)));
        scheduler.days.report_failure(
            day(2024, 5, 10),
            format!("final: {}", gt_ionex::text::MIRROR_SKIPPED_WITHOUT_TOKEN),
        );

        scheduler.set_earthdata_token(SecretToken::new("entered-token"));

        assert!(
            scheduler.days.requested_days().is_empty(),
            "the days requested without a token"
        );
        assert_eq!(scheduler.days.queued(), 0);
        assert!(scheduler.days.failures().is_empty());
    }

    #[test]
    fn setting_the_same_earthdata_token_changes_nothing() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));
        let seen = scheduler.days.requested_days().len();

        scheduler.set_earthdata_token(None);

        assert_eq!(scheduler.days.requested_days().len(), seen);
    }

    /// A mirror serving an archive that needs a token is not requested while
    /// none is set, and the day reports why instead of archiving nothing
    /// quietly.
    #[test]
    fn a_day_is_not_requested_from_an_authenticated_archive_without_a_token() {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Ok(BytesResponse {
            status: 200,
            body: gzipped(&published_file()),
        }));

        let message = ingest(
            &transport,
            &store,
            &MirrorList::single(gt_ionex::Mirror::publishing(MirrorLayout::Cddis)),
            None,
            day(2024, 5, 10),
            &PendingWrites::default(),
        );

        match message {
            MapDayMessage::Failed { detail, .. } => assert!(
                detail.contains(gt_ionex::text::MIRROR_SKIPPED_WITHOUT_TOKEN),
                "{detail}"
            ),
            MapDayMessage::Stored { .. } | MapDayMessage::NotArchived { .. } => {
                panic!("the archive was never requested")
            }
        }
        assert!(transport.requested_urls().is_empty());
        assert!(store.archived_days().expect("days").is_empty());
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

        let message = ingest(
            &transport,
            &store,
            &publishing_host(),
            None,
            ingested,
            &PendingWrites::default(),
        );

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

        ingest(
            &transport,
            &store,
            &publishing_host(),
            None,
            ingested,
            &PendingWrites::default(),
        );

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

        ingest(
            &transport,
            &store,
            &publishing_host(),
            None,
            ingested,
            &PendingWrites::default(),
        );
        ingest(
            &transport,
            &store,
            &publishing_host(),
            None,
            ingested,
            &PendingWrites::default(),
        );

        assert_eq!(store.archived_days().expect("days").len(), 1);
    }

    /// A day queued while the registry refuses writes stays queued: no worker
    /// starts a download whose archive insert would be refused.
    #[rstest]
    #[case::shutting_down(pending_writes::shutting_down_registry())]
    #[case::read_only_session(PendingWrites::new(WriteAccess::ReadOnly))]
    fn no_day_is_dispatched_while_writes_are_refused(#[case] pending_writes: PendingWrites) {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.pending_writes = pending_writes;

        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));

        assert!(
            scheduler.days.requested_days().contains(&day(2024, 5, 10)),
            "the day left the queue"
        );
        assert!(!scheduler.days.is_fetching());
    }

    /// A download that finishes where the insert is refused is discarded, and
    /// says which refusal discarded it.
    #[rstest]
    #[case::shutting_down(pending_writes::shutting_down_registry(), WriteRefusal::ShuttingDown)]
    #[case::read_only_session(
        PendingWrites::new(WriteAccess::ReadOnly),
        WriteRefusal::ReadOnlySession
    )]
    fn a_day_downloaded_where_the_insert_is_refused_is_discarded(
        #[case] pending_writes: PendingWrites,
        #[case] expected: WriteRefusal,
    ) {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Ok(BytesResponse {
            status: 200,
            body: gzipped(&published_file()),
        }));

        let message = ingest(
            &transport,
            &store,
            &publishing_host(),
            None,
            day(2024, 5, 10),
            &pending_writes,
        );

        let MapDayMessage::NotArchived { refusal, .. } = message else {
            panic!("the day was archived where the insert is refused");
        };
        assert_eq!(refusal, expected);
        assert!(store.archived_days().expect("days").is_empty());
    }

    #[rstest]
    #[case::a_body_that_is_not_a_file(200, b"<html>captive portal</html>".to_vec())]
    #[case::a_server_error(500, Vec::new())]
    #[case::a_refused_request(403, Vec::new())]
    #[case::no_file_at_either_product(404, Vec::new())]
    fn a_day_that_cannot_be_read_archives_nothing(#[case] status: u16, #[case] body: Vec<u8>) {
        let (_dir, store) = archive();
        let transport = ScriptedTransport::always(Ok(BytesResponse { status, body }));

        let message = ingest(
            &transport,
            &store,
            &publishing_host(),
            None,
            day(2024, 5, 10),
            &PendingWrites::default(),
        );

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
            None,
            ingested,
            &PendingWrites::default(),
        );

        match message {
            MapDayMessage::Stored {
                mirror, skipped, ..
            } => {
                assert_eq!(mirror, MirrorBaseUrl::new("https://second.example"));
                assert_eq!(skipped.len(), 1, "the first mirror holds no file");
            }
            MapDayMessage::Failed { detail, .. } => panic!("{detail}"),
            MapDayMessage::NotArchived { .. } => {
                panic!("the day should have been archived, not refused")
            }
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
            None,
            day(2024, 5, 10),
            &PendingWrites::default(),
        );

        match message {
            MapDayMessage::Failed { day, detail } => {
                assert_eq!(
                    DayFailure { day, detail }.to_string(),
                    "2024-05-10 - final: https://first.example: HTTP 500 Internal Server Error, \
                     https://second.example: HTTP 500 Internal Server Error"
                );
            }
            MapDayMessage::Stored { .. } | MapDayMessage::NotArchived { .. } => {
                panic!("no mirror served a file")
            }
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
    #[case::not_archived(MapDayMessage::NotArchived {
        day: day(2024, 5, 10),
        refusal: WriteRefusal::ShuttingDown,
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

        // The two recording days' windows run from 13 April to 9 May, 27 days
        // behind the recording day in flight.
        assert_eq!(
            scheduler.days.fetch_status(),
            DayFetchStatus {
                fetching: Some(day(2024, 5, 11)),
                queued: 27,
                recording_days: ArchivedDayCount {
                    days: 2,
                    archived: 1,
                },
            }
        );
        assert_eq!(
            scheduler.background_day_coverage(),
            ArchivedDayCount {
                days: 27,
                archived: 0,
            }
        );
    }

    /// A recording made before JPL's coverage begins leaves the count empty,
    /// which the settings page shows as an absent value.
    #[test]
    fn a_day_outside_coverage_is_no_recording_day() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(1970, 1, 1, 0), at(1970, 1, 1, 1)));

        assert_eq!(scheduler.days.fetch_status().recording_days.days, 0);
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

        assert_eq!(scheduler.days.fetch_status().recording_days.archived, 1);
    }

    /// Offline, a queued day is still dispatched: the transport declines the
    /// request rather than the day staying queued.
    #[test]
    fn a_queued_day_is_dispatched() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 17)));

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
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        }]
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
                &ionex_fixtures::uniform_maps(archived, &[(22, 10.0), (24, 20.0)]),
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

    /// One day of maps at `step_hours` epochs, every node standing at `scale`
    /// times the hour of day. It stands in for the diurnal rise, linearly, so
    /// a clock time between two epochs interpolates to an exact value.
    fn rising_maps(archived: NaiveDate, step_hours: i64, scale: f64) -> GlobalIonosphereMaps {
        let samples: Vec<(i64, f64)> = (0..=(24 / step_hours))
            .map(|step| {
                let hours = step * step_hours;
                (hours, scale * hours as f64)
            })
            .collect();
        ionex_fixtures::uniform_maps(archived, &samples)
    }

    /// Archive one day of `maps` and record it as archived in the scheduler.
    fn archive_maps(
        scheduler: &mut TecMapScheduler,
        store: &IonexStore,
        archived: NaiveDate,
        maps: &GlobalIonosphereMaps,
    ) {
        store
            .insert_or_replace_day(archived, "host", Utc::now(), IonexProduct::Final, maps)
            .expect("insert");
        scheduler.archived_days.insert(archived);
    }

    /// The day the deviation tests record on, far enough from the calendar's
    /// ends for a whole window to exist.
    fn recorded_day() -> NaiveDate {
        day(2024, 5, 20)
    }

    /// The peak deviation of the one loaded track, or [`None`] where its
    /// window yields no background.
    fn peak_deviation(
        scheduler: &mut TecMapScheduler,
        files: &[gt_types::LoadedFile],
    ) -> Option<QuietTimeDeviation> {
        scheduler
            .quiet_time_deviations(files)
            .values()
            .copied()
            .next()
    }

    /// Archive the whole window before [`recorded_day`], the recording day
    /// itself rising at `recorded_scale` and every background day at one.
    fn archive_whole_window(
        scheduler: &mut TecMapScheduler,
        store: &IonexStore,
        recorded_scale: f64,
    ) {
        let recorded = recorded_day();
        archive_maps(
            scheduler,
            store,
            recorded,
            &rising_maps(recorded, 2, recorded_scale),
        );
        for days_before in 1..=quiet_time::BACKGROUND_WINDOW_DAYS as i64 {
            let archived = recorded - TimeDelta::days(days_before);
            archive_maps(scheduler, store, archived, &rising_maps(archived, 2, 1.0));
        }
    }

    /// A median is formed only once a majority of the 27 days before the
    /// recording is archived, so a handful of days never stands in for the
    /// quiet level.
    #[rstest]
    #[case::one_short_of_the_minimum(13, false)]
    #[case::the_minimum(14, true)]
    fn a_deviation_needs_a_majority_of_the_window_archived(
        #[case] archived_days: i64,
        #[case] expected: bool,
    ) {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let recorded = recorded_day();
        archive_maps(
            &mut scheduler,
            &store,
            recorded,
            &rising_maps(recorded, 2, 1.75),
        );
        for days_before in 1..=archived_days {
            let background = recorded - TimeDelta::days(days_before);
            archive_maps(
                &mut scheduler,
                &store,
                background,
                &rising_maps(background, 2, 1.0),
            );
        }
        let files = loaded_files_of(track_over(at(2024, 5, 20, 12), 4, 600));

        let deviation = peak_deviation(&mut scheduler, &files);

        assert_eq!(deviation.is_some(), expected);
        if let Some(deviation) = deviation {
            assert!(
                (deviation.percent_from_median() - 75.0).abs() < 1e-9,
                "{deviation:?}"
            );
            assert_eq!(deviation.grade(), IonosphericStormGrade::ModerateStorm);
        }
    }

    /// The background is read at the fix's own time of day, so a day that only
    /// repeats the ordinary diurnal rise sits at its quiet level and is
    /// graded quiet.
    #[test]
    fn a_diurnal_rise_every_day_repeats_is_no_deviation() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_whole_window(&mut scheduler, &store, 1.0);
        let files = loaded_files_of(track_over(at(2024, 5, 20, 12), 4, 600));

        let deviation = peak_deviation(&mut scheduler, &files).expect("a fully archived window");

        assert!(
            deviation.percent_from_median().abs() < 1e-9,
            "{deviation:?}"
        );
        assert_eq!(deviation.grade(), IonosphericStormGrade::Quiet);
    }

    /// A rapid day publishes maps an hour apart and a final day two, so a
    /// window holding both is read at the clock time the fix's own epoch names
    /// rather than by matching epochs off against each other. The final days
    /// interpolate between the two epochs bracketing that time.
    #[test]
    fn a_window_of_hourly_and_two_hourly_days_is_read_at_one_clock_time() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let recorded = recorded_day();
        archive_maps(
            &mut scheduler,
            &store,
            recorded,
            &rising_maps(recorded, 1, 3.0),
        );
        for days_before in 1..=quiet_time::BACKGROUND_WINDOW_DAYS as i64 {
            let archived = recorded - TimeDelta::days(days_before);
            let step_hours = if days_before % 2 == 0 { 2 } else { 1 };
            archive_maps(
                &mut scheduler,
                &store,
                archived,
                &rising_maps(archived, step_hours, 1.0),
            );
        }
        // 13:07 sits nearest the recording day's own 13:00 epoch, which the
        // two-hourly days reach only between their 12:00 and 14:00 maps.
        let files = loaded_files_of(track_over(
            at(2024, 5, 20, 13) + TimeDelta::minutes(7),
            4,
            60,
        ));

        let deviation = peak_deviation(&mut scheduler, &files).expect("a fully archived window");

        assert!(
            (deviation.percent_from_median() - 200.0).abs() < 1e-9,
            "{deviation:?}"
        );
        assert_eq!(deviation.grade(), IonosphericStormGrade::IntenseStorm);
    }

    /// A rapid day replaced by a final one holds different values under the
    /// same date, so the day the ingest reports is read again.
    #[test]
    fn a_day_archived_again_is_read_into_the_deviation_again() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_whole_window(&mut scheduler, &store, 1.75);
        let files = loaded_files_of(track_over(at(2024, 5, 20, 12), 4, 600));

        let first = peak_deviation(&mut scheduler, &files).expect("a fully archived window");
        assert!(
            (first.percent_from_median() - 75.0).abs() < 1e-9,
            "{first:?}"
        );

        let recorded = recorded_day();
        store
            .insert_or_replace_day(
                recorded,
                "host",
                Utc::now(),
                IonexProduct::Final,
                &rising_maps(recorded, 2, 3.0),
            )
            .expect("insert");
        scheduler
            .tx
            .send(MapDayMessage::Stored {
                day: recorded,
                mirror: MirrorBaseUrl::new(DEFAULT_BASE_URL),
                product: IonexProduct::Final,
                map_count: 13,
                skipped: Vec::new(),
            })
            .expect("send");
        scheduler.poll();

        let revised = peak_deviation(&mut scheduler, &files).expect("a fully archived window");

        assert!(
            (revised.percent_from_median() - 200.0).abs() < 1e-9,
            "{revised:?}"
        );
    }

    /// A recording that is closed takes its deviation with it.
    #[test]
    fn unloading_a_recording_drops_its_deviation() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        archive_whole_window(&mut scheduler, &store, 1.75);
        let files = loaded_files_of(track_over(at(2024, 5, 20, 12), 4, 600));

        assert_eq!(scheduler.quiet_time_deviations(&files).len(), 1);
        assert!(scheduler.quiet_time_deviations(&[]).is_empty());
    }

    /// Loading a recording points the heatmap at its first fix's instant.
    #[test]
    fn the_earliest_loaded_recording_picks_the_instant() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 12, 8), at(2024, 5, 12, 9)));
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 8), at(2024, 5, 10, 9)));

        assert_eq!(
            scheduler.overlay_layer().instant.instant(),
            Some(at(2024, 5, 10, 8))
        );
    }

    /// The heatmap draws the archived day of the instant it shows, and reports
    /// how many nodes that day's grid holds.
    #[test]
    fn the_heatmap_draws_the_archived_day_of_the_shown_instant() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        archive_last_maps_of(&store, archived);
        scheduler.archived_days.insert(archived);
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 22), at(2024, 5, 10, 23)));

        let layer = scheduler.overlay_layer();
        let snapshot = layer.snapshot.expect("the archived day draws");
        assert_eq!(snapshot.instant, at(2024, 5, 10, 22));
        assert_eq!(snapshot.node_count(), 3 * 2);
        assert_eq!(layer.empty_reason, None);
    }

    /// A hovered or selected fix moves the heatmap to its own time, and
    /// letting go hands it back to the stepper.
    #[test]
    fn the_heatmap_follows_the_hovered_fix() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        archive_last_maps_of(&store, archived);
        scheduler.archived_days.insert(archived);
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 22), at(2024, 5, 10, 23)));

        scheduler.follow_instant(Some(at(2024, 5, 10, 23)));
        assert_eq!(
            scheduler
                .overlay_layer()
                .snapshot
                .map(|snapshot| snapshot.instant),
            Some(at(2024, 5, 10, 23))
        );

        scheduler.follow_instant(None);
        assert_eq!(
            scheduler
                .overlay_layer()
                .snapshot
                .map(|snapshot| snapshot.instant),
            Some(at(2024, 5, 10, 22))
        );
    }

    /// A fix hovered on another day moves the heatmap to that day's own maps.
    #[test]
    fn following_a_fix_on_another_day_reads_that_days_maps() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for (archived, tecu) in [(day(2024, 5, 10), 10.0), (day(2024, 5, 11), 40.0)] {
            store
                .insert_or_replace_day(
                    archived,
                    "host",
                    Utc::now(),
                    IonexProduct::Final,
                    &ionex_fixtures::uniform_maps(archived, &[(0, tecu), (24, tecu)]),
                )
                .expect("insert");
            scheduler.archived_days.insert(archived);
        }
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 12), at(2024, 5, 10, 13)));

        scheduler.follow_instant(Some(at(2024, 5, 11, 12)));
        let layer = scheduler.overlay_layer();
        let snapshot = layer.snapshot.expect("the second day draws");
        let value = snapshot
            .maps
            .total_electron_content_at(
                gt_types::Latitude::new(55.0),
                gt_types::Longitude::new(12.5),
                at(2024, 5, 11, 12),
            )
            .map(TotalElectronContent::tecu);
        assert_eq!(value, Some(40.0));
    }

    /// An instant whose day the archive does not hold draws nothing, and the
    /// display toggle says why.
    #[rstest]
    #[case::not_archived(at(2024, 5, 10, 12), Some(gt_ionex::TecEmptyReason::NotArchived))]
    #[case::before_coverage(at(2005, 1, 1, 12), Some(gt_ionex::TecEmptyReason::BeforeCoverage))]
    fn an_instant_without_archived_maps_draws_nothing(
        #[case] instant: DateTime<Utc>,
        #[case] expected: Option<gt_ionex::TecEmptyReason>,
    ) {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        scheduler.follow_instant(Some(instant));

        let layer = scheduler.overlay_layer();
        assert!(layer.snapshot.is_none());
        assert_eq!(layer.empty_reason, expected);
    }

    /// With no recording loaded and nothing hovered, the toggle says there is
    /// no instant to draw.
    #[test]
    fn no_loaded_recording_means_no_instant() {
        let (_dir, _store, mut scheduler) = scheduler_with_archive();
        let layer = scheduler.overlay_layer();
        assert!(layer.snapshot.is_none());
        assert_eq!(layer.instant.instant(), None);
        assert_eq!(layer.empty_reason, Some(gt_ionex::TecEmptyReason::NoTrack));
    }

    /// A day archived again from the settled product replaces what the heatmap
    /// draws.
    #[test]
    fn archiving_a_day_again_redraws_it() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        store
            .insert_or_replace_day(
                archived,
                "host",
                Utc::now(),
                IonexProduct::Rapid,
                &ionex_fixtures::uniform_maps(archived, &[(0, 10.0), (24, 10.0)]),
            )
            .expect("insert");
        scheduler.archived_days.insert(archived);
        scheduler.follow_instant(Some(at(2024, 5, 10, 12)));
        assert!(scheduler.overlay_layer().snapshot.is_some());

        store
            .insert_or_replace_day(
                archived,
                "host",
                Utc::now(),
                IonexProduct::Final,
                &ionex_fixtures::uniform_maps(archived, &[(0, 55.0), (24, 55.0)]),
            )
            .expect("insert");
        scheduler
            .tx
            .send(MapDayMessage::Stored {
                day: archived,
                mirror: MirrorBaseUrl::new("host"),
                product: IonexProduct::Final,
                map_count: 2,
                skipped: Vec::new(),
            })
            .expect("send");
        scheduler.poll();

        let layer = scheduler.overlay_layer();
        let value = layer
            .snapshot
            .expect("the replaced day draws")
            .maps
            .total_electron_content_at(
                gt_types::Latitude::new(55.0),
                gt_types::Longitude::new(12.5),
                at(2024, 5, 10, 12),
            )
            .map(TotalElectronContent::tecu);
        assert_eq!(value, Some(55.0));
    }

    /// The stepper moves by the interval the archived day declares.
    #[test]
    fn the_stepper_moves_by_the_archived_days_map_interval() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        archive_last_maps_of(&store, archived);
        scheduler.archived_days.insert(archived);
        scheduler.request_days_for(TimeRange::new(at(2024, 5, 10, 22), at(2024, 5, 10, 23)));

        scheduler.overlay_layer().instant.step_back();
        assert_eq!(
            scheduler.overlay_layer().instant.instant(),
            Some(at(2024, 5, 10, 20)),
            "the archived day publishes a map every two hours"
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

    /// The context line over the UTC days `from..=to`.
    fn context_line_over(
        scheduler: &mut TecMapScheduler,
        positions: &Arc<FixPositionTimeline>,
        days: std::ops::RangeInclusive<NaiveDate>,
    ) -> Arc<Vec<TecContextSample>> {
        let midnight =
            |day: NaiveDate| day.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as f64;
        scheduler.context_line(
            ContextSpan::covering(midnight(*days.start())..=midnight(*days.end())),
            positions,
        )
    }

    fn timeline_of(track: gt_types::LoadedTrack) -> Arc<FixPositionTimeline> {
        let mut positions = FixPositions::default();
        Arc::clone(positions.timeline(&loaded_files_of(track)))
    }

    /// The line carries one sample per archived map epoch, valued at the
    /// position of the fix nearest that epoch, including epochs hours away
    /// from any recording.
    #[test]
    fn the_context_line_samples_every_archived_epoch() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        let archived = day(2024, 5, 10);
        store
            .insert_or_replace_day(
                archived,
                "host",
                Utc::now(),
                IonexProduct::Final,
                &ionex_fixtures::uniform_maps(archived, &[(0, 5.0), (2, 10.0), (4, 20.0)]),
            )
            .expect("insert");
        scheduler.archived_days.insert(archived);
        let timeline = timeline_of(track_over(at(2024, 5, 10, 22), 4, 1800));

        let line = context_line_over(&mut scheduler, &timeline, archived..=archived);

        let midnight = at(2024, 5, 10, 0).timestamp() as f64;
        assert_eq!(
            line.iter()
                .map(|sample| (sample.x_secs - midnight, sample.tecu))
                .collect::<Vec<_>>(),
            [
                (0.0, Some(5.0)),
                (7200.0, Some(10.0)),
                (14400.0, Some(20.0)),
            ]
        );
    }

    /// A day the archive does not hold breaks the line between the days that
    /// surround it.
    #[test]
    fn an_unarchived_day_breaks_the_context_line() {
        let (_dir, store, mut scheduler) = scheduler_with_archive();
        for archived in [day(2024, 5, 10), day(2024, 5, 12)] {
            store
                .insert_or_replace_day(
                    archived,
                    "host",
                    Utc::now(),
                    IonexProduct::Final,
                    &ionex_fixtures::uniform_maps(archived, &[(0, 5.0)]),
                )
                .expect("insert");
            scheduler.archived_days.insert(archived);
        }
        let timeline = timeline_of(track_over(at(2024, 5, 10, 22), 4, 1800));

        let line = context_line_over(
            &mut scheduler,
            &timeline,
            day(2024, 5, 10)..=day(2024, 5, 12),
        );

        assert_eq!(
            line.iter().map(|sample| sample.tecu).collect::<Vec<_>>(),
            [Some(5.0), None, Some(5.0)]
        );
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
