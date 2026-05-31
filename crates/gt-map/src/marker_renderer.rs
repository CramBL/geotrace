use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_types::{
    CustomMarker, DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile,
    MapHighlight, MarkerIcon, SpatialPoint, TrackDataVisibility, filter,
};
use walkers::{MapMemory, Plugin, Projector};

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
            Some(HighlightScope::Track {
                file_index,
                track_index,
            }) => file_index == point_ref.file_index && track_index == point_ref.track_index,
            Some(HighlightScope::TrackCategory {
                file_index,
                track_index,
                category,
            }) => {
                file_index == point_ref.file_index
                    && track_index == point_ref.track_index
                    && category == DataCategory::CustomMarker
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
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

        for sp in &self.visible_custom {
            let Some(file_vis) = self.visibility.files.get(sp.file_index.0) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            let Some(trip_vis) = file_vis.tracks.get(sp.track_index.0) else {
                continue;
            };
            if !trip_vis.enabled || !trip_vis.custom_markers_visible {
                continue;
            }
            let Some(file) = self.files.get(sp.file_index.0) else {
                continue;
            };
            let Some(track) = file.tracks.get(sp.track_index.0) else {
                continue;
            };
            if !filter::track_passes_filter(&track.metadata, self.filter) {
                continue;
            }
            let Some(marker) = track.custom_markers.get(sp.point_index.0) else {
                continue;
            };
            if !filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                file_index: sp.file_index,
                track_index: sp.track_index,
                category: DataCategory::CustomMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc_x, sp.merc_y);
            let highlighted = self.is_marker_highlighted(point_ref);
            draw_marker_icon(ui, screen_pos, marker, highlighted);
        }

        // Show hover label for the currently hovered custom marker.
        // Suppressed when the sticky popup is already showing this point, or when
        // any popup (e.g. context menu) is open and would be painted underneath.
        if let Some(HighlightScope::Point(r)) = self.highlight.hover
            && r.category == DataCategory::CustomMarker
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && let Some(file) = self.files.get(r.file_index.0)
            && let Some(track) = file.tracks.get(r.track_index.0)
            && let Some(marker) = track.custom_markers.get(r.point_index.0)
        {
            let pos = transform.to_screen(marker.merc_x, marker.merc_y);
            show_marker_hover_label(ui, marker, pos);
        }
    }
}

/// Paint the marker's label directly onto the map canvas, below the icon.
fn show_marker_hover_label(ui: &Ui, marker: &CustomMarker, pos: Pos2) {
    let label_pos = pos + egui::vec2(0.0, 18.0);
    let galley = ui.painter().layout_no_wrap(
        marker.label.clone(),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    let text_origin = egui::pos2(label_pos.x - galley.size().x / 2.0, label_pos.y);
    let text_rect = egui::Rect::from_min_size(text_origin, galley.size());
    ui.painter().rect_filled(
        text_rect.expand(3.0),
        4.0,
        Color32::from_rgba_unmultiplied(20, 20, 20, 220),
    );
    ui.painter().galley(text_origin, galley, Color32::WHITE);
}

const LOG_COLORS: [Color32; 8] = [
    Color32::from_rgb(230, 57, 70),
    Color32::from_rgb(255, 149, 0),
    Color32::from_rgb(255, 190, 11),
    Color32::from_rgb(6, 214, 160),
    Color32::from_rgb(46, 196, 182),
    Color32::from_rgb(131, 56, 236),
    Color32::from_rgb(255, 45, 85),
    Color32::from_rgb(238, 66, 102),
];

fn draw_marker_icon(ui: &Ui, center: Pos2, marker: &CustomMarker, highlighted: bool) {
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
        ui.painter().circle_stroke(
            center,
            14.0,
            Stroke::new(2.0, Color32::from_rgb(100, 200, 255)),
        );
    }
    match marker.icon {
        MarkerIcon::Pin => draw_pin(ui, center, color),
        MarkerIcon::Cross => draw_cross(ui, center, color),
        MarkerIcon::Circle => draw_circle(ui, center, color),
        MarkerIcon::Lightning => draw_lightning(ui, center, color),
        MarkerIcon::Warning => draw_warning(ui, center, color),
        MarkerIcon::Error => draw_error_sign(ui, center, color),
        MarkerIcon::Check => draw_check(ui, center, color),
        MarkerIcon::Log => draw_log_pin(ui, center, color),
        MarkerIcon::Satellite => draw_svg_icon(ui, center, crate::ICON_URI_SATELLITE, 24.0),
        MarkerIcon::SatelliteLost => {
            draw_svg_icon(ui, center, crate::ICON_URI_SATELLITE_LOST, 24.0)
        }
        MarkerIcon::Gear => draw_svg_icon(ui, center, crate::ICON_URI_GEAR, 20.0),
        MarkerIcon::Refresh => draw_svg_icon(ui, center, crate::ICON_URI_REFRESH, 20.0),
        MarkerIcon::Download => draw_svg_icon(ui, center, crate::ICON_URI_DOWNLOAD, 20.0),
        MarkerIcon::Upload => draw_svg_icon(ui, center, crate::ICON_URI_UPLOAD, 20.0),
        MarkerIcon::Wrench => draw_svg_icon(ui, center, crate::ICON_URI_WRENCH, 20.0),
    }
}

fn draw_pin(ui: &Ui, center: Pos2, _color: Color32) {
    let icon_rect = egui::Rect::from_min_max(
        center - egui::vec2(9.0, 24.0),
        center + egui::vec2(9.0, 0.0),
    );
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_PIN,
    )))
    .paint_at(ui, icon_rect);
}

fn draw_cross(ui: &Ui, center: Pos2, _color: Color32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_CROSS,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_circle(ui: &Ui, center: Pos2, _color: Color32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_CIRCLE_MARKER,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_lightning(ui: &Ui, center: Pos2, _color: Color32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_LIGHTNING,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_warning(ui: &Ui, center: Pos2, _color: Color32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_WARNING,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(24.0, 24.0)),
    );
}

fn draw_error_sign(ui: &Ui, center: Pos2, _color: Color32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_ERROR,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_check(ui: &Ui, center: Pos2, _color: Color32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_CHECK,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_svg_icon(ui: &Ui, center: Pos2, uri: &'static str, size: f32) {
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(uri))).paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(size, size)),
    );
}

fn draw_log_pin(ui: &Ui, center: Pos2, color: Color32) {
    let icon_rect = egui::Rect::from_min_max(
        center - egui::vec2(9.0, 24.0),
        center + egui::vec2(9.0, 0.0),
    );
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_LOG_PIN,
    )))
    .tint(color)
    .paint_at(ui, icon_rect);
}
