use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable as _;
use gt_test_utils::By;
use gt_types::DataCategory;
use gt_ui_types::{DisplayCategory, LogMatchColor, LogMatches, MapElementRef};
use rstest::rstest;
use walkers::{MercatorProjection, Projector};

use crate::test_util::{self, CENTRE_FIX, MapScene, RenderedMap};
use crate::{DoubleClickBehavior, MapControls};

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
        map.map_mut()
            .expect("the rendered map")
            .set_controls(MapControls {
                double_click: DoubleClickBehavior::FitVisible,
                ..MapControls::default()
            });
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

    fn layout_popup(&mut self) {
        let now = self.map.harness.inner.ctx.input(|input| input.time);
        self.map.harness.inner.input_mut().time = Some(now);
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
fn a_navigation_double_click_restores_selection_after_the_second_release(
    #[case] target: Target,
    #[case] pinned: Option<usize>,
    #[values(DoubleClickBehavior::FitVisible, DoubleClickBehavior::ZoomAndCenter)]
    double_click: DoubleClickBehavior,
) {
    let mut scene = GestureScene::new(target);
    scene
        .map
        .map_mut()
        .expect("the rendered map")
        .set_controls(MapControls {
            double_click,
            ..MapControls::default()
        });
    let selected = pinned.map(|index| test_util::point_ref(DataCategory::Tpv, index));
    if let Some(point) = selected {
        assert!(scene.map.pin(point));
    }

    scene.release(PointerButton::Primary);
    let immediately_selected = match target {
        Target::Fix | Target::Log if selected.is_none() => {
            Some(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX))
        }
        Target::Ambiguous => selected,
        Target::Bare | Target::Fix | Target::Log => None,
    };
    assert_eq!(scene.sticky(), immediately_selected);
    assert_eq!(
        scene.map.disambiguation_popup_is_open(),
        matches!(target, Target::Ambiguous)
    );
    assert_eq!(scene.map.clicked_log_glyph(), None);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());

    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), selected);
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.map.clicked_log_glyph(), None);
    let fitted_zoom = scene.zoom();
    if double_click == DoubleClickBehavior::FitVisible {
        assert!(fitted_zoom < INITIAL_ZOOM);
    } else {
        assert_eq!(fitted_zoom.to_bits(), (INITIAL_ZOOM + 1.0).to_bits());
    }

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
fn a_single_click_applies_point_actions_on_the_first_release(
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
    assert_eq!(scene.map.clicked_log_glyph(), None);
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
    if expected.selected.is_some() {
        scene.layout_popup();
        scene.map.harness.inner.get(By::new().role(Role::Window));
    }
    scene.map.finish_primary_click();
    assert_eq!(scene.map.clicked_log_glyph().is_some(), expected.log);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[test]
fn two_distant_clicks_apply_each_point_action_immediately() {
    let mut scene = GestureScene::new(Target::Fix);
    scene.release(PointerButton::Primary);
    scene.target += egui::vec2(BARE_MAP_OFFSET, -BARE_MAP_OFFSET);
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), None);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[test]
fn a_point_selection_remains_after_the_camera_and_pointer_change() {
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

#[test]
fn pending_log_activation_requires_current_display_presence() {
    let mut scene = GestureScene::new(Target::Log);
    scene.release(PointerButton::Primary);
    scene
        .map
        .draw_state()
        .display_mask
        .set_visible(DisplayCategory::LogMatches, false);
    scene.map.finish_primary_click();
    assert_eq!(scene.map.clicked_log_glyph(), None);
}

#[test]
fn pending_log_activation_requires_an_entry_inside_the_time_filter() {
    let mut scene = GestureScene::new(Target::Log);
    scene.release(PointerButton::Primary);
    scene.map.draw_state().filter = test_util::window_ending_at(CENTRE_FIX - 1);
    scene.map.finish_primary_click();
    assert_eq!(scene.map.clicked_log_glyph(), None);
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
fn rebuilding_the_spatial_index_cancels_a_pending_navigation_restore() {
    let files = test_util::a_walking_recording();
    let mut scene = GestureScene::new(Target::Fix);
    assert!(
        scene
            .map
            .pin(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX - 1))
    );
    scene.release(PointerButton::Primary);
    scene
        .map
        .map_mut()
        .expect("the rendered map")
        .rebuild_spatial_index(&files);
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), None);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[rstest]
#[case::navigation(true)]
#[case::single_click(false)]
fn double_click_restores_an_immediately_dismissed_disambiguation_popup(#[case] navigation: bool) {
    let mut scene = GestureScene::new(Target::Ambiguous);
    scene.release(PointerButton::Primary);
    scene.map.finish_primary_click();
    assert!(scene.map.disambiguation_popup_is_open());
    scene.target += egui::vec2(0.0, -BARE_MAP_OFFSET);
    scene.map.move_pointer_to(scene.target);
    scene.release(PointerButton::Primary);
    assert!(!scene.map.disambiguation_popup_is_open());
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
    scene
        .map
        .map_mut()
        .expect("the rendered map")
        .set_controls(MapControls::default());
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

#[rstest]
#[case::point(Target::Fix)]
#[case::disambiguation(Target::Ambiguous)]
fn double_click_restores_selection_when_the_immediate_popup_covers_the_pointer(
    #[case] target: Target,
    #[values(false, true)] near_edge: bool,
) {
    let mut scene = GestureScene::new(target);
    if near_edge {
        let destination = egui::pos2(VIEWPORT.x - 40.0, VIEWPORT.y - 150.0);
        let map = scene.map.map_mut().expect("the rendered map");
        let projector = Projector::new(
            egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT),
            &MercatorProjection,
            &map.map_memory,
            crate::default_map_center(),
        );
        let center = projector.unproject(scene.target.to_vec2() - (destination - scene.target));
        map.map_memory.center_at(center);
        scene.target = destination;
        scene.map.move_pointer_to(scene.target);
        scene.map.render_one_more_frame();
    }
    scene.release(PointerButton::Primary);
    scene.layout_popup();
    let layer = scene
        .map
        .harness
        .inner
        .ctx
        .layer_id_at(scene.target)
        .expect("the popup layer");
    assert_ne!(layer.order, egui::Order::Background);
    assert_eq!(
        layer.id,
        if matches!(target, Target::Ambiguous) {
            egui::Id::unique("map_disambig")
        } else {
            egui::Id::unique((
                "sticky_popup",
                test_util::point_ref(DataCategory::Tpv, CENTRE_FIX),
            ))
        }
    );
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), None);
    assert!(!scene.map.disambiguation_popup_is_open());
    assert!(scene.zoom() < INITIAL_ZOOM);
}

#[test]
fn a_popup_covered_double_click_restores_the_previous_disambiguation_popup() {
    let mut scene = GestureScene::new(Target::Ambiguous);
    scene.release(PointerButton::Primary);
    scene.layout_popup();
    scene.map.finish_primary_click();
    let map = scene.map.map().expect("the rendered map");
    let original_candidates = map.disambiguation_candidates;
    let original_position = map.disambiguation_pos;

    let destination = egui::pos2(VIEWPORT.x - 40.0, VIEWPORT.y - 150.0);
    let point_index = CENTRE_FIX + 1;
    let (latitude, longitude) =
        test_util::fix_position(&test_util::a_walking_recording(), point_index);
    let map = scene.map.map_mut().expect("the rendered map");
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT);
    let projector = Projector::new(
        viewport,
        &MercatorProjection,
        &map.map_memory,
        crate::default_map_center(),
    );
    let point_position = projector.project(walkers::lat_lon(latitude, longitude));
    let center =
        projector.unproject(viewport.center().to_vec2() + point_position - destination.to_vec2());
    map.map_memory.center_at(center);
    map.set_controls(MapControls::default());
    scene.target = destination;
    scene.map.move_pointer_to(scene.target);
    scene.map.render_one_more_frame();

    scene.release(PointerButton::Primary);
    scene.layout_popup();
    let point = test_util::point_ref(DataCategory::Tpv, point_index);
    assert_eq!(scene.sticky(), Some(point));
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(
        scene.map.harness.inner.ctx.layer_id_at(scene.target),
        Some(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::unique((super::PINNED_POPUP_ID, point)),
        ))
    );

    scene.release(PointerButton::Primary);
    let map = scene.map.map().expect("the rendered map");
    assert_eq!(map.disambiguation_candidates, original_candidates);
    assert_eq!(map.disambiguation_pos, original_position);
    assert!(scene.map.disambiguation_popup_is_open());
    scene
        .map
        .harness
        .inner
        .get_by_label_contains(test_util::EVENT_MARKER_ON_A_FIX_PATH);
    let expected_zoom = (INITIAL_ZOOM + 1.0).to_bits();
    assert_eq!(scene.zoom().to_bits(), expected_zoom);
    scene.map.finish_primary_click();
    assert_eq!(scene.zoom().to_bits(), expected_zoom);
    assert!(scene.map.disambiguation_popup_is_open());
}

#[test]
fn restoring_a_pin_preserves_the_popup_position_and_current_hover_state() {
    let mut scene = GestureScene::new(Target::Bare);
    let point = test_util::point_ref(DataCategory::Tpv, CENTRE_FIX);
    assert!(scene.map.pin(point));
    let position = egui::pos2(35.0, 55.0);
    scene.map.map_mut().expect("the rendered map").sticky_pos = position;
    scene.layout_popup();
    scene.layout_popup();
    let popup_id = egui::Id::unique((super::PINNED_POPUP_ID, point));
    let original_rect = scene
        .map
        .harness
        .inner
        .ctx
        .memory(|memory| memory.area_rect(popup_id));
    assert!(original_rect.is_some());
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), None);
    let plot_time = chrono::DateTime::UNIX_EPOCH;
    scene.map.draw_state().highlight.plot_hover_time = Some(plot_time);
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), Some(point));
    assert_eq!(
        scene.map.map().expect("the rendered map").sticky_pos,
        position
    );
    assert_eq!(
        scene.map.draw_state().highlight.plot_hover_time,
        Some(plot_time)
    );
    assert_eq!(
        scene
            .map
            .harness
            .inner
            .ctx
            .memory(|memory| memory.area_rect(popup_id)),
        original_rect
    );
}

#[test]
fn an_external_selection_change_cancels_navigation_rollback() {
    let mut scene = GestureScene::new(Target::Fix);
    scene.release(PointerButton::Primary);
    let other = test_util::point_ref(DataCategory::Tpv, CENTRE_FIX - 1);
    assert!(scene.map.pin(other));
    scene.release(PointerButton::Primary);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
    assert_eq!(
        scene.sticky(),
        Some(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX))
    );
}

#[test]
fn navigation_rollback_rechecks_the_original_pins_presence() {
    let mut scene = GestureScene::new(Target::Bare);
    assert!(
        scene
            .map
            .pin(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX))
    );
    scene.release(PointerButton::Primary);
    scene
        .map
        .draw_state()
        .display_mask
        .set_visible(DisplayCategory::TrackPoints, false);
    scene.release(PointerButton::Primary);
    assert_eq!(scene.sticky(), None);
    assert!(scene.zoom() < INITIAL_ZOOM);
}

#[test]
fn immediate_disambiguation_rows_remain_interactive_during_the_double_click_window() {
    let mut scene = GestureScene::new(Target::Ambiguous);
    scene.release(PointerButton::Primary);
    scene.layout_popup();
    scene.target = scene
        .map
        .harness
        .inner
        .get_by_label_contains(test_util::EVENT_MARKER_ON_A_FIX_PATH)
        .rect()
        .center();
    scene
        .map
        .harness
        .inner
        .input_mut()
        .events
        .push(Event::PointerMoved(scene.target));
    scene.release(PointerButton::Primary);
    assert_eq!(
        scene.sticky(),
        Some(test_util::point_ref(DataCategory::EventMarker, 0))
    );
    assert!(!scene.map.disambiguation_popup_is_open());
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

const VIEWPORT: egui::Vec2 = egui::vec2(1600.0, 1200.0);
const INITIAL_ZOOM: f64 = 18.0;
const BARE_MAP_OFFSET: f32 = 200.0;
const RELEASE_INTERVAL: f64 = 0.05;
