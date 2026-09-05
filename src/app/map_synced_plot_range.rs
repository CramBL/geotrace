//! The plot's x range while it follows the map, held between the frames its
//! inputs stay the same.

use gt_loaded_files::LoadedFiles;
use gt_map::ViewportBounds;
use gt_types::{Generation, LoadedFile};
use gt_ui_types::TrackDataVisibility;

/// What the range held in [`MapSyncedPlotRangeCache`] was scanned from.
#[derive(Debug)]
struct ScanInputs {
    /// A NaN is never equal to itself and a negative zero is equal to a
    /// positive one, so the key compares bit patterns.
    bounds_bits: [u64; 4],
    visibility: TrackDataVisibility,
    files: Generation,
}

/// The time range of the fixes inside the map viewport, scanned again only
/// when the viewport, the track visibility or the loaded files change.
///
/// The scan reads every placed fix of every shown track, and the plot follows
/// the map on every frame it is open.
#[derive(Debug, Default)]
pub(super) struct MapSyncedPlotRangeCache {
    scanned_from: Option<ScanInputs>,
    range: Option<(f64, f64)>,
}

impl MapSyncedPlotRangeCache {
    pub(super) fn range(
        &mut self,
        bounds: ViewportBounds,
        visibility: &TrackDataVisibility,
        files: &LoadedFiles,
        scan: impl FnOnce(&[LoadedFile], &TrackDataVisibility, ViewportBounds) -> Option<(f64, f64)>,
    ) -> Option<(f64, f64)> {
        let bounds_bits = [
            bounds.lat_min.to_bits(),
            bounds.lat_max.to_bits(),
            bounds.lon_min.to_bits(),
            bounds.lon_max.to_bits(),
        ];
        let generation = files.generation();
        let scanned_from_the_same_inputs = self.scanned_from.as_ref().is_some_and(|inputs| {
            inputs.bounds_bits == bounds_bits
                && inputs.files == generation
                && inputs.visibility == *visibility
        });
        if !scanned_from_the_same_inputs {
            self.range = scan(files.files(), visibility, bounds);
            self.scanned_from = Some(ScanInputs {
                bounds_bits,
                visibility: visibility.clone(),
                files: generation,
            });
        }
        self.range
    }

    /// The range of the last scan, [`None`] until the plot pane requests one.
    #[cfg(test)]
    pub(super) fn scanned_range(&self) -> Option<(f64, f64)> {
        self.range
    }
}

#[cfg(test)]
mod tests {
    use gt_loaded_files::FileHistory;
    use rustc_hash::FxHashMap;

    use super::*;

    fn bounds() -> ViewportBounds {
        ViewportBounds {
            lat_min: 55.0,
            lat_max: 56.0,
            lon_min: 12.0,
            lon_max: 13.0,
        }
    }

    /// One file of one track, the smallest input that has a track to toggle.
    fn one_loaded_file() -> LoadedFiles {
        let file = LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![gt_test_utils::loaded_track_with_points(
                gt_test_utils::nav_points_from(chrono::DateTime::UNIX_EPOCH, 4, 1),
            )],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: gt_types::FileSource::GtdBytes(std::sync::Arc::from(Vec::<u8>::new())),
            load_warnings: Vec::new(),
        };
        let mut files = LoadedFiles::new();
        files.push(file, FileHistory::None);
        files
    }

    /// A test reads both how often the scan ran and which range the cache
    /// returned. This scan counts its calls and returns the count as the range
    /// end.
    fn counting_scan(
        calls: &mut usize,
    ) -> impl FnOnce(&[LoadedFile], &TrackDataVisibility, ViewportBounds) -> Option<(f64, f64)>
    {
        move |_, _, _| {
            *calls += 1;
            Some((0.0, *calls as f64))
        }
    }

    #[test]
    fn a_second_call_with_the_same_inputs_hands_back_the_scanned_range() {
        let files = one_loaded_file();
        let visibility = TrackDataVisibility::from_loaded(files.files());
        let mut cache = MapSyncedPlotRangeCache::default();
        let mut calls = 0;

        let first = cache.range(bounds(), &visibility, &files, counting_scan(&mut calls));
        let second = cache.range(bounds(), &visibility, &files, counting_scan(&mut calls));

        assert_eq!(calls, 1);
        assert_eq!(first, Some((0.0, 1.0)));
        assert_eq!(second, first);
    }

    #[test]
    fn a_changed_viewport_bound_scans_again() {
        let files = one_loaded_file();
        let visibility = TrackDataVisibility::from_loaded(files.files());
        let mut cache = MapSyncedPlotRangeCache::default();
        let mut calls = 0;
        assert_eq!(
            cache.range(bounds(), &visibility, &files, counting_scan(&mut calls)),
            Some((0.0, 1.0))
        );

        let panned = ViewportBounds {
            lon_max: 13.5,
            ..bounds()
        };
        let range = cache.range(panned, &visibility, &files, counting_scan(&mut calls));

        assert_eq!(calls, 2);
        assert_eq!(range, Some((0.0, 2.0)));
    }

    #[test]
    fn a_toggled_track_scans_again() {
        let files = one_loaded_file();
        let visibility = TrackDataVisibility::from_loaded(files.files());
        let mut cache = MapSyncedPlotRangeCache::default();
        let mut calls = 0;
        assert_eq!(
            cache.range(bounds(), &visibility, &files, counting_scan(&mut calls)),
            Some((0.0, 1.0))
        );

        let mut hidden = visibility.clone();
        if let Some(track) = hidden
            .files
            .first_mut()
            .and_then(|file| file.tracks.first_mut())
        {
            track.enabled = false;
        }
        let range = cache.range(bounds(), &hidden, &files, counting_scan(&mut calls));

        assert_eq!(calls, 2);
        assert_eq!(range, Some((0.0, 2.0)));
    }

    /// Re-segmentation and re-placement rewrite a track through
    /// [`LoadedFiles::files_mut`], leaving the file count and the visibility
    /// as they were.
    #[test]
    fn a_mutation_of_the_loaded_files_scans_again() {
        let mut files = one_loaded_file();
        let visibility = TrackDataVisibility::from_loaded(files.files());
        let mut cache = MapSyncedPlotRangeCache::default();
        let mut calls = 0;
        assert_eq!(
            cache.range(bounds(), &visibility, &files, counting_scan(&mut calls)),
            Some((0.0, 1.0))
        );

        files.files_mut();
        let range = cache.range(bounds(), &visibility, &files, counting_scan(&mut calls));

        assert_eq!(calls, 2);
        assert_eq!(range, Some((0.0, 2.0)));
    }
}
