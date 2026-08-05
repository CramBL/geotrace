//! Shared in-scope predicates: the per-track gating every per-element
//! consumer (the marker renderers, hover hit-testing, the display counts)
//! must agree on. One derivation point, so "is this element in scope"
//! cannot drift between what draws, what hovers, and what counts.
//!
//! The viewport's per-track plan intentionally keeps its own flattened
//! derivation: it is the per-frame hot path and hoists the per-file lookup
//! out of its track loop.

use chrono::{DateTime, Utc};
use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{DataCategory, LoadedFile, LoadedTrack, TrackRef};
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, QueryMatches, TrackDataVisibility, TrackVisibility,
};
use strum::EnumCount;

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

/// Why the element a [`DataPointRef`] addresses is, or is not, on the map.
///
/// The variants are ordered the way [`point_visibility`] decides, so an element
/// failing several gates reports the first gate the map itself applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount)]
pub enum PointVisibility {
    /// Drawn on the map.
    Shown,
    /// Nothing at that index, or a category addressing no element of its own: a
    /// trackline and a raw satellite report have no hover target.
    NoSuchElement,
    /// The file or track is off in the tree, or the track fails the track-level
    /// filter.
    TrackNotShown,
    /// The element's category is off, in the track's tree toggles or in the
    /// display mask.
    CategoryHidden,
    /// A `keep` or `hide` query removed the point.
    HiddenByQuery,
    /// Outside the global time filter's window.
    OutsideTimeFilter,
}

impl PointVisibility {
    pub fn is_shown(self) -> bool {
        self == Self::Shown
    }
}

/// Whether the element `point` addresses is currently on the map, and when it is
/// not, why: the gating the renderers apply (enablement, tree toggle, track
/// filter), the display category, the points a `keep`/`hide` query removed, and
/// the time window.
///
/// The one derivation point for "is this point shown", so the map's own hover
/// and click hit-testing and anything asserting on map state answer it the same
/// way.
pub fn point_visibility(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    display_mask: DisplayMask,
    query_matches: Option<&QueryMatches>,
    point: DataPointRef,
) -> PointVisibility {
    if matches!(
        point.category,
        DataCategory::Track | DataCategory::SatelliteReport
    ) {
        return PointVisibility::NoSuchElement;
    }
    let Some((track, trip_vis)) = track_in_scope(files, visibility, filter, point.track) else {
        return PointVisibility::TrackNotShown;
    };
    if !trip_vis.category_visible(point.category)
        || !display_mask.is_visible(DisplayCategory::from(point.category))
    {
        return PointVisibility::CategoryHidden;
    }
    let pi = point.point_index.as_usize();
    // A `keep`/`hide` query removes TPV points from the drawn line and icons.
    // Markers stay drawn (the hidden ranges index TPV points, not the marker
    // arrays), so only the TPV category consults the mask.
    if point.category == DataCategory::Tpv
        && query_matches.is_some_and(|m| m.is_hidden(point.track, pi))
    {
        return PointVisibility::HiddenByQuery;
    }
    let Some(time) = element_time(track, point.category, pi) else {
        return PointVisibility::NoSuchElement;
    };
    if point_passes_time_filter(time, filter) {
        PointVisibility::Shown
    } else {
        PointVisibility::OutsideTimeFilter
    }
}

/// The timestamp the time filter judges one element by, from the array its
/// category indexes into.
fn element_time(
    track: &LoadedTrack,
    category: DataCategory,
    point_index: usize,
) -> Option<DateTime<Utc>> {
    match category {
        DataCategory::Tpv => track.points.get(point_index).map(|p| p.tpv.time().utc()),
        DataCategory::CustomMarker => track.custom_markers.get(point_index).map(|m| m.time),
        DataCategory::GeneratedMarker => track.generated_markers.get(point_index).map(|m| m.time),
        DataCategory::EventMarker => track.event_markers.get(point_index).map(|m| m.time),
        DataCategory::Track | DataCategory::SatelliteReport => None,
    }
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
        vis.files[0].tracks[0].set_category_visible(DataCategory::CustomMarker, false);
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
