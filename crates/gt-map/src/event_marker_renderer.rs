use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_types::{
    DataCategory, DataPointRef, EventMarkerStyle, GlobalFilter, HighlightScope, LoadedFile,
    MapHighlight, MarkerIcon, SpatialPoint, TrackDataVisibility, filter,
};
use gt_ui_theme::HIGHLIGHT_BLUE;
use std::collections::HashMap;
use walkers::{MapMemory, Plugin, Projector};

pub struct EventMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    event_vis: &'a gt_types::EventMarkerVisibility,
    visible_event: Vec<SpatialPoint>,
}

impl<'a> EventMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a gt_types::TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        event_vis: &'a gt_types::EventMarkerVisibility,
        visible_event: Vec<SpatialPoint>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            event_vis,
            visible_event,
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
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

        for sp in &self.visible_event {
            let Some(file_vis) = self.visibility.files.get(sp.file_index.0) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            let Some(trip_vis) = file_vis.tracks.get(sp.track_index.0) else {
                continue;
            };
            if !trip_vis.enabled || !trip_vis.event_markers_visible {
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
            let Some(marker) = track.event_markers.get(sp.point_index.0) else {
                continue;
            };
            if !self
                .event_vis
                .is_visible(sp.file_index.0, sp.track_index.0, &marker.variant_path)
            {
                continue;
            }
            if !filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                file_index: sp.file_index,
                track_index: sp.track_index,
                category: DataCategory::EventMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc_x, sp.merc_y);
            let style_map = &file.event_marker_styles;
            let color = resolve_color(marker, style_map);
            let icon = resolve_icon(&marker.variant_path, style_map);
            let highlighted = is_highlighted(self.highlight, point_ref);
            draw_event_marker(ui, screen_pos, icon, color, highlighted);
        }

        // Show tooltip for the currently hovered event marker.
        // Suppressed when the sticky popup is already showing this point, or when
        // any popup (e.g. context menu) is open and would be painted underneath.
        if let Some(HighlightScope::Point(r)) = self.highlight.hover
            && r.category == DataCategory::EventMarker
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && let Some(file) = self.files.get(r.file_index.0)
            && let Some(track) = file.tracks.get(r.track_index.0)
            && let Some(marker) = track.event_markers.get(r.point_index.0)
        {
            let pos = transform.to_screen(marker.merc_x, marker.merc_y);
            show_tooltip(ui, r, marker, pos);
        }
    }
}

fn resolve_color(
    marker: &gt_types::EventMarker,
    style_map: &HashMap<String, EventMarkerStyle>,
) -> Color32 {
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
        Some(gt_types::HighlightScope::Track {
            file_index,
            track_index,
        }) => file_index == point_ref.file_index && track_index == point_ref.track_index,
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
            Stroke::new(1.5, HIGHLIGHT_BLUE),
        ));
    }
}

fn draw_event_icon(ui: &Ui, center: Pos2, uri: &'static str, size: f32, highlighted: bool) {
    if highlighted {
        ui.painter()
            .circle_stroke(center, (size / 2.0) + 4.0, Stroke::new(2.0, HIGHLIGHT_BLUE));
    }
    crate::draw_cached_icon(
        ui,
        uri,
        egui::Rect::from_center_size(center, egui::vec2(size, size)),
        egui::Color32::WHITE,
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

fn show_tooltip(ui: &Ui, point_ref: DataPointRef, marker: &gt_types::EventMarker, pos: Pos2) {
    let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(24.0, 24.0));
    let response = ui.interact(
        hit_rect,
        ui.id()
            .with("event_marker_hover")
            .with(point_ref.file_index.0)
            .with(point_ref.track_index.0)
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
