use egui::{Color32, Pos2, Response, Stroke, Ui};
use uom::si::angle::degree;
use walkers::{MapMemory, Plugin, Position, Projector};

use nav_types::{CustomMarker, MarkerIcon};

pub struct MarkerRenderer<'a> {
    markers: &'a [CustomMarker],
}

impl<'a> MarkerRenderer<'a> {
    pub fn new(markers: &'a [CustomMarker]) -> Self {
        Self { markers }
    }
}

impl<'a> Plugin for MarkerRenderer<'a> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        _map_memory: &MapMemory,
    ) {
        let view_rect = ui.max_rect().expand(20.0);
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let mut closest_marker: Option<(usize, Pos2, f32)> = None;
        let hover_threshold = 20.0;

        for (i, marker) in self.markers.iter().enumerate() {
            let lat = marker.lat.get::<degree>();
            let lon = marker.lon.get::<degree>();

            let position = Position::new(lon, lat);
            let screen_pos = projector.project(position).to_pos2();

            if !view_rect.contains(screen_pos) {
                continue;
            }

            draw_marker_icon(ui, screen_pos, marker.icon);

            // Track closest marker for hover
            if let Some(mouse_pos) = hover_pos {
                let dist = screen_pos.distance(mouse_pos);
                if dist < hover_threshold {
                    if let Some((_, _, d)) = closest_marker {
                        if dist < d {
                            closest_marker = Some((i, screen_pos, dist));
                        }
                    } else {
                        closest_marker = Some((i, screen_pos, dist));
                    }
                }
            }
        }

        // Show tooltip for the closest marker
        if let Some((idx, pos, _)) = closest_marker
            && let Some(marker) = self.markers.get(idx)
        {
            let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
            let response = ui.interact(
                hit_rect,
                ui.id().with("marker_hover").with(idx),
                egui::Sense::hover(),
            );
            response.on_hover_text(&marker.label);
        }
    }
}

fn draw_marker_icon(ui: &Ui, center: Pos2, icon: MarkerIcon) {
    let color = match icon {
        MarkerIcon::Pin => Color32::from_rgb(219, 68, 55), // Red
        MarkerIcon::Cross | MarkerIcon::Check => Color32::from_rgb(15, 157, 88), // Green
        MarkerIcon::Circle => Color32::from_rgb(66, 133, 244), // Blue
        MarkerIcon::Lightning => Color32::from_rgb(244, 180, 0), // Yellow
        MarkerIcon::Warning => Color32::from_rgb(255, 153, 0), // Orange
        MarkerIcon::Error => Color32::from_rgb(204, 0, 0), // Dark Red
    };

    match icon {
        MarkerIcon::Pin => draw_pin(ui, center, color),
        MarkerIcon::Cross => draw_cross(ui, center, color),
        MarkerIcon::Circle => draw_circle(ui, center, color),
        MarkerIcon::Lightning => draw_lightning(ui, center, color),
        MarkerIcon::Warning => draw_warning(ui, center, color),
        MarkerIcon::Error => draw_error_sign(ui, center, color),
        MarkerIcon::Check => draw_check(ui, center, color),
    }
}

fn draw_pin(ui: &Ui, center: Pos2, color: Color32) {
    let painter = ui.painter();
    let size = 12.0;
    let pin_top = center - egui::vec2(0.0, size);

    // Pin head
    painter.circle_filled(pin_top, size * 0.6, color);
    painter.circle_stroke(pin_top, size * 0.6, Stroke::new(1.0, Color32::WHITE));

    // Pin needle
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

    // Exclamation mark
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

    // Octagon
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

    // Dash in middle
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
