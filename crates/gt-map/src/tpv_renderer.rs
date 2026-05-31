use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, PopupAnchor, Pos2, Response, Stroke, Ui};
use gt_types::filter;
use gt_types::satellites::Constellation;
use gt_types::{
    DataCategory, DataPointRef, FileIdx, GlobalFilter, HighlightScope, LoadedFile, LoadedTrack,
    MapHighlight, NavPoint, PointIdx, SpatialPoint, TrackDataVisibility, TrackIdx,
};
use std::collections::HashMap;
use uom::si::angle::degree;
use uom::si::velocity::kilometer_per_hour;
use walkers::{MapMemory, Plugin, Projector};
const EM_DASH: &str = "—";
const DEGREE_SIGN: &str = "°";
/// U+0394 GREEK CAPITAL LETTER DELTA — used as a mathematical difference symbol.
const DELTA: &str = "Δ";
/// U+2212 MINUS SIGN — visually distinct from the ASCII hyphen-minus.
const MINUS_SIGN: &str = "−";

pub struct TpvRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    visible_tpv: Vec<SpatialPoint>,
}

impl<'a> TpvRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        visible_tpv: Vec<SpatialPoint>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            visible_tpv,
        }
    }

    fn is_arrow_highlighted(&self, point_ref: DataPointRef) -> bool {
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
                    && category == DataCategory::Tpv
            }
            _ => false,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "render context requires all parameters; a context struct would not add clarity"
    )]
    fn render_trip(
        &self,
        ui: &Ui,
        view_rect: egui::Rect,
        fi: FileIdx,
        ti: TrackIdx,
        track: &LoadedTrack,
        real_fix_indices: Option<&Vec<usize>>,
        style: &TpvDrawStyle,
        transform: &crate::MercTransform,
    ) {
        let mut last_label_pos: Option<Pos2> = None;

        // Real fixes: indices come from the global R-tree viewport query.
        if let Some(indices) = real_fix_indices {
            for &pi in indices {
                #[expect(
                    clippy::indexing_slicing,
                    reason = "index from global RTree built over track.points, so always in bounds"
                )]
                let point = &track.points[pi];
                if !filter::point_passes_time_filter(point.tpv.time().utc(), self.filter) {
                    continue;
                }
                let Some(h) = point.tpv.heading() else {
                    continue;
                };
                let screen_pos = transform.to_screen(point.merc_x, point.merc_y);
                let point_ref = DataPointRef {
                    file_index: fi,
                    track_index: ti,
                    category: DataCategory::Tpv,
                    point_index: PointIdx(pi),
                };
                let eph_m = point.tpv.eph_m();
                let pixels_per_meter = if eph_m.is_some() {
                    transform.pixels_per_meter(point.tpv.lat().as_degrees())
                } else {
                    0.0
                };
                draw_tpv_point(
                    ui,
                    screen_pos,
                    &PointKind::Real {
                        color: tpv_point_color(point),
                        heading_deg: h.get::<degree>(),
                    },
                    eph_m,
                    pixels_per_meter,
                    point.satellites.as_ref(),
                    self.is_arrow_highlighted(point_ref),
                    style,
                    &mut last_label_pos,
                );
            }
        }

        // Ghost fixes (heading == None): position interpolated at render time,
        // not in the global tree. Rare in practice.
        for (pi, point) in track.points.iter().enumerate() {
            if point.tpv.heading().is_some() {
                continue;
            }
            if !filter::point_passes_time_filter(point.tpv.time().utc(), self.filter) {
                continue;
            }
            let (lat, lon) = interpolate_position(&track.points, pi);
            let (merc_x, merc_y) = crate::normalize_merc(lon, lat);
            let screen_pos = transform.to_screen(merc_x, merc_y);
            if !view_rect.contains(screen_pos) {
                continue;
            }
            let point_ref = DataPointRef {
                file_index: fi,
                track_index: ti,
                category: DataCategory::Tpv,
                point_index: PointIdx(pi),
            };
            draw_tpv_point(
                ui,
                screen_pos,
                &PointKind::Ghost,
                None,
                0.0,
                None,
                self.is_arrow_highlighted(point_ref),
                style,
                &mut last_label_pos,
            );
        }
    }

    fn show_tooltip(&self, ui: &Ui, point_ref: DataPointRef) {
        let Some(file) = self.files.get(point_ref.file_index.0) else {
            return;
        };
        let Some(track) = file.tracks.get(point_ref.track_index.0) else {
            return;
        };
        let Some(point) = track.points.get(point_ref.point_index.0) else {
            return;
        };
        let tooltip_id = ui
            .id()
            .with("tpv_hover")
            .with(point_ref.file_index.0)
            .with(point_ref.track_index.0)
            .with(point_ref.point_index.0);
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            tooltip_id,
            PopupAnchor::Pointer,
        )
        .show(|ui| {
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
        let view_rect = ui.max_rect().expand(50.0);

        // Scale icon sizes and outline alpha with zoom level.
        //
        // At low zoom (≤ 12) icons are reduced to small dots so that dense
        // clusters of points — e.g. highway driving at 1-second resolution —
        // have visible air between them instead of blending into a solid mass.
        // At high zoom (≥ 18) icons reach their full design size.
        //
        // The outline alpha fades out below zoom 14 separately, following the
        // same principle: avoid a white mass at low zoom.
        let zoom = map_memory.zoom();
        let size_factor = ((zoom - 12.0) / 6.0).clamp(0.0, 1.0) as f32;
        let style = TpvDrawStyle {
            base_arrow_size: 3.0 + size_factor * 9.0, // 3 px at zoom ≤ 12, 12 px at zoom ≥ 18
            base_ghost_radius: 2.0 + size_factor * 4.0, // 2 px at zoom ≤ 12,  6 px at zoom ≥ 18
            // Require more pixel separation between satellite-count labels when
            // zoomed out so the label count doesn't explode at dense clusters.
            min_label_dist: 60.0 + (1.0 - size_factor) * 120.0, // 180 px at low zoom, 60 at high
            outline_alpha: ((zoom - 10.0) / 4.0).clamp(0.0, 1.0) as f32,
        };

        // Build the per-frame coordinate transform once.
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

        // Group visible real fixes by trip so render_trip gets O(k/trips) per trip.
        let mut by_trip: HashMap<(FileIdx, TrackIdx), Vec<usize>> = HashMap::new();
        for sp in &self.visible_tpv {
            by_trip
                .entry((sp.file_index, sp.track_index))
                .or_default()
                .push(sp.point_index.0);
        }

        for (fi, file) in self.files.iter().enumerate() {
            let fi = FileIdx(fi);
            let Some(file_vis) = self.visibility.files.get(fi.0) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            for (ti, track) in file.tracks.iter().enumerate() {
                let ti = TrackIdx(ti);
                let Some(trip_vis) = file_vis.tracks.get(ti.0) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.tpv_visible {
                    continue;
                }
                if !filter::track_passes_filter(&track.metadata, self.filter) {
                    continue;
                }
                self.render_trip(
                    ui,
                    view_rect,
                    fi,
                    ti,
                    track,
                    by_trip.get(&(fi, ti)),
                    &style,
                    &transform,
                );
            }
        }

        // Show TPV tooltip for the currently hovered point (set by NavMap the previous frame).
        // Suppressed when the sticky popup is already showing this exact point (the window is
        // in Order::Middle; the tooltip is in Order::Tooltip, so it would paint over the popup),
        // and suppressed when any popup (e.g. the context menu) is open.
        if let Some(HighlightScope::Point(r)) = self.highlight.hover
            && r.category == DataCategory::Tpv
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
        {
            self.show_tooltip(ui, r);
        }

        // Cross-highlight: when the trip plot cursor is active, draw a ring
        // indicator around the pre-computed closest point.
        // The app layer computes (fi, ti, pi) via find_closest_tpv and stores
        // it in MapHighlight::plot_hover_point — no O(n) scan needed here.
        if let Some((fi, ti, pi)) = self.highlight.plot_hover_point
            && let Some(point) = self
                .files
                .get(fi.0)
                .and_then(|f| f.tracks.get(ti.0))
                .and_then(|t| t.points.get(pi.0))
        {
            let pos = transform.to_screen(point.merc_x, point.merc_y);
            let painter = ui.painter();
            painter.circle_stroke(
                pos,
                style.base_arrow_size + 6.0,
                egui::Stroke::new(
                    2.0,
                    egui::Color32::from_rgba_unmultiplied(100, 200, 255, 230),
                ),
            );
            painter.circle_stroke(
                pos,
                style.base_arrow_size + 3.0,
                egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(100, 200, 255, 120),
                ),
            );
        }
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
                    let lat = prev_pt.tpv.lat().as_degrees()
                        + (next_pt.tpv.lat().as_degrees() - prev_pt.tpv.lat().as_degrees()) * f;
                    let lon = prev_pt.tpv.lon().as_degrees()
                        + (next_pt.tpv.lon().as_degrees() - prev_pt.tpv.lon().as_degrees()) * f;
                    (lat, lon)
                } else {
                    (
                        curr_pt.tpv.lat().as_degrees(),
                        curr_pt.tpv.lon().as_degrees(),
                    )
                }
            }
            _ => (0.0, 0.0),
        },
        (Some(pi), None) => points.get(pi).map_or((0.0, 0.0), |p| {
            (p.tpv.lat().as_degrees(), p.tpv.lon().as_degrees())
        }),
        (None, Some(ni)) => points.get(ni).map_or((0.0, 0.0), |p| {
            (p.tpv.lat().as_degrees(), p.tpv.lon().as_degrees())
        }),
        (None, None) => points.get(idx).map_or((0.0, 0.0), |p| {
            (p.tpv.lat().as_degrees(), p.tpv.lon().as_degrees())
        }),
    }
}

pub(crate) fn show_hover_table(ui: &mut Ui, p: &NavPoint) {
    egui::Grid::new("hover_grid")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Time");
            ui.label(p.tpv.time().utc().format("%Y-%m-%d %H:%M:%S").to_string());
            ui.end_row();

            ui.label("Speed");
            match p.tpv.velocity() {
                Some(v) => ui.label(format!("{:.1} km/h", v.get::<kilometer_per_hour>())),
                None => ui.label(EM_DASH), // em-dash: speed unknown (interpolated point)
            };
            ui.end_row();

            ui.label("Heading");
            match p.tpv.heading() {
                Some(h) => ui.label(format!("{:.1}{DEGREE_SIGN}", h.get::<degree>())),
                None => ui.label(EM_DASH), // em-dash: unknown direction
            };
            ui.end_row();

            if let Some(eph) = p.tpv.eph_m() {
                ui.label("Accuracy");
                ui.label(format!("±{eph:.1} m"));
                ui.end_row();
            }

            show_satellite_rows(ui, p);

            // Time delta between the GPS fix and the satellite report.
            // Only shown when the satellite report was GPS-timestamped — if it
            // only has sys_time, this delta equals the GPS/sys-clock delta below
            // and showing it would be redundant.
            if let Some(sats) = &p.satellites
                && let Some(sat_gps_time) = sats.gps_time()
            {
                let sat_delta_ms = (p.tpv.time() - sat_gps_time).num_milliseconds();
                if sat_delta_ms != 0 {
                    ui.label(format!("Sat {DELTA}t"));
                    ui.label(format_signed_delta(sat_delta_ms));
                    ui.end_row();
                }
            }

            // GPS/system-clock delta (if system timestamp is available).
            if let Some(sys) = p.tpv.sys_time() {
                let clock_delta_ms = p.tpv.time().offset_from_sys(sys).num_milliseconds();
                ui.label(format!("Clock {DELTA}t"));
                ui.label(format!(
                    "{} ({})",
                    format_signed_delta(clock_delta_ms),
                    if clock_delta_ms > 0 {
                        "GPS ahead"
                    } else if clock_delta_ms < 0 {
                        "system ahead"
                    } else {
                        "in sync"
                    }
                ));
                ui.end_row();
            }
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
                    ui.label(EM_DASH);
                }
            };
            ui.end_row();

            ui.label("Heading");
            match p.tpv.heading() {
                Some(h) => {
                    ui.label(format!("{:.1}{DEGREE_SIGN}", h.get::<degree>()));
                }
                None => {
                    ui.label(EM_DASH);
                }
            };
            ui.end_row();

            if let Some(eph) = p.tpv.eph_m() {
                ui.label("Accuracy");
                ui.label(format!("±{eph:.1} m"));
                ui.end_row();
            }

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

                    // Time delta between the GPS fix and the satellite report.
                    // A nonzero delta means the satellite data is from a slightly
                    // different moment than the fix — worth showing for diagnostics.
                    if let Some(sat_gps_time) = sats.gps_time() {
                        let sat_delta_ms = (p.tpv.time() - sat_gps_time).num_milliseconds();
                        if sat_delta_ms != 0 {
                            ui.label(format!("Sat {DELTA}t"));
                            ui.label(format_signed_delta(sat_delta_ms));
                            ui.end_row();
                        }
                    }
                }
                None => {
                    // No satellite report for this point — omit the row.
                    // A missing report does not mean there was no GPS fix.
                }
            }

            // GPS/system-clock delta: how far the GPS clock leads the host clock.
            // Only shown when the fix carries a system timestamp.
            if let Some(sys) = p.tpv.sys_time() {
                let clock_delta_ms = p.tpv.time().offset_from_sys(sys).num_milliseconds();
                ui.label(format!("Clock {DELTA}t"));
                ui.label(format_signed_delta(clock_delta_ms));
                ui.end_row();
            }
        });

    // Comprehensive per-PRN satellite table grouped by constellation.
    if let Some(sats) = &p.satellites {
        ui.add_space(6.0);

        // Collect non-empty constellations up-front. `Satellite` is `Copy` so
        // we own the data and can borrow-free inside the layout closures.
        let groups: Vec<(usize, &str, &str, Vec<gt_types::satellites::Satellite>)> = [
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
                                                egui::RichText::new(EM_DASH)
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
    // Only show satellite rows when a report is actually attached to this point.
    // Omit the section entirely when there is no report — a missing report
    // does not mean there was no GPS fix, just that no satellite data was
    // captured or associated for this particular point.
    let Some(sats) = &p.satellites else {
        return;
    };

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
}

/// Format a signed time delta (in milliseconds) for display in the point info panel.
///
/// - Sub-second deltas are shown as `+250ms` / `−50ms` (using `MINUS_SIGN`
///   so the negative sign is visually distinct from a hyphen).
/// - Larger deltas use a compact terse format (e.g. `+1s`, `+1m30s`) with a
///   leading sign.
fn format_signed_delta(delta_ms: i64) -> String {
    use std::fmt::Write as _;
    let sign = if delta_ms < 0 { MINUS_SIGN } else { "+" };
    let abs_ms = delta_ms.unsigned_abs();
    if abs_ms < 1_000 {
        format!("{sign}{abs_ms}ms")
    } else {
        let total_s = abs_ms / 1_000;
        let h = total_s / 3_600;
        let m = (total_s % 3_600) / 60;
        let s = total_s % 60;
        let mut out = sign.to_owned();
        if h > 0 {
            write!(out, "{h}h").ok();
        }
        if m > 0 {
            write!(out, "{m}m").ok();
        }
        if s > 0 || (h == 0 && m == 0) {
            write!(out, "{s}s").ok();
        }
        out
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

/// Zoom-derived visual parameters computed once per frame and shared across
/// all points in all trips.
struct TpvDrawStyle {
    outline_alpha: f32,
    base_arrow_size: f32,
    base_ghost_radius: f32,
    min_label_dist: f32,
}

/// Renders the three visual layers for a single on-screen GPS point:
/// the horizontal-accuracy circle, the directional icon (arrow or ghost), and
/// the satellite-count label.
///
/// `last_label_pos` is updated when a label is drawn so that the caller can
/// throttle label density across consecutive points.
#[expect(
    clippy::too_many_arguments,
    reason = "three independent drawing concerns each need distinct parameters; no natural grouping below 9"
)]
fn draw_tpv_point(
    ui: &Ui,
    screen_pos: Pos2,
    point_kind: &PointKind,
    eph_m: Option<f32>,
    pixels_per_meter: f64,
    satellites: Option<&gt_types::satellites::Satellites>,
    highlighted: bool,
    style: &TpvDrawStyle,
    last_label_pos: &mut Option<Pos2>,
) {
    // Accuracy circle — rendered beneath the icon.
    if let Some(eph_m) = eph_m {
        let radius = (f64::from(eph_m) * pixels_per_meter) as f32;
        if radius >= 2.0 {
            ui.painter().circle_filled(
                screen_pos,
                radius,
                egui::Color32::from_rgba_unmultiplied(30, 120, 255, 20),
            );
            ui.painter().circle_stroke(
                screen_pos,
                radius,
                egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(30, 120, 255, 60)),
            );
        }
    }

    // Directional icon.
    match point_kind {
        PointKind::Real { color, heading_deg } => {
            draw_navigation_arrow(
                ui,
                screen_pos,
                *heading_deg,
                *color,
                highlighted,
                style.outline_alpha,
                style.base_arrow_size,
            );
        }
        PointKind::Ghost => {
            draw_ghost_circle(
                ui,
                screen_pos,
                Color32::from_rgb(219, 68, 55),
                highlighted,
                style.outline_alpha,
                style.base_ghost_radius,
            );
        }
    }

    // Satellite-count label — throttled to avoid over-dense clusters.
    if let Some(sats) = satellites {
        let show =
            last_label_pos.is_none_or(|last| screen_pos.distance(last) > style.min_label_dist);
        if show {
            let label = format!("{}/{}", sats.fix_count(), sats.satellite_count());
            let text_pos =
                screen_pos + egui::vec2(style.base_arrow_size + 3.0, -style.base_arrow_size);
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
            *last_label_pos = Some(screen_pos);
        }
    }
}

/// Map a real (non-ghost) GPS point to its arrow colour based on satellite fix quality.
///
/// Three-tier scheme:
/// - **Blue** — strong fix (≥ 10 satellites) or no satellite report attached (unknown quality).
/// - **Yellow** — marginal fix (1–9 satellites in fix).
/// - **Red** — fix lost: satellite report present but zero satellites are in the fix.
///
/// Ghost/interpolated points (heading == `None`) are a separate visual category
/// rendered as red circles by `draw_ghost_circle`; they never reach this function.
fn tpv_point_color(point: &NavPoint) -> Color32 {
    match &point.satellites {
        None => Color32::from_rgb(66, 133, 244), // no satellite data — assume fine, show blue
        Some(sats) => match sats.fix_count() {
            n if n >= 10 => Color32::from_rgb(66, 133, 244), // blue: strong fix
            n if n > 0 => Color32::from_rgb(244, 180, 0),    // yellow: marginal fix
            _ => Color32::from_rgb(219, 68, 55),             // red: fix lost
        },
    }
}

/// Classifies a GPS point for a single render pass, carrying everything the
/// draw step needs so `render_trip` only matches `heading()` once.
enum PointKind {
    /// Real GPS fix — heading known, precomputed Mercator coordinates used.
    Real { color: Color32, heading_deg: f64 },
    /// Ghost/synthetic fix — heading absent, position interpolated from neighbours.
    Ghost,
}

/// Render a hollow circle for ghost/extrapolated fixes with no known heading.
fn draw_ghost_circle(
    ui: &Ui,
    center: Pos2,
    color: Color32,
    highlighted: bool,
    outline_alpha: f32,
    base_radius: f32,
) {
    let radius = if highlighted {
        base_radius + 2.0
    } else {
        base_radius
    };
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
    // Inner dot scales with the outer radius so the circle doesn't look hollow
    // at very small sizes.
    let dot_radius = (base_radius * 0.4).max(1.0);
    ui.painter().circle_filled(center, dot_radius, color);
}

fn draw_navigation_arrow(
    ui: &Ui,
    center: Pos2,
    heading_degrees: f64,
    color: Color32,
    highlighted: bool,
    outline_alpha: f32,
    base_size: f32,
) {
    let angle_rad = heading_degrees.to_radians() - std::f64::consts::FRAC_PI_2;
    let dir = egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32);
    let perp = egui::vec2(-dir.y, dir.x);

    let size = if highlighted {
        base_size + 3.0
    } else {
        base_size
    };
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

    // A car-GPS / Google-Maps style navigation arrow.
    // Vertices (dir points up = forward direction of travel):
    //
    //           *          tip  (+size forward)
    //          / \
    //         /   \
    //        /     \
    //       /   *   \        notch (0.1·size back — concave, pulled toward tip)
    //      /   / \   \
    //     *   /   \   *      wings (0.4·size back, ±0.5·size wide)
    //
    // The outer edges (/ \) run from the tip all the way down to the wings.
    // The inner edges (/ \) run from each wing up to the notch, creating the
    // concave dip at the rear centre.
    //
    // Because the shape is non-convex, the fill is drawn as two convex
    // triangles (tip–right–notch and tip–notch–left) and the outline as a
    // single closed PathShape.
    let tip = center + dir * size;
    let right = center - dir * (size * 0.4) + perp * (size * 0.5);
    let notch = center - dir * (size * 0.1);
    let left = center - dir * (size * 0.4) - perp * (size * 0.5);

    // Fill — two convex triangles avoid non-convex fill artefacts.
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, right, notch],
        color,
        Stroke::NONE,
    ));
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, notch, left],
        color,
        Stroke::NONE,
    ));

    // Outline — closed path drawn on top of the fill.
    if stroke_width > 0.0 {
        ui.painter().add(egui::Shape::Path(PathShape::closed_line(
            vec![tip, right, notch, left],
            PathStroke::new(stroke_width, stroke_color),
        )));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;
    use gt_types::NavPoint;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    fn make_point(satellites: Option<Satellites>) -> NavPoint {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(chrono::Utc::now()))
            .lat(Latitude::new(51.5))
            .lon(Longitude::new(-0.1))
            .heading(Angle::new::<degree>(90.0))
            .build();
        NavPoint::new(tpv, satellites)
    }

    fn sats_with_fix(fix_count: u32) -> Satellites {
        let satellites: Vec<_> = (1u32..=12)
            .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, prn <= fix_count))
            .collect();
        Satellites::new(None, None, satellites)
    }

    /// No satellite report → blue (unknown quality, assume fine).
    #[test]
    fn color_no_satellite_report_is_blue() {
        let point = make_point(None);
        assert_eq!(tpv_point_color(&point), Color32::from_rgb(66, 133, 244));
    }

    /// 10+ satellites in fix → blue (strong fix).
    #[test]
    fn color_strong_fix_is_blue() {
        let point = make_point(Some(sats_with_fix(10)));
        assert_eq!(tpv_point_color(&point), Color32::from_rgb(66, 133, 244));
    }

    /// 1–9 satellites in fix → yellow (marginal fix).
    #[test]
    fn color_marginal_fix_is_yellow() {
        let point = make_point(Some(sats_with_fix(5)));
        assert_eq!(tpv_point_color(&point), Color32::from_rgb(244, 180, 0));
    }

    /// 1 satellite in fix → yellow (lowest marginal threshold).
    #[test]
    fn color_single_sat_fix_is_yellow() {
        let point = make_point(Some(sats_with_fix(1)));
        assert_eq!(tpv_point_color(&point), Color32::from_rgb(244, 180, 0));
    }

    /// Satellite report present but 0 in fix → red (fix lost).
    #[test]
    fn color_fix_lost_is_red() {
        let point = make_point(Some(sats_with_fix(0)));
        assert_eq!(tpv_point_color(&point), Color32::from_rgb(219, 68, 55));
    }
}
