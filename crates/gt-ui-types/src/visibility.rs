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
/// This is the compatibility view used by renderers and tests. Internally,
/// [`MapEligibility`] owns semantic policy and [`MapPresence`] adds only the
/// display mask.
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
    /// The element's category is off, either in the track's tree toggles or in
    /// the display mask.
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

/// Why an existing element is withheld before display masking is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EligibilityWithheld {
    /// The file or track is off in the tree, or the track fails the filter.
    TrackNotShown,
    /// The element's category is off in the track's tree toggles.
    CategoryHidden,
    /// The marker's generated kind or event path is hidden.
    MarkerTypeHidden,
    /// A `keep` or `hide` query removed the fix-backed element.
    HiddenByQuery,
    /// The element lies outside the global time window.
    OutsideTimeFilter,
}

/// The result of applying all semantic map policy except the display mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapEligibilityResult {
    /// The addressed element exists and passes every semantic gate.
    Eligible,
    /// The raw identity does not resolve against the currently loaded files.
    Missing,
    /// The element exists, but semantic policy withholds it.
    Withheld(EligibilityWithheld),
}

/// Every policy source that determines whether a point-like element is
/// semantically eligible for the map, before render-side display masking.
///
/// All sources are mandatory at construction so callers cannot accidentally
/// evaluate an element against only a subset of map policy.
#[derive(Clone, Copy)]
pub struct MapEligibility<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    filter: &'a GlobalFilter,
    query_matches: Option<&'a QueryMatches>,
    generated_marker_visibility: &'a GeneratedMarkerVisibility,
    event_marker_visibility: &'a EventMarkerVisibility,
}

impl<'a> MapEligibility<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        filter: &'a GlobalFilter,
        query_matches: Option<&'a QueryMatches>,
        generated_marker_visibility: &'a GeneratedMarkerVisibility,
        event_marker_visibility: &'a EventMarkerVisibility,
    ) -> Self {
        Self {
            files,
            visibility,
            filter,
            query_matches,
            generated_marker_visibility,
            event_marker_visibility,
        }
    }

    /// Add render-side display policy to this complete semantic policy.
    pub fn with_display_mask(self, display_mask: DisplayMask) -> MapPresence<'a> {
        MapPresence::new(self, display_mask)
    }

    /// Classify `element_ref` before display masking.
    pub fn element_eligibility(self, element_ref: MapElementRef) -> MapEligibilityResult {
        match self.resolve(element_ref) {
            ResolvedEligibility::Eligible(_) => MapEligibilityResult::Eligible,
            ResolvedEligibility::Missing => MapEligibilityResult::Missing,
            ResolvedEligibility::Withheld(reason) => MapEligibilityResult::Withheld(reason),
        }
    }

    fn resolve(self, element_ref: MapElementRef) -> ResolvedEligibility<'a> {
        // Resolution comes first so a stale identity stays "missing" even when
        // the same track is also withheld by policy.
        let Some(element) = element_ref.resolve(self.files) else {
            return ResolvedEligibility::Missing;
        };
        let track = element_ref.track();
        let Some((_, track_visibility)) =
            track_in_scope(self.files, self.visibility, self.filter, track)
        else {
            return ResolvedEligibility::Withheld(EligibilityWithheld::TrackNotShown);
        };

        // This is the semantic authority for point-like map elements. Keep the
        // family match exhaustive so a new identity cannot silently inherit an
        // existing policy.
        let withheld = match element {
            ResolvedElement::Fix(fix) | ResolvedElement::SatelliteReport { fix, .. } => {
                if !track_visibility.category_visible(DataCategory::Tpv) {
                    Some(EligibilityWithheld::CategoryHidden)
                } else if self.query_matches.is_some_and(|matches| {
                    element_ref
                        .fix()
                        .is_some_and(|fix_ref| matches.is_hidden(track, fix_ref.point.as_usize()))
                }) {
                    Some(EligibilityWithheld::HiddenByQuery)
                } else if !gt_filter::point_passes_time_filter(fix.tpv.time().utc(), self.filter) {
                    Some(EligibilityWithheld::OutsideTimeFilter)
                } else {
                    None
                }
            }
            ResolvedElement::CustomMarker(marker) => {
                if !track_visibility.category_visible(DataCategory::CustomMarker) {
                    Some(EligibilityWithheld::CategoryHidden)
                } else if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                    Some(EligibilityWithheld::OutsideTimeFilter)
                } else {
                    None
                }
            }
            ResolvedElement::GeneratedMarker(marker) => {
                if !track_visibility.category_visible(DataCategory::GeneratedMarker) {
                    Some(EligibilityWithheld::CategoryHidden)
                } else if !self
                    .generated_marker_visibility
                    .is_visible(track, marker.kind.tag())
                {
                    Some(EligibilityWithheld::MarkerTypeHidden)
                } else if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                    Some(EligibilityWithheld::OutsideTimeFilter)
                } else {
                    None
                }
            }
            ResolvedElement::EventMarker(marker) => {
                if !track_visibility.category_visible(DataCategory::EventMarker) {
                    Some(EligibilityWithheld::CategoryHidden)
                } else if !self
                    .event_marker_visibility
                    .is_visible(track, &marker.variant_path)
                {
                    Some(EligibilityWithheld::MarkerTypeHidden)
                } else if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                    Some(EligibilityWithheld::OutsideTimeFilter)
                } else {
                    None
                }
            }
        };

        if let Some(reason) = withheld {
            ResolvedEligibility::Withheld(reason)
        } else {
            ResolvedEligibility::Eligible(EligibleElementRef {
                element_ref,
                element,
            })
        }
    }
}

#[derive(Clone, Copy)]
struct EligibleElementRef<'a> {
    element_ref: MapElementRef,
    element: ResolvedElement<'a>,
}

enum ResolvedEligibility<'a> {
    Eligible(EligibleElementRef<'a>),
    Missing,
    Withheld(EligibilityWithheld),
}

/// Why an existing semantically eligible element is absent from the rendered
/// map after display policy is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceWithheld {
    /// Pre-display semantic policy withheld the element.
    Eligibility(EligibilityWithheld),
    /// The element's render category is disabled by the display mask.
    DisplayMasked,
}

/// The result of resolving a raw map identity through all map policy.
#[derive(Clone, Copy)]
pub enum MapPresenceResult<'a> {
    /// The element is present and carries the proof required by present-only
    /// operations.
    Present(PresentElementRef<'a>),
    /// The raw identity no longer resolves against the loaded files.
    Missing,
    /// The element still exists but is withheld by policy.
    Withheld(PresenceWithheld),
}

/// Proof that a raw map identity resolves and is present under the complete
/// policy for this frame.
///
/// The fields are private so external code can obtain this value only by
/// resolving a [`MapElementRef`] through [`MapPresence`]. Do not persist this
/// value across frames. Persist the raw identity and resolve it again instead.
#[derive(Clone, Copy)]
pub struct PresentElementRef<'a> {
    element_ref: MapElementRef,
    element: ResolvedElement<'a>,
}

impl<'a> PresentElementRef<'a> {
    pub fn element_ref(self) -> MapElementRef {
        self.element_ref
    }

    pub fn element(self) -> ResolvedElement<'a> {
        self.element
    }
}

/// Complete point-like map policy for one frame: semantic eligibility plus the
/// render-side display mask.
#[derive(Clone, Copy)]
pub struct MapPresence<'a> {
    eligibility: MapEligibility<'a>,
    display_mask: DisplayMask,
}

impl<'a> MapPresence<'a> {
    pub fn new(eligibility: MapEligibility<'a>, display_mask: DisplayMask) -> Self {
        Self {
            eligibility,
            display_mask,
        }
    }

    /// The semantic policy beneath this display-presence view.
    pub fn eligibility(self) -> MapEligibility<'a> {
        self.eligibility
    }

    /// The loaded recordings backing this frame policy.
    pub fn files(self) -> &'a [LoadedFile] {
        self.eligibility.files
    }

    /// Return the same semantic authority with a different display mask.
    pub fn with_display_mask(self, display_mask: DisplayMask) -> Self {
        Self {
            display_mask,
            ..self
        }
    }

    /// Rebind the last-query policy while preserving every other policy source.
    pub fn with_query_matches<'b>(self, query_matches: Option<&'b QueryMatches>) -> MapPresence<'b>
    where
        'a: 'b,
    {
        MapPresence {
            eligibility: MapEligibility {
                files: self.eligibility.files,
                visibility: self.eligibility.visibility,
                filter: self.eligibility.filter,
                query_matches,
                generated_marker_visibility: self.eligibility.generated_marker_visibility,
                event_marker_visibility: self.eligibility.event_marker_visibility,
            },
            display_mask: self.display_mask,
        }
    }

    /// Resolve `element_ref` through semantic policy and the display mask.
    pub fn resolve(self, element_ref: MapElementRef) -> MapPresenceResult<'a> {
        match self.eligibility.resolve(element_ref) {
            ResolvedEligibility::Missing => MapPresenceResult::Missing,
            ResolvedEligibility::Withheld(reason) => {
                MapPresenceResult::Withheld(PresenceWithheld::Eligibility(reason))
            }
            ResolvedEligibility::Eligible(eligible) => {
                let display_category = match eligible.element {
                    ResolvedElement::Fix(fix) | ResolvedElement::SatelliteReport { fix, .. }
                        if fix.is_ghost_fix() =>
                    {
                        DisplayCategory::GhostFixes
                    }
                    ResolvedElement::Fix(_) | ResolvedElement::SatelliteReport { .. } => {
                        DisplayCategory::TrackPoints
                    }
                    ResolvedElement::CustomMarker(_) => DisplayCategory::CustomMarkers,
                    ResolvedElement::GeneratedMarker(_) => DisplayCategory::GeneratedMarkers,
                    ResolvedElement::EventMarker(_) => DisplayCategory::EventMarkers,
                };
                if self.display_mask.is_visible(display_category) {
                    MapPresenceResult::Present(PresentElementRef {
                        element_ref: eligible.element_ref,
                        element: eligible.element,
                    })
                } else {
                    MapPresenceResult::Withheld(PresenceWithheld::DisplayMasked)
                }
            }
        }
    }

    /// Whether the map draws the element `element_ref` addresses.
    pub fn draws(self, element_ref: MapElementRef) -> bool {
        matches!(self.resolve(element_ref), MapPresenceResult::Present(_))
    }

    /// Compatibility classification for existing map consumers.
    pub fn point_visibility(self, element_ref: MapElementRef) -> PointVisibility {
        match self.resolve(element_ref) {
            MapPresenceResult::Present(_) => PointVisibility::Shown,
            MapPresenceResult::Missing => PointVisibility::NoSuchElement,
            MapPresenceResult::Withheld(PresenceWithheld::DisplayMasked) => {
                PointVisibility::CategoryHidden
            }
            MapPresenceResult::Withheld(PresenceWithheld::Eligibility(reason)) => match reason {
                EligibilityWithheld::TrackNotShown => PointVisibility::TrackNotShown,
                EligibilityWithheld::CategoryHidden => PointVisibility::CategoryHidden,
                EligibilityWithheld::MarkerTypeHidden => PointVisibility::MarkerTypeHidden,
                EligibilityWithheld::HiddenByQuery => PointVisibility::HiddenByQuery,
                EligibilityWithheld::OutsideTimeFilter => PointVisibility::OutsideTimeFilter,
            },
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
    fn presence_distinguishes_missing_semantic_policy_and_display_masking() {
        let mut fixture = ScopeFixture::all_drawn();
        let satellite = MapElementRef::SatelliteReport(gt_types::FixRef::new(
            test_util::track0(),
            PointIdx::new(0),
        ));
        let presence = fixture.scope();

        assert_eq!(
            presence.eligibility().element_eligibility(satellite),
            MapEligibilityResult::Eligible
        );
        match presence.resolve(satellite) {
            MapPresenceResult::Present(present) => assert_eq!(present.element_ref(), satellite),
            MapPresenceResult::Missing | MapPresenceResult::Withheld(_) => {
                panic!("satellite report should be present")
            }
        }

        fixture
            .display_mask
            .set_visible(DisplayCategory::TrackPoints, false);
        let masked = fixture.scope();
        assert_eq!(
            masked.eligibility().element_eligibility(satellite),
            MapEligibilityResult::Eligible,
            "display masking must not leak into semantic eligibility"
        );
        assert!(matches!(
            masked.resolve(satellite),
            MapPresenceResult::Withheld(PresenceWithheld::DisplayMasked)
        ));

        let stale = MapElementRef::Fix(gt_types::FixRef::new(
            test_util::track0(),
            PointIdx::new(test_util::POINT_COUNT + 1),
        ));
        assert!(matches!(masked.resolve(stale), MapPresenceResult::Missing));
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
        let eligibility = MapEligibility::new(
            &files,
            &vis,
            &filter,
            None,
            &generated_marker_visibility,
            &event_marker_visibility,
        );
        let scope = eligibility.with_display_mask(mask);
        assert_eq!(scope.point_visibility(ghost_ref), PointVisibility::Shown);
        assert_eq!(scope.point_visibility(real_ref), PointVisibility::Shown);

        mask.set_visible(DisplayCategory::GhostFixes, false);
        let scope = scope.with_display_mask(mask);
        assert_eq!(
            scope.point_visibility(ghost_ref),
            PointVisibility::CategoryHidden
        );
        assert_eq!(scope.point_visibility(real_ref), PointVisibility::Shown);

        mask = DisplayMask::default();
        mask.solo(DisplayCategory::GhostFixes);
        let scope = scope.with_display_mask(mask);
        assert_eq!(scope.point_visibility(ghost_ref), PointVisibility::Shown);
        assert_eq!(
            scope.point_visibility(real_ref),
            PointVisibility::CategoryHidden
        );
    }
}
