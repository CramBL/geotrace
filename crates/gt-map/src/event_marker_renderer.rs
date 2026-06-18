use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_filter::GlobalFilter;
use gt_types::{DataCategory, EventMarkerStyle, LoadedFile, MarkerIcon, SpatialPoint};
use gt_ui_theme::HIGHLIGHT_BLUE;
use gt_ui_types::{
    DataPointRef, EventMarkerVisibility, HighlightScope, MapHighlight, TrackDataVisibility,
};
use std::collections::HashMap;
use walkers::{MapMemory, Plugin, Projector};

use crate::track_renderer;

pub struct EventMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    event_vis: &'a EventMarkerVisibility,
    visible_event: Vec<SpatialPoint>,
}

impl<'a> EventMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        event_vis: &'a EventMarkerVisibility,
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
        let transform =
            crate::transform::MercTransform::new(projector, map_memory, ui.max_rect().center());

        for sp in &self.visible_event {
            let Some(file_vis) = sp.file_index.get(&self.visibility.files) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            let Some(trip_vis) = sp.track_index.get(&file_vis.tracks) else {
                continue;
            };
            if !trip_vis.enabled || !trip_vis.event_markers_visible {
                continue;
            }
            let Some(file) = sp.file_index.get(self.files) else {
                continue;
            };
            let Some(track) = sp.track_index.get(&file.tracks) else {
                continue;
            };
            if !gt_filter::track_passes_filter(&track.metadata, self.filter) {
                continue;
            }
            let Some(marker) = sp.point_index.get(&track.event_markers) else {
                continue;
            };
            if !self
                .event_vis
                .is_visible(sp.track_ref(), &marker.variant_path)
            {
                continue;
            }
            if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                track: sp.track_ref(),
                category: DataCategory::EventMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc);
            let style_map = &file.event_marker_styles;
            let color = resolve_color(marker, style_map);
            let icon = resolve_icon(&marker.variant_path, style_map);
            let highlighted = is_highlighted(self.highlight, point_ref);
            let fade =
                track_renderer::track_fade_alpha(self.highlight, sp.file_index, sp.track_index);
            draw_event_marker(ui, screen_pos, icon, color, highlighted, fade);
        }

        // Show tooltip for the hovered event marker. Uses hover_candidates[1] so
        // the tooltip appears even when a Tpv point is the primary hover.
        if let Some(r) = self.highlight.hover_candidates[1]
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && !self.highlight.suppress_hover_labels
            && let Some(file) = r.track.fi.get(self.files)
            && let Some(track) = r.track.index.get(&file.tracks)
            && let Some(marker) = r.point_index.get(&track.event_markers)
        {
            let pos = transform.to_screen(marker.merc);
            show_tooltip(ui, r, marker, pos);
        }
    }
}

fn resolve_color(
    marker: &gt_types::EventMarker,
    style_map: &HashMap<String, EventMarkerStyle>,
) -> Color32 {
    let c = if let Some(style) = style_map.get(marker.variant_path.as_str()) {
        style.color
    } else {
        gt_types::event_marker_fallback_color(&marker.variant_path)
    };
    Color32::from_rgb(c.r, c.g, c.b)
}

fn resolve_icon(variant_path: &str, style_map: &HashMap<String, EventMarkerStyle>) -> MarkerIcon {
    style_map
        .get(variant_path)
        .map_or(MarkerIcon::Pin, |s| s.icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gt_types::{EventMarkerStyle, MarkerColor, MarkerIcon};

    fn style(path: &str, icon: MarkerIcon) -> EventMarkerStyle {
        EventMarkerStyle {
            variant_path: path.to_string(),
            icon,
            color: MarkerColor::new(0, 0, 0),
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
        Some(HighlightScope::Point(r)) => r == point_ref,
        Some(HighlightScope::Track(track)) => track == point_ref.track,
        _ => false,
    }
}

fn draw_event_marker(
    ui: &Ui,
    center: Pos2,
    icon: MarkerIcon,
    color: Color32,
    highlighted: bool,
    fade: f32,
) {
    match icon {
        MarkerIcon::Pin | MarkerIcon::Log => draw_diamond(ui, center, color, highlighted, fade),
        MarkerIcon::Cross => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_CROSS,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Circle => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_CIRCLE_MARKER,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Lightning => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_LIGHTNING,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Warning => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_WARNING,
            crate::icons::ICON_SIZE_LARGE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Error => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_ERROR,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Check => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_CHECK,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Satellite => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_SATELLITE,
            crate::icons::ICON_SIZE_LARGE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::SatelliteLost => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_SATELLITE_LOST,
            crate::icons::ICON_SIZE_LARGE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Gear => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_GEAR,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Refresh => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_REFRESH,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Download => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_DOWNLOAD,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Upload => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_UPLOAD,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
        MarkerIcon::Wrench => draw_event_icon(
            ui,
            center,
            crate::icons::ICON_URI_WRENCH,
            crate::icons::ICON_SIZE_PX,
            highlighted,
            fade,
        ),
    }
}

fn draw_diamond(ui: &Ui, center: Pos2, color: Color32, highlighted: bool, fade: f32) {
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
        track_renderer::apply_fade_alpha(color, fade),
        Stroke::new(1.5, track_renderer::apply_fade_alpha(Color32::WHITE, fade)),
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

fn draw_event_icon(
    ui: &Ui,
    center: Pos2,
    uri: &'static str,
    size: f32,
    highlighted: bool,
    fade: f32,
) {
    if highlighted {
        ui.painter()
            .circle_stroke(center, (size / 2.0) + 4.0, Stroke::new(2.0, HIGHLIGHT_BLUE));
    }
    crate::icons::draw_cached_icon(
        ui,
        uri,
        egui::Rect::from_center_size(center, egui::vec2(size, size)),
        track_renderer::apply_fade_alpha(Color32::WHITE, fade),
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
                    1.0,
                );
            }
        });

        harness.run();
        harness.snapshot("all_marker_icons");
    }
}

fn show_tooltip(ui: &Ui, point_ref: DataPointRef, marker: &gt_types::EventMarker, pos: Pos2) {
    let hit_rect = egui::Rect::from_center_size(
        pos,
        egui::vec2(
            crate::icons::ICON_SIZE_LARGE_PX,
            crate::icons::ICON_SIZE_LARGE_PX,
        ),
    );
    let response = ui.interact(
        hit_rect,
        ui.id()
            .with("event_marker_hover")
            .with(point_ref.track)
            .with(point_ref.point_index),
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
