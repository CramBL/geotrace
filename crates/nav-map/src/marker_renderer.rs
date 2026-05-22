use egui::{Color32, Pos2, Response, Stroke, Ui};
use nav_types::{
    CustomMarker, DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile,
    MapHighlight, MarkerIcon, TripDataVisibility, point_passes_time_filter, trip_passes_filter,
};
use std::cell::Cell;
use std::rc::Rc;
use uom::si::angle::degree;
use walkers::{MapMemory, Plugin, Position, Projector};

use crate::generated_marker_renderer::update_hover_candidate;

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
        _map_memory: &MapMemory,
    ) {
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let view_rect = ui.max_rect().expand(20.0);
        let mut local_closest: Option<(DataPointRef, Pos2, f32)> = None;

        for (fi, file) in self.files.iter().enumerate() {
            let Some(file_vis) = self.visibility.files.get(fi) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            for (ti, trip) in file.trips.iter().enumerate() {
                let Some(trip_vis) = file_vis.trips.get(ti) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.custom_markers_visible {
                    continue;
                }
                if !trip_passes_filter(&trip.metadata, self.filter) {
                    continue;
                }
                for (pi, marker) in trip.custom_markers.iter().enumerate() {
                    if !point_passes_time_filter(marker.time, self.filter) {
                        continue;
                    }
                    let pos = Position::new(marker.lon.get::<degree>(), marker.lat.get::<degree>());
                    let screen_pos = projector.project(pos).to_pos2();
                    let point_ref = DataPointRef {
                        file_index: fi,
                        trip_index: ti,
                        category: DataCategory::CustomMarker,
                        point_index: pi,
                    };
                    update_hover_candidate(&self.hover_out, screen_pos, hover_pos, point_ref);
                    if let Some(mouse) = hover_pos {
                        let dist = screen_pos.distance(mouse);
                        if dist < 20.0 && local_closest.is_none_or(|(_, _, d)| dist < d) {
                            local_closest = Some((point_ref, screen_pos, dist));
                        }
                    }
                    if !view_rect.contains(screen_pos) {
                        continue;
                    }
                    let highlighted = self.is_marker_highlighted(point_ref);
                    draw_marker_icon(ui, screen_pos, marker, highlighted);
                }
            }
        }

        if let Some((point_ref, pos, _)) = local_closest {
            show_marker_tooltip(ui, self.files, point_ref, pos);
        }
    }
}

fn show_marker_tooltip(ui: &Ui, files: &[LoadedFile], point_ref: DataPointRef, pos: Pos2) {
    let Some(file) = files.get(point_ref.file_index) else {
        return;
    };
    let Some(trip) = file.trips.get(point_ref.trip_index) else {
        return;
    };
    let Some(marker) = trip.custom_markers.get(point_ref.point_index) else {
        return;
    };
    let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
    let response = ui.interact(
        hit_rect,
        ui.id().with("marker_hover").with(point_ref.point_index),
        egui::Sense::hover(),
    );
    response.on_hover_text(&marker.label);
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
        MarkerIcon::Pin => Color32::from_rgb(219, 68, 55),
        MarkerIcon::Cross | MarkerIcon::Check => Color32::from_rgb(15, 157, 88),
        MarkerIcon::Circle => Color32::from_rgb(66, 133, 244),
        MarkerIcon::Lightning => Color32::from_rgb(244, 180, 0),
        MarkerIcon::Warning => Color32::from_rgb(255, 153, 0),
        MarkerIcon::Error => Color32::from_rgb(204, 0, 0),
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
    }
}

fn draw_pin(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let size = 12.0;
    let pin_top = center - egui::vec2(0.0, size);
    painter.circle_filled(pin_top, size * 0.6, color);
    painter.circle_stroke(pin_top, size * 0.6, Stroke::new(1.0, Color32::WHITE));
    painter.line_segment([pin_top, center], Stroke::new(2.0, color));
}

fn draw_cross(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let size = 8.0;
    let stroke = Stroke::new(2.5, color);
    painter.line_segment(
        [
            center - egui::vec2(size, size),
            center + egui::vec2(size, size),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center - egui::vec2(size, -size),
            center + egui::vec2(size, -size),
        ],
        stroke,
    );
}

fn draw_circle(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    painter.circle_filled(center, 8.0, color);
    painter.circle_stroke(center, 8.0, Stroke::new(1.5, Color32::WHITE));
}

fn draw_lightning(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let s = 10.0;
    let points = vec![
        center + egui::vec2(s * 0.2, -s),
        center + egui::vec2(-s * 0.4, s * 0.1),
        center + egui::vec2(s * 0.2, s * 0.1),
        center + egui::vec2(-s * 0.2, s),
        center + egui::vec2(s * 0.4, -s * 0.1),
        center + egui::vec2(-s * 0.2, -s * 0.1),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        Stroke::new(1.0, Color32::WHITE),
    ));
}

fn draw_warning(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let s = 12.0;
    let points = vec![
        center + egui::vec2(0.0, -s),
        center + egui::vec2(-s, s),
        center + egui::vec2(s, s),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        Stroke::new(1.5, Color32::WHITE),
    ));
    painter.line_segment(
        [
            center + egui::vec2(0.0, -s * 0.2),
            center + egui::vec2(0.0, s * 0.4),
        ],
        Stroke::new(2.0, Color32::WHITE),
    );
    painter.circle_filled(center + egui::vec2(0.0, s * 0.7), 1.5, Color32::WHITE);
}

fn draw_error_sign(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let s = 10.0;
    let offset = s * 0.4;
    let points = vec![
        center + egui::vec2(-offset, -s),
        center + egui::vec2(offset, -s),
        center + egui::vec2(s, -offset),
        center + egui::vec2(s, offset),
        center + egui::vec2(offset, s),
        center + egui::vec2(-offset, s),
        center + egui::vec2(-s, offset),
        center + egui::vec2(-s, -offset),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        Stroke::new(1.5, Color32::WHITE),
    ));
    painter.line_segment(
        [
            center - egui::vec2(s * 0.5, 0.0),
            center + egui::vec2(s * 0.5, 0.0),
        ],
        Stroke::new(2.5, Color32::WHITE),
    );
}

fn draw_check(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let s = 8.0;
    let stroke = Stroke::new(2.5, color);
    painter.line_segment(
        [
            center + egui::vec2(-s, 0.0),
            center + egui::vec2(-s * 0.3, s),
        ],
        stroke,
    );
    painter.line_segment(
        [center + egui::vec2(-s * 0.3, s), center + egui::vec2(s, -s)],
        stroke,
    );
}

fn draw_log_pin(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let size = 12.0;
    let r = size * 0.6;
    // Diamond head centered `size` pixels above center
    let diamond_center = center - egui::vec2(0.0, size);
    let top = diamond_center - egui::vec2(0.0, r);
    let right = diamond_center + egui::vec2(r, 0.0);
    let bottom = diamond_center + egui::vec2(0.0, r);
    let left = diamond_center - egui::vec2(r, 0.0);
    painter.add(egui::Shape::convex_polygon(
        vec![top, right, bottom, left],
        color,
        Stroke::new(1.0, Color32::WHITE),
    ));
    // Needle from diamond bottom vertex to center
    painter.line_segment([bottom, center], Stroke::new(2.0, color));
}
