use std::num::NonZeroUsize;

use egui_kittest::{Harness, kittest::Queryable as _};
use gt_ui_components::{ToolWindow, ToolWindowSizing};
use rstest::rstest;

struct State {
    open: bool,
    pending_rect: Option<egui::Rect>,
    window_rect: egui::Rect,
    body_rect: egui::Rect,
    viewport: egui::Rect,
    visible_rects: Vec<egui::Rect>,
    sizing_passes: usize,
    fill_height: bool,
    title: &'static str,
}

fn make_harness(viewport_size: egui::Vec2, scale: f32) -> Harness<'static, State> {
    Harness::builder()
        .with_size(viewport_size)
        .with_pixels_per_point(scale)
        .build_ui_state(
            |ui, state: &mut State| {
                ui.ctx()
                    .options_mut(|options| options.max_passes = NonZeroUsize::MIN);
                state.viewport = ui.ctx().content_rect().shrink(10.0);
                let mut visible = false;
                let shown = ToolWindow {
                    id: egui::Id::new(WINDOW_ID),
                    title: state.title,
                    viewport: state.viewport,
                    sizing: WINDOW_SIZING,
                    movable: true,
                    resizable: true,
                }
                .show_ui(
                    ui.ctx(),
                    &mut state.open,
                    Some(&mut state.pending_rect),
                    |ui| {
                        state.body_rect = ui.max_rect();
                        state.sizing_passes += usize::from(ui.is_sizing_pass());
                        visible = ui.is_visible();
                        if state.fill_height {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for _ in 0..100 {
                                        ui.label("Window content");
                                    }
                                });
                        } else {
                            ui.label("Short content");
                        }
                    },
                );
                if let Some(shown) = shown {
                    state.window_rect = shown.response.rect;
                    if visible {
                        state.visible_rects.push(shown.response.rect);
                    }
                }
            },
            State {
                open: true,
                pending_rect: None,
                window_rect: egui::Rect::NOTHING,
                body_rect: egui::Rect::NOTHING,
                viewport: egui::Rect::NOTHING,
                visible_rects: Vec::new(),
                sizing_passes: 0,
                fill_height: false,
                title: "Tool",
            },
        )
}

#[rstest]
#[case::normal(1.0)]
#[case::scaled(1.5)]
fn cold_measurement_centers_the_first_visible_content_rectangle(#[case] scale: f32) {
    let mut harness = make_harness(egui::vec2(1000.0, 800.0), scale);
    harness.run();
    let state = harness.state();
    assert!(state.sizing_passes > 0);
    let first = state.visible_rects.first().expect("first visible frame");
    assert!((first.center() - state.viewport.center()).length() < 1.0);
    assert!(first.height() < state.viewport.height() * WINDOW_SIZING.preferred_fraction.y);
    assert!(
        (first.width() - state.viewport.width() * WINDOW_SIZING.preferred_fraction.x).abs() < 1.0
    );
    assert!(state.viewport.contains_rect(*first));
}

#[rstest]
#[case::normal(1.0)]
#[case::scaled(1.5)]
fn user_move_resize_and_reopen_preserve_egui_geometry(#[case] scale: f32) {
    let mut harness = make_harness(egui::vec2(1000.0, 800.0), scale);
    harness.state_mut().fill_height = true;
    harness.run();
    let initial = harness.state().window_rect;
    let titlebar = initial.left_top() + egui::vec2(80.0, 12.0);
    harness.hover_at(titlebar);
    harness.drag_at(titlebar);
    harness.step();
    harness.hover_at(titlebar + egui::vec2(60.0, -40.0));
    harness.step();
    harness.drop_at(titlebar + egui::vec2(60.0, -40.0));
    harness.run();
    let moved = harness.state().window_rect;
    assert!((moved.min - initial.min - egui::vec2(60.0, -40.0)).length() < 2.0);
    let corner = moved.right_bottom() - egui::vec2(1.0, 1.0);
    harness.hover_at(corner);
    harness.drag_at(corner);
    harness.step();
    harness.hover_at(corner - egui::vec2(50.0, 60.0));
    harness.step();
    harness.drop_at(corner - egui::vec2(50.0, 60.0));
    harness.run();
    let resized = harness.state().window_rect;
    assert!((resized.size() - moved.size() + egui::vec2(50.0, 60.0)).length() < 2.0);
    harness.state_mut().open = false;
    harness.step();
    harness.state_mut().title = "Changed title";
    harness.state_mut().open = true;
    harness.run();
    assert!((harness.state().window_rect.min - resized.min).length() < 1.0);
    assert!((harness.state().window_rect.size() - resized.size()).length() < 1.0);
}

#[rstest]
#[case::small(egui::vec2(300.0, 240.0), 1.0)]
#[case::scaled_small(egui::vec2(300.0, 240.0), 1.5)]
#[case::wide_short(egui::vec2(1000.0, 240.0), 2.0)]
fn viewport_constraints_bound_cold_and_resized_windows(
    #[case] size: egui::Vec2,
    #[case] scale: f32,
) {
    let mut harness = make_harness(size, scale);
    harness.state_mut().fill_height = true;
    harness.run();
    let state = harness.state();
    assert!(state.viewport.contains_rect(state.window_rect));
    assert!(
        state.window_rect.width()
            <= state.viewport.width() * WINDOW_SIZING.maximum_fraction.x + 1.0
    );
    assert!(
        state.window_rect.height()
            <= state.viewport.height() * WINDOW_SIZING.maximum_fraction.y + 1.0
    );
    harness.set_size(egui::vec2(1000.0, 800.0));
    harness.run();
    harness.set_size(size);
    harness.run();
    assert!(
        harness
            .state()
            .viewport
            .contains_rect(harness.state().window_rect)
    );
}

#[test]
fn pending_rectangle_applies_once_to_current_geometry_and_waits_while_closed() {
    let mut harness = make_harness(egui::vec2(1000.0, 800.0), 1.0);
    harness.state_mut().fill_height = true;
    harness.run();
    let body_offset = harness.state().body_rect.min - harness.state().window_rect.min;
    let override_rect = egui::Rect::from_min_size(egui::pos2(70.0, 90.0), egui::vec2(450.0, 300.0));
    harness.state_mut().open = false;
    harness.state_mut().pending_rect = Some(override_rect);
    harness.step();
    assert_eq!(harness.state().pending_rect, Some(override_rect));
    harness.state_mut().open = true;
    harness.step();
    assert!(
        harness
            .query_all_by_label("Window content")
            .next()
            .is_some()
    );
    assert!((harness.state().window_rect.min - override_rect.min).length() < 1.0);
    assert!((harness.state().window_rect.size() - override_rect.size()).length() < 1.0);
    assert!((harness.state().body_rect.min - override_rect.min - body_offset).length() < 1.0);
    harness.run();
    assert_eq!(harness.state().pending_rect, None);
    assert!((harness.state().window_rect.min - override_rect.min).length() < 1.0);
    assert!((harness.state().window_rect.size() - override_rect.size()).length() < 1.0);
    let corner = harness.state().window_rect.right_bottom() - egui::vec2(1.0, 1.0);
    harness.hover_at(corner);
    harness.drag_at(corner);
    harness.step();
    harness.hover_at(corner + egui::vec2(40.0, 50.0));
    harness.step();
    harness.drop_at(corner + egui::vec2(40.0, 50.0));
    harness.run();
    assert!(
        (harness.state().window_rect.size() - override_rect.size() - egui::vec2(40.0, 50.0))
            .length()
            < 2.0
    );
    harness.state_mut().pending_rect = Some(egui::Rect::from_min_size(
        egui::pos2(-1000.0, 2000.0),
        egui::vec2(2000.0, 2000.0),
    ));
    harness.step();
    harness.run();
    assert!(
        harness
            .state()
            .viewport
            .contains_rect(harness.state().window_rect)
    );
}

const WINDOW_ID: &str = "geometry-test-window";
const WINDOW_SIZING: ToolWindowSizing = ToolWindowSizing {
    preferred_fraction: egui::vec2(0.7, 0.6),
    minimum_size: egui::vec2(320.0, 120.0),
    maximum_fraction: egui::vec2(0.9, 0.9),
};
