use std::sync::Arc;

use gt_filter::GlobalFilter;
use gt_loaded_files::{LoadedFileId, LoadedFilesView};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use gt_ui_types::{ArcIdentity, GeomagneticSeries, TecSeries, TrackDataVisibility};
use rustc_hash::FxHashMap;

/// Per-track dense snap error values, one entry per track with a completed
/// snap run - handed in by the app each frame, shared with its per-run cache
/// (so the `Arc` identities are stable and change exactly when a run does).
pub type SnapErrorValues = FxHashMap<TrackRef, Arc<Vec<Option<f64>>>>;

/// Per-track interference percentages, one entry per fix. Shaped like
/// [`SnapErrorValues`] so both reach the provider the same way.
pub type JammingValues = FxHashMap<TrackRef, Arc<Vec<Option<f64>>>>;

/// The state a query run depends on, handed in by the app each frame - the
/// inputs [`RunFingerprint`] snapshots to gray out outdated results.
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

/// Everything a run's results depend on besides the query text. Results
/// gray out when the current state no longer matches the snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct RunFingerprint {
    /// The loaded files the run read, by session id. Loading a file again
    /// gives it a new id, so results gray out when another file takes a
    /// loaded one's place under the name it had.
    files: Vec<LoadedFileId>,
    /// The tracks the run evaluated: enabled in the tree and passing the
    /// track-level global filter, in tree order.
    tracks: Vec<TrackRef>,
    filter: GlobalFilter,
    /// The snap run each evaluated track's `snap_error` values came from
    /// (by `Arc` identity, absent without a run) - a re-snap changes the
    /// values, so results referencing them gray out like any other input.
    snap_runs: Vec<Option<ArcIdentity>>,
    /// The interference values each evaluated track saw (by `Arc` identity,
    /// absent with no archived day). Archiving a day the track spans
    /// changes them, so results referencing them gray out.
    jamming_days: Vec<Option<ArcIdentity>>,
    /// The geomagnetic index values each evaluated track saw, tracked like
    /// [`Self::jamming_days`].
    geomagnetic_days: Vec<Option<ArcIdentity>>,
    /// The TEC values each evaluated track saw, tracked like
    /// [`Self::jamming_days`].
    tec_days: Vec<Option<ArcIdentity>>,
}

impl RunFingerprint {
    /// Snapshot the run inputs.
    pub fn of(inputs: RunInputs<'_>) -> Self {
        let RunInputs {
            loaded_files,
            visibility,
            filter,
            snap_errors,
            jamming,
            geomagnetic,
            tec,
        } = inputs;
        let mut files = Vec::with_capacity(loaded_files.entries().len());
        let mut tracks = Vec::new();
        for (fi, entry) in loaded_files.entries().enumerate() {
            files.push(entry.id());
            let file = entry.file();
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
                if visibility.track_enabled(track_ref)
                    && gt_filter::track_passes_filter(track, filter)
                {
                    tracks.push(track_ref);
                }
            }
        }
        let snap_runs = tracks
            .iter()
            .map(|track_ref| snap_errors.get(track_ref).map(ArcIdentity::of))
            .collect();
        let jamming_days = tracks
            .iter()
            .map(|track_ref| jamming.get(track_ref).map(ArcIdentity::of))
            .collect();
        let geomagnetic_days = tracks
            .iter()
            .map(|track_ref| {
                geomagnetic
                    .points_by_track
                    .get(track_ref)
                    .map(ArcIdentity::of)
            })
            .collect();
        let tec_days = tracks
            .iter()
            .map(|track_ref| tec.points_by_track.get(track_ref).map(ArcIdentity::of))
            .collect();
        Self {
            files,
            tracks,
            filter: *filter,
            snap_runs,
            jamming_days,
            geomagnetic_days,
            tec_days,
        }
    }

    /// The tracks a run over these inputs evaluates, in tree order.
    pub fn tracks(&self) -> &[TrackRef] {
        &self.tracks
    }
}

#[cfg(test)]
mod tests {
    use gt_loaded_files::{FileHistory, LoadedFiles};
    use rstest::rstest;

    use super::*;
    use crate::test_fixtures::{file_with_channels, loaded_file};

    #[test]
    fn fingerprint_changes_with_files_visibility_and_filter() {
        let loaded_files = LoadedFiles::new();
        let visibility = TrackDataVisibility::from_loaded(loaded_files.files());
        let fingerprint = |filter: &GlobalFilter| {
            RunFingerprint::of(RunInputs {
                loaded_files: loaded_files.view(),
                visibility: &visibility,
                filter,
                snap_errors: &SnapErrorValues::default(),
                jamming: &JammingValues::default(),
                geomagnetic: &GeomagneticSeries::default(),
                tec: &TecSeries::default(),
            })
        };

        let base = fingerprint(&GlobalFilter::default());
        assert_eq!(base, fingerprint(&GlobalFilter::default()));
        let filtered = GlobalFilter {
            min_distance_km: Some(uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0)),
            ..GlobalFilter::default()
        };
        assert_ne!(base, fingerprint(&filtered));
    }

    /// A new snap run for an evaluated track changes the fingerprint, so
    /// results gray out. Handing in the same run keeps it equal.
    #[test]
    fn fingerprint_tracks_snap_run_identity() {
        let mut loaded_files = LoadedFiles::new();
        loaded_files.push(loaded_file(), FileHistory::None);
        let visibility = TrackDataVisibility::from_loaded(loaded_files.files());
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let fingerprint = |snap_errors: &SnapErrorValues| {
            RunFingerprint::of(RunInputs {
                loaded_files: loaded_files.view(),
                visibility: &visibility,
                filter: &GlobalFilter::default(),
                snap_errors,
                jamming: &JammingValues::default(),
                geomagnetic: &GeomagneticSeries::default(),
                tec: &TecSeries::default(),
            })
        };

        let no_run = fingerprint(&SnapErrorValues::default());
        let run = SnapErrorValues::from_iter([(track, Arc::new(vec![Some(1.0)]))]);
        assert_ne!(no_run, fingerprint(&run), "a first run changes the input");
        assert_eq!(fingerprint(&run), fingerprint(&run), "same run, stable");
        let re_run = SnapErrorValues::from_iter([(track, Arc::new(vec![Some(2.0)]))]);
        assert_ne!(
            fingerprint(&run),
            fingerprint(&re_run),
            "a re-snap must gray results out"
        );
    }

    /// Every enabled, filter-passing track is an evaluation target.
    #[test]
    fn fingerprint_lists_the_evaluated_tracks() {
        let mut loaded_files = LoadedFiles::new();
        loaded_files.push(file_with_channels(vec![]), FileHistory::None);
        let visibility = TrackDataVisibility::from_loaded(loaded_files.files());
        let fingerprint = RunFingerprint::of(RunInputs {
            loaded_files: loaded_files.view(),
            visibility: &visibility,
            filter: &GlobalFilter::default(),
            snap_errors: &SnapErrorValues::default(),
            jamming: &JammingValues::default(),
            geomagnetic: &GeomagneticSeries::default(),
            tec: &TecSeries::default(),
        });
        assert_eq!(
            fingerprint.tracks(),
            [TrackRef::new(FileIdx::new(0), TrackIdx::new(0))]
        );
    }

    /// The day archives a query reads per fix. This fixture fills one for one
    /// track, from one archived day.
    #[derive(Default)]
    struct ArchivedSeries {
        geomagnetic: GeomagneticSeries,
        tec: TecSeries,
    }

    impl ArchivedSeries {
        /// A geomagnetic index of `hp30` at the epoch, on `track` alone.
        fn of_geomagnetic(track: TrackRef, hp30: f64) -> Self {
            let mut series = Self::default();
            series.geomagnetic.points_by_track.insert(
                track,
                Arc::new(vec![gt_ui_types::GeomagneticPoint {
                    x_secs: 0.0,
                    hp30: Some(hp30),
                    kp: None,
                }]),
            );
            series
        }

        /// A TEC reading of `tecu` at the epoch, on `track` alone.
        fn of_tec(track: TrackRef, tecu: f64) -> Self {
            let mut series = Self::default();
            series.tec.points_by_track.insert(
                track,
                Arc::new(vec![gt_ui_types::TecPoint {
                    x_secs: 0.0,
                    tecu: Some(tecu),
                }]),
            );
            series
        }
    }

    /// An archived day reaching an evaluated track grays out the results
    /// referencing it, and a revision of that day grays them out again.
    #[rstest]
    #[case::geomagnetic(ArchivedSeries::of_geomagnetic)]
    #[case::tec(ArchivedSeries::of_tec)]
    fn fingerprint_tracks_the_archived_values_of_an_evaluated_track(
        #[case] series_of: fn(TrackRef, f64) -> ArchivedSeries,
    ) {
        let mut loaded_files = LoadedFiles::new();
        loaded_files.push(loaded_file(), FileHistory::None);
        let visibility = TrackDataVisibility::from_loaded(loaded_files.files());
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let fingerprint = |series: &ArchivedSeries| {
            RunFingerprint::of(RunInputs {
                loaded_files: loaded_files.view(),
                visibility: &visibility,
                filter: &GlobalFilter::default(),
                snap_errors: &SnapErrorValues::default(),
                jamming: &JammingValues::default(),
                geomagnetic: &series.geomagnetic,
                tec: &series.tec,
            })
        };

        let archived = series_of(track, 5.0);
        assert_ne!(
            fingerprint(&ArchivedSeries::default()),
            fingerprint(&archived),
            "an archived day changes the input"
        );
        assert_eq!(fingerprint(&archived), fingerprint(&archived), "stable");
        assert_ne!(
            fingerprint(&archived),
            fingerprint(&series_of(track, 6.0)),
            "a revised day must gray results out"
        );
    }
}
