use std::num::NonZeroUsize;

use egui::ScrollArea;
use egui_kittest::Harness;
use gt_ui_components::{AnchoredWindow, AnchoredWindowPhase, AnchoredWindowSizing, HeldBodyLines};
use rstest::rstest;

struct State {
    shown: bool,
    open: bool,
    rows: usize,
    window_rect: egui::Rect,
    region_rect: egui::Rect,
    button_rect: egui::Rect,
    visible_windows: Vec<egui::Rect>,
    phases: Vec<AnchoredWindowPhase>,
    sizing_passes: usize,
    clicked: bool,
    line_height: f32,
}

fn make_harness(viewport: egui::Vec2, scale: f32) -> Harness<'static, State> {
    Harness::builder()
        .with_size(viewport)
        .with_pixels_per_point(scale)
        .build_ui_state(
            |ui, state: &mut State| {
                ui.ctx()
                    .options_mut(|options| options.max_passes = NonZeroUsize::MIN);
                if !state.shown {
                    return;
                }
                let window = AnchoredWindow {
                    layout_id: egui::Id::new("held-layout"),
                    window_id: egui::Id::new("window-area"),
                    title: "Anchored window".to_owned(),
                    sizing: WINDOW_SIZING,
                };
                let regions = window.regions();
                let mut visible = false;
                let shown = window.show_ui(ui.ctx(), Some(&mut state.open), |ui, phase| {
                    visible = ui.is_visible();
                    state.sizing_passes += usize::from(ui.is_sizing_pass());
                    state.phases.push(phase);
                    let reserved = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                    let body = |ui: &mut egui::Ui| {
                        state.line_height = ui.text_style_height(&egui::TextStyle::Body);
                        state.region_rect = ui
                            .scope(|ui| {
                                regions.freeze_at_open_ui(
                                    ui,
                                    "pending-result",
                                    HeldBodyLines::at_least(2).and_at_most(4),
                                    |ui| {
                                        for row in 0..state.rows {
                                            ui.label(format!("Result {row}"));
                                        }
                                    },
                                );
                            })
                            .response
                            .rect;
                    };
                    match phase {
                        AnchoredWindowPhase::MeasureContent => {
                            ui.scope(body);
                        }
                        AnchoredWindowPhase::HoldHeight => {
                            ScrollArea::vertical()
                                .auto_shrink(false)
                                .min_scrolled_height(0.0)
                                .max_height((ui.available_height() - reserved).max(0.0))
                                .show(ui, body);
                        }
                    }
                    let button = ui.button("Cancel");
                    state.button_rect = button.rect;
                    state.clicked |= button.clicked();
                });
                if let Some(shown) = shown {
                    state.window_rect = shown.response.rect;
                    if visible {
                        state.visible_windows.push(shown.response.rect);
                    }
                }
            },
            State {
                shown: true,
                open: true,
                rows: 0,
                window_rect: egui::Rect::NOTHING,
                region_rect: egui::Rect::NOTHING,
                button_rect: egui::Rect::NOTHING,
                visible_windows: Vec::new(),
                phases: Vec::new(),
                sizing_passes: 0,
                clicked: false,
                line_height: 0.0,
            },
        )
}

#[rstest]
#[case::normal(egui::vec2(1000.0, 800.0), 1.0)]
#[case::scaled_small(egui::vec2(300.0, 240.0), 1.5)]
fn cold_measurement_centers_and_bounds_the_first_visible_window(
    #[case] viewport: egui::Vec2,
    #[case] scale: f32,
) {
    let mut harness = make_harness(viewport, scale);
    harness.run_steps(5);
    let state = harness.state();
    assert!(state.sizing_passes > 0);
    let first = *state.visible_windows.first().expect("first visible window");
    let viewport = harness.ctx.content_rect();
    assert!((first.center() - viewport.center()).length() < 1.0);
    assert!(viewport.contains_rect(first));
    let expected_width = WINDOW_SIZING
        .preferred_width
        .min(viewport.width() * WINDOW_SIZING.maximum_viewport_fraction);
    assert!((first.width() - expected_width).abs() < 1.0);
    assert_eq!(
        state.phases.first(),
        Some(&AnchoredWindowPhase::MeasureContent)
    );
    assert!(first.height() < viewport.height() * WINDOW_SIZING.maximum_viewport_fraction);
    assert_eq!(state.phases.last(), Some(&AnchoredWindowPhase::HoldHeight));
    assert_eq!(state.window_rect, first);
}

#[rstest]
#[case::normal(1.0)]
#[case::scaled(1.5)]
fn frozen_region_growth_preserves_window_controls_and_pointer_hit_testing(#[case] scale: f32) {
    let mut harness = make_harness(egui::vec2(1000.0, 800.0), scale);
    harness.run_steps(5);
    let before = harness.state().window_rect;
    let region = harness.state().region_rect;
    let button = harness.state().button_rect;
    let target = button.center();
    harness.hover_at(target);
    harness.run_steps(2);
    harness.state_mut().rows = 40;
    harness.run_steps(4);
    assert_eq!(harness.state().window_rect, before);
    assert_eq!(harness.state().region_rect, region);
    assert_eq!(harness.state().button_rect, button);
    harness.drag_at(target);
    harness.step();
    harness.drop_at(target);
    harness.run_steps(2);
    assert!(harness.state().clicked);
}

#[rstest]
#[case::skipped(false)]
#[case::closed_flag(true)]
fn reopening_after_a_pass_gap_measures_regions_again(#[case] draw_while_closed: bool) {
    let mut harness = make_harness(egui::vec2(1000.0, 800.0), 1.0);
    harness.run_steps(5);
    let before = harness.state().window_rect;
    let region = harness.state().region_rect;
    harness.state_mut().shown = draw_while_closed;
    harness.state_mut().open = false;
    harness.run_steps(3);
    harness.state_mut().rows = 40;
    harness.state_mut().shown = true;
    harness.state_mut().open = true;
    harness.run_steps(5);
    let state = harness.state();
    assert!(state.region_rect.height() > region.height());
    assert!(state.region_rect.height() <= state.line_height * 4.0 + 1.0);
    assert!(state.window_rect.height() > before.height());
    let held = state.window_rect;
    harness.state_mut().rows = 0;
    harness.run_steps(4);
    assert_eq!(harness.state().window_rect, held);
}

#[test]
fn user_resizing_preserves_the_held_position_during_content_growth() {
    let mut harness = make_harness(egui::vec2(1000.0, 800.0), 1.0);
    harness.run_steps(5);
    let before = harness.state().window_rect;
    let corner = before.max - egui::vec2(2.0, 2.0);
    let moved = corner + egui::vec2(60.0, 40.0);
    harness.hover_at(corner);
    harness.step();
    harness.drag_at(corner);
    harness.step();
    harness.hover_at(moved);
    harness.step();
    harness.drop_at(moved);
    harness.run_steps(4);
    let resized = harness.state().window_rect;
    assert!(resized.width() > before.width());
    assert!(resized.height() > before.height());
    assert_eq!(resized.min, before.min);
    harness.state_mut().rows = 40;
    harness.run_steps(4);
    assert_eq!(harness.state().window_rect, resized);
}

const WINDOW_SIZING: AnchoredWindowSizing = AnchoredWindowSizing {
    preferred_width: 420.0,
    maximum_viewport_fraction: 0.9,
};
