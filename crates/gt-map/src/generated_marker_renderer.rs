use egui::{Color32, Pos2, Response, Stroke, Ui};
use gt_types::{
    DataCategory, DataPointRef, GeneratedMarkerKind, GlobalFilter, HighlightScope, LoadedFile,
    MapHighlight, MercBounds, TripDataVisibility,
};
use std::cell::Cell;
use std::rc::Rc;
use walkers::{MapMemory, Plugin, Projector};

const HOVER_THRESHOLD: f32 = 10.0;

pub struct GeneratedMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
}

impl<'a> GeneratedMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TripDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            hover_out,
        }
    }

    fn is_point_highlighted(&self, point_ref: DataPointRef) -> bool {
        if self.highlight.sticky.is_some_and(|r| r == point_ref) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::Point(r)) => r == point_ref,
            Some(HighlightScope::Trip {
                file_index,
                trip_index,
            }) => file_index == point_ref.file_index && trip_index == point_ref.trip_index,
            Some(HighlightScope::TripCategory {
                file_index,
                trip_index,
                category,
            }) => {
                file_index == point_ref.file_index
                    && trip_index == point_ref.trip_index
                    && category == DataCategory::GeneratedMarker
            }
            _ => false,
        }
    }

    fn show_tooltip(&self, ui: &Ui, local_closest: Option<(DataPointRef, Pos2)>) {
        let Some((point_ref, pos)) = local_closest else {
            return;
        };
        let Some(file) = self.files.get(point_ref.file_index.0) else {
            return;
        };
        let Some(trip) = file.trips.get(point_ref.trip_index.0) else {
            return;
        };
        let Some(marker) = trip.generated_markers.get(point_ref.point_index.0) else {
            return;
        };
        let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
        let response = ui.interact(
            hit_rect,
            ui.id()
                .with("gen_marker_hover")
                .with(point_ref.file_index.0)
                .with(point_ref.trip_index.0)
                .with(point_ref.point_index.0),
            egui::Sense::hover(),
        );
        response.show_tooltip_ui(|ui| match marker.kind {
            GeneratedMarkerKind::GpsFixLost => {
                ui.strong("GPS fix lost");
                let corresponding = trip
                    .points
                    .iter()
                    .find(|p| p.tpv.time().utc() == marker.time);
                if let Some(point) = corresponding {
                    ui.separator();
                    crate::tpv_renderer::show_hover_table(ui, point);
                }
            }
            GeneratedMarkerKind::GpsFixRegained => {
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

        crate::marker_iter::for_each_visible_map_point(
            self.files,
            self.visibility,
            self.filter,
            &self.hover_out,
            hover_pos,
            &transform,
            vp_bounds,
            |trip, trip_vis| {
                trip_vis.generated_markers_visible.then_some((
                    DataCategory::GeneratedMarker,
                    trip.generated_markers.as_slice(),
                ))
            },
            |point_ref, screen_pos, marker| {
                if let Some(mouse) = hover_pos {
                    // Use squared distance to avoid sqrt; threshold is HOVER_THRESHOLD² = 100.
                    let dist_sq = screen_pos.distance_sq(mouse);
                    if dist_sq < HOVER_THRESHOLD * HOVER_THRESHOLD {
                        let is_closer = local_closest.as_ref().is_none_or(|(_, closest_pos)| {
                            closest_pos.distance_sq(mouse) > dist_sq
                        });
                        if is_closer {
                            local_closest = Some((point_ref, screen_pos));
                        }
                    }
                }
                let highlighted = self.is_point_highlighted(point_ref);
                draw_generated_marker(ui, screen_pos, marker.kind, highlighted);
            },
        );
        self.show_tooltip(ui, local_closest);
    }
}

pub fn update_hover_candidate(
    hover_out: &Rc<Cell<Option<(DataPointRef, f32)>>>,
    screen_pos: Pos2,
    hover_pos: Option<Pos2>,
    point_ref: DataPointRef,
) {
    if let Some(mouse) = hover_pos {
        // Use squared distance to avoid sqrt; stored value is dist² for consistent comparison.
        let dist_sq = screen_pos.distance_sq(mouse);
        if dist_sq < HOVER_THRESHOLD * HOVER_THRESHOLD
            && hover_out.get().is_none_or(|(_, d)| dist_sq < d)
        {
            hover_out.set(Some((point_ref, dist_sq)));
        }
    }
}

/// Formats a duration (given in milliseconds) for display in "fix regained" tooltips.
/// Under 1 minute: shows seconds with up to 2 decimal places, no trailing zeros.
/// 1 minute or more: shows "XmYs" (omitting seconds if zero).
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

fn draw_generated_marker(ui: &Ui, center: Pos2, kind: GeneratedMarkerKind, highlighted: bool) {
    let painter = ui.painter();
    let (bg, stroke_color) = match kind {
        GeneratedMarkerKind::GpsFixLost => (Color32::from_rgb(219, 68, 55), Color32::WHITE),
        GeneratedMarkerKind::GpsFixRegained => (Color32::from_rgb(15, 157, 88), Color32::WHITE),
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
        GeneratedMarkerKind::GpsFixLost => {
            let st = Stroke::new(2.0, stroke_color);
            painter.line_segment([center - egui::vec2(s, s), center + egui::vec2(s, s)], st);
            painter.line_segment([center + egui::vec2(-s, s), center + egui::vec2(s, -s)], st);
        }
        GeneratedMarkerKind::GpsFixRegained => {
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
