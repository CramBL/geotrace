//! The warning indicator in the map's top-right corner: a storm glyph whose
//! hover lists the environment metrics that could have disturbed the loaded
//! recordings.

use egui::{Align2, Area, Frame, Id, RichText, Ui};
use egui_phosphor::regular::CLOUD_LIGHTNING as ICON_CLOUD_LIGHTNING;

/// Inset from the map's top-right corner, matching the layer picker's inset
/// from the bottom-right one.
const CORNER_INSET_PX: f32 = 8.0;

/// Show the indicator over the map's top-right corner, which the map's other
/// floating controls leave free.
///
/// Absent while `lines` is empty, matching the interference legend's
/// appearance: it is drawn only while its layer has cells to describe.
pub(crate) fn show_space_weather_warning(ui: &Ui, map_rect: egui::Rect, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    Area::new(Id::new("map_space_weather_warning"))
        .fixed_pos(egui::pos2(
            map_rect.right() - CORNER_INSET_PX,
            map_rect.top() + CORNER_INSET_PX,
        ))
        .pivot(Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(
                    RichText::new(ICON_CLOUD_LIGHTNING)
                        .color(gt_ui_theme::WARNING.resolve(ui.visuals().dark_mode)),
                )
                .on_hover_ui(|ui| {
                    for line in lines {
                        ui.label(line);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use rstest::rstest;

    use super::*;

    fn one_warning() -> Vec<String> {
        vec!["Geomagnetic storm: Hp30 reached 7.667 (G3)".to_owned()]
    }

    /// The glyph is there exactly while a metric reached its disturbance
    /// level, so an untroubled recording leaves the map clean.
    #[rstest]
    #[case::warned(one_warning(), true)]
    #[case::nothing_found(Vec::new(), false)]
    fn the_glyph_shows_exactly_when_a_metric_warns(
        #[case] lines: Vec<String>,
        #[case] expected: bool,
    ) {
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(200.0, 120.0))
            .ui(move |ui| {
                show_space_weather_warning(ui, ui.max_rect(), &lines);
            });
        harness.run();

        assert_eq!(
            harness.inner.query_by_label(ICON_CLOUD_LIGHTNING).is_some(),
            expected
        );
    }
}
