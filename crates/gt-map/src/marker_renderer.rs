use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_filter::GlobalFilter;
use gt_types::{CustomMarker, DataCategory, LoadedFile, MarkerIcon, SpatialPoint};
use gt_ui_theme::{HIGHLIGHT_BLUE, LOG_COLORS};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight, TrackDataVisibility};
use walkers::{MapMemory, Plugin, Projector};

use crate::icons;
use crate::track_renderer;

pub struct MarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    visible_custom: Vec<SpatialPoint>,
}

impl<'a> MarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        visible_custom: Vec<SpatialPoint>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            visible_custom,
        }
    }

    fn is_marker_highlighted(&self, point_ref: DataPointRef) -> bool {
        if self.highlight.sticky.is_some_and(|r| r == point_ref) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::Point(r)) => r == point_ref,
            Some(HighlightScope::Track(track)) => track == point_ref.track,
            Some(HighlightScope::TrackCategory { track, category }) => {
                track == point_ref.track && category == DataCategory::CustomMarker
            }
            _ => false,
        }
    }
}

impl Plugin for MarkerRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform =
            crate::transform::MercTransform::new(projector, map_memory, ui.max_rect().center());

        for sp in &self.visible_custom {
            let Some(track) = crate::scope::category_in_scope(
                self.files,
                self.visibility,
                self.filter,
                sp.track_ref(),
                DataCategory::CustomMarker,
            ) else {
                continue;
            };
            let Some(marker) = sp.point_index.get(&track.custom_markers) else {
                continue;
            };
            if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                track: sp.track_ref(),
                category: DataCategory::CustomMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc);
            let highlighted = self.is_marker_highlighted(point_ref);
            let fade =
                track_renderer::track_fade_alpha(self.highlight, sp.file_index, sp.track_index);
            draw_marker_icon(ui, screen_pos, marker, highlighted, fade);
        }

        // Show hover label for the hovered custom marker. Uses hover_candidates[2]
        // so the label appears even when a Tpv point is the primary hover.
        // When a TPV tooltip is also visible (hover_candidates[0] is Some), draw
        // the label above the icon to avoid overlapping the TPV tooltip which
        // egui places below/right of the cursor.
        if let Some(r) = self.highlight.hover_candidates[2]
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && !self.highlight.suppress_hover_labels
            && let Some(file) = r.track.fi.get(self.files)
            && let Some(track) = r.track.index.get(&file.tracks)
            && let Some(marker) = r.point_index.get(&track.custom_markers)
        {
            let pos = transform.to_screen(marker.merc);
            let tpv_also_hovered = self.highlight.hover_candidates[0].is_some();
            show_marker_hover_label(ui, marker, pos, tpv_also_hovered);
        }
    }
}

/// Paint the marker's label directly onto the map canvas.
///
/// When `tpv_also_hovered` is true, the TPV tooltip is also visible near the
/// cursor.  To avoid overlap, the label is drawn above the icon instead of
/// below.  If the label is too wide to share screen space alongside the tooltip
/// it is replaced with a "cannot be shown" message that includes the metric.
fn show_marker_hover_label(ui: &Ui, marker: &CustomMarker, pos: Pos2, tpv_also_hovered: bool) {
    const MAX_LABEL_WIDTH: f32 = 120.0;
    const FONT: egui::FontId = egui::FontId::proportional(13.0);

    let (galley, y_offset) = if tpv_also_hovered {
        let label_galley = ui
            .painter()
            .layout_no_wrap(marker.label.clone(), FONT, Color32::WHITE);
        let w = label_galley.size().x;
        if w > MAX_LABEL_WIDTH {
            #[expect(
                clippy::cast_sign_loss,
                reason = "galley width and MAX_LABEL_WIDTH are always non-negative"
            )]
            let msg = format!(
                "label cannot be shown (width {} > {} px)",
                w.round() as u32,
                MAX_LABEL_WIDTH.round() as u32
            );
            let fallback = ui.painter().layout_no_wrap(msg, FONT, Color32::WHITE);
            (fallback, 18.0_f32)
        } else {
            (label_galley, -22.0_f32)
        }
    } else {
        let galley = ui
            .painter()
            .layout_no_wrap(marker.label.clone(), FONT, Color32::WHITE);
        (galley, 18.0_f32)
    };

    let label_pos = pos + egui::vec2(0.0, y_offset);
    let text_origin = if y_offset < 0.0 {
        egui::pos2(
            label_pos.x - galley.size().x / 2.0,
            label_pos.y - galley.size().y,
        )
    } else {
        egui::pos2(label_pos.x - galley.size().x / 2.0, label_pos.y)
    };
    let text_rect = egui::Rect::from_min_size(text_origin, galley.size());
    ui.painter().rect_filled(
        text_rect.expand(3.0),
        4.0,
        Color32::from_rgba_unmultiplied(20, 20, 20, 220),
    );
    ui.painter().galley(text_origin, galley, Color32::WHITE);
}

fn draw_marker_icon(ui: &Ui, center: Pos2, marker: &CustomMarker, highlighted: bool, fade: f32) {
    let color = match marker.icon {
        MarkerIcon::Pin | MarkerIcon::Cross => Color32::from_rgb(219, 68, 55),
        MarkerIcon::Circle
        | MarkerIcon::Satellite
        | MarkerIcon::SatelliteLost
        | MarkerIcon::Upload
        | MarkerIcon::Check
        | MarkerIcon::Download => Color32::from_rgb(66, 133, 244),
        MarkerIcon::Lightning | MarkerIcon::Refresh => Color32::from_rgb(244, 180, 0),
        MarkerIcon::Warning => Color32::from_rgb(255, 153, 0),
        MarkerIcon::Error => Color32::from_rgb(204, 0, 0),
        MarkerIcon::Gear | MarkerIcon::Wrench => Color32::from_rgb(158, 158, 158),
        MarkerIcon::Log => {
            let idx = marker
                .color_group
                .map_or(0, |id| id as usize % LOG_COLORS.len());
            #[expect(
                clippy::indexing_slicing,
                reason = "idx is computed via modulo so always in bounds"
            )]
            LOG_COLORS[idx]
        }
    };
    if highlighted {
        ui.painter()
            .circle_stroke(center, 14.0, Stroke::new(2.0, HIGHLIGHT_BLUE));
    }
    // Apply the hover-fade by reducing the tint alpha so non-focused icons
    // recede against the map tiles instead of merely darkening.
    let white_tint = track_renderer::apply_fade_alpha(Color32::WHITE, fade);
    let color_tint = track_renderer::apply_fade_alpha(color, fade);
    match marker.icon {
        MarkerIcon::Pin => draw_pin(ui, center, white_tint),
        MarkerIcon::Cross => draw_cross(ui, center, white_tint),
        MarkerIcon::Circle => draw_circle(ui, center, white_tint),
        MarkerIcon::Lightning => draw_lightning(ui, center, white_tint),
        MarkerIcon::Warning => draw_warning(ui, center, white_tint),
        MarkerIcon::Error => draw_error_sign(ui, center, white_tint),
        MarkerIcon::Check => draw_check(ui, center, white_tint),
        MarkerIcon::Log => draw_log_pin(ui, center, color_tint),
        MarkerIcon::Satellite => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_SATELLITE,
            icons::ICON_SIZE_LARGE_PX,
            white_tint,
        ),
        MarkerIcon::SatelliteLost => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_SATELLITE_LOST,
            icons::ICON_SIZE_LARGE_PX,
            white_tint,
        ),
        MarkerIcon::Gear => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_GEAR,
            icons::ICON_SIZE_PX,
            white_tint,
        ),
        MarkerIcon::Refresh => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_REFRESH,
            icons::ICON_SIZE_PX,
            white_tint,
        ),
        MarkerIcon::Download => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_DOWNLOAD,
            20.0,
            white_tint,
        ),
        MarkerIcon::Upload => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_UPLOAD,
            icons::ICON_SIZE_PX,
            white_tint,
        ),
        MarkerIcon::Wrench => draw_svg_icon(
            ui,
            center,
            crate::icons::ICON_URI_WRENCH,
            icons::ICON_SIZE_PX,
            white_tint,
        ),
    }
}

fn draw_pin(ui: &Ui, center: Pos2, tint: Color32) {
    let icon_rect = egui::Rect::from_min_max(
        center - egui::vec2(9.0, icons::ICON_SIZE_LARGE_PX),
        center + egui::vec2(9.0, 0.0),
    );
    crate::icons::draw_cached_icon(ui, crate::icons::ICON_URI_PIN, icon_rect, tint);
}

fn draw_cross(ui: &Ui, center: Pos2, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        crate::icons::ICON_URI_CROSS,
        egui::Rect::from_center_size(center, egui::vec2(icons::ICON_SIZE_PX, icons::ICON_SIZE_PX)),
        tint,
    );
}

fn draw_circle(ui: &Ui, center: Pos2, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        crate::icons::ICON_URI_CIRCLE_MARKER,
        egui::Rect::from_center_size(center, egui::vec2(icons::ICON_SIZE_PX, icons::ICON_SIZE_PX)),
        tint,
    );
}

fn draw_lightning(ui: &Ui, center: Pos2, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        crate::icons::ICON_URI_LIGHTNING,
        egui::Rect::from_center_size(center, egui::vec2(icons::ICON_SIZE_PX, icons::ICON_SIZE_PX)),
        tint,
    );
}

fn draw_warning(ui: &Ui, center: Pos2, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        crate::icons::ICON_URI_WARNING,
        egui::Rect::from_center_size(
            center,
            egui::vec2(icons::ICON_SIZE_LARGE_PX, icons::ICON_SIZE_LARGE_PX),
        ),
        tint,
    );
}

fn draw_error_sign(ui: &Ui, center: Pos2, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        crate::icons::ICON_URI_ERROR,
        egui::Rect::from_center_size(center, egui::vec2(icons::ICON_SIZE_PX, icons::ICON_SIZE_PX)),
        tint,
    );
}

fn draw_check(ui: &Ui, center: Pos2, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        crate::icons::ICON_URI_CHECK,
        egui::Rect::from_center_size(center, egui::vec2(icons::ICON_SIZE_PX, icons::ICON_SIZE_PX)),
        tint,
    );
}

fn draw_svg_icon(ui: &Ui, center: Pos2, uri: &'static str, size: f32, tint: Color32) {
    crate::icons::draw_cached_icon(
        ui,
        uri,
        egui::Rect::from_center_size(center, egui::vec2(size, size)),
        tint,
    );
}

fn draw_log_pin(ui: &Ui, center: Pos2, tint: Color32) {
    let icon_rect = egui::Rect::from_min_max(
        center - egui::vec2(9.0, icons::ICON_SIZE_LARGE_PX),
        center + egui::vec2(9.0, 0.0),
    );
    crate::icons::draw_cached_icon(ui, crate::icons::ICON_URI_LOG_PIN, icon_rect, tint);
}
