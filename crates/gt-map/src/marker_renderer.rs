use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_types::{
    CustomMarker, DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile,
    MapHighlight, MarkerIcon, MercBounds, TripDataVisibility,
};
use std::cell::Cell;
use std::rc::Rc;
use walkers::{MapMemory, Plugin, Projector};

pub struct MarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
}

impl<'a> MarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TripDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            hover_out,
        }
    }

    fn is_marker_highlighted(&self, point_ref: DataPointRef) -> bool {
        if self.highlight.sticky.is_some_and(|r| r == point_ref) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::Point(r)) => r == point_ref,
            Some(HighlightScope::Trip {
                file_index,
                trip_index,
            }) => file_index == point_ref.file_index && trip_index == point_ref.trip_index,
            Some(HighlightScope::TripCategory {
                file_index,
                trip_index,
                category,
            }) => {
                file_index == point_ref.file_index
                    && trip_index == point_ref.trip_index
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
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let view_rect = ui.max_rect().expand(20.0);
        let mut local_closest: Option<(DataPointRef, Pos2, f32)> = None;
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());
        let vp_bounds = MercBounds {
            x_min: transform.merc_x_from_screen(view_rect.min.x),
            x_max: transform.merc_x_from_screen(view_rect.max.x),
            y_min: transform.merc_y_from_screen(view_rect.min.y),
            y_max: transform.merc_y_from_screen(view_rect.max.y),
        };

        crate::marker_iter::for_each_visible_map_point(
            self.files,
            self.visibility,
            self.filter,
            &self.hover_out,
            hover_pos,
            &transform,
            vp_bounds,
            |trip, trip_vis| {
                trip_vis
                    .custom_markers_visible
                    .then_some((DataCategory::CustomMarker, trip.custom_markers.as_slice()))
            },
            |point_ref, screen_pos, marker| {
                if let Some(mouse) = hover_pos {
                    // Use squared distance to avoid sqrt; threshold is 20² = 400.
                    let dist_sq = screen_pos.distance_sq(mouse);
                    if dist_sq < 400.0 && local_closest.is_none_or(|(_, _, d)| dist_sq < d) {
                        local_closest = Some((point_ref, screen_pos, dist_sq));
                    }
                }
                let highlighted = self.is_marker_highlighted(point_ref);
                draw_marker_icon(ui, screen_pos, marker, highlighted);
            },
        );

        if let Some((point_ref, pos, _)) = local_closest {
            show_marker_hover_label(ui, self.files, point_ref, pos);
        }
    }
}

/// Paint the marker's label directly onto the map canvas, below the icon.
///
/// Using direct canvas painting (instead of `on_hover_text`) means the label
/// is always visible when the marker is hovered, even when a TPV point is
/// nearby and showing its own egui tooltip — the two are independent layers.
fn show_marker_hover_label(ui: &Ui, files: &[LoadedFile], point_ref: DataPointRef, pos: Pos2) {
    let Some(file) = files.get(point_ref.file_index.0) else {
        return;
    };
    let Some(trip) = file.trips.get(point_ref.trip_index.0) else {
        return;
    };
    let Some(marker) = trip.custom_markers.get(point_ref.point_index.0) else {
        return;
    };

    // Place the label below the marker icon so it does not overlap the
    // TPV tooltip (which egui positions near the cursor).
    let label_pos = pos + egui::vec2(0.0, 18.0);
    let galley = ui.painter().layout_no_wrap(
        marker.label.clone(),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    // Centre the text horizontally under the icon.
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
    // Color (#DB4437) and white stroke are baked into the SVG.
    // The icon is 18×24 logical pixels with the pin tip at the bottom centre,
    // so the rect spans 9px left/right of `center` and 24px above it.
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
    // Color (#0F9D58) is baked into the SVG.
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_CROSS,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_circle(ui: &Ui, center: Pos2, _color: Color32) {
    // Color (#4285F4) and white stroke are baked into the SVG.
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_CIRCLE_MARKER,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_lightning(ui: &Ui, center: Pos2, _color: Color32) {
    // Color (#F4B400) and white stroke are baked into the SVG; no tint needed.
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_LIGHTNING,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_warning(ui: &Ui, center: Pos2, _color: Color32) {
    // Color (#FF9900), white stroke, and exclamation mark are all baked into the SVG.
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_WARNING,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(24.0, 24.0)),
    );
}

fn draw_error_sign(ui: &Ui, center: Pos2, _color: Color32) {
    // Color (#CC0000), white stroke, and minus bar are all baked into the SVG.
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(
        crate::ICON_URI_ERROR,
    )))
    .paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
    );
}

fn draw_check(ui: &Ui, center: Pos2, _color: Color32) {
    // Color (#0F9D58) is baked into the SVG.
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
    // White SVG tinted to the log-group color at render time.
    // The icon is 18×24 logical pixels with the pin tip at the bottom centre,
    // so the rect spans 9px left/right of `center` and 24px above it.
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
