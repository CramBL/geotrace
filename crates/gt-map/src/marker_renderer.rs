use egui::{Color32, Pos2, Response, Stroke, Ui, Vec2};
use gt_filter::GlobalFilter;
use gt_types::{CustomMarker, DataCategory, LoadedFile, MarkerIcon, SpatialPoint};
use gt_ui_theme::{HIGHLIGHT_BLUE, LOG_COLORS};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight, TrackDataVisibility, visibility};
use walkers::{MapMemory, Plugin, Projector};

use crate::icon_mesh::{IconInstance, IconMeshBatch, IconMeshLibrary, PIN_HALF_EXTENTS_PT};
use crate::track_renderer;

pub struct MarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    visible_custom: &'a [SpatialPoint],
    icon_meshes: Option<&'a IconMeshLibrary>,
}

impl<'a> MarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        visible_custom: &'a [SpatialPoint],
        icon_meshes: Option<&'a IconMeshLibrary>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            visible_custom,
            icon_meshes,
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

        let mut batch = IconMeshBatch::new(self.icon_meshes, ui.pixels_per_point());
        for sp in self.visible_custom {
            let Some(track) = visibility::category_in_scope(
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
            draw_marker_icon(ui, &mut batch, screen_pos, marker, highlighted, fade);
        }
        batch.paint(ui.painter());

        if let Some(r) = self.highlight.hover_candidates.custom_marker
            && self
                .highlight
                .shows_hover_label(r, ui.ctx().any_popup_open())
            && let Some(file) = r.track.fi.get(self.files)
            && let Some(track) = r.track.index.get(&file.tracks)
            && let Some(marker) = r.point_index.get(&track.custom_markers)
        {
            let pos = transform.to_screen(marker.merc);
            let tpv_also_hovered = self
                .highlight
                .hover_candidates
                .tpv_or_satellite_report
                .is_some();
            show_marker_hover_label(ui, marker, pos, tpv_also_hovered);
        }
    }
}

/// Paint the marker's label directly onto the map canvas.
///
/// With `tpv_also_hovered` the label is drawn above the icon, clear of the TPV
/// tooltip. A label too wide to fit there is replaced with a message giving its
/// width.
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

fn draw_marker_icon(
    ui: &Ui,
    batch: &mut IconMeshBatch<'_>,
    center: Pos2,
    marker: &CustomMarker,
    highlighted: bool,
    fade: f32,
) {
    if highlighted {
        ui.painter()
            .circle_stroke(center, 14.0, Stroke::new(2.0_f32, HIGHLIGHT_BLUE));
    }
    // The SVGs have their own colors. Only the log pin is recolored, cycling
    // through the per-logfile palette.
    let color = match marker.icon {
        MarkerIcon::Log => {
            let idx = marker
                .color_group
                .map_or(0, |id| id as usize % LOG_COLORS.len());
            LOG_COLORS.get(idx).copied().unwrap_or(Color32::WHITE)
        }
        _ => Color32::WHITE,
    };
    let tint = track_renderer::apply_fade_alpha(color, fade);
    let instance = match marker.icon {
        // The pins anchor their tip at the marker position.
        MarkerIcon::Pin | MarkerIcon::Log => IconInstance {
            icon: marker.icon.into(),
            center: center - egui::vec2(0.0, PIN_HALF_EXTENTS_PT.y),
            half_extents: PIN_HALF_EXTENTS_PT,
            direction: None,
            tints: [tint; 2],
        },
        icon => IconInstance {
            icon: icon.into(),
            center,
            half_extents: Vec2::splat(crate::icon_mesh::marker_icon_half_extent(icon)),
            direction: None,
            tints: [tint; 2],
        },
    };
    batch.push(instance);
}
