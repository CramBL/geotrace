use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_types::{
    DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile, MapHighlight,
    SpatialPoint, TrackDataVisibility, filter,
};
use walkers::{MapMemory, Plugin, Projector};

pub struct GeneratedMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    visible_generated: Vec<SpatialPoint>,
}

impl<'a> GeneratedMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        visible_generated: Vec<SpatialPoint>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            visible_generated,
        }
    }

    fn is_point_highlighted(&self, point_ref: DataPointRef) -> bool {
        if self.highlight.sticky.is_some_and(|r| r == point_ref) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::Point(r)) => r == point_ref,
            Some(HighlightScope::Track {
                file_index,
                track_index,
            }) => file_index == point_ref.file_index && track_index == point_ref.track_index,
            Some(HighlightScope::TrackCategory {
                file_index,
                track_index,
                category,
            }) => {
                file_index == point_ref.file_index
                    && track_index == point_ref.track_index
                    && category == DataCategory::GeneratedMarker
            }
            _ => false,
        }
    }

    fn show_tooltip(&self, ui: &Ui, point_ref: DataPointRef, pos: Pos2) {
        let Some(file) = point_ref.file_index.get(self.files) else {
            return;
        };
        let Some(track) = point_ref.track_index.get(&file.tracks) else {
            return;
        };
        let Some(marker) = point_ref.point_index.get(&track.generated_markers) else {
            return;
        };
        let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
        let response = ui.interact(
            hit_rect,
            ui.id()
                .with("gen_marker_hover")
                .with(point_ref.file_index)
                .with(point_ref.track_index)
                .with(point_ref.point_index),
            egui::Sense::hover(),
        );
        response.show_tooltip_ui(|ui| match marker.kind {
            gt_types::GeneratedMarkerKind::GpsFixLost => {
                ui.strong("GPS fix lost");
                let corresponding = track
                    .points
                    .iter()
                    .find(|p| p.tpv.time().utc() == marker.time);
                if let Some(point) = corresponding {
                    ui.separator();
                    crate::tpv_renderer::show_hover_table(ui, point);
                }
            }
            gt_types::GeneratedMarkerKind::GpsFixRegained => {
                let label = match marker.fix_lost_duration {
                    Some(dur) => {
                        format!(
                            "GPS fix regained after {}",
                            format_fix_duration(dur.num_milliseconds())
                        )
                    }
                    None => "GPS fix regained".to_owned(),
                };
                ui.strong(label);
            }
        });
    }
}

impl Plugin for GeneratedMarkerRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

        for sp in &self.visible_generated {
            let Some(file_vis) = sp.file_index.get(&self.visibility.files) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            let Some(trip_vis) = sp.track_index.get(&file_vis.tracks) else {
                continue;
            };
            if !trip_vis.enabled || !trip_vis.generated_markers_visible {
                continue;
            }
            let Some(file) = sp.file_index.get(self.files) else {
                continue;
            };
            let Some(track) = sp.track_index.get(&file.tracks) else {
                continue;
            };
            if !filter::track_passes_filter(&track.metadata, self.filter) {
                continue;
            }
            let Some(marker) = sp.point_index.get(&track.generated_markers) else {
                continue;
            };
            if !filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                file_index: sp.file_index,
                track_index: sp.track_index,
                category: DataCategory::GeneratedMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc);
            let highlighted = self.is_point_highlighted(point_ref);
            draw_generated_marker(ui, screen_pos, marker.kind, highlighted);
        }

        // Show tooltip for the currently hovered generated marker.
        // Suppressed when the sticky popup is already showing this point, or when
        // any popup (e.g. context menu) is open and would be painted underneath.
        if let Some(HighlightScope::Point(r)) = self.highlight.hover
            && r.category == DataCategory::GeneratedMarker
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && let Some(file) = r.file_index.get(self.files)
            && let Some(track) = r.track_index.get(&file.tracks)
            && let Some(marker) = r.point_index.get(&track.generated_markers)
        {
            let pos = transform.to_screen(marker.merc);
            self.show_tooltip(ui, r, pos);
        }
    }
}

/// Formats a duration (given in milliseconds) for display in "fix regained" tooltips.
fn format_fix_duration(total_ms: i64) -> String {
    let total_ms = total_ms.max(0);
    let secs = total_ms / 1000;
    let frac_cs = (total_ms % 1000) / 10;
    if secs < 60 {
        match frac_cs {
            0 => format!("{secs}s"),
            cs if cs % 10 == 0 => format!("{secs}.{}s", cs / 10),
            cs => format!("{secs}.{cs:02}s"),
        }
    } else {
        let minutes = secs / 60;
        let remaining_secs = secs % 60;
        if remaining_secs == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m{remaining_secs}s")
        }
    }
}

fn draw_generated_marker(
    ui: &Ui,
    center: Pos2,
    kind: gt_types::GeneratedMarkerKind,
    highlighted: bool,
) {
    let painter = ui.painter();
    let (bg, stroke_color) = match kind {
        gt_types::GeneratedMarkerKind::GpsFixLost => {
            (Color32::from_rgb(219, 68, 55), Color32::WHITE)
        }
        gt_types::GeneratedMarkerKind::GpsFixRegained => {
            (Color32::from_rgb(15, 157, 88), Color32::WHITE)
        }
    };
    let radius = if highlighted { 11.0 } else { 8.0 };
    painter.circle_filled(center, radius, bg);
    painter.circle_stroke(center, radius, Stroke::new(1.5, stroke_color));
    if highlighted {
        painter.circle_stroke(
            center,
            radius + 3.5,
            Stroke::new(1.5, Color32::from_rgb(100, 200, 255)),
        );
    }
    let s = 4.0;
    match kind {
        gt_types::GeneratedMarkerKind::GpsFixLost => {
            let st = Stroke::new(2.0, stroke_color);
            painter.line_segment([center - egui::vec2(s, s), center + egui::vec2(s, s)], st);
            painter.line_segment([center + egui::vec2(-s, s), center + egui::vec2(s, -s)], st);
        }
        gt_types::GeneratedMarkerKind::GpsFixRegained => {
            let st = Stroke::new(2.0, stroke_color);
            painter.line_segment(
                [
                    center + egui::vec2(-s, 0.0),
                    center + egui::vec2(-s * 0.3, s),
                ],
                st,
            );
            painter.line_segment(
                [center + egui::vec2(-s * 0.3, s), center + egui::vec2(s, -s)],
                st,
            );
        }
    }
}
