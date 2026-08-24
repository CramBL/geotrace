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
//! Every day of a point's window is read at the same UTC clock time, which
//! keeps the diurnal cycle out of the comparison and lets a final day at
//! two-hour epochs and a rapid day at one-hour epochs sit in one window: a
//! clock time a day's own epochs do not name is interpolated between the two
//! that bracket it.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};

use gt_ionex::grid::GridPoint;
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::quiet_time::{self, QuietTimeDeviation};
use gt_ionex::tec::TotalElectronContent;
use gt_store::ReadOnlyIonexStore;
use gt_types::{FileIdx, LoadedFile, LoadedTrack, TrackIdx, TrackRef};
use rustc_hash::{FxHashMap, FxHashSet};

use super::environment_storage::PrunedDays;
use super::tec::read_archived_maps;

/// One grid node and map epoch a track's fixes reach, which one deviation is
/// assessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AssessmentPoint {
    node: GridPoint,
    epoch: DateTime<Utc>,
}

/// One node's value on one archived day at one UTC clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NodeSample {
    day: NaiveDate,
    node: GridPoint,
    time_of_day: NaiveTime,
}

/// The strongest deviation one track reaches, and the archived days it was
/// read from.
struct TrackPeak {
    resolved_from: Vec<NaiveDate>,
    deviation: Option<QuietTimeDeviation>,
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
    ) -> FxHashMap<TrackRef, QuietTimeDeviation> {
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
            .filter_map(|(track, peak)| Some((*track, peak.deviation?)))
            .collect()
    }

    /// Assess `pending`, reading each day of their windows once.
    fn read_pending_tracks(
        &mut self,
        store: Option<&ReadOnlyIonexStore>,
        pending: Vec<PendingTrack<'_>>,
    ) {
        let mut own_days: FxHashMap<NaiveDate, Option<GlobalIonosphereMaps>> = FxHashMap::default();
        let assessed: Vec<(PendingTrack<'_>, FxHashSet<AssessmentPoint>)> = pending
            .into_iter()
            .map(|pending| {
                let points = assessment_points(store, &mut own_days, pending.track);
                (pending, points)
            })
            .collect();
        drop(own_days);

        let points: FxHashSet<AssessmentPoint> = assessed
            .iter()
            .flat_map(|(_, points)| points.iter().copied())
            .collect();
        self.read_missing_samples(store, &points);

        for (pending, points) in assessed {
            let deviation = self.peak_deviation_of(&points);
            self.peaks.insert(
                pending.track_ref,
                TrackPeak {
                    resolved_from: pending.resolved_from,
                    deviation,
                },
            );
        }
    }

    /// Read every sample the points need that is not already held, one day's
    /// maps at a time.
    fn read_missing_samples(
        &mut self,
        store: Option<&ReadOnlyIonexStore>,
        points: &FxHashSet<AssessmentPoint>,
    ) {
        let mut missing: BTreeMap<NaiveDate, Vec<NodeSample>> = BTreeMap::new();
        for point in points {
            for day in days_read_for_day(point.epoch.date_naive()) {
                let sample = NodeSample {
                    day,
                    node: point.node,
                    time_of_day: point.epoch.time(),
                };
                if !self.samples.contains_key(&sample) {
                    missing.entry(day).or_default().push(sample);
                }
            }
        }

        for (day, samples) in missing {
            let maps = read_archived_maps(store, day);
            for sample in samples {
                let value = maps.as_ref().and_then(|maps| {
                    maps.node_value_at(sample.node, day.and_time(sample.time_of_day).and_utc())
                });
                self.samples.insert(sample, value);
            }
        }
    }

    /// The point standing furthest from its own quiet-time median.
    fn peak_deviation_of(&self, points: &FxHashSet<AssessmentPoint>) -> Option<QuietTimeDeviation> {
        points
            .iter()
            .filter_map(|point| self.deviation_at(*point))
            .max_by(|left, right| left.log_ratio().abs().total_cmp(&right.log_ratio().abs()))
    }

    fn deviation_at(&self, point: AssessmentPoint) -> Option<QuietTimeDeviation> {
        let day = point.epoch.date_naive();
        let sample_on = |day: NaiveDate| {
            self.samples
                .get(&NodeSample {
                    day,
                    node: point.node,
                    time_of_day: point.epoch.time(),
                })
                .copied()
                .flatten()
        };
        let window: Vec<TotalElectronContent> = quiet_time::background_days(day)
            .into_iter()
            .filter_map(sample_on)
            .collect();
        quiet_time::deviation_from_quiet_time(sample_on(day)?, &window)
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
/// epochs both come from that day's own published file.
fn assessment_points(
    store: Option<&ReadOnlyIonexStore>,
    own_days: &mut FxHashMap<NaiveDate, Option<GlobalIonosphereMaps>>,
    track: &LoadedTrack,
) -> FxHashSet<AssessmentPoint> {
    track
        .points
        .iter()
        .filter_map(|point| {
            let time = point.tpv.time().utc();
            let maps = own_days
                .entry(time.date_naive())
                .or_insert_with(|| read_archived_maps(store, time.date_naive()))
                .as_ref()?;
            Some(AssessmentPoint {
                node: maps.grid().nearest_node(point.tpv.lat(), point.tpv.lon())?,
                epoch: maps.nearest_epoch(time)?,
            })
        })
        .collect()
}
