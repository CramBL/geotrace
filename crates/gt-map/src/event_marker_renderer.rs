use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_types::{
    DataCategory, DataPointRef, EventMarker, EventMarkerStyle, EventMarkerVisibility, FileIdx,
    GlobalFilter, LoadedFile, MapHighlight, MarkerIcon, MercBounds, PointIdx, TripIdx, filter,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use walkers::{MapMemory, Plugin, Projector};

const HOVER_THRESHOLD: f32 = 12.0;

pub struct EventMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a gt_types::TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    event_vis: &'a EventMarkerVisibility,
    hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
}

impl<'a> EventMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a gt_types::TripDataVisibility,
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

            let style_map = &file.event_marker_styles;

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

                    let color = resolve_color(marker, style_map);
                    let icon = resolve_icon(&marker.variant_path, style_map);
                    let highlighted = is_highlighted(self.highlight, point_ref);
                    draw_event_marker(ui, screen_pos, icon, color, highlighted);
                }
            }
        }

        show_tooltip(ui, self.files, local_closest);
    }
}

fn resolve_color(marker: &EventMarker, style_map: &HashMap<String, EventMarkerStyle>) -> Color32 {
    if let Some(style) = style_map.get(marker.variant_path.as_str()) {
        Color32::from_rgb(style.color.0, style.color.1, style.color.2)
    } else {
        let (r, g, b) = gt_types::event_marker_fallback_color(&marker.variant_path);
        Color32::from_rgb(r, g, b)
    }
}

fn resolve_icon(variant_path: &str, style_map: &HashMap<String, EventMarkerStyle>) -> MarkerIcon {
    style_map
        .get(variant_path)
        .map_or(MarkerIcon::Pin, |s| s.icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gt_types::{EventMarkerStyle, MarkerIcon};

    fn style(path: &str, icon: MarkerIcon) -> EventMarkerStyle {
        EventMarkerStyle {
            variant_path: path.to_string(),
            icon,
            color: (0, 0, 0),
        }
    }

    #[test]
    fn resolve_icon_returns_icon_from_style_map() {
        let s = style("power/turn_on", MarkerIcon::Lightning);
        let map = HashMap::from([(s.variant_path.clone(), s)]);
        assert_eq!(resolve_icon("power/turn_on", &map), MarkerIcon::Lightning);
    }

    #[test]
    fn resolve_icon_falls_back_to_pin_when_path_not_in_map() {
        let map: HashMap<String, EventMarkerStyle> = HashMap::new();
        assert_eq!(resolve_icon("unknown/path", &map), MarkerIcon::Pin);
    }

    #[test]
    fn resolve_icon_distinguishes_all_icon_variants() {
        let icons = [
            MarkerIcon::Pin,
            MarkerIcon::Cross,
            MarkerIcon::Circle,
            MarkerIcon::Lightning,
            MarkerIcon::Warning,
            MarkerIcon::Error,
            MarkerIcon::Check,
            MarkerIcon::Satellite,
            MarkerIcon::SatelliteLost,
            MarkerIcon::Gear,
            MarkerIcon::Refresh,
            MarkerIcon::Download,
            MarkerIcon::Upload,
            MarkerIcon::Wrench,
        ];
        for icon in icons {
            let s = style("p", icon);
            let map = HashMap::from([(s.variant_path.clone(), s)]);
            assert_eq!(resolve_icon("p", &map), icon);
        }
    }
}

fn is_highlighted(highlight: &MapHighlight, point_ref: DataPointRef) -> bool {
    if highlight.sticky.is_some_and(|r| r == point_ref) {
        return true;
    }
    match highlight.hover {
        Some(gt_types::HighlightScope::Point(r)) => r == point_ref,
        Some(gt_types::HighlightScope::Trip {
            file_index,
            trip_index,
        }) => file_index == point_ref.file_index && trip_index == point_ref.trip_index,
        _ => false,
    }
}

fn draw_event_marker(ui: &Ui, center: Pos2, icon: MarkerIcon, color: Color32, highlighted: bool) {
    match icon {
        MarkerIcon::Pin | MarkerIcon::Log => draw_diamond(ui, center, color, highlighted),
        MarkerIcon::Cross => draw_event_icon(ui, center, crate::ICON_URI_CROSS, 20.0, highlighted),
        MarkerIcon::Circle => {
            draw_event_icon(ui, center, crate::ICON_URI_CIRCLE_MARKER, 20.0, highlighted)
        }
        MarkerIcon::Lightning => {
            draw_event_icon(ui, center, crate::ICON_URI_LIGHTNING, 20.0, highlighted)
        }
        MarkerIcon::Warning => {
            draw_event_icon(ui, center, crate::ICON_URI_WARNING, 24.0, highlighted)
        }
        MarkerIcon::Error => draw_event_icon(ui, center, crate::ICON_URI_ERROR, 20.0, highlighted),
        MarkerIcon::Check => draw_event_icon(ui, center, crate::ICON_URI_CHECK, 20.0, highlighted),
        MarkerIcon::Satellite => {
            draw_event_icon(ui, center, crate::ICON_URI_SATELLITE, 24.0, highlighted)
        }
        MarkerIcon::SatelliteLost => draw_event_icon(
            ui,
            center,
            crate::ICON_URI_SATELLITE_LOST,
            24.0,
            highlighted,
        ),
        MarkerIcon::Gear => draw_event_icon(ui, center, crate::ICON_URI_GEAR, 20.0, highlighted),
        MarkerIcon::Refresh => {
            draw_event_icon(ui, center, crate::ICON_URI_REFRESH, 20.0, highlighted)
        }
        MarkerIcon::Download => {
            draw_event_icon(ui, center, crate::ICON_URI_DOWNLOAD, 20.0, highlighted)
        }
        MarkerIcon::Upload => {
            draw_event_icon(ui, center, crate::ICON_URI_UPLOAD, 20.0, highlighted)
        }
        MarkerIcon::Wrench => {
            draw_event_icon(ui, center, crate::ICON_URI_WRENCH, 20.0, highlighted)
        }
    }
}

fn draw_diamond(ui: &Ui, center: Pos2, color: Color32, highlighted: bool) {
    let painter = ui.painter();
    let size = if highlighted { 10.0 } else { 7.0 };
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

fn draw_event_icon(ui: &Ui, center: Pos2, uri: &'static str, size: f32, highlighted: bool) {
    if highlighted {
        ui.painter().circle_stroke(
            center,
            (size / 2.0) + 4.0,
            Stroke::new(2.0, Color32::from_rgb(100, 200, 255)),
        );
    }
    egui::Image::new(egui::ImageSource::Uri(std::borrow::Cow::Borrowed(uri))).paint_at(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(size, size)),
    );
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::test_harness::TestHarness;
    use strum::IntoEnumIterator;

    #[test]
    fn all_icons_render_correctly() {
        let icons: Vec<gt_types::MarkerIcon> = gt_types::MarkerIcon::iter().collect();
        let cols: usize = 5;
        let spacing = 64.0_f32;
        let margin = 40.0_f32;
        let rows = icons.len().div_ceil(cols);
        let width = margin * 2.0 + (cols - 1) as f32 * spacing;
        let height = margin * 2.0 + (rows - 1) as f32 * spacing;

        let mut harness = TestHarness::new_wgpu(egui::vec2(width, height), move |ui| {
            egui_extras::install_image_loaders(ui.ctx());
            crate::register_marker_icons(ui.ctx());

            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(30, 30, 30));

            for (i, &icon) in icons.iter().enumerate() {
                let row = i / cols;
                let col = if row.is_multiple_of(2) {
                    i % cols
                } else {
                    cols - 1 - (i % cols)
                };
                let x = margin + col as f32 * spacing;
                let y = margin + row as f32 * spacing;
                draw_event_marker(
                    ui,
                    egui::pos2(x, y),
                    icon,
                    egui::Color32::from_rgb(230, 150, 50),
                    false,
                );
            }
        });

        harness.run();
        harness.snapshot("all_marker_icons");
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
