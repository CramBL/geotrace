use chrono::{DateTime, Utc};
use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{
    DataCategory, DataCategorySet, FileIdx, LoadedFile, LoadedTrack, TrackIdx, TrackRef,
};
use strum::EnumCount;

use crate::display_mask::{DisplayCategory, DisplayMask};
use crate::highlight::DataPointRef;
use crate::query_matches::QueryMatches;

#[derive(Debug, Clone, PartialEq)]
pub struct TrackDataVisibility {
    pub files: Vec<FileVisibility>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileVisibility {
    pub enabled: bool,
    pub tracks: Vec<TrackVisibility>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackVisibility {
    pub enabled: bool,
    /// Per-category tree toggles. `enabled` is separate: it gates the whole
    /// track, while this only hides individual element categories.
    categories: DataCategorySet,
}

impl TrackVisibility {
    /// This track's tree toggle for the given element category, the single
    /// mapping renderers and counts consult.
    pub fn category_visible(self, category: DataCategory) -> bool {
        self.categories.contains(category)
    }

    /// Show or hide the given element category for this track.
    pub fn set_category_visible(&mut self, category: DataCategory, visible: bool) {
        self.categories.set(category, visible);
    }

    pub fn all_visible() -> Self {
        Self {
            enabled: true,
            categories: DataCategorySet::all(),
        }
    }
}

impl TrackDataVisibility {
    pub fn from_loaded(files: &[LoadedFile]) -> Self {
        Self {
            files: files
                .iter()
                .map(|f| FileVisibility {
                    enabled: true,
                    tracks: f
                        .tracks
                        .iter()
                        .map(|_| TrackVisibility::all_visible())
                        .collect(),
                })
                .collect(),
        }
    }

    /// Enable or disable every file and track at once.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for file in &mut self.files {
            file.enabled = enabled;
            for track in &mut file.tracks {
                track.enabled = enabled;
            }
        }
    }

    /// Whether the given track (and its file) is enabled.
    pub fn track_enabled(&self, track_ref: TrackRef) -> bool {
        track_ref
            .fi
            .get(&self.files)
            .is_some_and(|f| f.enabled && track_ref.index.get(&f.tracks).is_some_and(|t| t.enabled))
    }

    /// Whether the given track's line is shown on the map: its file and the
    /// track are enabled, and the track-line toggle is on. The predicate the
    /// snapped-track rendering and the snap queue's visibility priority use.
    pub fn track_shown(&self, track_ref: TrackRef) -> bool {
        track_ref.fi.get(&self.files).is_some_and(|f| {
            f.enabled
                && track_ref
                    .index
                    .get(&f.tracks)
                    .is_some_and(|t| t.enabled && t.category_visible(DataCategory::Track))
        })
    }

    /// Show only the given file. Hide all others. Trip visibility within files
    /// is preserved so that re-enabling a file restores its previous state.
    pub fn show_only_file(&mut self, fi: FileIdx) {
        for (i, file) in self.files.iter_mut().enumerate() {
            file.enabled = FileIdx::new(i) == fi;
        }
    }

    /// Show only the given track (and its parent file). Hide everything else.
    pub fn show_only_track(&mut self, track: TrackRef) {
        for (i, file) in self.files.iter_mut().enumerate() {
            if FileIdx::new(i) == track.fi {
                file.enabled = true;
                for (j, t) in file.tracks.iter_mut().enumerate() {
                    t.enabled = TrackIdx::new(j) == track.index;
                }
            } else {
                file.enabled = false;
            }
        }
    }
}

/// The track-level gate: file and track enabled, track filter passed.
/// Returns the resolved track and its tree toggles when everything passes.
pub fn track_in_scope<'a>(
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
pub fn category_in_scope<'a>(
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
/// The variants are ordered the way [`MapScope::point_visibility`] decides, so an
/// element failing several gates reports the first gate the map itself applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount)]
pub enum PointVisibility {
    /// Drawn on the map.
    Shown,
    /// Nothing is addressed: the file or track is not loaded, the index is past
    /// the end of its array, or the category is a trackline, which addresses no
    /// element of its own.
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

/// Everything that decides whether the map draws one addressed element: the
/// loaded recordings, the tree, the global filter, the display mask, and the last
/// query's effect.
///
/// The one derivation point for "is this element on the map", consulted by hover
/// and click hit-testing, the pinned popup, the point rows that create a pin,
/// and the headless tests.
#[derive(Clone, Copy)]
pub struct MapScope<'a> {
    pub files: &'a [LoadedFile],
    pub visibility: &'a TrackDataVisibility,
    pub filter: &'a GlobalFilter,
    pub display_mask: DisplayMask,
    /// The last query run's effect, absent when no query has run.
    pub query_matches: Option<&'a QueryMatches>,
}

impl MapScope<'_> {
    /// Whether the map draws the element `point` addresses.
    pub fn draws(&self, point: DataPointRef) -> bool {
        self.point_visibility(point).is_shown()
    }

    /// Whether the map draws the element, and when it does not, why: the gating
    /// the renderers apply (enablement, tree toggle, track filter), the display
    /// category, the points a `keep`/`hide` query removed, and the time window.
    ///
    /// A satellite report is judged as the fix it belongs to: it has no ink of
    /// its own, so its detail is on the map exactly while that point is.
    pub fn point_visibility(&self, point: DataPointRef) -> PointVisibility {
        let category = match point.category {
            DataCategory::Track => return PointVisibility::NoSuchElement,
            DataCategory::SatelliteReport => DataCategory::Tpv,
            category => category,
        };
        let index = point.point_index.as_usize();
        // Resolved before the tree and filter gates so an index past the end of
        // its array reads as addressing nothing, whatever those gates would say.
        let Some(time) = point
            .track
            .resolve(self.files)
            .and_then(|track| element_time(track, category, index))
        else {
            return PointVisibility::NoSuchElement;
        };
        let Some((_, trip_vis)) =
            track_in_scope(self.files, self.visibility, self.filter, point.track)
        else {
            return PointVisibility::TrackNotShown;
        };
        if !trip_vis.category_visible(category)
            || !self
                .display_mask
                .is_visible(DisplayCategory::from(category))
        {
            return PointVisibility::CategoryHidden;
        }
        // A `keep`/`hide` query removes TPV points from the drawn line and icons.
        // Markers stay drawn (the hidden ranges index TPV points, not the marker
        // arrays), so only the TPV category consults the mask.
        if category == DataCategory::Tpv
            && self
                .query_matches
                .is_some_and(|matches| matches.is_hidden(point.track, index))
        {
            return PointVisibility::HiddenByQuery;
        }
        if point_passes_time_filter(time, self.filter) {
            PointVisibility::Shown
        } else {
            PointVisibility::OutsideTimeFilter
        }
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
    use chrono::TimeDelta;

    use super::*;
    use crate::scope_fixture::{one_track_file, track0};

    fn vis_all() -> TrackDataVisibility {
        TrackDataVisibility::from_loaded(&one_track_file())
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
            min_duration: Some(TimeDelta::hours(1)),
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

    /// A [`TrackRef`] left over from before a file shrank addresses nothing, and
    /// must read as out of scope rather than index past the end.
    #[test]
    fn stale_indices_are_out_of_scope() {
        let files = one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        let stale = TrackRef::new(FileIdx::new(0), TrackIdx::new(7));
        assert!(track_in_scope(&files, &vis, &filter, stale).is_none());
        assert!(
            category_in_scope(&files, &vis, &filter, stale, DataCategory::Tpv).is_none(),
            "a stale ref is out of scope for a category too"
        );
    }

    #[rstest::rstest]
    #[case(DataCategory::Track)]
    #[case(DataCategory::Tpv)]
    #[case(DataCategory::SatelliteReport)]
    #[case(DataCategory::CustomMarker)]
    #[case(DataCategory::GeneratedMarker)]
    #[case(DataCategory::EventMarker)]
    fn category_visible_reads_exactly_its_flag(#[case] category: DataCategory) {
        let mut tv = TrackVisibility::all_visible();
        assert!(tv.category_visible(category));
        tv.set_category_visible(category, false);
        assert!(!tv.category_visible(category));
        // Exactly one category flag changed: every other category still on.
        let others = [
            DataCategory::Track,
            DataCategory::Tpv,
            DataCategory::SatelliteReport,
            DataCategory::CustomMarker,
            DataCategory::GeneratedMarker,
            DataCategory::EventMarker,
        ]
        .into_iter()
        .filter(|&c| c != category)
        .all(|c| tv.category_visible(c));
        assert!(others);
    }

    /// `track_shown` requires the file, the track, and the track-line
    /// toggle; any one of them off hides the track. Out-of-range refs are
    /// simply not shown.
    #[rstest::rstest]
    #[case::all_on(true, true, true, true)]
    #[case::file_disabled(false, true, true, false)]
    #[case::track_disabled(true, false, true, false)]
    #[case::trackline_hidden(true, true, false, false)]
    fn track_shown_needs_file_track_and_line(
        #[case] file_enabled: bool,
        #[case] track_enabled: bool,
        #[case] track_visible: bool,
        #[case] expected: bool,
    ) {
        let mut tv = TrackVisibility::all_visible();
        tv.enabled = track_enabled;
        tv.set_category_visible(DataCategory::Track, track_visible);
        let vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: file_enabled,
                tracks: vec![tv],
            }],
        };
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        assert_eq!(vis.track_shown(track_ref), expected);
        assert!(
            !vis.track_shown(TrackRef::new(FileIdx::new(1), TrackIdx::new(0))),
            "an out-of-range ref is never shown"
        );
    }
}
