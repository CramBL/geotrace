//! How far each loaded track's archived TEC stands from the quiet-time
//! background of the same grid node and time of day, which is the median over
//! the 27 days before each day the track records on.
//!
//! A track is reduced to one assessment point per grid node and map epoch its
//! fixes fall on, so a track sitting in one cell costs one node whatever its
//! fix rate. Both sides of the comparison are read the same way, as one node's
//! value between the two maps bracketing an instant, so a gradient across the
//! cell cannot read as a deviation.
//!
//! How long the storm grade held around a track's peak is read across every
//! epoch that day published at the peak's own node, not only the epochs the
//! track's fixes fall on: the environment persists whether or not the receiver
//! kept recording through it.
//!
//! Every day of a point's window is read at the same offset from the start of
//! the day, which keeps the diurnal cycle out of the comparison and lets a
//! final day at two-hour epochs and a rapid day at one-hour epochs sit in one
//! window: an offset a day's own epochs do not name is interpolated between
//! the two that bracket it. The offset reaches 24 h, since a published file
//! dates its last map to the next day's midnight, and every sample is read
//! from the file of the day it is filed under.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};

use gt_ionex::grid::GridPoint;
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::quiet_time::{self, QuietTimeDeviation, QuietTimeDeviationPeak, StormGradeRun};
use gt_ionex::tec::TotalElectronContent;
use gt_store::ReadOnlyIonexStore;
use gt_types::{FileIdx, LoadedFile, LoadedTrack, TrackIdx, TrackRef};
use rustc_hash::{FxHashMap, FxHashSet};

use super::environment_storage::PrunedDays;
use super::tec::read_archived_maps;

/// One node's value on one archived day, read from that day's own file at an
/// offset from the start of the day.
///
/// A track's fixes reduce to these, one deviation assessed at each, and the
/// window behind one is the same node and offset on the days before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NodeSample {
    day: NaiveDate,
    node: GridPoint,
    epoch_since_day_start: TimeDelta,
}

impl NodeSample {
    fn on_day(self, day: NaiveDate) -> Self {
        Self { day, ..self }
    }

    /// [`None`] for an offset that runs off the end of the calendar.
    fn instant(self) -> Option<DateTime<Utc>> {
        start_of_day(self.day).checked_add_signed(self.epoch_since_day_start)
    }
}

/// The strongest deviation one track reaches, and the archived days it was
/// read from.
struct TrackPeak {
    resolved_from: Vec<NaiveDate>,
    peak: Option<QuietTimeDeviationPeak>,
}

/// The epochs one archived day's file names, as offsets into that day, and how
/// long one of them covers.
struct PublishedEpochs {
    interval: TimeDelta,
    offsets_since_day_start: Vec<TimeDelta>,
}

impl PublishedEpochs {
    fn of(maps: &GlobalIonosphereMaps, day: NaiveDate) -> Self {
        Self {
            interval: maps.interval(),
            offsets_since_day_start: maps
                .maps()
                .iter()
                .map(|map| map.epoch().signed_duration_since(start_of_day(day)))
                .collect(),
        }
    }

    /// `peak`'s own node at every epoch of its day, which is what the
    /// storm-grade run is read over: the environment persists whether or not
    /// the track kept recording through it.
    fn samples_across_the_day(&self, peak: NodeSample) -> Vec<NodeSample> {
        self.offsets_since_day_start
            .iter()
            .map(|offset| NodeSample {
                epoch_since_day_start: *offset,
                ..peak
            })
            .collect()
    }
}

/// One track waiting to be read, with the archived days its peak will be
/// filed under.
struct PendingTrack<'a> {
    track_ref: TrackRef,
    track: &'a LoadedTrack,
    resolved_from: Vec<NaiveDate>,
}

/// The peak quiet-time deviation of every loaded track, and the node samples
/// the medians behind them were formed from.
///
/// A track is read again only when the archived days its windows cover change,
/// and a day's samples are kept, so a background day arriving late costs one
/// day's read.
#[derive(Default)]
pub struct QuietTimeDeviationCache {
    peaks: FxHashMap<TrackRef, TrackPeak>,
    samples: FxHashMap<NodeSample, Option<TotalElectronContent>>,
}

impl QuietTimeDeviationCache {
    /// Drop what was read for `day`, so an archive the fetch worker revised is
    /// read again.
    pub fn forget(&mut self, day: NaiveDate) {
        self.samples.retain(|sample, _| sample.day != day);
        self.peaks
            .retain(|_, peak| !peak.resolved_from.contains(&day));
    }

    /// Drop what was read for every day a delete removed from the archive.
    pub fn forget_pruned_days(&mut self, pruned: PrunedDays) {
        self.samples.retain(|sample, _| !pruned.covers(sample.day));
        self.peaks
            .retain(|_, peak| !peak.resolved_from.iter().any(|day| pruned.covers(*day)));
    }

    /// Read every loaded track whose archived days moved, and report the peak
    /// deviation of each track that has one.
    pub fn resolve(
        &mut self,
        store: Option<&ReadOnlyIonexStore>,
        archived_days: &BTreeSet<NaiveDate>,
        files: &[LoadedFile],
    ) -> FxHashMap<TrackRef, QuietTimeDeviationPeak> {
        let mut live: FxHashSet<TrackRef> = FxHashSet::default();
        let mut assessed_days: FxHashSet<NaiveDate> = FxHashSet::default();
        let mut pending: Vec<PendingTrack<'_>> = Vec::new();

        for (file_index, file) in files.iter().enumerate() {
            for (track_index, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(file_index), TrackIdx::new(track_index));
                live.insert(track_ref);
                let days_read = days_read_for_track(track);
                assessed_days.extend(days_read.iter().copied());
                let resolved_from: Vec<NaiveDate> = days_read
                    .into_iter()
                    .filter(|day| archived_days.contains(day))
                    .collect();
                if self
                    .peaks
                    .get(&track_ref)
                    .is_some_and(|peak| peak.resolved_from == resolved_from)
                {
                    continue;
                }
                pending.push(PendingTrack {
                    track_ref,
                    track,
                    resolved_from,
                });
            }
        }

        if !pending.is_empty() {
            self.read_pending_tracks(store, pending);
        }
        self.peaks.retain(|track, _| live.contains(track));
        self.samples
            .retain(|sample, _| assessed_days.contains(&sample.day));

        self.peaks
            .iter()
            .filter_map(|(track, held)| Some((*track, held.peak?)))
            .collect()
    }

    /// Assess `pending`, reading each day of their windows once.
    ///
    /// The peak of each track is read first, and the rest of its own day is
    /// read after it: the storm-grade run beside a peak covers epochs no fix
    /// of the track falls on.
    fn read_pending_tracks(
        &mut self,
        store: Option<&ReadOnlyIonexStore>,
        pending: Vec<PendingTrack<'_>>,
    ) {
        let mut own_days: FxHashMap<NaiveDate, Option<GlobalIonosphereMaps>> = FxHashMap::default();
        let assessed: Vec<(PendingTrack<'_>, FxHashSet<NodeSample>)> = pending
            .into_iter()
            .map(|pending| {
                let points = assessment_points(store, &mut own_days, pending.track);
                (pending, points)
            })
            .collect();
        let published_epochs: FxHashMap<NaiveDate, PublishedEpochs> = own_days
            .iter()
            .filter_map(|(day, maps)| Some((*day, PublishedEpochs::of(maps.as_ref()?, *day))))
            .collect();
        drop(own_days);

        let points: FxHashSet<NodeSample> = assessed
            .iter()
            .flat_map(|(_, points)| points.iter().copied())
            .collect();
        self.read_missing_samples(store, &points);

        let peaks: Vec<(PendingTrack<'_>, Option<NodeSample>)> = assessed
            .into_iter()
            .map(|(pending, points)| {
                let peak = self.peak_point_of(&points);
                (pending, peak)
            })
            .collect();
        let across_the_peak_days: FxHashSet<NodeSample> = peaks
            .iter()
            .filter_map(|(_, peak)| *peak)
            .filter_map(|peak| {
                Some(
                    published_epochs
                        .get(&peak.day)?
                        .samples_across_the_day(peak),
                )
            })
            .flatten()
            .collect();
        self.read_missing_samples(store, &across_the_peak_days);

        for (pending, peak) in peaks {
            let peak = peak.and_then(|point| self.peak_at(point, published_epochs.get(&point.day)));
            self.peaks.insert(
                pending.track_ref,
                TrackPeak {
                    resolved_from: pending.resolved_from,
                    peak,
                },
            );
        }
    }

    /// Read every sample the points need that is not already held, one day's
    /// maps at a time.
    fn read_missing_samples(
        &mut self,
        store: Option<&ReadOnlyIonexStore>,
        points: &FxHashSet<NodeSample>,
    ) {
        let mut missing: BTreeMap<NaiveDate, Vec<NodeSample>> = BTreeMap::new();
        for point in points {
            for day in days_read_for_day(point.day) {
                let sample = point.on_day(day);
                if !self.samples.contains_key(&sample) {
                    missing.entry(day).or_default().push(sample);
                }
            }
        }

        for (day, samples) in missing {
            let maps = read_archived_maps(store, day);
            for sample in samples {
                let value = maps
                    .as_ref()
                    .zip(sample.instant())
                    .and_then(|(maps, instant)| maps.node_value_at(sample.node, instant));
                self.samples.insert(sample, value);
            }
        }
    }

    /// The point standing furthest from its own quiet-time median, the
    /// earliest of them where two stand equally far.
    fn peak_point_of(&self, points: &FxHashSet<NodeSample>) -> Option<NodeSample> {
        points
            .iter()
            .filter_map(|point| Some((*point, self.deviation_at(*point)?)))
            .max_by(|(left_point, left), (right_point, right)| {
                left.log_ratio()
                    .abs()
                    .total_cmp(&right.log_ratio().abs())
                    .then_with(|| {
                        left_point
                            .day
                            .cmp(&right_point.day)
                            .then_with(|| {
                                left_point
                                    .epoch_since_day_start
                                    .cmp(&right_point.epoch_since_day_start)
                            })
                            .reverse()
                    })
            })
            .map(|(point, _)| point)
    }

    /// The deviation at `point` with the storm-grade run it stands in, read
    /// over `epochs`, the epochs its own day publishes.
    fn peak_at(
        &self,
        point: NodeSample,
        epochs: Option<&PublishedEpochs>,
    ) -> Option<QuietTimeDeviationPeak> {
        Some(QuietTimeDeviationPeak {
            deviation: self.deviation_at(point)?,
            epoch: point.instant()?,
            storm_grade_run: epochs.and_then(|epochs| self.storm_grade_run_at(point, epochs)),
        })
    }

    fn storm_grade_run_at(
        &self,
        peak: NodeSample,
        epochs: &PublishedEpochs,
    ) -> Option<StormGradeRun> {
        let day_epochs: Vec<Option<QuietTimeDeviation>> = epochs
            .samples_across_the_day(peak)
            .into_iter()
            .map(|sample| self.deviation_at(sample))
            .collect();
        let epoch_index = epochs
            .offsets_since_day_start
            .iter()
            .position(|offset| *offset == peak.epoch_since_day_start)?;
        StormGradeRun::containing_epoch(&day_epochs, epoch_index, epochs.interval)
    }

    fn deviation_at(&self, point: NodeSample) -> Option<QuietTimeDeviation> {
        let sample_on = |day: NaiveDate| self.samples.get(&point.on_day(day)).copied().flatten();
        let window: Vec<TotalElectronContent> = quiet_time::background_days(point.day)
            .into_iter()
            .filter_map(sample_on)
            .collect();
        quiet_time::deviation_from_quiet_time(sample_on(point.day)?, &window)
    }
}

/// Every UTC day a track's deviations are read from: each day its fixes fall
/// in and the quiet-time window before that day, oldest first.
fn days_read_for_track(track: &LoadedTrack) -> BTreeSet<NaiveDate> {
    let range = track.metadata.time_range;
    gt_types::utc_days::days_in_range(range.start.date_naive()..=range.end.date_naive(), |_| true)
        .into_iter()
        .flat_map(days_read_for_day)
        .collect()
}

/// One assessed day and the quiet-time window before it, oldest first.
fn days_read_for_day(day: NaiveDate) -> Vec<NaiveDate> {
    let mut days = quiet_time::background_days(day);
    days.push(day);
    days
}

/// The grid node and map epoch each of a track's fixes falls on, one entry per
/// distinct pair.
///
/// A fix whose day the archive does not hold places no point: the grid and the
/// epochs both come from that day's own published file. A fix past the last
/// full epoch of its day falls on the map that file dates to the next day's
/// midnight, which is 24 h into the fix's own day.
fn assessment_points(
    store: Option<&ReadOnlyIonexStore>,
    own_days: &mut FxHashMap<NaiveDate, Option<GlobalIonosphereMaps>>,
    track: &LoadedTrack,
) -> FxHashSet<NodeSample> {
    track
        .placed_points()
        .into_iter()
        .flat_map(|placed| placed.iter())
        .filter_map(|point| {
            let time = point.fix.tpv.time().utc();
            let day = time.date_naive();
            let maps = own_days
                .entry(day)
                .or_insert_with(|| read_archived_maps(store, day))
                .as_ref()?;
            let epoch = maps.nearest_epoch(time)?;
            let (latitude, longitude) = point.resolved_position();
            Some(NodeSample {
                day,
                node: maps.grid().nearest_node(latitude, longitude)?,
                epoch_since_day_start: epoch.signed_duration_since(start_of_day(day)),
            })
        })
        .collect()
}

fn start_of_day(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}
