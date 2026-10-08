use chrono::{DateTime, Utc};
use gt_filter::GlobalFilter;
use gt_types::satellites::Satellites;
use gt_types::{
    CustomMarker, DataCategory, DataCategorySet, EventMarker, FileIdx, GeneratedMarker, LoadedFile,
    LoadedTrack, NavPoint, TrackIdx, TrackRef,
};
use strum::EnumCount;

use crate::display_mask::{DisplayCategory, DisplayMask};
use crate::event_marker_visibility::EventMarkerVisibility;
use crate::generated_marker_visibility::GeneratedMarkerVisibility;
use crate::highlight::MapElementRef;
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
    /// This track's tree toggle for `category`, the single mapping renderers
    /// and counts consult.
    pub fn category_visible(self, category: DataCategory) -> bool {
        self.categories.contains(category)
    }

    /// Show or hide `category` for this track.
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

    /// Whether `track_ref` and its file are enabled.
    pub fn track_enabled(&self, track_ref: TrackRef) -> bool {
        track_ref
            .fi
            .get(&self.files)
            .is_some_and(|f| f.enabled && track_ref.index.get(&f.tracks).is_some_and(|t| t.enabled))
    }

    /// Whether `track_ref`'s line is shown on the map: its file and the track
    /// are enabled, and the track-line toggle is on. The predicate the
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

    /// Show only `fi`. Hide all others. Track visibility within files is
    /// preserved so that re-enabling a file restores its previous state.
    pub fn show_only_file(&mut self, fi: FileIdx) {
        for (i, file) in self.files.iter_mut().enumerate() {
            file.enabled = FileIdx::new(i) == fi;
        }
    }

    /// Show only `track` and its parent file. Hide everything else.
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
    let track_vis = *track_ref.index.get(&file_vis.tracks)?;
    if !track_vis.enabled {
        return None;
    }
    let track = track_ref.resolve(files)?;
    gt_filter::track_passes_filter(track, filter).then_some((track, track_vis))
}

/// Why the element a [`MapElementRef`] addresses is, or is not, on the map.
///
/// The variants are ordered the way [`MapScope::point_visibility`] evaluates
/// them, so an element failing several gates reports the first gate the map
/// itself applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount)]
pub enum PointVisibility {
    /// Drawn on the map.
    Shown,
    /// Nothing is addressed: the file or track is not loaded, or the typed
    /// index is past the end of its array.
    NoSuchElement,
    /// The file or track is off in the tree, or the track fails the track-level
    /// filter.
    TrackNotShown,
    /// The element's category is off, in the track's tree toggles or in the
    /// display mask.
    CategoryHidden,
    /// The marker's type is off in the tree: the kind of a generated marker,
    /// or the variant path of an event marker or a parent path of it.
    MarkerTypeHidden,
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

/// Everything that determines whether the map draws one addressed element: the
/// loaded recordings, the tree with its per-type marker toggles, the global
/// filter, the display mask, and the last query's effect.
///
/// Hover and click hit-testing, the marker renderers, the pinned popup, the
/// point rows that create a pin, and the headless tests read
/// [`MapScope::point_visibility`] for whether an element is on the map.
#[derive(Clone, Copy)]
pub struct MapScope<'a> {
    pub files: &'a [LoadedFile],
    pub visibility: &'a TrackDataVisibility,
    pub event_marker_visibility: &'a EventMarkerVisibility,
    pub generated_marker_visibility: &'a GeneratedMarkerVisibility,
    pub filter: &'a GlobalFilter,
    pub display_mask: DisplayMask,
    /// The last query run's effect, absent when no query has run.
    pub query_matches: Option<&'a QueryMatches>,
}

impl MapScope<'_> {
    /// Whether the map draws the element `point` addresses.
    pub fn draws(&self, point: MapElementRef) -> bool {
        self.point_visibility(point).is_shown()
    }

    /// Whether the map draws the element, and when it does not, why: the gating
    /// the renderers apply (enablement, tree toggle, track filter), the display
    /// category, the marker's type, the points a `keep`/`hide` query removed,
    /// and the time window.
    ///
    /// A satellite report is judged as the fix it belongs to: it has no ink of
    /// its own, so its detail is on the map exactly while that point is.
    pub fn point_visibility(&self, point: MapElementRef) -> PointVisibility {
        let category = match point {
            MapElementRef::Fix(_) | MapElementRef::SatelliteReport(_) => DataCategory::Tpv,
            MapElementRef::CustomMarker(_) => DataCategory::CustomMarker,
            MapElementRef::GeneratedMarker(_) => DataCategory::GeneratedMarker,
            MapElementRef::EventMarker(_) => DataCategory::EventMarker,
        };
        // Resolved before the tree and filter gates so an index past the end of
        // its array reads as addressing nothing, whatever those gates would say.
        let Some(element) = point.resolve(self.files) else {
            return PointVisibility::NoSuchElement;
        };
        let track = point.track();
        let Some((_, track_vis)) = track_in_scope(self.files, self.visibility, self.filter, track)
        else {
            return PointVisibility::TrackNotShown;
        };
        let display_category = match element {
            ResolvedElement::Fix(fix) | ResolvedElement::SatelliteReport { fix, .. }
                if fix.is_ghost_fix() =>
            {
                DisplayCategory::GhostFixes
            }
            ResolvedElement::Fix(_)
            | ResolvedElement::SatelliteReport { .. }
            | ResolvedElement::CustomMarker(_)
            | ResolvedElement::GeneratedMarker(_)
            | ResolvedElement::EventMarker(_) => DisplayCategory::from(category),
        };
        if !track_vis.category_visible(category) || !self.display_mask.is_visible(display_category)
        {
            return PointVisibility::CategoryHidden;
        }
        if !self.marker_type_visible(track, element) {
            return PointVisibility::MarkerTypeHidden;
        }
        // A `keep`/`hide` query removes TPV points from the drawn line and icons.
        // Markers stay drawn (the hidden ranges index TPV points, not the marker
        // arrays), so only fix-backed elements consult the mask.
        if let Some(fix) = point.fix()
            && self
                .query_matches
                .is_some_and(|matches| matches.is_hidden(track, fix.point.as_usize()))
        {
            return PointVisibility::HiddenByQuery;
        }
        if gt_filter::point_passes_time_filter(element.time(), self.filter) {
            PointVisibility::Shown
        } else {
            PointVisibility::OutsideTimeFilter
        }
    }

    /// The per-type tree toggle of a generated or an event marker, `true` for a
    /// fix and a custom marker.
    fn marker_type_visible(&self, track: TrackRef, element: ResolvedElement<'_>) -> bool {
        match element {
            ResolvedElement::EventMarker(marker) => self
                .event_marker_visibility
                .is_visible(track, &marker.variant_path),
            ResolvedElement::GeneratedMarker(marker) => self
                .generated_marker_visibility
                .is_visible(track, marker.kind.tag()),
            ResolvedElement::Fix(_)
            | ResolvedElement::SatelliteReport { .. }
            | ResolvedElement::CustomMarker(_) => true,
        }
    }
}

/// The borrowed element that a [`MapElementRef`] resolves to.
#[derive(Clone, Copy)]
pub enum ResolvedElement<'a> {
    Fix(&'a NavPoint),
    SatelliteReport {
        fix: &'a NavPoint,
        report: &'a Satellites,
    },
    CustomMarker(&'a CustomMarker),
    GeneratedMarker(&'a GeneratedMarker),
    EventMarker(&'a EventMarker),
}

impl MapElementRef {
    /// Resolve this typed identity against the currently loaded recordings.
    pub fn resolve<'a>(self, files: &'a [LoadedFile]) -> Option<ResolvedElement<'a>> {
        match self {
            Self::Fix(reference) => reference
                .track
                .resolve(files)?
                .points
                .get(reference.point.as_usize())
                .map(ResolvedElement::Fix),
            Self::SatelliteReport(reference) => {
                let fix = reference
                    .track
                    .resolve(files)?
                    .points
                    .get(reference.point.as_usize())?;
                Some(ResolvedElement::SatelliteReport {
                    fix,
                    report: fix.satellites.as_ref()?,
                })
            }
            Self::CustomMarker(reference) => reference
                .track
                .resolve(files)?
                .custom_markers
                .get(reference.index.as_usize())
                .map(ResolvedElement::CustomMarker),
            Self::GeneratedMarker(reference) => reference
                .track
                .resolve(files)?
                .generated_markers
                .get(reference.index.as_usize())
                .map(ResolvedElement::GeneratedMarker),
            Self::EventMarker(reference) => reference
                .track
                .resolve(files)?
                .event_markers
                .get(reference.index.as_usize())
                .map(ResolvedElement::EventMarker),
        }
    }
}

impl ResolvedElement<'_> {
    /// The timestamp that the time filter compares against its window.
    pub fn time(self) -> DateTime<Utc> {
        match self {
            Self::Fix(fix) | Self::SatelliteReport { fix, .. } => fix.tpv.time().utc(),
            Self::CustomMarker(marker) => marker.time,
            Self::GeneratedMarker(marker) => marker.time,
            Self::EventMarker(marker) => marker.time,
        }
    }
}

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeDelta;
    use gt_types::fixtures::FixKind;
    use gt_types::{
        CustomMarkerIdx, CustomMarkerRef, EventMarkerIdx, EventMarkerRef, FileSource,
        GeneratedMarkerIdx, GeneratedMarkerKindTag, GeneratedMarkerRef, GpsTime, Latitude,
        Longitude, PointIdx, TimePositionVelocity,
    };

    use super::*;
    use crate::test_util::{self, ScopeFixture};

    fn vis_all() -> TrackDataVisibility {
        TrackDataVisibility::from_loaded(&test_util::one_track_file())
    }

    #[test]
    fn everything_enabled_is_in_scope() {
        let files = test_util::one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        assert!(track_in_scope(&files, &vis, &filter, test_util::track0()).is_some());
    }

    #[rstest::rstest]
    #[case::file_disabled(|vis: &mut TrackDataVisibility| vis.files[0].enabled = false)]
    #[case::track_disabled(|vis: &mut TrackDataVisibility| vis.files[0].tracks[0].enabled = false)]
    fn disabled_nodes_are_out_of_scope(#[case] disable: fn(&mut TrackDataVisibility)) {
        let files = test_util::one_track_file();
        let mut vis = vis_all();
        disable(&mut vis);
        assert!(
            track_in_scope(&files, &vis, &GlobalFilter::default(), test_util::track0()).is_none()
        );
    }

    #[test]
    fn failing_the_track_filter_is_out_of_scope() {
        let files = test_util::one_track_file();
        let filter = GlobalFilter::default().with_minimum_duration(Some(TimeDelta::hours(1)));
        assert!(track_in_scope(&files, &vis_all(), &filter, test_util::track0()).is_none());
    }

    /// A [`TrackRef`] left over from before a file shrank addresses nothing and
    /// is out of scope.
    #[test]
    fn stale_indices_are_out_of_scope() {
        let files = test_util::one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        let stale = TrackRef::new(FileIdx::new(0), TrackIdx::new(7));
        assert!(track_in_scope(&files, &vis, &filter, stale).is_none());
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

    #[test]
    fn stale_typed_marker_refs_resolve_as_missing() {
        let files = test_util::one_track_file();
        let track = test_util::track0();
        let stale_index = 10_000;
        let stale = [
            MapElementRef::CustomMarker(CustomMarkerRef::new(
                track,
                CustomMarkerIdx::new(stale_index),
            )),
            MapElementRef::GeneratedMarker(GeneratedMarkerRef::new(
                track,
                GeneratedMarkerIdx::new(stale_index),
            )),
            MapElementRef::EventMarker(EventMarkerRef::new(
                track,
                EventMarkerIdx::new(stale_index),
            )),
        ];

        for element in stale {
            assert!(element.resolve(&files).is_none());
        }
    }

    /// `track_shown` requires the file, the track, and the track-line
    /// toggle. Any one of them off hides the track. Out-of-range refs are
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

    /// The tree hides an event marker by its own path or by a path above it,
    /// and a generated marker by its own kind. A fix has no marker type.
    #[rstest::rstest]
    #[case::event_marker_with_its_path_hidden(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| fixture.hide_event_marker_path(test_util::EVENT_MARKER_PATH),
        PointVisibility::MarkerTypeHidden
    )]
    #[case::event_marker_under_a_hidden_parent_path(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| {
            fixture.hide_event_marker_path(test_util::EVENT_MARKER_PARENT_PATH);
        },
        PointVisibility::MarkerTypeHidden
    )]
    #[case::event_marker_above_a_hidden_child_path(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| fixture.hide_event_marker_path("power/boot/cold"),
        PointVisibility::Shown
    )]
    #[case::event_marker_while_a_text_prefix_of_its_parent_path_is_hidden(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| fixture.hide_event_marker_path("pow"),
        PointVisibility::Shown
    )]
    #[case::generated_marker_of_a_hidden_kind(
        test_util::generated_marker(),
        |fixture: &mut ScopeFixture| {
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixLost);
        },
        PointVisibility::MarkerTypeHidden
    )]
    #[case::generated_marker_while_another_kind_is_hidden(
        test_util::generated_marker(),
        |fixture: &mut ScopeFixture| {
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixRegained);
        },
        PointVisibility::Shown
    )]
    #[case::fix_while_every_marker_type_is_hidden(
        test_util::point(0),
        |fixture: &mut ScopeFixture| {
            fixture.hide_event_marker_path(test_util::EVENT_MARKER_PARENT_PATH);
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixLost);
        },
        PointVisibility::Shown
    )]
    fn the_tree_hides_a_marker_by_its_path_its_parent_path_or_its_kind(
        #[case] point: MapElementRef,
        #[case] hide: fn(&mut ScopeFixture),
        #[case] expected: PointVisibility,
    ) {
        let mut fixture = ScopeFixture::all_drawn();
        hide(&mut fixture);

        assert_eq!(fixture.scope().point_visibility(point), expected);
    }

    #[test]
    fn ghost_fixes_track_ghost_fixes_display_category() {
        let ghost_point = NavPoint::new(
            TimePositionVelocity::builder()
                .time(GpsTime::from_utc(test_util::start()))
                .lat(Latitude::new(55.0))
                .lon(Longitude::new(12.0))
                .build(),
            None,
        );
        let real_point = gt_types::fixtures::nav_point(
            test_util::start() + TimeDelta::seconds(1),
            Latitude::new(55.0),
            Longitude::new(12.0),
            FixKind::Measured,
        );
        let track = gt_track_builder::build_loaded_file(
            "test.gtd".to_owned(),
            &[ghost_point, real_point],
            &[],
            Vec::new(),
            Vec::new(),
            &[],
            &gt_track_builder::SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("test.gtd")),
            gt_track_builder::FileMeta::default(),
            Vec::new(),
        );
        let files = vec![track];
        let vis = TrackDataVisibility::from_loaded(&files);
        let filter = GlobalFilter::default();

        let ghost_ref =
            MapElementRef::Fix(gt_types::FixRef::new(test_util::track0(), PointIdx::new(0)));
        let real_ref =
            MapElementRef::Fix(gt_types::FixRef::new(test_util::track0(), PointIdx::new(1)));

        let mut mask = DisplayMask::default();
        let event_marker_visibility = EventMarkerVisibility::default();
        let generated_marker_visibility = GeneratedMarkerVisibility::default();
        let scope = MapScope {
            files: &files,
            visibility: &vis,
            event_marker_visibility: &event_marker_visibility,
            generated_marker_visibility: &generated_marker_visibility,
            filter: &filter,
            display_mask: mask,
            query_matches: None,
        };
        assert_eq!(scope.point_visibility(ghost_ref), PointVisibility::Shown);
        assert_eq!(scope.point_visibility(real_ref), PointVisibility::Shown);

        mask.set_visible(DisplayCategory::GhostFixes, false);
        let scope = MapScope {
            display_mask: mask,
            ..scope
        };
        assert_eq!(
            scope.point_visibility(ghost_ref),
            PointVisibility::CategoryHidden
        );
        assert_eq!(scope.point_visibility(real_ref), PointVisibility::Shown);

        mask = DisplayMask::default();
        mask.solo(DisplayCategory::GhostFixes);
        let scope = MapScope {
            display_mask: mask,
            ..scope
        };
        assert_eq!(scope.point_visibility(ghost_ref), PointVisibility::Shown);
        assert_eq!(
            scope.point_visibility(real_ref),
            PointVisibility::CategoryHidden
        );
    }
}
