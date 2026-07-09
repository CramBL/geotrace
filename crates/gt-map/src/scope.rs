//! Shared in-scope predicates: the per-track gating every per-element
//! consumer (the marker renderers, hover hit-testing, the display counts)
//! must agree on. One derivation point, so "is this element in scope"
//! cannot drift between what draws, what hovers, and what counts.
//!
//! [`crate::viewport::TrackPlan`] intentionally keeps its own flattened
//! derivation: it is the per-frame hot path and hoists the per-file lookup
//! out of its track loop.

use gt_filter::{GlobalFilter, track_passes_filter};
use gt_types::{DataCategory, LoadedFile, LoadedTrack, TrackRef};
use gt_ui_types::{TrackDataVisibility, TrackVisibility};

/// The track-level gate: file and track enabled, track filter passed.
/// Returns the resolved track and its tree toggles when everything passes.
pub(crate) fn track_in_scope<'a>(
    files: &'a [LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    track_ref: TrackRef,
) -> Option<(&'a LoadedTrack, TrackVisibility)> {
    let file_vis = track_ref.fi.get(&visibility.files)?;
    if !file_vis.enabled {
        return None;
    }
    let trip_vis = *track_ref.index.get(&file_vis.tracks)?;
    if !trip_vis.enabled {
        return None;
    }
    let track = track_ref.resolve(files)?;
    track_passes_filter(&track.metadata, filter).then_some((track, trip_vis))
}

/// [`track_in_scope`] refined by one category's tree toggle - the gate the
/// per-marker renderers apply before touching an element.
pub(crate) fn category_in_scope<'a>(
    files: &'a [LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    track_ref: TrackRef,
    category: DataCategory,
) -> Option<&'a LoadedTrack> {
    let (track, trip_vis) = track_in_scope(files, visibility, filter, track_ref)?;
    trip_vis.category_visible(category).then_some(track)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use gt_types::{FileIdx, FileMetadata, FileSource, TrackIdx, TrackMetadata};
    use gt_ui_types::FileVisibility;
    use uom::si::f64::Length;
    use uom::si::length::kilometer;

    use super::*;

    fn one_track_file() -> Vec<LoadedFile> {
        vec![LoadedFile {
            metadata: FileMetadata::default(),
            tracks: vec![LoadedTrack {
                metadata: TrackMetadata::default(),
                points: Vec::new(),
                lod: gt_types::TrackLod::default(),
                sat_label_anchors: Vec::new(),
                custom_markers: Vec::new(),
                generated_markers: Vec::new(),
                event_markers: Vec::new(),
                channels: Vec::new(),
            }],
            event_marker_styles: HashMap::new(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(PathBuf::from("scope.gtd")),
            load_warnings: Vec::new(),
        }]
    }

    fn vis_all() -> TrackDataVisibility {
        TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![TrackVisibility::all_visible()],
            }],
        }
    }

    fn track0() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    #[test]
    fn everything_enabled_is_in_scope() {
        let files = one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        assert!(track_in_scope(&files, &vis, &filter, track0()).is_some());
        assert!(
            category_in_scope(&files, &vis, &filter, track0(), DataCategory::CustomMarker)
                .is_some()
        );
    }

    #[rstest::rstest]
    #[case::file_disabled(|vis: &mut TrackDataVisibility| vis.files[0].enabled = false)]
    #[case::track_disabled(|vis: &mut TrackDataVisibility| vis.files[0].tracks[0].enabled = false)]
    fn disabled_nodes_are_out_of_scope(#[case] disable: fn(&mut TrackDataVisibility)) {
        let files = one_track_file();
        let mut vis = vis_all();
        disable(&mut vis);
        assert!(track_in_scope(&files, &vis, &GlobalFilter::default(), track0()).is_none());
    }

    #[test]
    fn failing_the_track_filter_is_out_of_scope() {
        let files = one_track_file();
        let filter = GlobalFilter {
            min_distance_km: Some(Length::new::<kilometer>(1.0)),
            ..GlobalFilter::default()
        };
        assert!(track_in_scope(&files, &vis_all(), &filter, track0()).is_none());
    }

    #[test]
    fn category_toggle_gates_only_its_category() {
        let files = one_track_file();
        let mut vis = vis_all();
        vis.files[0].tracks[0].custom_markers_visible = false;
        let filter = GlobalFilter::default();
        assert!(
            category_in_scope(&files, &vis, &filter, track0(), DataCategory::CustomMarker)
                .is_none()
        );
        assert!(
            category_in_scope(&files, &vis, &filter, track0(), DataCategory::EventMarker).is_some()
        );
    }

    #[test]
    fn stale_indices_are_out_of_scope() {
        let files = one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        let stale = TrackRef::new(FileIdx::new(0), TrackIdx::new(7));
        assert!(track_in_scope(&files, &vis, &filter, stale).is_none());
    }
}
