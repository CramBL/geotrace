use egui::{Color32, Pos2, Response, Stroke, Ui};
use nav_types::satellites::Constellation;
use nav_types::{
    DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, NavPoint,
    TripDataVisibility, point_passes_time_filter, trip_passes_filter,
};
use std::cell::Cell;
use std::rc::Rc;
use uom::si::angle::degree;
use uom::si::velocity::kilometer_per_hour;
use walkers::{MapMemory, Plugin, Position, Projector};

use crate::generated_marker_renderer::update_hover_candidate;

const HOVER_THRESHOLD: f32 = 10.0;
const MIN_LABEL_DIST: f32 = 60.0;

pub struct TpvRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TripDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    hover_out: Rc<Cell<Option<(DataPointRef, f32)>>>,
}

impl<'a> TpvRenderer<'a> {
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

    fn is_arrow_highlighted(&self, point_ref: DataPointRef) -> bool {
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
                    && category == DataCategory::Tpv
            }
            _ => false,
        }
    }

    fn point_color(&self, p: &NavPoint, points: &[NavPoint], idx: usize) -> (f64, f64, Color32) {
        let fix = p.fix_count();
        if fix >= 10 {
            (
                p.tpv.lat().get::<degree>(),
                p.tpv.lon().get::<degree>(),
                Color32::from_rgb(66, 133, 244),
            )
        } else if fix > 0 {
            (
                p.tpv.lat().get::<degree>(),
                p.tpv.lon().get::<degree>(),
                Color32::from_rgb(244, 180, 0),
            )
        } else {
            let (lat, lon) = interpolate_position(points, idx);
            (lat, lon, Color32::from_rgb(219, 68, 55))
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "render context requires all parameters"
    )]
    fn render_trip(
        &self,
        ui: &Ui,
        projector: &Projector,
        hover_pos: Option<Pos2>,
        view_rect: egui::Rect,
        fi: usize,
        ti: usize,
        points: &[NavPoint],
        local_closest: &mut Option<(DataPointRef, Pos2)>,
    ) {
        let mut last_label_pos: Option<Pos2> = None;
        for (pi, point) in points.iter().enumerate() {
            if !point_passes_time_filter(point.tpv.time(), self.filter) {
                continue;
            }
            let (lat, lon, color) = self.point_color(point, points, pi);
            let screen_pos = projector.project(Position::new(lon, lat)).to_pos2();
            let point_ref = DataPointRef {
                file_index: fi,
                trip_index: ti,
                category: DataCategory::Tpv,
                point_index: pi,
            };
            update_hover_candidate(&self.hover_out, screen_pos, hover_pos, point_ref);
            if let Some(mouse) = hover_pos
                && screen_pos.distance(mouse) < HOVER_THRESHOLD
                && local_closest
                    .as_ref()
                    .is_none_or(|_| screen_pos.distance(mouse) < HOVER_THRESHOLD)
            {
                *local_closest = Some((point_ref, screen_pos));
            }
            if !view_rect.contains(screen_pos) {
                continue;
            }
            let highlighted = self.is_arrow_highlighted(point_ref);
            draw_navigation_arrow(
                ui,
                screen_pos,
                point.tpv.heading().get::<degree>(),
                color,
                highlighted,
            );
            if let Some(sats) = &point.satellites {
                let show =
                    last_label_pos.is_none_or(|last| screen_pos.distance(last) > MIN_LABEL_DIST);
                if show {
                    let label = format!("{}/{}", sats.fix_count(), sats.satellite_count());
                    let text_pos = screen_pos + egui::vec2(15.0, -15.0);
                    let galley = ui.painter().layout_no_wrap(
                        label,
                        egui::FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                    let text_rect = egui::Rect::from_min_size(
                        egui::pos2(text_pos.x, text_pos.y - galley.size().y),
                        galley.size(),
                    );
                    ui.painter().rect_filled(
                        text_rect.expand(2.0),
                        2.0,
                        Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                    );
                    ui.painter().galley(text_rect.min, galley, Color32::WHITE);
                    last_label_pos = Some(screen_pos);
                }
            }
        }
    }

    fn show_tooltip(&self, ui: &Ui, local_closest: Option<(DataPointRef, Pos2)>) {
        let Some((point_ref, pos)) = local_closest else {
            return;
        };
        let Some(file) = self.files.get(point_ref.file_index) else {
            return;
        };
        let Some(trip) = file.trips.get(point_ref.trip_index) else {
            return;
        };
        let Some(point) = trip.points.get(point_ref.point_index) else {
            return;
        };
        let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
        let response = ui.interact(
            hit_rect,
            ui.id().with("tpv_hover").with(point_ref.point_index),
            egui::Sense::hover(),
        );
        response.show_tooltip_ui(|ui| {
            show_hover_table(ui, point);
        });
    }
}

impl Plugin for TpvRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        _map_memory: &MapMemory,
    ) {
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let view_rect = ui.max_rect().expand(50.0);
        let mut local_closest: Option<(DataPointRef, Pos2)> = None;

        for (fi, file) in self.files.iter().enumerate() {
            let Some(file_vis) = self.visibility.files.get(fi) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            for (ti, trip) in file.trips.iter().enumerate() {
                let Some(trip_vis) = file_vis.trips.get(ti) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.tpv_visible {
                    continue;
                }
                if !trip_passes_filter(&trip.metadata, self.filter) {
                    continue;
                }
                self.render_trip(
                    ui,
                    projector,
                    hover_pos,
                    view_rect,
                    fi,
                    ti,
                    &trip.points,
                    &mut local_closest,
                );
            }
        }
        self.show_tooltip(ui, local_closest);
    }
}

fn interpolate_position(points: &[NavPoint], idx: usize) -> (f64, f64) {
    let prev = (0..idx)
        .rev()
        .find(|&i| points.get(i).is_some_and(|p| p.fix_count() > 0));
    let next = (idx + 1..points.len()).find(|&i| points.get(i).is_some_and(|p| p.fix_count() > 0));

    match (prev, next) {
        (Some(pi), Some(ni)) => match (points.get(pi), points.get(ni), points.get(idx)) {
            (Some(prev_pt), Some(next_pt), Some(curr_pt)) => {
                let t_total = (next_pt.tpv.time() - prev_pt.tpv.time()).num_seconds() as f64;
                let t_curr = (curr_pt.tpv.time() - prev_pt.tpv.time()).num_seconds() as f64;
                if t_total > 0.0 {
                    let f = t_curr / t_total;
                    let lat = prev_pt.tpv.lat().get::<degree>()
                        + (next_pt.tpv.lat().get::<degree>() - prev_pt.tpv.lat().get::<degree>())
                            * f;
                    let lon = prev_pt.tpv.lon().get::<degree>()
                        + (next_pt.tpv.lon().get::<degree>() - prev_pt.tpv.lon().get::<degree>())
                            * f;
                    (lat, lon)
                } else {
                    (
                        curr_pt.tpv.lat().get::<degree>(),
                        curr_pt.tpv.lon().get::<degree>(),
                    )
                }
            }
            _ => (0.0, 0.0),
        },
        (Some(pi), None) => points.get(pi).map_or((0.0, 0.0), |p| {
            (p.tpv.lat().get::<degree>(), p.tpv.lon().get::<degree>())
        }),
        (None, Some(ni)) => points.get(ni).map_or((0.0, 0.0), |p| {
            (p.tpv.lat().get::<degree>(), p.tpv.lon().get::<degree>())
        }),
        (None, None) => points.get(idx).map_or((0.0, 0.0), |p| {
            (p.tpv.lat().get::<degree>(), p.tpv.lon().get::<degree>())
        }),
    }
}

pub(crate) fn show_hover_table(ui: &mut Ui, p: &NavPoint) {
    egui::Grid::new("hover_grid")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Time:");
            ui.label(p.tpv.time().format("%Y-%m-%d %H:%M:%S").to_string());
            ui.end_row();

            ui.label("Speed:");
            let vel = p
                .tpv
                .velocity()
                .map_or(0.0, |v| v.get::<kilometer_per_hour>());
            ui.label(format!("{vel:.1} km/h"));
            ui.end_row();

            ui.label("Heading:");
            ui.label(format!("{:.1}\u{00b0}", p.tpv.heading().get::<degree>()));
            ui.end_row();

            show_satellite_rows(ui, p);
        });
}

fn show_satellite_rows(ui: &mut Ui, p: &NavPoint) {
    if let Some(sats) = &p.satellites {
        ui.label("Satellites:");
        ui.label(format!("{}/{}", sats.fix_count(), sats.satellite_count()));
        ui.end_row();

        for constellation in [
            Constellation::Gps,
            Constellation::Galileo,
            Constellation::Glonass,
            Constellation::Beidou,
        ] {
            let count = sats.by_constellation(constellation).count();
            if count > 0 {
                let fix_count = sats
                    .satellites_with_fix()
                    .filter(|s| s.constellation() == constellation)
                    .count();
                let max_snr = sats.max_snr_by_constellation(constellation);
                ui.label(format!("{constellation:?}:"));
                ui.vertical(|ui| {
                    ui.label(format!("Fix/Seen: {fix_count}/{count}"));
                    if let Some(snr) = max_snr {
                        ui.label(format!("Max SNR: {snr:.1}"));
                    }
                });
                ui.end_row();
            }
        }
    } else {
        ui.label("Satellites:");
        ui.colored_label(Color32::RED, "NO FIX");
        ui.end_row();
    }
}

fn draw_navigation_arrow(
    ui: &Ui,
    center: Pos2,
    heading_degrees: f64,
    color: Color32,
    highlighted: bool,
) {
    let angle_rad = heading_degrees.to_radians() - std::f64::consts::FRAC_PI_2;
    let dir = egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32);
    let perp = egui::vec2(-dir.y, dir.x);

    let size = if highlighted { 17.0 } else { 12.0 };
    let stroke_color = if highlighted {
        Color32::from_rgb(100, 200, 255)
    } else {
        Color32::WHITE
    };
    let stroke_width = if highlighted { 2.0 } else { 1.5 };

    let center_offset = dir * (size * 0.4);
    let tip = center + dir * size - center_offset;
    let left = center - dir * size - perp * (size * 0.7) - center_offset;
    let right = center - dir * size + perp * (size * 0.7) - center_offset;
    let back_indent = center - dir * (size * 0.2) - center_offset;

    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, right, back_indent, left],
        color,
        Stroke::new(stroke_width, stroke_color),
    ));
}
