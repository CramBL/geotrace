use egui::{Color32, Pos2, Response, Stroke, Ui};
use uom::si::angle::degree;
use uom::si::velocity::kilometer_per_hour;
use walkers::{MapMemory, Plugin, Position, Projector};

use nav_types::NavPoint;

pub struct TpvRenderer<'a> {
    points: &'a [NavPoint],
}

impl<'a> TpvRenderer<'a> {
    pub fn new(points: &'a [NavPoint]) -> Self {
        Self { points }
    }
}

impl<'a> Plugin for TpvRenderer<'a> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        _map_memory: &MapMemory,
    ) {
        let view_rect = ui.max_rect().expand(50.0);
        let mut last_label_pos: Option<Pos2> = None;
        let min_label_dist = 60.0;

        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let mut closest_point: Option<(usize, Pos2, f32)> = None;
        let hover_threshold = 20.0;

        // Pre-calculate segments for path drawing
        let mut path_points = Vec::new();

        for (i, p) in self.points.iter().enumerate() {
            let Some((lat, lon, color)) = self.get_point_data(i) else {
                continue;
            };
            let position = Position::new(lon, lat);
            let screen_pos = projector.project(position).to_pos2();

            path_points.push(screen_pos);

            if !view_rect.contains(screen_pos) {
                continue;
            }

            let heading_deg = p.tpv.heading().get::<degree>();
            draw_navigation_arrow(ui, screen_pos, heading_deg, color);

            // Satellite Label: FIX/TOTAL
            if let Some(sats) = &p.satellites {
                let show_label = last_label_pos
                    .is_none_or(|last_pos| screen_pos.distance(last_pos) > min_label_dist);

                if show_label {
                    let label = format!("{}/{}", sats.fix_count(), sats.satellite_count());
                    ui.painter().text(
                        screen_pos + egui::vec2(15.0, -15.0),
                        egui::Align2::LEFT_BOTTOM,
                        label,
                        egui::FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                    last_label_pos = Some(screen_pos);
                }
            }

            // Track closest point for hover
            if let Some(mouse_pos) = hover_pos {
                let dist = screen_pos.distance(mouse_pos);
                if dist < hover_threshold {
                    if let Some((_, _, d)) = closest_point {
                        if dist < d {
                            closest_point = Some((i, screen_pos, dist));
                        }
                    } else {
                        closest_point = Some((i, screen_pos, dist));
                    }
                }
            }
        }

        // Show tooltip for the closest point
        if let Some((idx, pos, _)) = closest_point
            && let Some(p) = self.points.get(idx)
        {
            let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
            let response = ui.interact(
                hit_rect,
                ui.id().with("nav_hover").with(idx),
                egui::Sense::hover(),
            );
            response.show_tooltip_ui(|ui| {
                self.show_hover_table(ui, p);
            });
        }

        // Draw path
        if path_points.len() > 1 {
            ui.painter().add(egui::Shape::line(
                path_points,
                Stroke::new(2.0, Color32::from_white_alpha(100)),
            ));
        }
    }
}

impl<'a> TpvRenderer<'a> {
    fn get_point_data(&self, idx: usize) -> Option<(f64, f64, Color32)> {
        let p = self.points.get(idx)?;
        let fix_count = p.fix_count();

        if fix_count >= 10 {
            Some((
                p.tpv.lat().get::<degree>(),
                p.tpv.lon().get::<degree>(),
                Color32::from_rgb(66, 133, 244), // Blue
            ))
        } else if fix_count > 0 {
            Some((
                p.tpv.lat().get::<degree>(),
                p.tpv.lon().get::<degree>(),
                Color32::from_rgb(244, 180, 0), // Yellow
            ))
        } else {
            // Interpolate
            let (lat, lon) = self.interpolate_position(idx);
            Some((lat, lon, Color32::from_rgb(219, 68, 55))) // Red
        }
    }

    fn interpolate_position(&self, idx: usize) -> (f64, f64) {
        let mut prev_fix = None;
        for i in (0..idx).rev() {
            if self.points.get(i).is_some_and(|p| p.fix_count() > 0) {
                prev_fix = Some(i);
                break;
            }
        }

        let mut next_fix = None;
        for i in (idx + 1)..self.points.len() {
            if self.points.get(i).is_some_and(|p| p.fix_count() > 0) {
                next_fix = Some(i);
                break;
            }
        }

        match (prev_fix, next_fix) {
            (Some(prev_idx), Some(next_idx)) => {
                if let (Some(p_prev), Some(p_next), Some(p_curr)) = (
                    self.points.get(prev_idx),
                    self.points.get(next_idx),
                    self.points.get(idx),
                ) {
                    let t_total = (p_next.tpv.time() - p_prev.tpv.time()).num_seconds() as f64;
                    let t_curr = (p_curr.tpv.time() - p_prev.tpv.time()).num_seconds() as f64;

                    if t_total > 0.0 {
                        let factor = t_curr / t_total;
                        let lat = p_prev.tpv.lat().get::<degree>()
                            + (p_next.tpv.lat().get::<degree>() - p_prev.tpv.lat().get::<degree>())
                                * factor;
                        let lon = p_prev.tpv.lon().get::<degree>()
                            + (p_next.tpv.lon().get::<degree>() - p_prev.tpv.lon().get::<degree>())
                                * factor;
                        (lat, lon)
                    } else {
                        (
                            p_curr.tpv.lat().get::<degree>(),
                            p_curr.tpv.lon().get::<degree>(),
                        )
                    }
                } else {
                    (0.0, 0.0)
                }
            }
            (Some(prev_idx), None) => self.points.get(prev_idx).map_or((0.0, 0.0), |p| {
                (p.tpv.lat().get::<degree>(), p.tpv.lon().get::<degree>())
            }),
            (None, Some(next_idx)) => self.points.get(next_idx).map_or((0.0, 0.0), |p| {
                (p.tpv.lat().get::<degree>(), p.tpv.lon().get::<degree>())
            }),
            (None, None) => self.points.get(idx).map_or((0.0, 0.0), |p| {
                (p.tpv.lat().get::<degree>(), p.tpv.lon().get::<degree>())
            }),
        }
    }

    fn show_hover_table(&self, ui: &mut Ui, p: &NavPoint) {
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
                ui.label(format!("{:.1} km/h", vel));
                ui.end_row();

                ui.label("Heading:");
                ui.label(format!("{:.1}°", p.tpv.heading().get::<degree>()));
                ui.end_row();

                if let Some(sats) = &p.satellites {
                    ui.label("Satellites:");
                    ui.label(format!("{}/{}", sats.fix_count(), sats.satellite_count()));
                    ui.end_row();

                    for constellation in [
                        nav_types::satellites::Constellation::Gps,
                        nav_types::satellites::Constellation::Galileo,
                        nav_types::satellites::Constellation::Glonass,
                        nav_types::satellites::Constellation::Beidou,
                    ] {
                        let count = sats.by_constellation(constellation).count();
                        if count > 0 {
                            let fix_count = sats
                                .satellites_with_fix()
                                .filter(|s| s.constellation() == constellation)
                                .count();
                            let max_snr = sats.max_snr_by_constellation(constellation);

                            ui.label(format!("{:?}:", constellation));
                            ui.vertical(|ui| {
                                ui.label(format!("Fix/Seen: {}/{}", fix_count, count));
                                if let Some(snr) = max_snr {
                                    ui.label(format!("Max SNR: {:.1}", snr));
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
            });
    }
}

fn draw_navigation_arrow(ui: &Ui, center: Pos2, heading_degrees: f64, color: Color32) {
    let angle_rad = heading_degrees.to_radians() - std::f64::consts::FRAC_PI_2;
    let dir = egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32);
    let perp = egui::vec2(-dir.y, dir.x);

    let size = 12.0;
    let center_offset = dir * (size * 0.4);

    let tip = center + dir * size - center_offset;
    let left = center - dir * size - perp * (size * 0.7) - center_offset;
    let right = center - dir * size + perp * (size * 0.7) - center_offset;
    let back_indent = center - dir * (size * 0.2) - center_offset;

    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, right, back_indent, left],
        color,
        Stroke::new(1.5, Color32::WHITE),
    ));
}
