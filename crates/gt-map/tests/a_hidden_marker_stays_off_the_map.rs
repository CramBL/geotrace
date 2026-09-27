//! Each marker renderer draws a marker's icon exactly while hover and click
//! reach the marker: the renderers and the hit test read one predicate.

use std::iter;

use gt_map::test_util::{self, CENTRE_FIX, MapScene, RenderedMap};
use gt_types::{DataCategory, GeneratedMarkerKindTag, LoadedFile};
use gt_ui_types::{DataPointRef, DisplayCategory};

/// A marker on the fix at [`CENTRE_FIX`] and one way to hide it, for each
/// marker renderer.
///
/// A custom marker has two: the display toggle drops its category before the
/// renderer runs, and the renderer drops a marker outside the time window
/// itself.
#[derive(Clone, Copy, Debug)]
enum HiddenMarker {
    CustomMarkerMaskedByTheDisplayToggle,
    CustomMarkerOutsideTheTimeWindow,
    EventMarkerOfAHiddenPath,
    GeneratedMarkerOfAHiddenKind,
}

impl HiddenMarker {
    fn recording(self) -> Vec<LoadedFile> {
        let files = test_util::a_walking_recording();
        match self {
            Self::CustomMarkerMaskedByTheDisplayToggle | Self::CustomMarkerOutsideTheTimeWindow => {
                test_util::with_a_custom_marker_on_a_fix(files, CENTRE_FIX)
            }
            Self::EventMarkerOfAHiddenPath => {
                test_util::with_an_event_marker_on_a_fix(files, CENTRE_FIX)
            }
            Self::GeneratedMarkerOfAHiddenKind => {
                test_util::with_a_generated_marker_on_a_fix(files, CENTRE_FIX)
            }
        }
    }

    fn point_ref(self) -> DataPointRef {
        let category = match self {
            Self::CustomMarkerMaskedByTheDisplayToggle | Self::CustomMarkerOutsideTheTimeWindow => {
                DataCategory::CustomMarker
            }
            Self::EventMarkerOfAHiddenPath => DataCategory::EventMarker,
            Self::GeneratedMarkerOfAHiddenKind => DataCategory::GeneratedMarker,
        };
        test_util::point_ref(category, 0)
    }

    fn hide(self, map: &mut RenderedMap) {
        let state = map.draw_state();
        match self {
            Self::CustomMarkerMaskedByTheDisplayToggle => state
                .display_mask
                .set_visible(DisplayCategory::CustomMarkers, false),
            Self::CustomMarkerOutsideTheTimeWindow => {
                state.filter = test_util::window_ending_at(CENTRE_FIX - 1);
            }
            Self::EventMarkerOfAHiddenPath => state.event_marker_visibility.set_hidden(
                test_util::track0(),
                iter::once(test_util::EVENT_MARKER_ON_A_FIX_PATH.to_owned()),
            ),
            Self::GeneratedMarkerOfAHiddenKind => state.generated_marker_visibility.set_hidden(
                test_util::track0(),
                iter::once(GeneratedMarkerKindTag::GnssFixLost),
            ),
        }
    }
}

/// `files` centred on the fix at [`CENTRE_FIX`] with the fix icons hidden,
/// framed with everything shown, then, if `hidden` holds a marker, drawn one
/// more frame with that marker hidden. At the viewport centre, the pointer
/// reaches the marker on that fix and nothing else.
fn map_centred_on_the_marker(files: Vec<LoadedFile>, hidden: Option<HiddenMarker>) -> RenderedMap {
    let mut map = MapScene::of(files)
        .centred_on_fix(CENTRE_FIX)
        .hiding_the_fix_icons()
        .render();
    if let Some(marker) = hidden {
        marker.hide(&mut map);
    }
    map.render_one_more_frame();
    map
}

/// How many shapes [`map_centred_on_the_marker`] paints over
/// [`test_util::a_walking_recording`], which has no marker.
fn shapes_painted_without_a_marker(hidden: Option<HiddenMarker>) -> usize {
    map_centred_on_the_marker(test_util::a_walking_recording(), hidden).shapes_painted()
}

#[rstest::rstest]
#[case::custom_marker_masked_by_the_display_toggle(
    HiddenMarker::CustomMarkerMaskedByTheDisplayToggle
)]
#[case::custom_marker_outside_the_time_window(HiddenMarker::CustomMarkerOutsideTheTimeWindow)]
#[case::event_marker_of_a_hidden_path(HiddenMarker::EventMarkerOfAHiddenPath)]
#[case::generated_marker_of_a_hidden_kind(HiddenMarker::GeneratedMarkerOfAHiddenKind)]
fn a_marker_draws_an_icon_exactly_while_the_pointer_reaches_it(#[case] marker: HiddenMarker) {
    let mut shown = map_centred_on_the_marker(marker.recording(), None);
    let mut hidden = map_centred_on_the_marker(marker.recording(), Some(marker));

    assert!(
        shown.shapes_painted() > shapes_painted_without_a_marker(None),
        "the shown marker put no ink on the map"
    );
    assert_eq!(
        shown.primary_hover_candidate_at(test_util::viewport_center()),
        Some(marker.point_ref())
    );
    assert_eq!(
        hidden.shapes_painted(),
        shapes_painted_without_a_marker(Some(marker))
    );
    assert_eq!(
        hidden.primary_hover_candidate_at(test_util::viewport_center()),
        None
    );
}

#[rstest::rstest]
#[case::event_marker_of_a_hidden_path(HiddenMarker::EventMarkerOfAHiddenPath)]
#[case::generated_marker_of_a_hidden_kind(HiddenMarker::GeneratedMarkerOfAHiddenKind)]
fn a_click_on_a_marker_pins_it_only_while_it_is_shown(#[case] marker: HiddenMarker) {
    let mut shown = map_centred_on_the_marker(marker.recording(), None);
    let mut hidden = map_centred_on_the_marker(marker.recording(), Some(marker));

    assert_eq!(
        shown.point_pinned_by_a_click_at(test_util::viewport_center()),
        Some(marker.point_ref())
    );
    assert_eq!(
        hidden.point_pinned_by_a_click_at(test_util::viewport_center()),
        None
    );
}
