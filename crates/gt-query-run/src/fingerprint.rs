use std::collections::HashMap;
use std::sync::Arc;

use gt_filter::GlobalFilter;
use gt_loaded_files::LoadedFilesView;
use gt_types::{FileIdx, TrackIdx, TrackRef};
use gt_ui_types::{ArcIdentity, TrackDataVisibility};

/// Per-track dense snap error values, one entry per track with a completed
/// snap run - handed in by the app each frame, shared with its per-run cache
/// (so the `Arc` identities are stable and change exactly when a run does).
pub type SnapErrorValues = HashMap<TrackRef, Arc<Vec<Option<f64>>>>;

/// Per-track interference percentages, one entry per fix. Shaped like
/// [`SnapErrorValues`] so both reach the provider the same way.
pub type JammingValues = HashMap<TrackRef, Arc<Vec<Option<f64>>>>;

/// The state a query run depends on, handed in by the app each frame - the
/// inputs [`RunFingerprint`] snapshots to gray out outdated results.
#[derive(Clone, Copy)]
pub struct RunInputs<'a> {
    pub loaded_files: LoadedFilesView<'a>,
    pub visibility: &'a TrackDataVisibility,
    pub filter: &'a GlobalFilter,
    pub snap_errors: &'a SnapErrorValues,
    pub jamming: &'a JammingValues,
}

/// Everything a run's results depend on besides the query text. Results
/// gray out when the current state no longer matches the snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct RunFingerprint {
    file_identities: Vec<String>,
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
        } = inputs;
        let mut file_identities = Vec::with_capacity(loaded_files.entries().len());
        let mut tracks = Vec::new();
        for (fi, entry) in loaded_files.entries().enumerate() {
            file_identities.push(entry.identity_key().into_owned());
            let file = entry.file();
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
                if visibility.track_enabled(track_ref)
                    && gt_filter::track_passes_filter(&track.metadata, filter)
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
        Self {
            file_identities,
            tracks,
            filter: *filter,
            snap_runs,
            jamming_days,
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
            })
        };

        let no_run = fingerprint(&SnapErrorValues::default());
        let run = SnapErrorValues::from([(track, Arc::new(vec![Some(1.0)]))]);
        assert_ne!(no_run, fingerprint(&run), "a first run changes the input");
        assert_eq!(fingerprint(&run), fingerprint(&run), "same run, stable");
        let re_run = SnapErrorValues::from([(track, Arc::new(vec![Some(2.0)]))]);
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
        });
        assert_eq!(
            fingerprint.tracks(),
            [TrackRef::new(FileIdx::new(0), TrackIdx::new(0))]
        );
    }
}
