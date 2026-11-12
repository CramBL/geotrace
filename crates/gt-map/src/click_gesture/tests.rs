use egui::{Event, Modifiers, PointerButton};
use gt_types::DataCategory;
use gt_ui_types::{DisplayCategory, LogMatchColor, LogMatches, MapElementRef};
use rstest::rstest;

use crate::test_util::{self, CENTRE_FIX, MapScene, RenderedMap};

#[derive(Clone, Copy)]
enum Target {
    Ambiguous,
    Bare,
    Fix,
    Log,
}

struct GestureScene {
    map: RenderedMap,
    target: egui::Pos2,
}

impl GestureScene {
    fn new(target: Target) -> Self {
        let mut files = test_util::a_walking_recording();
        if matches!(target, Target::Ambiguous) {
            files = test_util::with_an_event_marker_on_a_fix(files, CENTRE_FIX);
        }
        let log = test_util::a_log_over(&files);
        let matches = test_util::matches_over(&files, &log, CENTRE_FIX..CENTRE_FIX + 1);
        let mut map = MapScene::of(files)
            .size(VIEWPORT)
            .centred_on_fix(CENTRE_FIX)
            .zoomed_to(INITIAL_ZOOM)
            .draw_state(|draw| {
                if matches!(target, Target::Log) {
                    draw.log_matches = matches;
                }
            })
            .render();
        map.release_camera_overrides();
        let position = egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT).center();
        let target = if matches!(target, Target::Bare) {
            position + egui::vec2(0.0, -BARE_MAP_OFFSET)
        } else {
            position
        };
        map.move_pointer_to(target);
        map.render_one_more_frame();
        Self { map, target }
    }

    fn release(&mut self, button: PointerButton) {
        let now = self.map.harness.inner.ctx.input(|input| input.time);
        let input = self.map.harness.inner.input_mut();
        input.time = Some(now + RELEASE_INTERVAL);
        for pressed in [true, false] {
            input.events.push(Event::PointerButton {
                pos: self.target,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        self.map.render_one_more_frame();
        self.map.harness.inner.input_mut().time = None;
    }

    fn sticky(&mut self) -> Option<MapElementRef> {
        self.map.draw_state().highlight.sticky()
    }

    fn zoom(&self) -> f64 {
        self.map.map().expect("the rendered map").map_memory.zoom()
    }
}

#[rstest]
#[case::unselected_fix(Target::Fix, None)]
#[case::pinned_fix(Target::Fix, Some(CENTRE_FIX))]
#[case::empty_map(Target::Bare, Some(CENTRE_FIX))]
#[case::overlapping_elements(Target::Ambiguous, None)]
#[case::log_glyph(Target::Log, None)]
fn a_navigation_double_click_preserves_selection_on_both_releases(
    #[case] target: Target,
    #[case] pinned: Option<usize>,
) {
    let mut scene = GestureScene::new(target);
    let selected = pinned.map(|index| test_util::point_ref(DataCategory::Tpv, index));
    if let Some(point) = selected {
        assert!(scene.map.pin(point));
    }

    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), selected);
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.map.clicked_log_glyph(), None);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());

    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), selected);
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.map.clicked_log_glyph(), None);
    let fitted_zoom = scene.zoom();
    assert!(fitted_zoom < INITIAL_ZOOM);

    scene.map.finish_primary_click();
    assert_eq!(scene.sticky(), selected);
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.map.clicked_log_glyph(), None);
    assert_eq!(scene.zoom(), fitted_zoom);
}

struct SingleClickEffect {
    selected: Option<usize>,
    disambiguation: bool,
    log: bool,
}

#[rstest]
#[case::pin(Target::Fix, None, SingleClickEffect {selected: Some(CENTRE_FIX), disambiguation: false, log: false})]
#[case::unpin(Target::Fix, Some(CENTRE_FIX), SingleClickEffect {selected: None, disambiguation: false, log: false})]
#[case::deselect(Target::Bare, Some(CENTRE_FIX), SingleClickEffect {selected: None, disambiguation: false, log: false})]
#[case::disambiguate(Target::Ambiguous, None, SingleClickEffect {selected: None, disambiguation: true, log: false})]
#[case::open_log(Target::Log, None, SingleClickEffect {selected: Some(CENTRE_FIX), disambiguation: false, log: true})]
fn a_single_click_applies_its_action_after_the_double_click_window(
    #[case] target: Target,
    #[case] pinned: Option<usize>,
    #[case] expected: SingleClickEffect,
) {
    let mut scene = GestureScene::new(target);
    let selected = pinned.map(|index| test_util::point_ref(DataCategory::Tpv, index));
    if let Some(point) = selected {
        assert!(scene.map.pin(point));
    }
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), selected);
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.map.clicked_log_glyph(), None);

    scene.map.finish_primary_click();
    assert_eq!(
        scene.sticky(),
        expected
            .selected
            .map(|index| test_util::point_ref(DataCategory::Tpv, index))
    );
    assert_eq!(
        scene.map.disambiguation_popup_is_open(),
        expected.disambiguation
    );
    assert_eq!(scene.map.clicked_log_glyph().is_some(), expected.log);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[test]
fn two_distant_clicks_commit_the_first_target_and_defer_the_second() {
    let mut scene = GestureScene::new(Target::Fix);
    scene.release(PointerButton::Primary);
    scene.target += egui::vec2(BARE_MAP_OFFSET, -BARE_MAP_OFFSET);
    scene.release(PointerButton::Primary);
    assert_eq!(
        scene.sticky(),
        Some(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX))
    );
    scene.map.finish_primary_click();
    assert_eq!(scene.sticky(), None);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[test]
fn a_pending_click_keeps_its_original_target_after_the_camera_and_pointer_change() {
    let mut scene = GestureScene::new(Target::Fix);
    scene.release(PointerButton::Primary);
    let map = scene.map.map_mut().expect("the rendered map");
    map.map_memory.center_at(walkers::lat_lon(0.0, 0.0));
    scene.map.move_pointer_to(egui::pos2(20.0, 20.0));
    scene.map.finish_primary_click();
    assert_eq!(
        scene.sticky(),
        Some(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX))
    );
}

#[rstest]
#[case::fix(Target::Fix, DisplayCategory::TrackPoints)]
#[case::log(Target::Log, DisplayCategory::LogMatches)]
fn pending_click_targets_require_current_display_presence(
    #[case] target: Target,
    #[case] hidden: DisplayCategory,
) {
    let mut scene = GestureScene::new(target);
    scene.release(PointerButton::Primary);
    scene
        .map
        .draw_state()
        .display_mask
        .set_visible(hidden, false);
    scene.map.finish_primary_click();
    if matches!(target, Target::Fix) {
        assert_eq!(scene.sticky(), None);
    }
    assert_eq!(scene.map.clicked_log_glyph(), None);
}

#[test]
fn pending_log_activation_requires_an_entry_inside_the_time_filter() {
    let mut scene = GestureScene::new(Target::Log);
    scene.release(PointerButton::Primary);
    scene.map.draw_state().filter = test_util::window_ending_at(CENTRE_FIX - 1);
    scene.map.finish_primary_click();
    assert_eq!(scene.map.clicked_log_glyph(), None);
    assert_eq!(scene.sticky(), None);
}

#[test]
fn pending_log_activation_includes_entries_from_later_layers_with_the_same_color() {
    let mut scene = GestureScene::new(Target::Log);
    let mut layers = scene.map.draw_state().log_matches.layers().to_vec();
    let mut earlier = layers.first().expect("the log layer").clone();
    earlier
        .matches
        .first_mut()
        .expect("the log entry")
        .entry_index = CENTRE_FIX - 1;
    layers.insert(0, earlier);
    scene.map.draw_state().log_matches = LogMatches::from_layers(layers);
    scene.release(PointerButton::Primary);
    scene.map.finish_primary_click();
    assert_eq!(
        scene
            .map
            .clicked_log_glyph()
            .expect("the clicked log entry")
            .entry_indices,
        [CENTRE_FIX],
    );
}

#[test]
fn pending_log_activation_is_preserved_after_a_shared_color_flag_change() {
    let mut scene = GestureScene::new(Target::Log);
    scene.release(PointerButton::Primary);
    let mut layers = scene.map.draw_state().log_matches.layers().to_vec();
    layers.first_mut().expect("the log layer").color = LogMatchColor::LayerSlot {
        index: 0,
        shared: true,
    };
    scene.map.draw_state().log_matches = LogMatches::from_layers(layers);
    scene.map.finish_primary_click();
    assert_eq!(
        scene
            .map
            .clicked_log_glyph()
            .expect("the clicked log entry")
            .entry_indices,
        [CENTRE_FIX],
    );
}

#[test]
fn rebuilding_the_spatial_index_cancels_a_pending_click() {
    let files = test_util::a_walking_recording();
    let mut scene = GestureScene::new(Target::Fix);
    scene.release(PointerButton::Primary);
    scene
        .map
        .map_mut()
        .expect("the rendered map")
        .rebuild_spatial_index(&files);
    scene.map.finish_primary_click();
    assert_eq!(scene.sticky(), None);
}

#[rstest]
#[case::navigation(true)]
#[case::single_click(false)]
fn disambiguation_popup_dismissal_is_deferred_until_gesture_classification(
    #[case] navigation: bool,
) {
    let mut scene = GestureScene::new(Target::Ambiguous);
    scene.release(PointerButton::Primary);
    scene.map.finish_primary_click();
    assert!(scene.map.disambiguation_popup_is_open());
    scene.target += egui::vec2(0.0, -BARE_MAP_OFFSET);
    scene.map.move_pointer_to(scene.target);
    scene.release(PointerButton::Primary);
    assert!(scene.map.disambiguation_popup_is_open());
    if navigation {
        scene.release(PointerButton::Primary);
        assert!(scene.map.disambiguation_popup_is_open());
    }
    scene.map.finish_primary_click();
    assert_eq!(scene.map.disambiguation_popup_is_open(), navigation);
}

#[test]
fn escape_cancels_pending_disambiguation() {
    let mut scene = GestureScene::new(Target::Ambiguous);
    scene.release(PointerButton::Primary);
    scene.map.press_escape();
    scene.map.finish_primary_click();
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.sticky(), None);
}

#[test]
fn secondary_clicks_open_the_context_menu_without_fitting_or_selecting() {
    let mut scene = GestureScene::new(Target::Fix);
    scene.release(PointerButton::Secondary);
    let now = scene.map.harness.inner.ctx.input(|input| input.time);
    scene.map.harness.inner.input_mut().time = Some(now + RELEASE_INTERVAL);
    scene.map.render_one_more_frame();
    scene.map.harness.inner.input_mut().time = None;
    assert!(scene.map.any_popup_is_open());
    assert_eq!(scene.sticky(), None);
    scene.release(PointerButton::Secondary);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
    assert_eq!(scene.sticky(), None);
}

const VIEWPORT: egui::Vec2 = egui::vec2(1600.0, 1200.0);
const INITIAL_ZOOM: f64 = 18.0;
const BARE_MAP_OFFSET: f32 = 200.0;
const RELEASE_INTERVAL: f64 = 0.05;
