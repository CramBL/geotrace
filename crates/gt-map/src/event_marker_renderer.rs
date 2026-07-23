use egui::{Color32, Pos2, Response, Stroke, Ui, Vec2};
use gt_filter::GlobalFilter;
use gt_types::{DataCategory, EventMarkerStyle, LoadedFile, MarkerIcon, SpatialPoint};
use gt_ui_theme::HIGHLIGHT_BLUE;
use gt_ui_types::{
    DataPointRef, EventMarkerVisibility, HighlightScope, MapHighlight, TrackDataVisibility,
};
use std::collections::HashMap;
use walkers::{MapMemory, Plugin, Projector};

use crate::icon_mesh::{IconInstance, IconMeshBatch, IconMeshLibrary};
use crate::track_renderer;

pub struct EventMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    event_vis: &'a EventMarkerVisibility,
    visible_event: &'a [SpatialPoint],
    icon_meshes: Option<&'a IconMeshLibrary>,
}

impl<'a> EventMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        event_vis: &'a EventMarkerVisibility,
        visible_event: &'a [SpatialPoint],
        icon_meshes: Option<&'a IconMeshLibrary>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            event_vis,
            visible_event,
            icon_meshes,
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

        let mut batch = IconMeshBatch::new(self.icon_meshes, ui.pixels_per_point());
        for sp in self.visible_event {
            let Some(track) = crate::scope::category_in_scope(
                self.files,
                self.visibility,
                self.filter,
                sp.track_ref(),
                DataCategory::EventMarker,
            ) else {
                continue;
            };
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
            let Some(file) = sp.file_index.get(self.files) else {
                continue;
            };
            let style_map = &file.event_marker_styles;
            let color = resolve_color(marker, style_map);
            let icon = resolve_icon(&marker.variant_path, style_map);
            let highlighted = is_highlighted(self.highlight, point_ref);
            let fade =
                track_renderer::track_fade_alpha(self.highlight, sp.file_index, sp.track_index);
            draw_event_marker(ui, &mut batch, screen_pos, icon, color, highlighted, fade);
        }
        batch.paint(ui.painter());

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
    batch: &mut IconMeshBatch<'_>,
    center: Pos2,
    icon: MarkerIcon,
    color: Color32,
    highlighted: bool,
    fade: f32,
) {
    match icon {
        // Event markers render the pin styles as colorable diamonds instead
        // of the custom-marker pin glyphs.
        MarkerIcon::Pin | MarkerIcon::Log => draw_diamond(ui, center, color, highlighted, fade),
        MarkerIcon::Cross
        | MarkerIcon::Circle
        | MarkerIcon::Lightning
        | MarkerIcon::Warning
        | MarkerIcon::Error
        | MarkerIcon::Check
        | MarkerIcon::Satellite
        | MarkerIcon::SatelliteLost
        | MarkerIcon::Gear
        | MarkerIcon::Refresh
        | MarkerIcon::Download
        | MarkerIcon::Upload
        | MarkerIcon::Wrench => draw_event_icon(ui, batch, center, icon, highlighted, fade),
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
    batch: &mut IconMeshBatch<'_>,
    center: Pos2,
    icon: MarkerIcon,
    highlighted: bool,
    fade: f32,
) {
    let half_extent = crate::icon_mesh::marker_icon_half_extent(icon);
    if highlighted {
        ui.painter()
            .circle_stroke(center, half_extent + 4.0, Stroke::new(2.0, HIGHLIGHT_BLUE));
    }
    batch.push(IconInstance {
        icon: icon.into(),
        center,
        half_extents: Vec2::splat(half_extent),
        direction: None,
        tints: [track_renderer::apply_fade_alpha(Color32::WHITE, fade); 2],
    });
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
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

        let library = IconMeshLibrary::embedded().unwrap();
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(width, height))
            .ui(move |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(30, 30, 30));

                let mut batch = IconMeshBatch::new(Some(&library), ui.pixels_per_point());
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
                        &mut batch,
                        egui::pos2(x, y),
                        icon,
                        egui::Color32::from_rgb(230, 150, 50),
                        false,
                        1.0,
                    );
                }
                batch.paint(ui.painter());
            });

        harness.run();
        // Loose: mesh edges rasterize a few pixels differently between the
        // Linux baseline and the macOS CI runner's Metal backend.
        harness.snapshot_loose("all_marker_icons");
    }
}

fn show_tooltip(ui: &Ui, point_ref: DataPointRef, marker: &gt_types::EventMarker, pos: Pos2) {
    let hit_rect = egui::Rect::from_center_size(
        pos,
        Vec2::splat(2.0 * crate::icon_mesh::ICON_HALF_EXTENT_LARGE_PT),
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
