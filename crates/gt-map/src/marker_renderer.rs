use egui::{Color32, Pos2, Response, Stroke, Ui, Vec2};
use gt_types::{CustomMarker, DataCategory, MarkerIcon, SpatialPoint};
use gt_ui_theme::HIGHLIGHT_BLUE;
use gt_ui_types::{HighlightScope, MapElementRef, MapHighlight, ResolvedElement};
use walkers::{MapMemory, Plugin, Projector};

use crate::icon_mesh::{IconInstance, IconMeshBatch, IconMeshLibrary, PIN_HALF_EXTENTS_PT};
use crate::{track_renderer, viewport};

pub(crate) struct MarkerRenderer<'a, 'p> {
    plan: &'a viewport::MapFramePlan<'p>,
    highlight: &'a MapHighlight,
    visible_custom: &'a [SpatialPoint],
    icon_meshes: Option<&'a IconMeshLibrary>,
}

impl<'a, 'p> MarkerRenderer<'a, 'p> {
    pub(crate) fn new(
        plan: &'a viewport::MapFramePlan<'p>,
        highlight: &'a MapHighlight,
        visible_custom: &'a [SpatialPoint],
        icon_meshes: Option<&'a IconMeshLibrary>,
    ) -> Self {
        Self {
            plan,
            highlight,
            visible_custom,
            icon_meshes,
        }
    }

    fn is_marker_highlighted(&self, point_ref: MapElementRef) -> bool {
        if self.highlight.is_sticky(point_ref) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::Point(r)) => r == point_ref,
            Some(HighlightScope::Track(track)) => track == point_ref.track(),
            Some(HighlightScope::TrackCategory { track, category }) => {
                track == point_ref.track() && category == DataCategory::CustomMarker
            }
            _ => false,
        }
    }
}

impl Plugin for MarkerRenderer<'_, '_> {
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
            let Some(present) = self.plan.resolve_spatial(sp) else {
                continue;
            };
            let ResolvedElement::CustomMarker(marker) = present.element() else {
                continue;
            };
            let point_ref = present.element_ref();
            let screen_pos = transform.to_screen(sp.merc);
            let highlighted = self.is_marker_highlighted(point_ref);
            let fade =
                track_renderer::track_fade_alpha(self.highlight, sp.file_index, sp.track_index);
            draw_marker_icon(ui, &mut batch, screen_pos, marker, highlighted, fade);
        }
        batch.paint(ui.painter());
    }
}

pub(crate) fn show_hover_label(ui: &mut Ui, marker: &CustomMarker) {
    ui.label(&marker.label);
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
    // The SVGs paint their own colors: the tint only applies the fade.
    let tint = track_renderer::apply_fade_alpha(Color32::WHITE, fade);
    let instance = match marker.icon {
        // The pin anchors its tip at the marker position.
        MarkerIcon::Pin => IconInstance {
            icon: marker.icon.into(),
            center: center - egui::vec2(0.0, PIN_HALF_EXTENTS_PT.y),
            half_extents: PIN_HALF_EXTENTS_PT,
            direction: None,
            tints: [tint; 3],
        },
        icon => IconInstance {
            icon: icon.into(),
            center,
            half_extents: Vec2::splat(crate::icon_mesh::marker_icon_half_extent(icon)),
            direction: None,
            tints: [tint; 3],
        },
    };
    batch.push(instance);
}
