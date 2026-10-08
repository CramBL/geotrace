use chrono::TimeDelta;
use gt_types::{
    CustomMarker, CustomMarkerIdx, CustomMarkerRef, DataCategory, FileIdx, FixRef,
    GeneratedMarkerKindTag, Latitude, Longitude, MarkerIcon, PointIdx, TrackIdx, TrackRef,
};
use proptest::prelude::*;

use super::{EligibilityWithheld, MapEligibilityResult, PointVisibility};
use crate::display_mask::{DisplayCategory, DisplayMask};
use crate::highlight::{MapElementRef, MapHighlight};
use crate::query_matches::{QueryMatches, TrackRanges};
use crate::test_util::{self, ScopeFixture};

#[derive(Debug, Clone, Copy)]
enum ElementCase {
    MeasuredFix,
    GhostFix,
    SatelliteReport,
    CustomMarker,
    GeneratedMarker,
    EventMarker,
    StalePoint,
    StaleTrack,
}

impl ElementCase {
    fn point_ref(self) -> MapElementRef {
        match self {
            Self::MeasuredFix => test_util::point(0),
            Self::GhostFix => test_util::point(test_util::POINT_COUNT - 1),
            Self::SatelliteReport => {
                MapElementRef::SatelliteReport(FixRef::new(test_util::track0(), PointIdx::new(0)))
            }
            Self::CustomMarker => MapElementRef::CustomMarker(CustomMarkerRef::new(
                test_util::track0(),
                CustomMarkerIdx::new(0),
            )),
            Self::GeneratedMarker => test_util::generated_marker(),
            Self::EventMarker => test_util::event_marker(),
            Self::StalePoint => MapElementRef::Fix(FixRef::new(
                test_util::track0(),
                PointIdx::new(test_util::POINT_COUNT + 7),
            )),
            Self::StaleTrack => MapElementRef::Fix(FixRef::new(
                TrackRef::new(FileIdx::new(0), TrackIdx::new(7)),
                PointIdx::new(0),
            )),
        }
    }

    fn policy_category(self) -> DataCategory {
        match self {
            Self::SatelliteReport => DataCategory::Tpv,
            _ => self.point_ref().category(),
        }
    }

    fn display_category(self) -> DisplayCategory {
        match self {
            Self::GhostFix => DisplayCategory::GhostFixes,
            _ => DisplayCategory::from(self.policy_category()),
        }
    }

    fn is_missing(self) -> bool {
        matches!(self, Self::StalePoint | Self::StaleTrack)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventPathVisibility {
    Visible,
    ExactHidden,
    ParentHidden,
}

#[derive(Debug, Clone, Copy)]
struct VisibilityScenario {
    element: ElementCase,
    file_enabled: bool,
    track_enabled: bool,
    tree_category_visible: bool,
    display_category_visible: bool,
    track_filter_passes: bool,
    element_time_passes: bool,
    query_keeps_point: bool,
    generated_kind_visible: bool,
    event_path_visibility: EventPathVisibility,
}

impl VisibilityScenario {
    fn apply(&self) -> ScopeFixture {
        let mut fixture = ScopeFixture::all_drawn();
        let custom_marker = CustomMarker::new(
            test_util::start() + TimeDelta::seconds(1),
            "note".to_owned(),
            MarkerIcon::Pin,
            Latitude::new(55.0),
            Longitude::new(12.0),
        );
        fixture.files[0].tracks[0]
            .custom_markers
            .push(custom_marker);
        let point = self.element.point_ref();
        let category = self.element.policy_category();

        fixture.visibility.files[0].enabled = self.file_enabled;
        fixture.visibility.files[0].tracks[0].enabled = self.track_enabled;
        fixture.visibility.files[0].tracks[0]
            .set_category_visible(category, self.tree_category_visible);

        let mut display_mask = DisplayMask::default();
        display_mask.set_visible(
            self.element.display_category(),
            self.display_category_visible,
        );
        fixture.display_mask = display_mask;

        if !self.track_filter_passes {
            fixture
                .filter
                .set_minimum_duration(Some(TimeDelta::hours(1)));
        }
        if !self.element_time_passes && !self.element.is_missing() {
            withhold_element_by_time(&mut fixture, point);
        }
        if !self.query_keeps_point && category == DataCategory::Tpv && !self.element.is_missing() {
            fixture.query_matches = Some(QueryMatches {
                hidden: TrackRanges::from_iter([(
                    test_util::track0(),
                    std::iter::once({
                        let index = point
                            .fix()
                            .expect("TPV policy elements are fix-backed")
                            .point;
                        index.as_usize()..index.as_usize() + 1
                    })
                    .collect(),
                )]),
                ..QueryMatches::default()
            });
        }
        if !self.generated_kind_visible {
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixLost);
        }
        match self.event_path_visibility {
            EventPathVisibility::Visible => {}
            EventPathVisibility::ExactHidden => {
                fixture.hide_event_marker_path(test_util::EVENT_MARKER_PATH);
            }
            EventPathVisibility::ParentHidden => {
                fixture.hide_event_marker_path(test_util::EVENT_MARKER_PARENT_PATH);
            }
        }
        fixture
    }

    fn eligibility_oracle(&self) -> MapEligibilityResult {
        if self.element.is_missing() {
            return MapEligibilityResult::Missing;
        }
        if !self.file_enabled || !self.track_enabled || !self.track_filter_passes {
            return MapEligibilityResult::Withheld(EligibilityWithheld::TrackNotShown);
        }
        if !self.tree_category_visible {
            return MapEligibilityResult::Withheld(EligibilityWithheld::CategoryHidden);
        }
        if matches!(self.element, ElementCase::GeneratedMarker) && !self.generated_kind_visible {
            return MapEligibilityResult::Withheld(EligibilityWithheld::MarkerTypeHidden);
        }
        if matches!(self.element, ElementCase::EventMarker)
            && self.event_path_visibility != EventPathVisibility::Visible
        {
            return MapEligibilityResult::Withheld(EligibilityWithheld::MarkerTypeHidden);
        }
        if self.element.policy_category() == DataCategory::Tpv && !self.query_keeps_point {
            return MapEligibilityResult::Withheld(EligibilityWithheld::HiddenByQuery);
        }
        if !self.element_time_passes {
            return MapEligibilityResult::Withheld(EligibilityWithheld::OutsideTimeFilter);
        }
        MapEligibilityResult::Eligible
    }

    fn oracle(&self) -> PointVisibility {
        match self.eligibility_oracle() {
            MapEligibilityResult::Missing => PointVisibility::NoSuchElement,
            MapEligibilityResult::Withheld(EligibilityWithheld::TrackNotShown) => {
                PointVisibility::TrackNotShown
            }
            MapEligibilityResult::Withheld(EligibilityWithheld::CategoryHidden) => {
                PointVisibility::CategoryHidden
            }
            MapEligibilityResult::Withheld(EligibilityWithheld::MarkerTypeHidden) => {
                PointVisibility::MarkerTypeHidden
            }
            MapEligibilityResult::Withheld(EligibilityWithheld::HiddenByQuery) => {
                PointVisibility::HiddenByQuery
            }
            MapEligibilityResult::Withheld(EligibilityWithheld::OutsideTimeFilter) => {
                PointVisibility::OutsideTimeFilter
            }
            MapEligibilityResult::Eligible if !self.display_category_visible => {
                PointVisibility::CategoryHidden
            }
            MapEligibilityResult::Eligible => PointVisibility::Shown,
        }
    }

    fn additional_hides(&self) -> Vec<Self> {
        let mut variants = Vec::with_capacity(9);
        let mut push = |edit: fn(&mut Self)| {
            let mut scenario = *self;
            edit(&mut scenario);
            variants.push(scenario);
        };
        push(|scenario| scenario.file_enabled = false);
        push(|scenario| scenario.track_enabled = false);
        push(|scenario| scenario.tree_category_visible = false);
        push(|scenario| scenario.display_category_visible = false);
        push(|scenario| scenario.track_filter_passes = false);
        push(|scenario| scenario.element_time_passes = false);
        push(|scenario| scenario.query_keeps_point = false);
        push(|scenario| scenario.generated_kind_visible = false);
        push(|scenario| scenario.event_path_visibility = EventPathVisibility::ParentHidden);
        variants
    }
}

fn withhold_element_by_time(fixture: &mut ScopeFixture, point: MapElementRef) {
    let Some(track) = point.track().resolve(&fixture.files) else {
        return;
    };
    let Some(element) = point.resolve(&fixture.files) else {
        return;
    };
    let time = element.time();
    let track_start = track.metadata.time_range.start;
    let track_end = track.metadata.time_range.end;
    let tick = TimeDelta::nanoseconds(1);
    if time < track_end {
        let end = fixture
            .filter
            .time_window()
            .bounds()
            .and_then(|(_, end)| end);
        fixture.filter.set_time_bounds(Some(time + tick), end);
    } else {
        debug_assert!(time > track_start);
        let start = fixture
            .filter
            .time_window()
            .bounds()
            .and_then(|(start, _)| start);
        fixture.filter.set_time_bounds(start, Some(time - tick));
    }
}

fn element_case() -> impl Strategy<Value = ElementCase> {
    prop_oneof![
        Just(ElementCase::MeasuredFix),
        Just(ElementCase::GhostFix),
        Just(ElementCase::SatelliteReport),
        Just(ElementCase::CustomMarker),
        Just(ElementCase::GeneratedMarker),
        Just(ElementCase::EventMarker),
        Just(ElementCase::StalePoint),
        Just(ElementCase::StaleTrack),
    ]
}

fn event_path_visibility() -> impl Strategy<Value = EventPathVisibility> {
    prop_oneof![
        Just(EventPathVisibility::Visible),
        Just(EventPathVisibility::ExactHidden),
        Just(EventPathVisibility::ParentHidden),
    ]
}

fn scenario() -> impl Strategy<Value = VisibilityScenario> {
    (
        element_case(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        event_path_visibility(),
    )
        .prop_map(
            |(
                element,
                file_enabled,
                track_enabled,
                tree_category_visible,
                display_category_visible,
                track_filter_passes,
                element_time_passes,
                query_keeps_point,
                generated_kind_visible,
                event_path_visibility,
            )| VisibilityScenario {
                element,
                file_enabled,
                track_enabled,
                tree_category_visible,
                display_category_visible,
                track_filter_passes,
                element_time_passes,
                query_keeps_point,
                generated_kind_visible,
                event_path_visibility,
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn map_presence_agrees_with_an_independent_visibility_oracle(case in scenario()) {
        let fixture = case.apply();
        let point = case.element.point_ref();
        let presence = fixture.scope();
        prop_assert_eq!(
            presence.point_visibility(point),
            case.oracle(),
        );
        prop_assert_eq!(
            presence.eligibility().element_eligibility(point),
            case.eligibility_oracle(),
        );
    }

    #[test]
    fn adding_a_hide_gate_never_resurrects_an_element(case in scenario()) {
        let point = case.element.point_ref();
        let before = case.apply().scope().draws(point);
        for restricted in case.additional_hides() {
            let after = restricted.apply().scope().draws(point);
            prop_assert!(!after || before, "{case:?} -> {restricted:?}");
        }
    }

    #[test]
    fn checked_pinning_agrees_with_visibility(case in scenario()) {
        let point = case.element.point_ref();
        let fixture = case.apply();
        let shown = fixture.scope().draws(point);
        let mut highlight = MapHighlight::default();

        prop_assert_eq!(highlight.toggle_sticky_if_drawn(fixture.scope(), point), shown);
        prop_assert_eq!(highlight.sticky, shown.then_some(point));
    }

    #[test]
    fn stale_refs_remain_missing_instead_of_becoming_withheld(
        mut case in scenario(),
        stale_track in any::<bool>(),
    ) {
        case.element = if stale_track {
            ElementCase::StaleTrack
        } else {
            ElementCase::StalePoint
        };
        let point = case.element.point_ref();
        let fixture = case.apply();
        let mut highlight = MapHighlight {
            sticky: Some(point),
            ..MapHighlight::default()
        };

        prop_assert_eq!(fixture.scope().point_visibility(point), PointVisibility::NoSuchElement);
        prop_assert_eq!(highlight.pin_this_frame(fixture.scope()), None);
        prop_assert_eq!(highlight.sticky, None);
    }

    #[test]
    fn hidden_marker_refinements_cannot_be_pinned(
        generated in any::<bool>(),
        parent_path in any::<bool>(),
    ) {
        let element = if generated {
            ElementCase::GeneratedMarker
        } else {
            ElementCase::EventMarker
        };
        let case = VisibilityScenario {
            element,
            file_enabled: true,
            track_enabled: true,
            tree_category_visible: true,
            display_category_visible: true,
            track_filter_passes: true,
            element_time_passes: true,
            query_keeps_point: true,
            generated_kind_visible: !generated,
            event_path_visibility: if generated {
                EventPathVisibility::Visible
            } else if parent_path {
                EventPathVisibility::ParentHidden
            } else {
                EventPathVisibility::ExactHidden
            },
        };
        let point = element.point_ref();
        let fixture = case.apply();
        let mut highlight = MapHighlight::default();

        prop_assert_eq!(fixture.scope().point_visibility(point), PointVisibility::MarkerTypeHidden);
        prop_assert!(!highlight.toggle_sticky_if_drawn(fixture.scope(), point));
        prop_assert_eq!(highlight.sticky, None);
    }
}
