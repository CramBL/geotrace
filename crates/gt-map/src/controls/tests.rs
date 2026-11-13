use egui::{
    Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, TouchDeviceId, TouchId,
    TouchPhase, UiBuilder,
};
use gt_test_utils::TestHarness;
use gt_types::{DataCategory, LoadedFile};
use gt_ui_types::TrackDataVisibility;
use rstest::rstest;
use strum::IntoEnumIterator as _;
use walkers::{MapMemory, MercatorProjection, Position, Projector};

use crate::test_util::{self, CENTRE_FIX, DrawState};
use crate::{DoubleClickBehavior, DragPanButton, MapControls, NavMap, ScrollBehavior, TileAccess};

struct ControlState {
    map: Option<NavMap>,
    draw: DrawState,
    files: Vec<LoadedFile>,
    visibility: TrackDataVisibility,
}

struct ControlScene {
    harness: TestHarness<'static, ControlState>,
    pointer: egui::Pos2,
}

impl ControlScene {
    fn new(controls: MapControls) -> Self {
        let files = test_util::a_walking_recording();
        let center = test_util::fix_position(&files, CENTRE_FIX);
        let visibility = TrackDataVisibility::from_loaded(&files);
        let mut harness = test_util::harness_builder()
            .size(VIEWPORT)
            .step_dt(FRAME_DURATION)
            .ui_state(
                |ui, state: &mut ControlState| {
                    let map = state
                        .map
                        .get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                    let mut map_ui = ui.new_child(UiBuilder::new().max_rect(MAP_RECT));
                    map.draw(
                        &mut map_ui,
                        state.draw.context(&state.files, &state.visibility),
                    );
                },
                ControlState {
                    map: None,
                    draw: DrawState::default(),
                    files,
                    visibility,
                },
            );
        harness.inner.run_steps(4);
        let map = harness.state_mut().map.as_mut().expect("the rendered map");
        if controls != MapControls::default() {
            map.set_controls(controls);
        }
        map.map_memory
            .center_at(walkers::lat_lon(center.0, center.1));
        map.map_memory
            .set_zoom(INITIAL_ZOOM)
            .expect("a supported zoom");
        harness.step();
        let mut scene = Self {
            harness,
            pointer: MAP_RECT.center(),
        };
        scene.frame([Event::PointerMoved(scene.pointer)]);
        scene
    }

    fn frame(&mut self, events: impl IntoIterator<Item = Event>) {
        self.harness.inner.input_mut().events.extend(events);
        self.harness.step();
    }

    fn release(&mut self, button: PointerButton) {
        self.frame([Event::PointerMoved(self.pointer)]);
        self.frame([true, false].map(|pressed| Event::PointerButton {
            pos: self.pointer,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }));
    }

    fn drag(&mut self, button: PointerButton) {
        self.frame([
            Event::PointerMoved(self.pointer),
            Event::PointerButton {
                pos: self.pointer,
                button,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        self.pointer += DRAG_DELTA;
        self.frame([Event::PointerMoved(self.pointer)]);
        self.frame([Event::PointerButton {
            pos: self.pointer,
            button,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn touch(&mut self, phase: TouchPhase, positions: &[egui::Pos2]) {
        let mut events = Vec::new();
        for (index, &pos) in positions.iter().enumerate() {
            events.push(Event::Touch {
                device_id: TOUCH_DEVICE,
                id: TouchId(index as u64),
                phase,
                pos,
                force: None,
            });
            if index == 0 {
                match phase {
                    TouchPhase::Start => {
                        events.push(Event::PointerMoved(pos));
                        events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    TouchPhase::Move => events.push(Event::PointerMoved(pos)),
                    TouchPhase::End | TouchPhase::Cancel => {
                        events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        });
                        events.push(Event::PointerGone);
                    }
                }
            }
        }
        self.frame(events);
    }

    fn map(&self) -> &NavMap {
        self.harness.state().map.as_ref().expect("the rendered map")
    }

    fn map_mut(&mut self) -> &mut NavMap {
        self.harness
            .state_mut()
            .map
            .as_mut()
            .expect("the rendered map")
    }

    fn camera(&self) -> MapMemory {
        self.map().map_memory.clone()
    }

    fn center(&self) -> Position {
        self.map().map_memory.detached().expect("a detached camera")
    }

    fn zoom(&self) -> f64 {
        self.map().map_memory.zoom()
    }
}

#[test]
fn semantic_controls_roundtrip_with_defaults_and_closed_input_policies() {
    let defaults = MapControls {
        scroll: ScrollBehavior::Zoom,
        double_click: DoubleClickBehavior::ZoomAndCenter,
        drag_pan: DragPanButton::Primary,
    };
    assert_eq!(
        serde_json::from_str::<MapControls>("{}").expect("missing fields default"),
        defaults
    );
    assert_eq!(
        serde_json::from_str::<MapControls>(r#"{"scroll":"pan"}"#)
            .expect("a partial configuration"),
        MapControls {
            scroll: ScrollBehavior::Pan,
            ..defaults
        }
    );
    assert_eq!(
        serde_json::to_value(defaults).expect("serialize defaults"),
        serde_json::json!({"scroll": "zoom", "double_click": "zoom_and_center", "drag_pan": "primary"}),
    );
    for scroll in ScrollBehavior::iter() {
        for double_click in DoubleClickBehavior::iter() {
            for drag_pan in DragPanButton::iter() {
                let controls = MapControls {
                    scroll,
                    double_click,
                    drag_pan,
                };
                let json = serde_json::to_string(&controls).expect("serialize controls");
                assert_eq!(
                    serde_json::from_str::<MapControls>(&json).expect("deserialize controls"),
                    controls
                );
            }
        }
    }
    for invalid in [
        r#"{"drag_pan":"secondary"}"#,
        r#"{"scroll":"disabled"}"#,
        r#"{"double_click":"zoom_out"}"#,
    ] {
        serde_json::from_str::<MapControls>(invalid).expect_err("an unsupported map control");
    }
}

struct ZoomChange {
    before: f64,
    after: f64,
}

#[rstest]
#[case::one_level(ZoomChange {before: 12.0, after: 13.0})]
#[case::fractional_ceiling(ZoomChange {before: 25.5, after: 26.0})]
#[case::at_ceiling(ZoomChange {before: 26.0, after: 26.0})]
fn default_double_click_centers_the_clicked_geography_and_saturates_zoom(#[case] zoom: ZoomChange) {
    let mut scene = ControlScene::new(MapControls::default());
    scene
        .map_mut()
        .map_memory
        .set_zoom(zoom.before)
        .expect("a supported zoom");
    scene.pointer += POINTER_OFFSET;
    let camera = scene.camera();
    let projector = Projector::new(MAP_RECT, &MercatorProjection, &camera, scene.center());
    let expected = projector.unproject(scene.pointer.to_vec2());
    scene.release(PointerButton::Primary);
    scene.release(PointerButton::Primary);
    assert!((scene.center().x() - expected.x()).abs() < COORDINATE_EPSILON);
    assert!((scene.center().y() - expected.y()).abs() < COORDINATE_EPSILON);
    assert_eq!(scene.zoom().to_bits(), zoom.after.to_bits());
}

#[test]
fn disabling_double_click_navigation_applies_single_selection_immediately() {
    let mut scene = ControlScene::new(MapControls {
        double_click: DoubleClickBehavior::None,
        ..MapControls::default()
    });
    let center = scene.center();
    scene.release(PointerButton::Primary);
    assert_eq!(
        scene.harness.state().draw.highlight.sticky(),
        Some(test_util::point_ref(DataCategory::Tpv, CENTRE_FIX))
    );
    scene.release(PointerButton::Primary);
    assert_eq!(scene.center(), center);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[rstest]
#[case::changed(ScrollBehavior::Pan, false)]
#[case::unchanged(ScrollBehavior::Zoom, true)]
fn control_updates_preserve_the_camera_and_cancel_pending_selection_only_on_changes(
    #[case] scroll: ScrollBehavior,
    #[case] selected: bool,
) {
    let mut scene = ControlScene::new(MapControls::default());
    scene.release(PointerButton::Primary);
    let center = scene.center();
    let controls = MapControls {
        scroll,
        ..MapControls::default()
    };
    scene.map_mut().set_controls(controls);
    scene.harness.inner.run_steps(8);
    assert_eq!(scene.map().controls(), controls);
    assert_eq!(scene.center(), center);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
    assert_eq!(
        scene.harness.state().draw.highlight.sticky().is_some(),
        selected
    );
}

#[rstest]
#[case::zoom(ScrollBehavior::Zoom, Modifiers::NONE, true)]
#[case::zoom_ctrl(ScrollBehavior::Zoom, Modifiers::CTRL, true)]
#[case::zoom_cmd(ScrollBehavior::Zoom, Modifiers::MAC_CMD, true)]
#[case::pan(ScrollBehavior::Pan, Modifiers::NONE, false)]
#[case::pan_ctrl(ScrollBehavior::Pan, Modifiers::CTRL, true)]
#[case::pan_cmd(ScrollBehavior::Pan, Modifiers::MAC_CMD, true)]
fn scroll_behavior_selects_panning_or_zooming(
    #[case] scroll: ScrollBehavior,
    #[case] modifiers: Modifiers,
    #[case] zoom: bool,
    #[values(MouseWheelSource::Wheel, MouseWheelSource::Trackpad)] source: MouseWheelSource,
) {
    let mut scene = ControlScene::new(MapControls {
        scroll,
        ..MapControls::default()
    });
    let center = scene.center();
    scene.frame([Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        source,
        delta: SCROLL_DELTA,
        phase: TouchPhase::Move,
        modifiers,
    }]);
    scene.harness.inner.run_steps(8);
    assert_eq!(scene.zoom() > INITIAL_ZOOM, zoom);
    if !zoom {
        assert_ne!(scene.center(), center);
        assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
    }
}

#[rstest]
#[case::primary(DragPanButton::Primary, PointerButton::Primary, true)]
#[case::primary_middle(DragPanButton::Primary, PointerButton::Middle, false)]
#[case::primary_secondary(DragPanButton::Primary, PointerButton::Secondary, false)]
#[case::middle_primary(DragPanButton::Middle, PointerButton::Primary, false)]
#[case::middle(DragPanButton::Middle, PointerButton::Middle, true)]
#[case::middle_secondary(DragPanButton::Middle, PointerButton::Secondary, false)]
#[case::both_primary(DragPanButton::PrimaryOrMiddle, PointerButton::Primary, true)]
#[case::both_middle(DragPanButton::PrimaryOrMiddle, PointerButton::Middle, true)]
#[case::both_secondary(DragPanButton::PrimaryOrMiddle, PointerButton::Secondary, false)]
fn mouse_drag_uses_the_selected_button_policy(
    #[case] drag_pan: DragPanButton,
    #[case] button: PointerButton,
    #[case] pans: bool,
    #[values(ScrollBehavior::Pan, ScrollBehavior::Zoom)] scroll: ScrollBehavior,
) {
    let mut scene = ControlScene::new(MapControls {
        scroll,
        drag_pan,
        ..MapControls::default()
    });
    let center = scene.center();
    scene.drag(button);
    assert_eq!(scene.center() != center, pans);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
}

#[test]
fn primary_mouse_drag_stays_disabled_while_another_touch_is_active() {
    let mut scene = ControlScene::new(MapControls {
        drag_pan: DragPanButton::Middle,
        ..MapControls::default()
    });
    let center = scene.center();
    scene.touch(TouchPhase::Start, &[TOUCH_OUTSIDE_MAP]);
    scene.drag(PointerButton::Primary);
    assert_eq!(scene.center(), center);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
    scene.touch(TouchPhase::End, &[TOUCH_OUTSIDE_MAP]);
}

#[rstest]
fn touch_pans_and_pinches_under_every_mouse_policy(
    #[values(ScrollBehavior::Pan, ScrollBehavior::Zoom)] scroll: ScrollBehavior,
    #[values(
        DragPanButton::Primary,
        DragPanButton::Middle,
        DragPanButton::PrimaryOrMiddle
    )]
    drag_pan: DragPanButton,
) {
    let mut scene = ControlScene::new(MapControls {
        scroll,
        drag_pan,
        ..MapControls::default()
    });
    let center = scene.center();
    let start = scene.pointer;
    scene.touch(TouchPhase::Start, &[start]);
    scene.touch(TouchPhase::Move, &[start + DRAG_DELTA]);
    assert_ne!(scene.center(), center);
    assert_eq!(scene.zoom().to_bits(), INITIAL_ZOOM.to_bits());
    scene.touch(TouchPhase::End, &[start + DRAG_DELTA]);
    scene.map_mut().map_memory.center_at(center);
    scene.frame([Event::PointerMoved(scene.pointer)]);
    let separation = egui::vec2(PINCH_SEPARATION, 0.0);
    scene.touch(TouchPhase::Start, &[start - separation, start + separation]);
    scene.touch(
        TouchPhase::Move,
        &[start - separation * 2.0, start + separation * 2.0],
    );
    assert!(scene.zoom() > INITIAL_ZOOM);
    scene.touch(
        TouchPhase::End,
        &[start - separation * 2.0, start + separation * 2.0],
    );
    scene.map_mut().map_memory.center_at(center);
    scene
        .map_mut()
        .map_memory
        .set_zoom(INITIAL_ZOOM)
        .expect("a supported zoom");
    scene.drag(PointerButton::Primary);
    assert_eq!(scene.center() != center, drag_pan != DragPanButton::Middle);
    scene.map_mut().map_memory.center_at(center);
    scene
        .map_mut()
        .map_memory
        .set_zoom(INITIAL_ZOOM)
        .expect("a supported zoom");
    scene.frame([Event::Zoom(TRACKPAD_PINCH_FACTOR)]);
    assert!(scene.zoom() > INITIAL_ZOOM);
}

const VIEWPORT: egui::Vec2 = egui::vec2(1100.0, 800.0);
const MAP_RECT: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(120.0, 70.0), egui::pos2(880.0, 590.0));
const INITIAL_ZOOM: f64 = 12.0;
const FRAME_DURATION: f32 = 0.05;
const POINTER_OFFSET: egui::Vec2 = egui::vec2(120.0, -80.0);
const DRAG_DELTA: egui::Vec2 = egui::vec2(100.0, 40.0);
const SCROLL_DELTA: egui::Vec2 = egui::vec2(0.0, 60.0);
const PINCH_SEPARATION: f32 = 50.0;
const TOUCH_DEVICE: TouchDeviceId = TouchDeviceId(1);
const COORDINATE_EPSILON: f64 = 1.0e-9;

const TOUCH_OUTSIDE_MAP: egui::Pos2 = egui::pos2(30.0, 30.0);

const TRACKPAD_PINCH_FACTOR: f32 = 1.2;
