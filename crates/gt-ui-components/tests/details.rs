use std::num::NonZeroUsize;

use egui_kittest::Harness;
use gt_ui_components::{DetailRow, DetailsLayout, DetailsTooltip, MetadataView};
use rstest::rstest;

#[derive(Default)]
struct TooltipGeometry {
    popup_rects: Vec<egui::Rect>,
    body_rects: Vec<egui::Rect>,
    sizing_passes: usize,
}

#[rstest]
#[case::normal(1.0, 800.0, 320.0)]
#[case::scaled(1.5, 800.0, 320.0)]
#[case::narrow(1.0, 280.0, 320.0)]
#[case::minimum(1.0, 800.0, 120.0)]
#[case::maximum(1.0, 800.0, 640.0)]
fn tooltip_first_sizing_matches_reopen_with_wrapping_metadata(
    #[case] pixels_per_point: f32,
    #[case] viewport_width: f32,
    #[case] preferred_tooltip_width: f32,
) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(viewport_width, 900.0))
        .with_pixels_per_point(pixels_per_point)
        .build_ui_state(
            |ui, state: &mut TooltipGeometry| {
                let label = ui.label("Recording");
                if let Some(shown) = DetailsTooltip::new(&label).show(|ui| {
                    state.sizing_passes += usize::from(ui.is_sizing_pass());
                    state.body_rects.push(
                        MetadataView {
                            title: Some("A recording with a long title and several words to wrap"),
                            device: Some("A receiver with a descriptive device name"),
                            notes: Some("A long note about the recording, with several observations and a second sentence.\nAnother line of recording notes."),
                            ..MetadataView::default()
                        }
                        .show_ui(ui)
                        .rect,
                    );
                }) {
                    state.popup_rects.push(shown.response.rect);
                }
            },
            TooltipGeometry::default(),
        );
    gt_ui_theme::install_app_style(&harness.ctx);
    harness
        .ctx
        .options_mut(|options| options.max_passes = NonZeroUsize::MIN);
    harness.ctx.all_styles_mut(|style| {
        style.spacing.tooltip_width = preferred_tooltip_width;
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    harness.hover_at(egui::pos2(28.0, 14.0));
    for _ in 0..4 {
        harness.step();
    }
    let cold_body = *harness
        .state()
        .body_rects
        .first()
        .expect("cold tooltip layout");
    let visible_popup = *harness.state().popup_rects.last().expect("visible tooltip");
    assert!(harness.state().sizing_passes > 0);
    assert!(cold_body.width() >= 240.0);
    assert!(cold_body.height() < 240.0);
    assert!(cold_body.width() <= 481.0);
    assert!(visible_popup.width() <= viewport_width + 1.0);

    harness.hover_at(egui::pos2(viewport_width - 10.0, 880.0));
    for _ in 0..4 {
        harness.step();
    }
    harness.state_mut().body_rects.clear();
    harness.state_mut().popup_rects.clear();
    harness.hover_at(egui::pos2(28.0, 14.0));
    for _ in 0..4 {
        harness.step();
    }
    let reopened_body = *harness
        .state()
        .body_rects
        .first()
        .expect("reopened tooltip layout");
    let reopened_popup = *harness
        .state()
        .popup_rects
        .last()
        .expect("reopened tooltip");
    assert!((cold_body.width() - reopened_body.width()).abs() <= 1.0);
    assert!((cold_body.height() - reopened_body.height()).abs() <= 1.0);
    assert!((visible_popup.width() - reopened_popup.width()).abs() <= 1.0);
    assert!((visible_popup.height() - reopened_popup.height()).abs() <= 1.0);
}

#[rstest]
#[case::narrow(240.0)]
#[case::wide(640.0)]
fn details_use_parent_width_during_first_layout(#[case] parent_width: f32) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_ui_state(
            |ui, rect: &mut egui::Rect| {
                ui.set_width(parent_width);
                *rect = DetailsLayout::new(&[
                    DetailRow { caption: "Caption", value: "A sufficiently long value that wraps within the enclosing parent width and remains available to select and copy" },
                    DetailRow { caption: "Second caption", value: "A second value" },
                ]).show_ui(ui).rect;
            },
            egui::Rect::NOTHING,
        );
    let first = *harness.state();
    harness.step();
    let next = *harness.state();
    assert!((first.width() - parent_width).abs() <= 1.0);
    assert!((first.height() - next.height()).abs() <= 1.0);
}
