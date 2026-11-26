use std::sync::{Arc, Weak};

use gt_filter::GlobalFilter;
use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_types::{FileIdx, Generation, TrackIdx, TrackRef};
use gt_ui_types::{
    GeomagneticPoint, GeomagneticSeries, TecPoint, TecSeries, TrackDataVisibility, WeakArcIdentity,
};
use rustc_hash::FxHashMap;

use crate::dependencies::RunDependencies;
use crate::provider::CapturedTrackValues;

/// Per-track dense snap error values, one entry per track with a completed
/// snap run - handed in by the app each frame, shared with its per-run cache
/// (so the `Arc` identities are stable and change exactly when a run does).
pub type SnapErrorValues = FxHashMap<TrackRef, Arc<Vec<Option<f64>>>>;

/// Per-track interference percentages, one entry per fix. Shaped like
/// [`SnapErrorValues`] so both reach the provider the same way.
pub type JammingValues = FxHashMap<TrackRef, Arc<Vec<Option<f64>>>>;

/// Current recordings, filters and derived series for query runs.
#[derive(Clone, Copy)]
pub struct RunInputs<'a> {
    pub loaded_files: LoadedFilesView<'a>,
    pub visibility: &'a TrackDataVisibility,
    pub filter: &'a GlobalFilter,
    pub snap_errors: &'a SnapErrorValues,
    pub jamming: &'a JammingValues,
    /// The same per-fix geomagnetic index points the plot draws, so a query
    /// and the plot line read one resolution of the archive.
    pub geomagnetic: &'a GeomagneticSeries,
    /// The same per-fix TEC values the plot draws, shared like
    /// [`Self::geomagnetic`].
    pub tec: &'a TecSeries,
}

impl<'a> RunInputs<'a> {
    fn evaluated_tracks(self) -> impl Iterator<Item = TrackRef> + 'a {
        self.loaded_files
            .files()
            .iter()
            .enumerate()
            .flat_map(move |(fi, file)| {
                file.tracks
                    .iter()
                    .enumerate()
                    .filter_map(move |(ti, track)| {
                        let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                        (self.visibility.track_enabled(track_ref)
                            && gt_filter::track_passes_filter(track, self.filter))
                        .then_some(track_ref)
                    })
            })
    }

    pub(crate) fn values(
        self,
        track: TrackRef,
        dependencies: RunDependencies,
    ) -> TrackValueSources<'a> {
        TrackValueSources {
            snap_error: dependencies
                .uses_snap_error()
                .then(|| self.snap_errors.get(&track))
                .flatten(),
            jamming: dependencies
                .uses_jamming()
                .then(|| self.jamming.get(&track))
                .flatten(),
            geomagnetic: dependencies
                .uses_geomagnetic()
                .then(|| self.geomagnetic.points_by_track.get(&track))
                .flatten(),
            tec: dependencies
                .uses_tec()
                .then(|| self.tec.points_by_track.get(&track))
                .flatten(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TrackValueSources<'a> {
    snap_error: Option<&'a Arc<Vec<Option<f64>>>>,
    jamming: Option<&'a Arc<Vec<Option<f64>>>>,
    geomagnetic: Option<&'a Arc<Vec<GeomagneticPoint>>>,
    tec: Option<&'a Arc<Vec<TecPoint>>>,
}

impl TrackValueSources<'_> {
    pub(crate) fn capture(self) -> CapturedTrackValues {
        CapturedTrackValues {
            snap_error: self.snap_error.cloned(),
            jamming: self.jamming.cloned(),
            geomagnetic: self.geomagnetic.cloned(),
            tec: self.tec.cloned(),
        }
    }
}

struct TrackValueIdentities {
    snap_error: Option<WeakArcIdentity<Vec<Option<f64>>>>,
    jamming: Option<WeakArcIdentity<Vec<Option<f64>>>>,
    geomagnetic: Option<Weak<Vec<GeomagneticPoint>>>,
    tec: Option<WeakArcIdentity<Vec<TecPoint>>>,
}

impl TrackValueIdentities {
    fn of(values: TrackValueSources<'_>) -> Self {
        Self {
            snap_error: values.snap_error.map(WeakArcIdentity::of),
            jamming: values.jamming.map(WeakArcIdentity::of),
            geomagnetic: values.geomagnetic.map(Arc::downgrade),
            tec: values.tec.map(WeakArcIdentity::of),
        }
    }

    fn matches(&self, values: TrackValueSources<'_>, dependencies: RunDependencies) -> bool {
        self.snap_error == values.snap_error.map(WeakArcIdentity::of)
            && self.jamming == values.jamming.map(WeakArcIdentity::of)
            && self.tec == values.tec.map(WeakArcIdentity::of)
            && self.geomagnetic_matches(values.geomagnetic, dependencies)
    }

    fn geomagnetic_matches(
        &self,
        current: Option<&Arc<Vec<GeomagneticPoint>>>,
        dependencies: RunDependencies,
    ) -> bool {
        match (&self.geomagnetic, current) {
            (None, None) => return true,
            (Some(original), Some(current)) if original.as_ptr() == Arc::as_ptr(current) => {
                return true;
            }
            _ => {}
        }

        // The prepared run and point results retain the captured series.
        let original = self.geomagnetic.as_ref().and_then(Weak::upgrade);
        if original.is_none() && self.geomagnetic.is_some() {
            return false;
        }
        let before = original.as_deref().map_or(&[][..], Vec::as_slice);
        let after = current.map_or(&[][..], |points| points.as_slice());
        (0..before.len().max(after.len())).all(|index| {
            let before = before.get(index);
            let after = after.get(index);
            (!dependencies.hp30
                || before.and_then(|p| p.hp30).map(f64::to_bits)
                    == after.and_then(|p| p.hp30).map(f64::to_bits))
                && (!dependencies.kp
                    || before.and_then(|p| p.kp).map(f64::to_bits)
                        == after.and_then(|p| p.kp).map(f64::to_bits))
        })
    }
}

pub(crate) struct RunFingerprint {
    pub(crate) dependencies: RunDependencies,
    data_generation: Generation<RecordingData>,
    tracks: Vec<TrackRef>,
    filter: GlobalFilter,
    track_values: Vec<TrackValueIdentities>,
}

impl RunFingerprint {
    pub(crate) fn of(inputs: RunInputs<'_>, dependencies: RunDependencies) -> Self {
        let tracks: Vec<_> = inputs.evaluated_tracks().collect();
        let track_values = if dependencies.external_values() {
            tracks
                .iter()
                .map(|&track| TrackValueIdentities::of(inputs.values(track, dependencies)))
                .collect()
        } else {
            Vec::new()
        };
        Self {
            dependencies,
            data_generation: inputs.loaded_files.data_generation(),
            tracks,
            filter: *inputs.filter,
            track_values,
        }
    }

    pub(crate) fn tracks(&self) -> &[TrackRef] {
        &self.tracks
    }

    pub(crate) fn matches_loaded_files(&self, files: LoadedFilesView<'_>) -> bool {
        self.data_generation == files.data_generation()
    }

    pub(crate) fn matches(&self, inputs: RunInputs<'_>) -> bool {
        self.matches_loaded_files(inputs.loaded_files)
            && self.filter == *inputs.filter
            && self.tracks.iter().copied().eq(inputs.evaluated_tracks())
            && self
                .tracks
                .iter()
                .zip(&self.track_values)
                .all(|(&track, original)| {
                    original.matches(inputs.values(track, self.dependencies), self.dependencies)
                })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check;
    use crate::test_util::LoadedState;

    #[test]
    fn fingerprint_tracks_snap_run_identity_after_the_series_drops() {
        let mut state = LoadedState::with_channels(vec![]);
        let chunks = check::check_all("points | table snap_error", &state.schema());
        let dependencies = RunDependencies::of(&chunks);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let no_run = RunFingerprint::of(state.inputs(), dependencies);
        let values = Arc::new(vec![Some(1.0)]);
        state.snap_errors.insert(track, values);
        let initial = RunFingerprint::of(state.inputs(), dependencies);
        assert_eq!(initial.tracks(), [track]);
        assert!(!no_run.matches(state.inputs()));
        assert!(initial.matches(state.inputs()));
        let values = &state.snap_errors[&track];
        assert_eq!(Arc::strong_count(values), 1);
        assert_eq!(Arc::weak_count(values), 1);
        state.snap_errors.remove(&track);
        state.snap_errors.insert(track, Arc::new(vec![Some(2.0)]));
        assert!(!initial.matches(state.inputs()));
    }
}
