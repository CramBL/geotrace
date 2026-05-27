use egui::{Color32, Pos2, Response, Stroke, Ui};
use nav_types::{
    DataCategory, DataPointRef, EventMarker, EventMarkerStyle, EventMarkerVisibility, FileIdx,
    GlobalFilter, LoadedFile, MapHighlight, MercBounds, PointIdx, TripIdx, filter,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use walkers::{MapMemory, Plugin, Projector};

const HOVER_THRESHOLD: f32 = 12.0;

pub struct EventMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a nav_types::TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    event_vis: &'a EventMarkerVisibility,
    hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
}

impl<'a> EventMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a nav_types::TripDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        event_vis: &'a EventMarkerVisibility,
        hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            event_vis,
            hover_out,
        }
    }
}

impl Plugin for EventMarkerRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let view_rect = ui.max_rect().expand(20.0);
        let mut local_closest: Option<(DataPointRef, Pos2)> = None;
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());
        let vp_bounds = MercBounds {
            x_min: transform.merc_x_from_screen(view_rect.min.x),
            x_max: transform.merc_x_from_screen(view_rect.max.x),
            y_min: transform.merc_y_from_screen(view_rect.min.y),
            y_max: transform.merc_y_from_screen(view_rect.max.y),
        };

        for (fi, file) in self.files.iter().enumerate() {
            let Some(file_vis) = self.visibility.files.get(fi) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }

            let style_map: HashMap<&str, &EventMarkerStyle> = file
                .event_marker_styles
                .iter()
                .map(|s| (s.variant_path.as_str(), s))
                .collect();

            for (ti, trip) in file.trips.iter().enumerate() {
                let Some(trip_vis) = file_vis.trips.get(ti) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.event_markers_visible {
                    continue;
                }
                if !filter::trip_passes_filter(&trip.metadata, self.filter) {
                    continue;
                }

                for (pi, marker) in trip.event_markers.iter().enumerate() {
                    if !self.event_vis.is_visible(fi, ti, &marker.variant_path) {
                        continue;
                    }
                    if !filter::point_passes_time_filter(marker.time, self.filter) {
                        continue;
                    }
                    if marker.merc_x < vp_bounds.x_min
                        || marker.merc_x > vp_bounds.x_max
                        || marker.merc_y < vp_bounds.y_min
                        || marker.merc_y > vp_bounds.y_max
                    {
                        continue;
                    }

                    let screen_pos = transform.to_screen(marker.merc_x, marker.merc_y);
                    let point_ref = DataPointRef {
                        file_index: FileIdx(fi),
                        trip_index: TripIdx(ti),
                        category: DataCategory::EventMarker,
                        point_index: PointIdx(pi),
                    };

                    if let Some(mouse) = hover_pos {
                        let dist_sq = screen_pos.distance_sq(mouse);
                        if dist_sq < HOVER_THRESHOLD * HOVER_THRESHOLD {
                            let hover_val = self.hover_out.get();
                            if hover_val.is_none_or(|(_, d)| dist_sq < d) {
                                self.hover_out.set(Some((point_ref, dist_sq)));
                            }
                            let is_closer = local_closest
                                .as_ref()
                                .is_none_or(|(_, p)| p.distance_sq(mouse) > dist_sq);
                            if is_closer {
                                local_closest = Some((point_ref, screen_pos));
                            }
                        }
                    }

                    let color = resolve_color(marker, &style_map);
                    let highlighted = is_highlighted(self.highlight, point_ref);
                    draw_event_marker(ui, screen_pos, color, highlighted);
                }
            }
        }

        show_tooltip(ui, self.files, local_closest);
    }
}

fn resolve_color(marker: &EventMarker, style_map: &HashMap<&str, &EventMarkerStyle>) -> Color32 {
    if let Some(style) = style_map.get(marker.variant_path.as_str()) {
        Color32::from_rgb(style.color.0, style.color.1, style.color.2)
    } else {
        let (r, g, b) = nav_types::event_marker_fallback_color(&marker.variant_path);
        Color32::from_rgb(r, g, b)
    }
}

fn is_highlighted(highlight: &MapHighlight, point_ref: DataPointRef) -> bool {
    if highlight.sticky.is_some_and(|r| r == point_ref) {
        return true;
    }
    match highlight.hover {
        Some(nav_types::HighlightScope::Point(r)) => r == point_ref,
        Some(nav_types::HighlightScope::Trip {
            file_index,
            trip_index,
        }) => file_index == point_ref.file_index && trip_index == point_ref.trip_index,
        _ => false,
    }
}

fn draw_event_marker(ui: &Ui, center: Pos2, color: Color32, highlighted: bool) {
    let painter = ui.painter();
    let size = if highlighted { 10.0 } else { 7.0 };

    // Draw a diamond (rotated square) filled with the variant color.
    let points = [
        center + egui::vec2(0.0, -size),
        center + egui::vec2(size, 0.0),
        center + egui::vec2(0.0, size),
        center + egui::vec2(-size, 0.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        points.to_vec(),
        color,
        Stroke::new(1.5, Color32::WHITE),
    ));

    if highlighted {
        painter.add(egui::Shape::convex_polygon(
            [
                center + egui::vec2(0.0, -(size + 4.0)),
                center + egui::vec2(size + 4.0, 0.0),
                center + egui::vec2(0.0, size + 4.0),
                center + egui::vec2(-(size + 4.0), 0.0),
            ]
            .to_vec(),
            Color32::TRANSPARENT,
            Stroke::new(1.5, Color32::from_rgb(100, 200, 255)),
        ));
    }
}

fn show_tooltip(ui: &Ui, files: &[LoadedFile], local_closest: Option<(DataPointRef, Pos2)>) {
    let Some((point_ref, pos)) = local_closest else {
        return;
    };
    let Some(file) = files.get(point_ref.file_index.0) else {
        return;
    };
    let Some(trip) = file.trips.get(point_ref.trip_index.0) else {
        return;
    };
    let Some(marker) = trip.event_markers.get(point_ref.point_index.0) else {
        return;
    };

    let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(24.0, 24.0));
    let response = ui.interact(
        hit_rect,
        ui.id()
            .with("event_marker_hover")
            .with(point_ref.file_index.0)
            .with(point_ref.trip_index.0)
            .with(point_ref.point_index.0),
        egui::Sense::hover(),
    );
    response.show_tooltip_ui(|ui| {
        ui.strong(&marker.variant_path);
        ui.label(marker.time.format("%Y-%m-%d %H:%M:%S").to_string());
        if let Some(ann) = &marker.annotation {
            ui.separator();
            ui.label(ann);
        }
    });
}
