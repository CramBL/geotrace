use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_filter::GlobalFilter;
use gt_types::{DataCategory, LoadedFile, SpatialPoint};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight, TrackDataVisibility};
use walkers::{MapMemory, Plugin, Projector};

use crate::track_renderer;

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
            Some(HighlightScope::Track(track)) => track == point_ref.track,
            Some(HighlightScope::TrackCategory { track, category }) => {
                track == point_ref.track && category == DataCategory::GeneratedMarker
            }
            _ => false,
        }
    }

    fn show_tooltip(&self, ui: &Ui, point_ref: DataPointRef, pos: Pos2) {
        let Some(file) = point_ref.track.fi.get(self.files) else {
            return;
        };
        let Some(track) = point_ref.track.index.get(&file.tracks) else {
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
                .with(point_ref.track)
                .with(point_ref.point_index),
            egui::Sense::hover(),
        );
        response.show_tooltip_ui(|ui| {
            ui.strong(generated_marker_header(marker.kind));
            // Fix-lost and clock-discontinuity markers sit on a specific
            // anomalous sample, so also show that point's data. Fix-regained is
            // a transition with no single underlying point to detail.
            let show_point = match marker.kind {
                gt_types::GeneratedMarkerKind::GnssFixLost
                | gt_types::GeneratedMarkerKind::ClockDiscontinuity { .. } => true,
                gt_types::GeneratedMarkerKind::GnssFixRegained { .. } => false,
            };
            if show_point
                && let Some(point) = track
                    .points
                    .iter()
                    .find(|p| p.tpv.time().utc() == marker.time)
            {
                ui.separator();
                crate::tpv_renderer::show_hover_table(ui, point);
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
        let transform =
            crate::transform::MercTransform::new(projector, map_memory, ui.max_rect().center());

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
            if !gt_filter::track_passes_filter(&track.metadata, self.filter) {
                continue;
            }
            let Some(marker) = sp.point_index.get(&track.generated_markers) else {
                continue;
            };
            if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                track: sp.track_ref(),
                category: DataCategory::GeneratedMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc);
            let highlighted = self.is_point_highlighted(point_ref);
            let fade =
                track_renderer::track_fade_alpha(self.highlight, sp.file_index, sp.track_index);
            draw_generated_marker(ui, screen_pos, marker.kind, highlighted, fade);
        }

        // Show tooltip for the hovered generated marker. Suppressed when the primary
        // hover is already a TPV point - the TPV tooltip covers the same data and
        // showing both would produce two overlapping labels at the same map position.
        let primary_is_tpv = matches!(
            self.highlight.hover,
            Some(HighlightScope::Point(r)) if r.category == DataCategory::Tpv
        );
        if !primary_is_tpv
            && let Some(r) = self.highlight.hover_candidates[3]
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && !self.highlight.suppress_hover_labels
            && let Some(file) = r.track.fi.get(self.files)
            && let Some(track) = r.track.index.get(&file.tracks)
            && let Some(marker) = r.point_index.get(&track.generated_markers)
        {
            let pos = transform.to_screen(marker.merc);
            self.show_tooltip(ui, r, pos);
        }
    }
}

/// Returns the header string for a generated-marker tooltip or hover section.
///
/// Centralises the "GNSS fix regained after Xs" formatting so both the live
/// tooltip (`show_tooltip`) and the multi-hover compound label
/// (`draw_candidate_section`) always produce identical text.
pub(crate) fn generated_marker_header(kind: gt_types::GeneratedMarkerKind) -> String {
    match kind {
        gt_types::GeneratedMarkerKind::GnssFixRegained { fix_lost_duration } => format!(
            "{kind} after {}",
            format_fix_duration(fix_lost_duration.num_milliseconds())
        ),
        gt_types::GeneratedMarkerKind::ClockDiscontinuity { step } => {
            let ms = step.num_milliseconds();
            let sign = if ms < 0 { "-" } else { "+" };
            // saturating_abs, not abs: avoids the i64::MIN panic surface, matching
            // the structural .abs() avoidance in the detector.
            format!(
                "{kind} ({sign}{})",
                format_fix_duration(ms.saturating_abs())
            )
        }
        gt_types::GeneratedMarkerKind::GnssFixLost => kind.to_string(),
    }
}

/// Formats `total_ms` milliseconds as a human-readable duration string (e.g. `"12.3s"`, `"1m30s"`).
/// Negative values are clamped to zero.
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
    fade: f32,
) {
    let painter = ui.painter();
    let (bg, stroke_color) = match kind {
        gt_types::GeneratedMarkerKind::GnssFixLost => {
            (Color32::from_rgb(219, 68, 55), Color32::WHITE)
        }
        gt_types::GeneratedMarkerKind::GnssFixRegained { .. } => {
            (Color32::from_rgb(15, 157, 88), Color32::WHITE)
        }
        gt_types::GeneratedMarkerKind::ClockDiscontinuity { .. } => {
            (Color32::from_rgb(255, 149, 0), Color32::WHITE)
        }
    };
    let faded_bg = track_renderer::apply_fade_alpha(bg, fade);
    let faded_stroke = track_renderer::apply_fade_alpha(stroke_color, fade);
    let radius = if highlighted { 11.0 } else { 8.0 };
    painter.circle_filled(center, radius, faded_bg);
    painter.circle_stroke(center, radius, Stroke::new(1.5, faded_stroke));
    if highlighted {
        painter.circle_stroke(
            center,
            radius + 3.5,
            Stroke::new(1.5, Color32::from_rgb(100, 200, 255)),
        );
    }
    let s = 4.0;
    match kind {
        gt_types::GeneratedMarkerKind::GnssFixLost => {
            let st = Stroke::new(2.0, faded_stroke);
            painter.line_segment([center - egui::vec2(s, s), center + egui::vec2(s, s)], st);
            painter.line_segment([center + egui::vec2(-s, s), center + egui::vec2(s, -s)], st);
        }
        gt_types::GeneratedMarkerKind::GnssFixRegained { .. } => {
            let st = Stroke::new(2.0, faded_stroke);
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
        gt_types::GeneratedMarkerKind::ClockDiscontinuity { .. } => {
            // Exclamation mark: an anomaly to inspect.
            let st = Stroke::new(2.0, faded_stroke);
            painter.line_segment(
                [
                    center - egui::vec2(0.0, s),
                    center + egui::vec2(0.0, s * 0.2),
                ],
                st,
            );
            painter.circle_filled(center + egui::vec2(0.0, s * 0.85), 1.3, faded_stroke);
        }
    }
}
