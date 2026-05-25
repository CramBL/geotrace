use egui::{Color32, Pos2, Response, Stroke, Ui};
use nav_types::filter;
use nav_types::satellites::Constellation;
use nav_types::{
    DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, NavPoint,
    TripDataVisibility,
};
use std::cell::Cell;
use std::rc::Rc;
use uom::si::angle::degree;
use uom::si::velocity::kilometer_per_hour;
use walkers::{MapMemory, Plugin, Projector};

use crate::generated_marker_renderer;

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

    #[expect(
        clippy::too_many_arguments,
        reason = "render context requires all parameters"
    )]
    fn render_trip(
        &self,
        ui: &Ui,
        hover_pos: Option<Pos2>,
        view_rect: egui::Rect,
        fi: usize,
        ti: usize,
        points: &[NavPoint],
        local_closest: &mut Option<(DataPointRef, Pos2)>,
        outline_alpha: f32,
        transform: &crate::MercTransform,
    ) {
        let mut last_label_pos: Option<Pos2> = None;
        for (pi, point) in points.iter().enumerate() {
            if !filter::point_passes_time_filter(point.tpv.time(), self.filter) {
                continue;
            }
            // For points with a GPS fix, use the pre-computed Mercator coordinates
            // (cached at load time) to avoid per-frame trig. Ghost points (fix==0)
            // interpolate their lat/lon, normalise to Mercator, then use the same
            // transform path — no walkers projector involved, ensuring consistent
            // f64 precision for all point types.
            let fix = point.fix_count();
            let (screen_pos, color) = if fix >= 10 {
                (
                    transform.to_screen(point.merc_x, point.merc_y),
                    Color32::from_rgb(66, 133, 244),
                )
            } else if fix > 0 {
                (
                    transform.to_screen(point.merc_x, point.merc_y),
                    Color32::from_rgb(244, 180, 0),
                )
            } else {
                let (lat, lon) = interpolate_position(points, pi);
                let (merc_x, merc_y) = crate::normalize_merc(lon, lat);
                (
                    transform.to_screen(merc_x, merc_y),
                    Color32::from_rgb(219, 68, 55),
                )
            };
            let point_ref = DataPointRef {
                file_index: fi,
                trip_index: ti,
                category: DataCategory::Tpv,
                point_index: pi,
            };
            generated_marker_renderer::update_hover_candidate(
                &self.hover_out,
                screen_pos,
                hover_pos,
                point_ref,
            );
            if let Some(mouse) = hover_pos {
                // Use squared distance to avoid sqrt; threshold is HOVER_THRESHOLD² = 100.
                let dist_sq = screen_pos.distance_sq(mouse);
                if dist_sq < HOVER_THRESHOLD * HOVER_THRESHOLD
                    && local_closest
                        .as_ref()
                        .is_none_or(|(_, p)| p.distance_sq(mouse) > dist_sq)
                {
                    *local_closest = Some((point_ref, screen_pos));
                }
            }
            if !view_rect.contains(screen_pos) {
                continue;
            }
            let highlighted = self.is_arrow_highlighted(point_ref);
            match point.tpv.heading() {
                Some(h) => {
                    draw_navigation_arrow(
                        ui,
                        screen_pos,
                        h.get::<degree>(),
                        color,
                        highlighted,
                        outline_alpha,
                    );
                }
                None => {
                    draw_ghost_circle(ui, screen_pos, color, highlighted, outline_alpha);
                }
            }
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
        map_memory: &MapMemory,
    ) {
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let view_rect = ui.max_rect().expand(50.0);
        let mut local_closest: Option<(DataPointRef, Pos2)> = None;

        // Scale the white outline alpha with zoom level: fully visible when zoomed
        // in (zoom ≥ 14), fades to transparent when zoomed out (zoom ≤ 10).
        // This prevents the outlines from blending into a white mass at low zoom.
        let zoom = map_memory.zoom();
        let outline_alpha = ((zoom - 10.0) / 4.0).clamp(0.0, 1.0) as f32;

        // Build the per-frame coordinate transform once.
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

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
                if !filter::trip_passes_filter(&trip.metadata, self.filter) {
                    continue;
                }
                self.render_trip(
                    ui,
                    hover_pos,
                    view_rect,
                    fi,
                    ti,
                    &trip.points,
                    &mut local_closest,
                    outline_alpha,
                    &transform,
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
            ui.label("Time");
            ui.label(p.tpv.time().format("%Y-%m-%d %H:%M:%S").to_string());
            ui.end_row();

            ui.label("Speed");
            match p.tpv.velocity() {
                Some(v) => ui.label(format!("{:.1} km/h", v.get::<kilometer_per_hour>())),
                None => ui.label("\u{2014}"), // em-dash: speed unknown (interpolated point)
            };
            ui.end_row();

            ui.label("Heading");
            match p.tpv.heading() {
                Some(h) => ui.label(format!("{:.1}\u{00b0}", h.get::<degree>())),
                None => ui.label("\u{2014}"), // em-dash: unknown direction
            };
            ui.end_row();

            show_satellite_rows(ui, p);
        });
}

/// Content for the sticky popup window when a TPV point is clicked.
/// Unlike `show_hover_table`, the time is omitted here because it is shown
/// in the window title. The satellite section expands into a full per-PRN
/// breakdown grouped by constellation.
pub(crate) fn show_sticky_tpv_content(ui: &mut Ui, p: &NavPoint) {
    // Basic metrics (2-column grid).
    egui::Grid::new("sticky_tpv_basic")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Speed");
            match p.tpv.velocity() {
                Some(v) => {
                    ui.label(format!("{:.1} km/h", v.get::<kilometer_per_hour>()));
                }
                None => {
                    ui.label("\u{2014}");
                }
            };
            ui.end_row();

            ui.label("Heading");
            match p.tpv.heading() {
                Some(h) => {
                    ui.label(format!("{:.1}\u{00b0}", h.get::<degree>()));
                }
                None => {
                    ui.label("\u{2014}");
                }
            };
            ui.end_row();

            match &p.satellites {
                Some(sats) => {
                    let fix = sats.fix_count();
                    let seen = sats.satellite_count();
                    ui.label("Satellites");
                    ui.horizontal(|ui| {
                        ui.colored_label(fix_count_color(fix), fix.to_string());
                        ui.label("/");
                        ui.colored_label(seen_count_color(seen), seen.to_string());
                    });
                    ui.end_row();
                }
                None => {
                    ui.label("Satellites");
                    ui.colored_label(Color32::RED, "NO FIX");
                    ui.end_row();
                }
            }
        });

    // Comprehensive per-PRN satellite table grouped by constellation.
    if let Some(sats) = &p.satellites {
        ui.add_space(6.0);

        // Collect non-empty constellations up-front. `Satellite` is `Copy` so
        // we own the data and can borrow-free inside the layout closures.
        let groups: Vec<(usize, &str, &str, Vec<nav_types::satellites::Satellite>)> = [
            (0usize, "GPS", "G", Constellation::Gps),
            (1, "Galileo", "E", Constellation::Galileo),
            (2, "GLONASS", "R", Constellation::Glonass),
            (3, "BeiDou", "C", Constellation::Beidou),
        ]
        .iter()
        .filter_map(|&(id, name, prefix, constellation)| {
            let mut const_sats: Vec<_> = sats.by_constellation(constellation).copied().collect();
            if const_sats.is_empty() {
                return None;
            }
            const_sats.sort_by_key(|s| s.prn());
            Some((id, name, prefix, const_sats))
        })
        .collect();

        // Two constellation panels per row; each panel sizes to its own content.
        for chunk in groups.chunks(2) {
            ui.horizontal_top(|ui| {
                for (panel_i, (id, name, prefix, const_sats)) in chunk.iter().enumerate() {
                    if panel_i > 0 {
                        ui.add_space(12.0);
                    }
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{name} ({})", const_sats.len())).strong(),
                        );
                        egui::Grid::new(("sticky_sats", *id))
                            .num_columns(3)
                            .striped(true)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("PRN").weak().small());
                                ui.label(egui::RichText::new("SNR").weak().small());
                                ui.label(egui::RichText::new("Fix").weak().small());
                                ui.end_row();

                                for sat in const_sats {
                                    let in_fix = sat.in_fix();
                                    let prn_color = if in_fix {
                                        Color32::GREEN
                                    } else {
                                        Color32::GRAY
                                    };
                                    ui.label(
                                        egui::RichText::new(format!("{}{:02}", prefix, sat.prn()))
                                            .color(prn_color),
                                    );
                                    match sat.snr() {
                                        Some(snr) => {
                                            ui.label(
                                                egui::RichText::new(format!("{snr:.1}"))
                                                    .color(snr_color(snr)),
                                            );
                                        }
                                        None => {
                                            ui.label(
                                                egui::RichText::new("\u{2014}")
                                                    .color(Color32::from_gray(110)),
                                            );
                                        }
                                    }
                                    if in_fix {
                                        ui.label(
                                            egui::RichText::new(egui_phosphor::regular::CHECK)
                                                .color(Color32::GREEN),
                                        );
                                    } else {
                                        ui.label("");
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                }
            });
            ui.add_space(6.0);
        }
    }
}

fn show_satellite_rows(ui: &mut Ui, p: &NavPoint) {
    if let Some(sats) = &p.satellites {
        let fix = sats.fix_count();
        let seen = sats.satellite_count();

        // Total summary row — bold to signal it is the aggregate.
        ui.label(egui::RichText::new("Satellites").strong());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(fix.to_string())
                    .color(fix_count_color(fix))
                    .strong(),
            );
            ui.label(egui::RichText::new("/").strong());
            ui.label(
                egui::RichText::new(seen.to_string())
                    .color(seen_count_color(seen))
                    .strong(),
            );
        });
        ui.end_row();

        // Per-constellation breakdown — each with its own colored fix/seen counts.
        for constellation in [
            Constellation::Gps,
            Constellation::Galileo,
            Constellation::Glonass,
            Constellation::Beidou,
        ] {
            let const_total = sats.by_constellation(constellation).count() as u32;
            if const_total == 0 {
                continue;
            }
            let const_fix = sats
                .satellites_with_fix()
                .filter(|s| s.constellation() == constellation)
                .count() as u32;
            let label = match constellation {
                Constellation::Gps => "GPS",
                Constellation::Galileo => "Galileo",
                Constellation::Glonass => "GLONASS",
                Constellation::Beidou => "BeiDou",
            };
            ui.label(label);
            ui.horizontal(|ui| {
                ui.colored_label(fix_count_color(const_fix), const_fix.to_string());
                ui.label("/");
                ui.colored_label(seen_count_color(const_total), const_total.to_string());
            });
            ui.end_row();
        }
    } else {
        ui.label("Sats");
        ui.colored_label(Color32::RED, "NO FIX");
        ui.end_row();
    }
}

/// Map an SNR value (dB-Hz) to a colour on a green → red gradient.
///
/// Typical GPS SNR ranges:
/// - ≥ 40 dB-Hz: excellent lock
/// - 35–40: good
/// - 30–35: moderate
/// - 25–30: weak
/// - < 25: very weak / marginal
fn snr_color(snr: f32) -> Color32 {
    if snr >= 40.0 {
        Color32::from_rgb(0, 200, 0) // green — excellent
    } else if snr >= 35.0 {
        Color32::from_rgb(120, 200, 0) // yellow-green — good
    } else if snr >= 30.0 {
        Color32::from_rgb(220, 200, 0) // yellow — moderate
    } else if snr >= 25.0 {
        Color32::from_rgb(255, 140, 0) // orange — weak
    } else {
        Color32::from_rgb(220, 60, 0) // red — very weak
    }
}

/// Color for the "fix used" count in the satellite badge.
fn fix_count_color(count: u32) -> Color32 {
    if count == 0 {
        Color32::RED
    } else if count <= 2 {
        Color32::from_rgb(255, 140, 0) // orange
    } else if count <= 4 {
        Color32::YELLOW
    } else {
        Color32::GREEN
    }
}

/// Color for the "total seen" count in the satellite badge.
fn seen_count_color(count: u32) -> Color32 {
    if count < 5 {
        Color32::from_rgb(255, 140, 0) // orange
    } else if count < 8 {
        Color32::YELLOW
    } else {
        Color32::GREEN
    }
}

/// Render a hollow circle for ghost/extrapolated fixes with no known heading.
fn draw_ghost_circle(ui: &Ui, center: Pos2, color: Color32, highlighted: bool, outline_alpha: f32) {
    let radius = if highlighted { 8.0 } else { 6.0 };
    let stroke_color = if highlighted {
        Color32::from_rgb(100, 200, 255)
    } else {
        // Fade out the white outline as we zoom out to avoid a white-mass effect.
        Color32::from_rgba_unmultiplied(255, 255, 255, alpha_u8(outline_alpha))
    };
    let stroke_width = if highlighted {
        2.0
    } else {
        1.5 * outline_alpha
    };
    if stroke_width > 0.0 {
        ui.painter()
            .circle_stroke(center, radius, Stroke::new(stroke_width, stroke_color));
    }
    ui.painter().circle_filled(center, 2.5, color);
}

fn draw_navigation_arrow(
    ui: &Ui,
    center: Pos2,
    heading_degrees: f64,
    color: Color32,
    highlighted: bool,
    outline_alpha: f32,
) {
    let angle_rad = heading_degrees.to_radians() - std::f64::consts::FRAC_PI_2;
    let dir = egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32);
    let perp = egui::vec2(-dir.y, dir.x);

    let size = if highlighted { 17.0 } else { 12.0 };
    let stroke_color = if highlighted {
        Color32::from_rgb(100, 200, 255)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, alpha_u8(outline_alpha))
    };
    let stroke_width = if highlighted {
        2.0
    } else {
        1.5 * outline_alpha
    };

    // Shift the whole triangle forward so the tip is prominent (arrowhead look)
    // while keeping the rear as a straight base — three segments, no concave notch.
    let center_offset = dir * (size * 0.4);
    let tip = center + dir * size - center_offset;
    let left = center - dir * size - perp * (size * 0.7) - center_offset;
    let right = center - dir * size + perp * (size * 0.7) - center_offset;

    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, right, left],
        color,
        Stroke::new(stroke_width, stroke_color),
    ));
}

/// Convert a [0.0, 1.0] alpha value to a u8. The `clamp` call guarantees the
/// result is in [0, 255], so sign loss is impossible despite the lint warning.
#[inline]
fn alpha_u8(alpha: f32) -> u8 {
    #[expect(
        clippy::cast_sign_loss,
        reason = "value is clamped to [0.0,1.0] so the product is always non-negative"
    )]
    let v = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    v
}
