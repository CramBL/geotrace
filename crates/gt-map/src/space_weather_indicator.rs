//! The warning indicator in the map's top-right corner: a storm glyph whose
//! hover lists the environment metrics that could have disturbed the loaded
//! recordings, or explains the indicator while none of them warns.

use egui::{Align2, Area, Frame, Id, RichText, Ui};
use egui_phosphor::regular::CLOUD_LIGHTNING as ICON_CLOUD_LIGHTNING;

/// Inset from the map's top-right corner, matching the layer picker's inset
/// from the bottom-right one.
const CORNER_INSET_PX: f32 = 8.0;

const IDLE_HOVER_TEXT: &str = "No environment warnings. Warnings appear here when archived \
                               interference, geomagnetic or solar flare data crosses a warning \
                               level during a loaded recording.";

/// Show the indicator over the map's top-right corner, which the map's other
/// floating controls leave free.
///
/// Drawn whether or not `lines` holds a warning, so the glyph can be found
/// before one is raised. With no line it renders in the weak text colour and
/// hovers with [`IDLE_HOVER_TEXT`] in place of the list.
pub(crate) fn show_space_weather_warning(ui: &Ui, map_rect: egui::Rect, lines: &[String]) {
    Area::new(Id::new("map_space_weather_warning"))
        .fixed_pos(egui::pos2(
            map_rect.right() - CORNER_INSET_PX,
            map_rect.top() + CORNER_INSET_PX,
        ))
        .pivot(Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| match lines {
                [] => {
                    ui.label(
                        RichText::new(ICON_CLOUD_LIGHTNING).color(ui.visuals().weak_text_color()),
                    )
                    .on_hover_text(IDLE_HOVER_TEXT);
                }
                lines => {
                    ui.label(
                        RichText::new(ICON_CLOUD_LIGHTNING)
                            .color(gt_ui_theme::WARNING.resolve(ui.visuals().dark_mode)),
                    )
                    .on_hover_ui(|ui| {
                        for line in lines {
                            ui.label(line);
                        }
                    });
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{By, Queryable as _};
    use gt_test_utils::HarnessInteraction as _;
    use rstest::rstest;

    use super::*;

    /// Frames the pointer holds still for, which covers egui's tooltip delay.
    const TOOLTIP_SETTLE_FRAMES: usize = 60;

    fn one_warning() -> Vec<String> {
        vec!["Geomagnetic storm: Hp30 reached 7.667 (G3)".to_owned()]
    }

    /// The glyph is on the map either way. What a metric reaching its
    /// disturbance level changes is the hover behind it.
    #[rstest]
    #[case::warned(one_warning(), "Geomagnetic storm: Hp30 reached 7.667 (G3)")]
    #[case::idle(Vec::new(), IDLE_HOVER_TEXT)]
    fn the_hover_lists_the_warnings_or_explains_the_idle_glyph(
        #[case] lines: Vec<String>,
        #[case] expected_hover: &str,
    ) {
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(400.0, 200.0))
            .ui(move |ui| {
                show_space_weather_warning(ui, ui.max_rect(), &lines);
            });
        harness.run();

        assert!(harness.inner.query_by_label(ICON_CLOUD_LIGHTNING).is_some());
        harness
            .inner
            .hover_and_settle(By::new().label(ICON_CLOUD_LIGHTNING), TOOLTIP_SETTLE_FRAMES);
        assert!(harness.inner.query_by_label(expected_hover).is_some());
    }
}
