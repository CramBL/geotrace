use gt_types::FixStats;
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight};

use crate::tree::CheckState;

/// Caret icon for an expand/collapse toggle.
pub fn expand_arrow(expanded: bool) -> &'static str {
    if expanded {
        egui_phosphor::regular::CARET_DOWN
    } else {
        egui_phosphor::regular::CARET_RIGHT
    }
}

/// Frameless button showing On/Off/Mixed state with Phosphor icons.
///
/// Sized to match egui's standard checkbox: icon at `icon_width`, total hit area `interact_size.y²`.
/// Width of the tri-state checkbox column, for padding a checkbox-less row so it
/// aligns with the checkboxed sections. Single source of truth for [`tri_checkbox`].
pub fn checkbox_width(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y + 4.0
}

pub fn tri_checkbox(ui: &mut egui::Ui, state: CheckState) -> egui::Response {
    let icon = match state {
        CheckState::On => egui_phosphor::regular::CHECK_SQUARE,
        CheckState::Off => egui_phosphor::regular::SQUARE,
        CheckState::Mixed => egui_phosphor::regular::MINUS_SQUARE,
    };
    let icon_size = ui.spacing().icon_width + 4.0;
    let side = checkbox_width(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(icon).size(icon_size))
            .frame(false)
            .min_size(egui::vec2(side, side)),
    )
}

/// Paints a translucent highlight behind `rect` to mark map-hover correspondence.
pub fn paint_map_hover_bg(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let bg_layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("map_hover_bg"));
    let painter = ui
        .ctx()
        .layer_painter(bg_layer)
        .with_clip_rect(ui.clip_rect());
    painter.rect_filled(rect.expand2(egui::vec2(2.0, 1.0)), 3.0, color);
}

/// Renders the colour-coded fix-percentage label followed by the remaining
/// tooltip details (time without fix, loss count, max gap).
///
/// Disables egui's automatic item spacing so the details' own `  ·  ` joiner
/// (or empty string at 100% fix) renders without an extra gap.
pub fn fix_stats_tooltip_row(ui: &mut egui::Ui, stats: FixStats) {
    let pct_color = gt_ui_theme::fix_quality_color(gt_fmt::fix_percentage(stats));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.colored_label(pct_color, gt_fmt::format_fix_percentage(stats));
        ui.label(gt_fmt::format_fix_tooltip_details(stats));
    });
}

/// A selectable, sticky, double-click-to-focus row for a single data point.
///
/// Handles hover highlight, click-to-pin, and double-click map centering
/// uniformly across all point-list categories (TPV, satellite, markers).
pub fn point_item_row(
    ui: &mut egui::Ui,
    point_ref: DataPointRef,
    label: impl Into<egui::WidgetText>,
    lat_lon: (f64, f64),
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    let is_sticky = highlight.sticky.is_some_and(|r| r == point_ref);
    let response = ui.selectable_label(is_sticky, label);
    if response.hovered() {
        highlight.hover = Some(HighlightScope::Point(point_ref));
    }
    apply_point_click(
        ui,
        &response,
        point_ref,
        lat_lon,
        highlight,
        map_center_request,
        popup_pos_request,
    );
}

/// The click reactions every point row shares (here and in the query window's
/// match tables): clicking pins the point's map popup right of the containing
/// panel at the row's height, clicking again unpins, double-clicking centers
/// the map on the point.
pub fn apply_point_click(
    ui: &egui::Ui,
    response: &egui::Response,
    point_ref: DataPointRef,
    lat_lon: (f64, f64),
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    if response.clicked() {
        if highlight.sticky == Some(point_ref) {
            highlight.sticky = None;
        } else {
            highlight.sticky = Some(point_ref);
            *popup_pos_request = Some(egui::pos2(ui.clip_rect().max.x + 8.0, response.rect.min.y));
        }
    }
    if response.double_clicked() {
        *map_center_request = Some(lat_lon);
    }
}
