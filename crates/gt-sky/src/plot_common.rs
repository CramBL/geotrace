//! Small helpers shared by the per-report sky plot and the whole-track
//! trails plot: satellite labels, hover tooltips, and pointer hit-testing.

use egui::{Align, Layout, Pos2, RichText, Sense, Stroke, Vec2};

use gt_types::GpsTime;
use gt_types::satellites::{Constellation, Prn, Satellite};

/// Width of the tooltip's value column. Values are right-aligned in it.
const TOOLTIP_VALUE_WIDTH_PX: f32 = 74.0;

/// Gap between the tooltip's designator and its fix-state chip.
const TOOLTIP_HEADER_GAP_PX: f32 = 12.0;

/// Diameter of the fix-state chip's glyph, matching the plot's scrub marker.
const CHIP_GLYPH_PX: f32 = 9.0;

/// Stroke width of the hollow (tracked, not in fix) chip glyph.
const CHIP_RING_PX: f32 = 1.5;

/// The `"G05 GPS"` designator: RINEX prefix, zero-padded PRN, constellation
/// name. Single source for both plots' hover labels.
pub(crate) fn satellite_designator(constellation: Constellation, prn: Prn) -> String {
    format!(
        "{}{:02} {}",
        constellation.prn_prefix(),
        prn.value(),
        constellation.display_name()
    )
}

/// The hover tooltip body for one satellite: the designator and fix state on a
/// header line, then the measurements in a label/value table. Shared so a
/// satellite reads the same whether hovered on the per-report plot or on a
/// trail's scrub marker.
///
/// `at` names the report the values came from. The trails plot passes it: its
/// scrubber sits between reports for most of its travel, so the values can be up
/// to one report interval old. The per-report plot leaves it unset.
pub(crate) fn satellite_tooltip(ui: &mut egui::Ui, satellite: &Satellite, at: Option<GpsTime>) {
    ui.horizontal(|ui| {
        let label = satellite_designator(satellite.constellation(), satellite.prn());
        ui.label(RichText::new(label).strong());
        ui.add_space(TOOLTIP_HEADER_GAP_PX);
        fix_chip(ui, satellite.in_fix());
    });
    let degree = |value: Option<f32>| {
        value.map_or_else(
            || gt_ui_theme::EM_DASH.to_owned(),
            |v| format!("{v:.0}{}", gt_ui_theme::DEGREE_SIGN),
        )
    };
    let snr = satellite.snr().map_or_else(
        || gt_ui_theme::EM_DASH.to_owned(),
        |snr| format!("{:.0} dB-Hz", snr.value()),
    );
    // Keyed by satellite, and by the containing `Ui`: a grid's column widths
    // live under its id, so two tooltips sharing one would each keep resizing
    // to the other's measurements and repaint without ever settling.
    let grid_id = ui.id().with((
        "satellite_tooltip",
        satellite.constellation(),
        satellite.prn(),
    ));
    egui::Grid::new(grid_id).num_columns(2).show(ui, |ui| {
        tooltip_row(ui, "Elevation", &degree(satellite.elevation()));
        tooltip_row(ui, "Azimuth", &degree(satellite.azimuth()));
        tooltip_row(ui, "SNR", &snr);
    });
    if let Some(at) = at {
        ui.label(
            RichText::new(format!("at {}", at.utc().format("%H:%M:%S")))
                .weak()
                .small(),
        );
    }
}

fn tooltip_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).weak());
    ui.allocate_ui_with_layout(
        Vec2::new(TOOLTIP_VALUE_WIDTH_PX, ui.spacing().interact_size.y),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.label(RichText::new(value).monospace());
        },
    );
    ui.end_row();
}

/// The fix-state chip: a filled-dot / hollow-ring glyph beside a one-word
/// state.
///
/// The fill matches the plot's marker: filled in the fix, hollow when only
/// tracked. The colour is a status cue (green for in the fix, weak grey for
/// tracked), not the constellation colour.
fn fix_chip(ui: &mut egui::Ui, in_fix: bool) {
    let color = if in_fix {
        gt_ui_theme::SUCCESS_GREEN
    } else {
        ui.visuals().weak_text_color()
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(CHIP_GLYPH_PX), Sense::hover());
    let radius = CHIP_GLYPH_PX / 2.0;
    if in_fix {
        ui.painter().circle_filled(rect.center(), radius, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), radius, Stroke::new(CHIP_RING_PX, color));
    }
    let state = if in_fix { "In fix" } else { "Tracked" };
    ui.label(RichText::new(state).color(color));
}

/// The candidate nearest to `pointer` within `radius`, where `candidates` yields
/// each item with its screen position.
pub(crate) fn nearest_within<'a, T>(
    candidates: impl Iterator<Item = (&'a T, Pos2)>,
    pointer: Pos2,
    radius: f32,
) -> Option<&'a T> {
    candidates
        .map(|(item, pos)| (item, pos.distance(pointer)))
        .filter(|(_, distance)| *distance <= radius)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(item, _)| item)
}

#[cfg(test)]
mod tests {
    use super::nearest_within;

    #[test]
    fn nearest_within_picks_the_closest_inside_the_radius() {
        let a = (1u32, egui::pos2(0.0, 0.0));
        let b = (2u32, egui::pos2(10.0, 0.0));
        let items = [a, b];
        let candidates = || items.iter().map(|(v, p)| (v, *p));

        // Closest of the two, both inside radius.
        assert_eq!(
            nearest_within(candidates(), egui::pos2(6.0, 0.0), 8.0),
            Some(&2)
        );
        assert_eq!(
            nearest_within(candidates(), egui::pos2(3.0, 0.0), 8.0),
            Some(&1)
        );
        // Nothing within the radius.
        assert_eq!(
            nearest_within(candidates(), egui::pos2(20.0, 0.0), 5.0),
            None
        );
    }
}
