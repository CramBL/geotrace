use egui_kittest::Harness;
use gt_ui_components::{FractionalSection, FractionalSectionSizing};
use rstest::rstest;

struct State {
    preferred_fraction: f32,
    detached: bool,
    section_rect: egui::Rect,
    region_rect: egui::Rect,
    divider_rect: egui::Rect,
}

fn make_harness() -> Harness<'static, State> {
    Harness::builder()
        .with_size(egui::vec2(300.0, 600.0))
        .build_ui_state(
            |ui, state: &mut State| {
                let shown = FractionalSection {
                    id: egui::Id::new(("section", state.detached)),
                    sizing: FractionalSectionSizing {
                        preferred_fraction: state.preferred_fraction,
                        minimum_height: 80.0,
                        maximum_fraction: 0.75,
                    },
                    frame: egui::Frame::side_top_panel(ui.style()),
                }
                .show_ui(ui, |ui| {
                    ui.label("Section");
                });
                state.section_rect = shown.response.rect;
                state.region_rect = shown.region_rect;
                state.divider_rect = shown.divider_response.rect;
                if let Some(fraction) = shown.changed_fraction {
                    state.preferred_fraction = fraction;
                }
                ui.label("Sibling");
            },
            State {
                preferred_fraction: 0.5,
                detached: false,
                section_rect: egui::Rect::NOTHING,
                region_rect: egui::Rect::NOTHING,
                divider_rect: egui::Rect::NOTHING,
            },
        )
}

#[rstest]
#[case::normal(1.0)]
#[case::scaled(1.5)]
fn parent_resize_and_temporary_minimum_preserve_the_preferred_fraction(
    #[case] pixels_per_point: f32,
) {
    let mut harness = make_harness();
    harness.set_pixels_per_point(pixels_per_point);
    for height in [600.0, 160.0, 48.0, 16.0, 900.0, 600.0] {
        harness.set_size(egui::vec2(300.0, height));
        harness.run();
        let state = harness.state();
        let region_height = state.region_rect.height();
        let minimum = 80.0_f32.min(region_height);
        assert_eq!((state.preferred_fraction).to_bits(), 0.5_f32.to_bits());
        assert!((state.section_rect.height() - (region_height * 0.5).max(minimum)).abs() < 1.0);
        assert!(state.region_rect.contains_rect(state.section_rect));
    }
}

#[test]
fn temporary_maximum_preserves_the_preferred_fraction_without_pointer_movement() {
    let mut harness = make_harness();
    harness.state_mut().preferred_fraction = 1.0;
    harness.run();
    let divider = harness.state().divider_rect.center();
    harness.hover_at(divider);
    harness.drag_at(divider);
    harness.step();
    harness.drop_at(divider);
    harness.run();
    assert_eq!(
        (harness.state().preferred_fraction).to_bits(),
        1.0_f32.to_bits()
    );
    assert!(
        (harness.state().section_rect.height() / harness.state().region_rect.height() - 0.75).abs()
            < 0.002
    );
}

#[test]
fn divider_movement_updates_fraction_and_surface_ids_isolate_drag_state() {
    let mut harness = make_harness();
    harness.run();
    let divider = harness.state().divider_rect.center();
    harness.hover_at(divider);
    harness.drag_at(divider);
    harness.step();
    harness.hover_at(divider + egui::vec2(0.0, 60.0));
    harness.step();
    let changed = harness.state().preferred_fraction;
    assert!((changed - (0.5 + 60.0 / harness.state().region_rect.height())).abs() < 0.002);
    harness.state_mut().detached = true;
    harness.hover_at(divider + egui::vec2(0.0, 120.0));
    harness.step();
    assert_eq!(
        (harness.state().preferred_fraction).to_bits(),
        changed.to_bits()
    );
    harness.drop_at(divider);
    harness.run();
    harness.state_mut().detached = false;
    harness.run();
    assert_eq!(
        (harness.state().preferred_fraction).to_bits(),
        changed.to_bits()
    );
}

#[rstest]
#[case::minimum(-400.0, true)]
#[case::maximum(400.0, true)]
#[case::release_movement(60.0, false)]
fn divider_drag_uses_signed_height_limits_and_final_release_movement(
    #[case] delta_y: f32,
    #[case] move_before_release: bool,
) {
    let mut harness = make_harness();
    harness.run();
    let divider = harness.state().divider_rect.center();
    let region_height = harness.state().region_rect.height();
    harness.hover_at(divider);
    harness.step();
    harness.drag_at(divider);
    harness.step();
    if move_before_release {
        harness.hover_at(divider + egui::vec2(0.0, delta_y));
        harness.step();
    }
    harness.drop_at(divider + egui::vec2(0.0, delta_y));
    harness.run();
    let expected_height = (region_height * 0.5 + delta_y).clamp(80.0, region_height * 0.75);
    assert!((harness.state().preferred_fraction - expected_height / region_height).abs() < 0.002);
    assert!((harness.state().section_rect.height() - expected_height).abs() < 1.0);
}
