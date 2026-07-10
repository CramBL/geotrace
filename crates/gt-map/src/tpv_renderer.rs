use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, PopupAnchor, Pos2, Stroke, Ui, Vec2};
use gt_filter::{self as filter, GlobalFilter};
use gt_types::coordinates::Latitude;
use gt_types::satellites::Constellation;
use gt_types::{
    DataCategory, FileIdx, LoadedFile, LoadedTrack, NavPoint, PointIdx, TrackIdx, TrackRef,
};
use gt_ui_theme::{DEGREE_SIGN, DELTA, EM_DASH, MINUS_SIGN};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight};
use uom::si::angle::{degree, radian};
use uom::si::f64::Angle;
use uom::si::length::meter;

use crate::transform::MapScale;

/// Local on-screen fix spacing, in units of the icon size, at which a fix
/// icon is fully opaque (`HI`) respectively fully transparent (`LO`).
/// Between the two, the icon crossfades with the continuous quality line.
///
/// Icons are deliberately kept while they merely overlap - partially
/// overlapping arrows are still readable. Fading starts only below half an
/// icon size of spacing and completes when neighbouring arrows share almost
/// all of their pixels, at which point skipping them also keeps the
/// tessellated vertex count bounded by screen content instead of recording
/// size.
///
/// The spacing is measured per fix (see [`local_fix_spacing_px`]), not per
/// track, so a parked phase fades out without dragging down the rest of the
/// track.
const ICON_FADE_HI_SPACING_FACTOR: f32 = 0.5;
const ICON_FADE_LO_SPACING_FACTOR: f32 = 0.2;

/// Absolute floors for the fade band, in pixels. At low zoom the proportional
/// thresholds collapse below one pixel of spacing, where packed icons are
/// already unreadable and better replaced by the quality line. Fading completes
/// below [`ICON_FADE_LO_MIN_SPACING_PX`] and starts below
/// [`ICON_FADE_HI_MIN_SPACING_PX`]. The HI floor exceeding the LO floor keeps
/// the band non-empty for any icon size.
const ICON_FADE_LO_MIN_SPACING_PX: f32 = 2.0;
const ICON_FADE_HI_MIN_SPACING_PX: f32 = 5.0;

/// Number of discrete opacity steps for the quality line's crossfade.
/// Per-point line alphas are quantized to this many levels so that long
/// stretches share one key and stay mergeable into single polyline spans.
const QUALITY_LINE_ALPHA_STEPS: u8 = 3;

/// Stroke width of the continuous fix-quality line that stands in for the
/// fix icons when they fade out - slightly thicker than the 3 px trackline
/// underneath so the quality colors stay readable on top of it.
pub(crate) const QUALITY_LINE_WIDTH: f32 = 5.0;

/// Accuracy circles with a smaller pixel radius than this are skipped - they
/// would be invisible at that size.
const MIN_ACCURACY_CIRCLE_RADIUS_PX: f32 = 2.0;

/// An accuracy circle smaller than this fraction of the directional icon's
/// size is entirely covered by the icon, so drawing it is wasted geometry.
const ACCURACY_CIRCLE_MIN_VISIBLE_FACTOR: f32 = 0.5;

/// Fix-quality palette shared by the per-fix icons and the continuous
/// quality line.
const FIX_STRONG_BLUE: Color32 = Color32::from_rgb(66, 133, 244);
const FIX_MARGINAL_YELLOW: Color32 = Color32::from_rgb(244, 180, 0);
const FIX_LOST_RED: Color32 = Color32::from_rgb(219, 68, 55);

fn is_arrow_highlighted(highlight: &MapHighlight, point_ref: DataPointRef) -> bool {
    if highlight.sticky.is_some_and(|r| r == point_ref) {
        return true;
    }
    match highlight.hover {
        Some(HighlightScope::Point(r)) => r == point_ref,
        Some(HighlightScope::Track(track)) => track == point_ref.track,
        Some(HighlightScope::TrackCategory { track, category }) => {
            track == point_ref.track && category == DataCategory::Tpv
        }
        _ => false,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "render context requires all parameters; a context struct would not add clarity"
)]
pub(crate) fn draw_track_icons(
    ui: &Ui,
    view_rect: egui::Rect,
    fi: FileIdx,
    ti: TrackIdx,
    track: &LoadedTrack,
    real_fix_indices: Option<&Vec<usize>>,
    ghost_points: &[usize],
    style: &TpvDrawStyle,
    fade: TrackIconFade,
    transform: &crate::transform::MercTransform,
    highlight: &MapHighlight,
    filter: &GlobalFilter,
) {
    // Real fixes: indices come from the global R-tree viewport query.
    if let Some(indices) = real_fix_indices {
        for &pi in indices {
            #[expect(
                clippy::indexing_slicing,
                reason = "index from global RTree built over track.points, so always in bounds"
            )]
            let point = &track.points[pi];
            if !filter::point_passes_time_filter(point.tpv.time().utc(), filter) {
                continue;
            }
            let Some(h) = point.tpv.heading() else {
                continue;
            };
            // Fix-lost points (0 satellites in fix) are drawn by the ghost loop.
            if point.is_ghost_fix() {
                continue;
            }
            let screen_pos = transform.to_screen(point.merc);
            let icon_alpha = fix_icon_alpha(
                fade,
                track,
                pi,
                screen_pos,
                style.base_arrow_size,
                transform,
            );
            if icon_alpha <= 0.0 {
                continue;
            }
            let point_style = TpvDrawStyle {
                icon_alpha,
                ..*style
            };
            let point_ref = DataPointRef {
                track: TrackRef::new(fi, ti),
                category: DataCategory::Tpv,
                point_index: PointIdx::new(pi),
            };
            let eph_m = point.tpv.eph_m();
            let pixels_per_meter = if eph_m.is_some() {
                transform.pixels_per_meter(point.tpv.lat())
            } else {
                0.0
            };
            draw_tpv_point(
                ui,
                screen_pos,
                &PointKind::Real {
                    color: tpv_point_color(point),
                    heading: h,
                },
                eph_m,
                pixels_per_meter,
                is_arrow_highlighted(highlight, point_ref),
                &point_style,
            );
        }
    }

    // Ghost fixes: heading absent, or satellite fix count dropped to zero.
    // The latter covers devices that continue outputting positions and headings
    // during fix loss - the heading field is present but unreliable as a
    // "real" direction indicator, so we still show a hollow chevron.
    // `ghost_points` was collected on the LOD level during the geometry
    // walk (time-filtered there). Ghost/real transitions survive every
    // level, so faded stretches lose only sub-pixel interior chevrons.
    for (pi, point) in ghost_points
        .iter()
        .filter_map(|&pi| track.points.get(pi).map(|p| (pi, p)))
    {
        let screen_pos = transform.to_screen(point.merc);
        if !view_rect.contains(screen_pos) {
            continue;
        }
        let icon_alpha = fix_icon_alpha(
            fade,
            track,
            pi,
            screen_pos,
            style.base_arrow_size,
            transform,
        );
        if icon_alpha <= 0.0 {
            continue;
        }
        let point_style = TpvDrawStyle {
            icon_alpha,
            ..*style
        };
        // Direction for the chevron:
        // - If the GPS reported a heading (fix-lost but device maintained estimate),
        //   use it - it is more accurate than deriving from neighbour positions.
        // - Otherwise derive from neighbouring Mercator positions (Mercator y
        //   increases southward, matching egui y-down, so no Y-flip needed).
        let direction = if let Some(h) = point.tpv.heading() {
            let angle_rad = h.get::<radian>() - std::f64::consts::FRAC_PI_2;
            egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32)
        } else {
            let merc_prev = pi
                .checked_sub(1)
                .and_then(|i| track.points.get(i))
                .map_or(point.merc, |p| p.merc);
            let merc_next = track.points.get(pi + 1).map_or(point.merc, |p| p.merc);
            ghost_direction(merc_prev, merc_next)
        };
        let point_ref = DataPointRef {
            track: TrackRef::new(fi, ti),
            category: DataCategory::Tpv,
            point_index: PointIdx::new(pi),
        };
        draw_tpv_point(
            ui,
            screen_pos,
            &PointKind::Ghost { direction },
            None,
            0.0,
            is_arrow_highlighted(highlight, point_ref),
            &point_style,
        );
    }
}

/// Show the hover tooltip for the given TPV point. When the point lies
/// inside a query match, `match_header` renders the match context above the
/// point table.
pub(crate) fn show_tooltip(
    ui: &Ui,
    files: &[LoadedFile],
    point_ref: DataPointRef,
    match_header: Option<impl FnOnce(&mut Ui)>,
) {
    let Some(file) = point_ref.track.fi.get(files) else {
        return;
    };
    let Some(track) = point_ref.track.index.get(&file.tracks) else {
        return;
    };
    let Some(point) = point_ref.point_index.get(&track.points) else {
        return;
    };
    let tooltip_id = ui
        .id()
        .with("tpv_hover")
        .with(point_ref.track)
        .with(point_ref.point_index);
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        tooltip_id,
        PopupAnchor::Pointer,
    )
    .show(|ui| {
        if let Some(header) = match_header {
            header(ui);
        }
        show_hover_table(ui, point);
    });
}

/// Zoom-derived visual parameters computed once per frame and shared
/// across all tracks: icon sizes scale with zoom (see [`base_arrow_size`])
/// and the outline alpha fades out below zoom 14 so dense clusters don't
/// blend into a white mass.
pub(crate) fn frame_style(zoom: f64) -> TpvDrawStyle {
    let size_factor = zoom_size_factor(zoom);
    TpvDrawStyle {
        base_arrow_size: base_arrow_size(zoom),
        // Larger label-collision cells when zoomed out so the label count
        // doesn't explode at dense clusters.
        label_cell_px: 60.0 + (1.0 - size_factor) * 120.0, // 180 px at low zoom, 60 at high
        outline_alpha: ((zoom - 10.0) / 4.0).clamp(0.0, 1.0) as f32,
        icon_alpha: 1.0,
    }
}

/// Cross-highlight: when the track plot cursor is active, draw a ring
/// indicator around the pre-computed closest point. The app layer computes
/// the point via find_closest_tpv and stores it in
/// `MapHighlight::plot_hover_point` - no O(n) scan needed here.
pub(crate) fn draw_plot_hover_ring(
    ui: &Ui,
    files: &[LoadedFile],
    highlight: &MapHighlight,
    style: &TpvDrawStyle,
    transform: &crate::transform::MercTransform,
) {
    if let Some((fi, ti, pi)) = highlight.plot_hover_point
        && let Some(point) = fi
            .get(files)
            .and_then(|f| ti.get(&f.tracks))
            .and_then(|t| pi.get(&t.points))
    {
        let pos = transform.to_screen(point.merc);
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

pub(crate) fn show_hover_table(ui: &mut Ui, p: &NavPoint) {
    egui::Grid::new("hover_grid")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Time");
            ui.label(p.tpv.time().utc().format("%Y-%m-%d %H:%M:%S").to_string());
            ui.end_row();

            let lat = p.tpv.lat().as_degrees();
            let lon = p.tpv.lon().as_degrees();
            ui.label("Lat");
            ui.label(format!(
                "{:.6}{DEGREE_SIGN} {}",
                lat.abs(),
                if lat >= 0.0 { "N" } else { "S" }
            ));
            ui.end_row();
            ui.label("Lon");
            ui.label(format!(
                "{:.6}{DEGREE_SIGN} {}",
                lon.abs(),
                if lon >= 0.0 { "E" } else { "W" }
            ));
            ui.end_row();

            ui.label("Speed");
            match p.tpv.velocity_kmh() {
                Some(v) => ui.label(format!("{:.1} km/h", v)),
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
            // Only shown when the satellite report was GPS-timestamped - if it
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
            match p.tpv.velocity_kmh() {
                Some(v) => {
                    ui.label(format!("{:.1} km/h", v));
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
                    let dark_mode = ui.visuals().dark_mode;
                    ui.horizontal(|ui| {
                        ui.colored_label(fix_count_color(fix, dark_mode), fix.to_string());
                        ui.label("/");
                        ui.colored_label(seen_count_color(seen, dark_mode), seen.to_string());
                    });
                    ui.end_row();

                    // Time delta between the GPS fix and the satellite report.
                    // A nonzero delta means the satellite data is from a slightly
                    // different moment than the fix - worth showing for diagnostics.
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
                    // No satellite report for this point - omit the row.
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
            (0usize, "G", Constellation::Gps),
            (1, "E", Constellation::Galileo),
            (2, "R", Constellation::Glonass),
            (3, "C", Constellation::Beidou),
            (4, "I", Constellation::Navic),
            (5, "J", Constellation::Qzss),
        ]
        .iter()
        .filter_map(|&(id, prefix, constellation)| {
            let mut const_sats: Vec<_> = sats.by_constellation(constellation).copied().collect();
            if const_sats.is_empty() {
                return None;
            }
            const_sats.sort_by_key(|s| s.prn());
            Some((id, constellation.display_name(), prefix, const_sats))
        })
        .collect();

        // Two constellation panels per row, each panel sizes to its own content.
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

                                let dark_mode = ui.visuals().dark_mode;
                                // A satellite contributing to the fix reads as the
                                // "good" tier green; an idle one is de-emphasised
                                // with egui's own weak-text colour, which already
                                // tracks the theme.
                                let in_fix_color = gt_ui_theme::SatCountTier::Good.color(dark_mode);
                                let muted_color = ui.visuals().weak_text_color();
                                for sat in const_sats {
                                    let in_fix = sat.in_fix();
                                    let prn_color = if in_fix { in_fix_color } else { muted_color };
                                    ui.label(
                                        egui::RichText::new(format!("{}{:02}", prefix, sat.prn()))
                                            .color(prn_color),
                                    );
                                    match sat.snr() {
                                        Some(snr) => {
                                            ui.label(
                                                egui::RichText::new(format!("{:.1}", snr.value()))
                                                    .color(gt_ui_theme::snr_color(
                                                        snr.quality(),
                                                        dark_mode,
                                                    )),
                                            );
                                        }
                                        None => {
                                            ui.label(
                                                egui::RichText::new(EM_DASH).color(muted_color),
                                            );
                                        }
                                    }
                                    if in_fix {
                                        ui.label(
                                            egui::RichText::new(egui_phosphor::regular::CHECK)
                                                .color(in_fix_color),
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
    // Omit the section entirely when there is no report - a missing report
    // does not mean there was no GPS fix, just that no satellite data was
    // captured or associated for this particular point.
    let Some(sats) = &p.satellites else {
        return;
    };

    let fix = sats.fix_count();
    let seen = sats.satellite_count();
    let dark_mode = ui.visuals().dark_mode;

    // Total summary row - bold to signal it is the aggregate.
    ui.label(egui::RichText::new("Satellites").strong());
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(fix.to_string())
                .color(fix_count_color(fix, dark_mode))
                .strong(),
        );
        ui.label(egui::RichText::new("/").strong());
        ui.label(
            egui::RichText::new(seen.to_string())
                .color(seen_count_color(seen, dark_mode))
                .strong(),
        );
    });
    ui.end_row();

    // Per-constellation breakdown - each with its own colored fix/seen counts.
    for constellation in [
        Constellation::Gps,
        Constellation::Galileo,
        Constellation::Glonass,
        Constellation::Beidou,
        Constellation::Navic,
        Constellation::Qzss,
    ] {
        let const_total = sats.by_constellation(constellation).count() as u32;
        if const_total == 0 {
            continue;
        }
        let const_fix = sats
            .satellites_with_fix()
            .filter(|s| s.constellation() == constellation)
            .count() as u32;
        ui.label(constellation.display_name());
        ui.horizontal(|ui| {
            ui.colored_label(fix_count_color(const_fix, dark_mode), const_fix.to_string());
            ui.label("/");
            ui.colored_label(
                seen_count_color(const_total, dark_mode),
                const_total.to_string(),
            );
        });
        ui.end_row();
    }
}

/// Format a signed time delta (in milliseconds) for display in the point info panel.
///
/// - Sub-2-second deltas are shown as `+250ms` / `−1500ms`.
/// - 2s–59s: fractional seconds up to 2 decimal places with trailing zeros
///   dropped (`+2.1s`, `+9.23s`).
/// - ≥1 minute: compact terse format (`+1m9s`, `+1h2m`).
///
/// The negative sign uses `MINUS_SIGN` so it is visually distinct from a hyphen.
fn format_signed_delta(delta_ms: i64) -> String {
    use std::fmt::Write as _;
    let sign = if delta_ms < 0 { MINUS_SIGN } else { "+" };
    let abs_ms = delta_ms.unsigned_abs();
    if abs_ms < 2_000 {
        format!("{sign}{abs_ms}ms")
    } else if abs_ms < 60_000 {
        let secs = abs_ms / 1_000;
        let frac = (abs_ms % 1_000) / 10;
        if frac == 0 {
            format!("{sign}{secs}s")
        } else if frac.is_multiple_of(10) {
            format!("{sign}{secs}.{}s", frac / 10)
        } else {
            format!("{sign}{secs}.{frac:02}s")
        }
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

/// Color for the "fix used" count in the satellite badge, legible on the
/// current theme. Pass `ui.visuals().dark_mode`.
fn fix_count_color(count: u32, dark_mode: bool) -> Color32 {
    gt_ui_theme::fix_count_tier(count).color(dark_mode)
}

/// Color for the "total seen" count in the satellite badge, legible on the
/// current theme. Pass `ui.visuals().dark_mode`.
fn seen_count_color(count: u32, dark_mode: bool) -> Color32 {
    gt_ui_theme::seen_count_tier(count).color(dark_mode)
}

/// Zoom-derived visual parameters computed once per frame and shared across
/// all points in all tracks.
#[derive(Clone, Copy)]
pub(crate) struct TpvDrawStyle {
    pub(crate) outline_alpha: f32,
    pub(crate) base_arrow_size: f32,
    /// On-screen size, in pixels, of a satellite-label collision cell -
    /// at most one label draws per cell (see [`crate::sat_labels`]).
    pub(crate) label_cell_px: f32,
    /// Opacity of the fix icon currently being drawn, in (0.0, 1.0].
    /// Decided per fix from its local on-screen spacing (see
    /// [`fix_icon_alpha`]). Below 1.0 the icon is crossfading into the
    /// continuous quality line, and fully transparent icons are skipped
    /// before drawing.
    pub(crate) icon_alpha: f32,
}

/// Zoom range over which the fix icons scale from dot size up to their
/// full design size: dots at low zoom keep dense clusters from blending
/// into a solid mass.
const ICON_MIN_SIZE_ZOOM: f64 = 12.0;
const ICON_MAX_SIZE_ZOOM: f64 = 18.0;

/// Fix-icon size at [`ICON_MIN_SIZE_ZOOM`] and below respectively
/// [`ICON_MAX_SIZE_ZOOM`] and above.
const MIN_ARROW_SIZE_PX: f32 = 3.0;
const MAX_ARROW_SIZE_PX: f32 = 12.0;

/// Zoom interpolation factor for icon sizing: 0.0 at
/// [`ICON_MIN_SIZE_ZOOM`] and below, 1.0 at [`ICON_MAX_SIZE_ZOOM`] and
/// above, linear in between.
fn zoom_size_factor(zoom: f64) -> f32 {
    ((zoom - ICON_MIN_SIZE_ZOOM) / (ICON_MAX_SIZE_ZOOM - ICON_MIN_SIZE_ZOOM)).clamp(0.0, 1.0) as f32
}

/// Zoom-scaled base size of the per-fix icons. Shared with viewport
/// collection so fade is classified with the renderer's exact size.
pub(crate) fn base_arrow_size(zoom: f64) -> f32 {
    MIN_ARROW_SIZE_PX + zoom_size_factor(zoom) * (MAX_ARROW_SIZE_PX - MIN_ARROW_SIZE_PX)
}

/// How a track's fix icons fade this frame, decided O(1) from the track's
/// precomputed segment-length range before any per-point work.
///
/// At a given map scale every fix's local spacing lies between the track's
/// shortest and longest segment in pixels, so when the whole range falls on
/// one side of the fade band the per-fix computation can be skipped: a dense
/// zoomed-out track costs nothing per point, and a well-spaced zoomed-in
/// track skips the quality line entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackIconFade {
    /// Even the longest segment is below the fade-out spacing: every icon
    /// is fully transparent and the quality line stands in alone.
    AllHidden,
    /// Even the shortest segment is at or above the fade-in spacing: every
    /// icon is fully opaque and the quality line is not needed.
    AllVisible,
    /// Spacing crosses the fade band (e.g. parked phases on an otherwise
    /// moving track): opacity is decided per fix from its local spacing.
    PerFix,
}

/// Classify how `track`'s icons fade at the current map scale.
///
/// A track without segments (a lone fix) has nothing to overlap with and is
/// always [`TrackIconFade::AllVisible`] - a spacing of zero would hide it at
/// every zoom.
pub(crate) fn classify_icon_fade(
    track: &LoadedTrack,
    scale: MapScale,
    icon_size_px: f32,
) -> TrackIconFade {
    let Some(range) = track.metadata.segment_length_range else {
        return TrackIconFade::AllVisible;
    };
    let bbox = track.metadata.bounding_box;
    let mid_lat = Latitude::new((bbox.min().y + bbox.max().y) / 2.0);
    let ppm = scale.pixels_per_meter(mid_lat);
    let min_px = (range.min.get::<meter>() * ppm) as f32;
    let max_px = (range.max.get::<meter>() * ppm) as f32;
    let (lo, hi) = fade_band(icon_size_px);
    if max_px < lo {
        TrackIconFade::AllHidden
    } else if min_px >= hi {
        TrackIconFade::AllVisible
    } else {
        TrackIconFade::PerFix
    }
}

/// The fade band for a given icon size: local spacings at or below the
/// first bound render fully transparent icons, at or above the second fully
/// opaque ones. Proportional to the icon size, floored in absolute pixels
/// (see [`ICON_FADE_LO_MIN_SPACING_PX`]).
fn fade_band(icon_size_px: f32) -> (f32, f32) {
    (
        (icon_size_px * ICON_FADE_LO_SPACING_FACTOR).max(ICON_FADE_LO_MIN_SPACING_PX),
        (icon_size_px * ICON_FADE_HI_SPACING_FACTOR).max(ICON_FADE_HI_MIN_SPACING_PX),
    )
}

/// Local on-screen spacing of the fix at `pi`: the larger of the screen
/// distances to its temporal neighbours. `None` when the fix has no
/// neighbours at all (lone fix).
///
/// Taking the larger distance keeps cluster boundaries visible: the
/// departure arrow of a parked phase has a far next-neighbour even though
/// its previous neighbour sits on top of it. Endpoints of the track have
/// only one neighbour, so a track that starts or ends parked fades its
/// first/last arrow with the rest of the cluster.
fn local_fix_spacing_px(
    track: &LoadedTrack,
    pi: usize,
    screen_pos: Pos2,
    transform: &crate::transform::MercTransform,
) -> Option<f32> {
    let dist_to = |i: usize| {
        track
            .points
            .get(i)
            .map(|p| (transform.to_screen(p.merc) - screen_pos).length())
    };
    let prev = pi.checked_sub(1).and_then(dist_to);
    let next = dist_to(pi + 1);
    match (prev, next) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(d), None) | (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

/// Opacity of a fix icon for a given local on-screen spacing: 1.0 at or
/// above the [`fade_band`]'s upper bound, 0.0 at or below its lower bound,
/// linear in between.
fn icon_fade_alpha(spacing_px: f32, icon_size_px: f32) -> f32 {
    let (lo, hi) = fade_band(icon_size_px);
    if hi <= lo {
        // Unreachable while the HI floor and factor exceed their LO
        // counterparts. Kept so a future constant change degrades to opaque
        // icons instead of dividing by zero.
        return 1.0;
    }
    ((spacing_px - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Opacity of the fix at `pi` under the given per-track fade mode.
pub(crate) fn fix_icon_alpha(
    fade: TrackIconFade,
    track: &LoadedTrack,
    pi: usize,
    screen_pos: Pos2,
    icon_size_px: f32,
    transform: &crate::transform::MercTransform,
) -> f32 {
    match fade {
        TrackIconFade::AllHidden => 0.0,
        TrackIconFade::AllVisible => 1.0,
        TrackIconFade::PerFix => local_fix_spacing_px(track, pi, screen_pos, transform)
            .map_or(1.0, |spacing| icon_fade_alpha(spacing, icon_size_px)),
    }
}

/// Quantize a quality-line alpha into one of [`QUALITY_LINE_ALPHA_STEPS`]+1
/// discrete levels (0 = invisible, max = fully opaque).
pub(crate) fn line_alpha_bucket(line_alpha: f32) -> u8 {
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "clamped to [0, 1] so the product is in [0, QUALITY_LINE_ALPHA_STEPS]"
    )]
    let bucket = (line_alpha.clamp(0.0, 1.0) * f32::from(QUALITY_LINE_ALPHA_STEPS)).round() as u8;
    bucket
}

/// The drawable alpha for a quantized quality-line bucket.
pub(crate) fn bucket_alpha(bucket: u8) -> f32 {
    f32::from(bucket) / f32::from(QUALITY_LINE_ALPHA_STEPS)
}

/// Color of the continuous quality line at a given point. Same palette as
/// the icons: ghost fixes match the red ghost chevron, real fixes use the
/// satellite-count tiers of [`tpv_point_color`].
pub(crate) fn quality_line_color(point: &NavPoint) -> Color32 {
    if point.is_ghost_fix() {
        FIX_LOST_RED
    } else {
        tpv_point_color(point)
    }
}

/// Split a key-styled polyline span into maximal sub-spans whose points
/// share the same projected key.
///
/// Each edge takes the key of its starting point, so e.g. a stretch of
/// marginal fixes shows as one yellow segment up to the first good fix.
/// Boundary points are shared between adjacent sub-spans to keep the line
/// continuous.
pub(crate) fn split_spans_by<K: Copy, P: Copy + PartialEq>(
    span: &[(K, Pos2)],
    project: impl Fn(K) -> P,
) -> Vec<(P, Vec<Pos2>)> {
    let mut out: Vec<(P, Vec<Pos2>)> = Vec::new();
    for w in span.windows(2) {
        let [(key, pos_a), (_, pos_b)] = w else {
            continue;
        };
        let key = project(*key);
        match out.last_mut() {
            Some((k, pts)) if *k == key => pts.push(*pos_b),
            _ => out.push((key, vec![*pos_a, *pos_b])),
        }
    }
    out
}

/// Renders the two visual layers for a single on-screen GPS point: the
/// horizontal-accuracy circle and the directional icon (arrow or ghost).
/// The satellite-count labels are a separate anchor-based pass, see
/// [`draw_sat_labels`].
fn draw_tpv_point(
    ui: &Ui,
    screen_pos: Pos2,
    point_kind: &PointKind,
    eph_m: Option<f32>,
    pixels_per_meter: f64,
    highlighted: bool,
    style: &TpvDrawStyle,
) {
    // Accuracy circle - rendered beneath the icon. Skipped when too small to
    // see at all, and when small enough to be entirely covered by the icon.
    if let Some(eph_m) = eph_m {
        let radius = (f64::from(eph_m) * pixels_per_meter) as f32;
        let min_visible_radius = (style.base_arrow_size * ACCURACY_CIRCLE_MIN_VISIBLE_FACTOR)
            .max(MIN_ACCURACY_CIRCLE_RADIUS_PX);
        if radius >= min_visible_radius {
            ui.painter().circle_filled(
                screen_pos,
                radius,
                egui::Color32::from_rgba_unmultiplied(30, 120, 255, 20)
                    .gamma_multiply(style.icon_alpha),
            );
            ui.painter().circle_stroke(
                screen_pos,
                radius,
                egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(30, 120, 255, 60)
                        .gamma_multiply(style.icon_alpha),
                ),
            );
        }
    }

    // Directional icon.
    match point_kind {
        PointKind::Real { color, heading } => {
            draw_navigation_arrow(
                ui,
                screen_pos,
                *heading,
                color.gamma_multiply(style.icon_alpha),
                highlighted,
                style.outline_alpha * style.icon_alpha,
                style.base_arrow_size,
            );
        }
        PointKind::Ghost { direction } => {
            draw_ghost_chevron(
                ui,
                screen_pos,
                *direction,
                highlighted,
                style.base_arrow_size,
                style.icon_alpha,
            );
        }
    }
}

/// Draw the satellite-count labels for a track's selected anchor points.
///
/// Which anchors get a label this frame is decided by
/// [`crate::sat_labels::select_sat_labels`]; this pass just renders them.
/// Labels draw at full opacity regardless of the icon crossfade - the
/// anchor selection already bounds their density, and a faded-out cluster
/// is exactly where the surviving label carries the information.
pub(crate) fn draw_sat_labels(
    ui: &Ui,
    track: &LoadedTrack,
    label_indices: &[usize],
    style: &TpvDrawStyle,
    transform: &crate::transform::MercTransform,
) {
    for point in label_indices.iter().filter_map(|&pi| track.points.get(pi)) {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let screen_pos = transform.to_screen(point.merc);
        let label = format!("{}/{}", sats.fix_count(), sats.satellite_count());
        let text_pos = screen_pos + egui::vec2(style.base_arrow_size + 3.0, -style.base_arrow_size);
        let text_color = Color32::WHITE;
        let galley =
            ui.painter()
                .layout_no_wrap(label, egui::FontId::proportional(12.0), text_color);
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(text_pos.x, text_pos.y - galley.size().y),
            galley.size(),
        );
        ui.painter().rect_filled(
            text_rect.expand(2.0),
            2.0,
            Color32::from_rgba_unmultiplied(0, 0, 0, 160),
        );
        ui.painter().galley(text_rect.min, galley, text_color);
    }
}

/// Map a real (non-ghost) GPS point to its arrow colour from its canonical
/// [`gt_types::FixQuality`] tier: blue for strong or unknown quality, yellow
/// for marginal, red for lost.
///
/// Ghost fixes (no heading, or fix count zero) are rendered as red hollow chevrons
/// by `draw_ghost_chevron` and never reach this function.
fn tpv_point_color(point: &NavPoint) -> Color32 {
    match point.fix_quality() {
        gt_types::FixQuality::Unknown | gt_types::FixQuality::Strong => FIX_STRONG_BLUE,
        gt_types::FixQuality::Marginal => FIX_MARGINAL_YELLOW,
        gt_types::FixQuality::Lost => FIX_LOST_RED,
    }
}

/// Classifies a GPS point for a single render pass, carrying everything the
/// draw step needs so `render_track` only matches `heading()` once.
enum PointKind {
    /// Real GPS fix - heading known, precomputed Mercator coordinates used.
    Real { color: Color32, heading: Angle },
    /// Ghost fix - either heading is absent, or the satellite fix count is zero.
    ///
    /// `direction` is a normalised screen-space vector pointing in the inferred
    /// travel direction. When the GPS reported a heading it is converted directly,
    /// otherwise it is derived from the surrounding fixes' Mercator positions.
    Ghost { direction: Vec2 },
}

/// Compute the travel direction for a ghost fix from its neighbouring Mercator positions.
///
/// Mercator y increases southward, so dx/dy map directly to egui screen space without
/// a Y-flip. Falls back to [`Vec2::DOWN`] when both neighbours coincide (isolated point).
fn ghost_direction(prev: gt_types::MercPoint, next: gt_types::MercPoint) -> Vec2 {
    let raw = egui::vec2((next.x - prev.x) as f32, (next.y - prev.y) as f32);
    if raw.length_sq() > 1e-12 {
        raw.normalized()
    } else {
        Vec2::DOWN
    }
}

/// Render a hollow chevron for a ghost fix using the pre-loaded SVG texture.
///
/// The chevron tip points in `direction` (the inferred travel direction).
/// The icon is rendered as a rotated mesh quad so a single SVG asset handles
/// all orientations without re-rasterising.
fn draw_ghost_chevron(
    ui: &Ui,
    center: Pos2,
    direction: Vec2,
    highlighted: bool,
    base_size: f32,
    icon_alpha: f32,
) {
    let size = if highlighted {
        base_size + 3.0
    } else {
        base_size
    };
    let tint = if highlighted {
        Color32::from_rgb(100, 200, 255)
    } else {
        FIX_LOST_RED
    }
    .gamma_multiply(icon_alpha);
    crate::icons::draw_rotated_cached_icon(
        ui,
        crate::icons::ICON_URI_GHOST_FIX,
        center,
        direction,
        size,
        tint,
    );
}

fn draw_navigation_arrow(
    ui: &Ui,
    center: Pos2,
    heading: Angle,
    color: Color32,
    highlighted: bool,
    outline_alpha: f32,
    base_size: f32,
) {
    let angle_rad = heading.get::<radian>() - std::f64::consts::FRAC_PI_2;
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
    //       /   *   \        notch (0.1·size back - concave, pulled toward tip)
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

    // Fill - two convex triangles avoid non-convex fill artefacts.
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

    // Outline - closed path drawn on top of the fill.
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
    use crate::test_harness::TestHarness;
    use egui::Color32;
    use gt_types::MercPoint;
    use gt_types::NavPoint;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use uom::si::angle::degree;
    use uom::si::f64::{Angle, Length};

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

    /// A report spanning several constellations with a spread of SNR values and
    /// fix membership, so the satellite badge exercises every count tier, the
    /// full SNR gradient, and both the in-fix and idle PRN colours.
    fn sats_multi_constellation() -> Satellites {
        let satellites = vec![
            Satellite::new(Constellation::Gps, 1, None, None, Some(48.0), true),
            Satellite::new(Constellation::Gps, 2, None, None, Some(41.0), true),
            Satellite::new(Constellation::Gps, 3, None, None, Some(33.0), true),
            Satellite::new(Constellation::Gps, 4, None, None, Some(22.0), false),
            Satellite::new(Constellation::Galileo, 5, None, None, Some(37.0), true),
            Satellite::new(Constellation::Galileo, 6, None, None, Some(14.0), false),
            Satellite::new(Constellation::Glonass, 7, None, None, None, false),
            Satellite::new(Constellation::Beidou, 8, None, None, Some(45.0), true),
        ];
        Satellites::new(None, None, satellites)
    }

    /// The satellite badge (counts, SNR gradient, PRN colours) must stay
    /// legible on both themes. These render the same content under light and
    /// dark visuals; the light baseline is what catches colours that only read
    /// on a dark surface.
    #[test]
    fn satellite_badge_dark() {
        let point = make_point(Some(sats_multi_constellation()));
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 620.0))
            .theme(true)
            .ui(move |ui| show_sticky_tpv_content(ui, &point));
        harness.snapshot("satellite_badge_dark");
    }

    #[test]
    fn satellite_badge_light() {
        let point = make_point(Some(sats_multi_constellation()));
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 620.0))
            .theme(false)
            .ui(move |ui| show_sticky_tpv_content(ui, &point));
        harness.snapshot("satellite_badge_light");
    }

    fn track_with_points(points: Vec<NavPoint>) -> LoadedTrack {
        LoadedTrack {
            metadata: gt_types::TrackMetadata::default(),
            points,
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: Vec::new(),
            generated_markers: Vec::new(),
            event_markers: Vec::new(),
            channels: Vec::new(),
        }
    }

    fn make_tpv(lat: f64, lon: f64, heading: Option<f64>) -> TimePositionVelocity {
        if let Some(h) = heading {
            TimePositionVelocity::builder()
                .time(GpsTime::from_utc(chrono::Utc::now()))
                .lat(Latitude::new(lat))
                .lon(Longitude::new(lon))
                .heading(Angle::new::<degree>(h))
                .build()
        } else {
            TimePositionVelocity::builder()
                .time(GpsTime::from_utc(chrono::Utc::now()))
                .lat(Latitude::new(lat))
                .lon(Longitude::new(lon))
                .build()
        }
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

    /// A point with no heading → classified as ghost (hollow chevron).
    #[test]
    fn no_heading_is_ghost() {
        let tpv = make_tpv(51.5, -0.1, None);
        let point = NavPoint::new(tpv, None);
        assert!(point.is_ghost_fix());
    }

    /// A point with heading and no satellite report → classified as Real (blue arrow).
    #[test]
    fn heading_no_satellite_report_is_real() {
        let tpv = make_tpv(51.5, -0.1, Some(90.0));
        let point = NavPoint::new(tpv, None);
        assert!(!point.is_ghost_fix());
    }

    /// Fix count > 0 with heading → classified as Real (filled arrow, good fix).
    ///
    /// Dead reckoning or any device that supplies heading during a genuine fix
    /// is rendered as a filled arrow.
    #[test]
    fn heading_with_good_fix_is_real() {
        let tpv = make_tpv(51.5, -0.1, Some(225.0));
        let point = NavPoint::new(tpv, Some(sats_with_fix(5)));
        assert!(!point.is_ghost_fix());
    }

    /// Fix count == 0 → ghost even when heading is present.
    ///
    /// This is the common case for devices that continue outputting heading
    /// estimates after fix loss. Without any satellite in the fix, the heading
    /// is an internal guess and the icon should clearly signal uncertainty.
    #[test]
    fn heading_with_fix_lost_is_ghost() {
        let tpv = make_tpv(51.5, -0.1, Some(180.0));
        let point = NavPoint::new(tpv, Some(sats_with_fix(0)));
        assert!(point.is_ghost_fix());
    }

    /// Ghost chevron points east when the surrounding fixes move eastward.
    #[test]
    fn ghost_direction_points_east_for_eastward_movement() {
        let prev = MercPoint { x: 0.50, y: 0.50 };
        let next = MercPoint { x: 0.60, y: 0.50 };
        let dir = ghost_direction(prev, next);
        assert!(
            dir.x > 0.99,
            "eastward movement → large positive x; got {dir:?}"
        );
        assert!(
            dir.y.abs() < 0.01,
            "eastward movement → near-zero y; got {dir:?}"
        );
    }

    /// Ghost chevron points south when the surrounding fixes move southward.
    /// Mercator y increases southward, so this also tests that no Y-flip is applied.
    #[test]
    fn ghost_direction_points_south_for_southward_movement() {
        let prev = MercPoint { x: 0.50, y: 0.40 };
        let next = MercPoint { x: 0.50, y: 0.60 };
        let dir = ghost_direction(prev, next);
        assert!(
            dir.y > 0.99,
            "southward movement → large positive y; got {dir:?}"
        );
        assert!(
            dir.x.abs() < 0.01,
            "southward movement → near-zero x; got {dir:?}"
        );
    }

    /// When prev and next coincide (isolated point) the direction falls back to DOWN.
    #[test]
    fn ghost_direction_falls_back_when_neighbours_coincide() {
        let pt = MercPoint { x: 0.5, y: 0.5 };
        let dir = ghost_direction(pt, pt);
        assert_eq!(
            dir,
            Vec2::DOWN,
            "coincident neighbours → fallback direction DOWN"
        );
    }

    #[test]
    fn signed_delta_sub_2s_shows_ms() {
        assert_eq!(format_signed_delta(250), "+250ms");
        assert_eq!(format_signed_delta(-50), "\u{2212}50ms");
        assert_eq!(format_signed_delta(1999), "+1999ms");
    }

    #[test]
    fn signed_delta_fractional_seconds() {
        assert_eq!(format_signed_delta(2000), "+2s");
        assert_eq!(format_signed_delta(2100), "+2.1s");
        assert_eq!(format_signed_delta(2140), "+2.14s");
        assert_eq!(format_signed_delta(9230), "+9.23s");
        assert_eq!(format_signed_delta(-2140), "\u{2212}2.14s");
        assert_eq!(format_signed_delta(59990), "+59.99s");
    }

    #[test]
    fn signed_delta_terse_minutes() {
        assert_eq!(format_signed_delta(60_000), "+1m");
        assert_eq!(format_signed_delta(69_000), "+1m9s");
        assert_eq!(format_signed_delta(3_661_000), "+1h1m1s");
    }

    // With a 12 px icon, the fade band spans local spacings of 2.4 px
    // (LO, 0.2 icon sizes - arrows share almost all pixels) to 6 px
    // (HI, 0.5 icon sizes - arrows overlap but stay readable).
    const TEST_ICON_PX: f32 = 12.0;

    #[test]
    fn icon_fade_is_opaque_while_arrows_merely_overlap() {
        assert!(icon_fade_alpha(100.0, TEST_ICON_PX) >= 1.0);
        assert!(icon_fade_alpha(12.0, TEST_ICON_PX) >= 1.0); // fully side by side
        assert!(icon_fade_alpha(8.0, TEST_ICON_PX) >= 1.0); // overlapping a bit
        assert!(icon_fade_alpha(6.0, TEST_ICON_PX) >= 1.0); // exactly at the HI bound
    }

    #[test]
    fn icon_fade_is_transparent_when_arrows_blend_together() {
        assert!(icon_fade_alpha(2.4, TEST_ICON_PX) <= 0.0); // exactly at the LO bound
        assert!(icon_fade_alpha(0.0, TEST_ICON_PX) <= 0.0); // stacked on one point
    }

    #[test]
    fn icon_fade_is_linear_between_the_bounds() {
        let alpha = icon_fade_alpha(4.2, TEST_ICON_PX); // midway between 2.4 and 6
        assert!((alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn icon_fade_stays_opaque_for_degenerate_icon_size() {
        assert!(icon_fade_alpha(10.0, 0.0) >= 1.0);
        assert!(icon_fade_alpha(10.0, -1.0) >= 1.0);
    }

    // At low zoom icons shrink to 3 px and the proportional band would be
    // 0.6-1.5 px. The absolute floors widen it to 2-5 px so dot-sized
    // arrows stacked a couple of pixels apart fade into the quality line.
    const SMALL_ICON_PX: f32 = 3.0;

    #[test]
    fn icon_fade_band_is_floored_for_small_icons() {
        assert!(icon_fade_alpha(1.2, SMALL_ICON_PX) <= 0.0); // below the 2 px floor
        assert!(icon_fade_alpha(2.0, SMALL_ICON_PX) <= 0.0); // exactly at the LO floor
        assert!(icon_fade_alpha(5.0, SMALL_ICON_PX) >= 1.0); // exactly at the HI floor
        let alpha = icon_fade_alpha(3.5, SMALL_ICON_PX); // midway between 2 and 5
        assert!((alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn classify_uses_the_floored_band_for_small_icons() {
        // 1.9 m segments at 1 px/m: below the 2 px floor, fully hidden even
        // though 1.9 px is well above 0.2 x 3 px.
        let track = track_with_segment_range(0.0, 1.9);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), SMALL_ICON_PX),
            TrackIconFade::AllHidden
        );
        let track = track_with_segment_range(5.0, 50.0);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), SMALL_ICON_PX),
            TrackIconFade::AllVisible
        );
        let track = track_with_segment_range(3.0, 4.0);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), SMALL_ICON_PX),
            TrackIconFade::PerFix
        );
    }

    #[test]
    fn icon_fade_stays_opaque_for_infinite_spacing() {
        // Spacing can overflow to infinity when a long track meets an
        // extreme zoom. The result must clamp to opaque, not turn NaN.
        assert!(icon_fade_alpha(f32::INFINITY, TEST_ICON_PX) >= 1.0);
    }

    /// Same value as `MercTransform::pixels_per_meter`'s internal constant;
    /// with `for_test(EARTH_CIRCUMFERENCE_M)` the map scale is 1 px/m at the
    /// equator, so test geometry can be written directly in metres.
    const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;

    fn unit_transform() -> crate::transform::MercTransform {
        crate::transform::MercTransform::for_test(EARTH_CIRCUMFERENCE_M)
    }

    /// A real fix on the equator, `x_m` metres east of the origin.
    fn nav_point_at_meters(x_m: f64) -> NavPoint {
        let lon_deg = x_m * 360.0 / EARTH_CIRCUMFERENCE_M;
        NavPoint::new(make_tpv(0.0, lon_deg, Some(90.0)), None)
    }

    fn track_with_segment_range(min_m: f64, max_m: f64) -> LoadedTrack {
        LoadedTrack {
            metadata: gt_types::TrackMetadata {
                segment_length_range: Some(gt_types::SegmentLengthRange {
                    min: Length::new::<meter>(min_m),
                    max: Length::new::<meter>(max_m),
                }),
                ..gt_types::TrackMetadata::default()
            },
            points: Vec::new(),
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: Vec::new(),
            generated_markers: Vec::new(),
            event_markers: Vec::new(),
            channels: Vec::new(),
        }
    }

    #[test]
    fn classify_keeps_lone_fix_visible_at_every_zoom() {
        // No segments means nothing can overlap. A spacing of zero would
        // hide the lone fix forever.
        let track = track_with_points(Vec::new());
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
            TrackIconFade::AllVisible
        );
    }

    #[test]
    fn classify_hides_all_icons_when_even_the_longest_segment_blends() {
        // Longest segment 2 m = 2 px, below the 2.4 px fade-out bound.
        let track = track_with_segment_range(0.0, 2.0);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
            TrackIconFade::AllHidden
        );
    }

    #[test]
    fn classify_shows_all_icons_when_even_the_shortest_segment_is_spaced() {
        // Shortest segment 6 m = 6 px, exactly the fade-in bound.
        let track = track_with_segment_range(6.0, 100.0);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
            TrackIconFade::AllVisible
        );
    }

    #[test]
    fn classify_mixed_spacing_decides_per_fix() {
        // Parked-then-highway: zero-length segments next to 100 m hops.
        let track = track_with_segment_range(0.0, 100.0);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
            TrackIconFade::PerFix
        );
        // A range entirely inside the fade band is also per-fix.
        let track = track_with_segment_range(3.0, 5.0);
        assert_eq!(
            classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
            TrackIconFade::PerFix
        );
    }

    fn spacing_at(track: &LoadedTrack, pi: usize) -> Option<f32> {
        let transform = unit_transform();
        let screen_pos = transform.to_screen(track.points[pi].merc);
        local_fix_spacing_px(track, pi, screen_pos, &transform)
    }

    #[test]
    fn local_spacing_is_none_for_a_lone_fix() {
        let track = track_with_points(vec![nav_point_at_meters(0.0)]);
        assert_eq!(spacing_at(&track, 0), None);
    }

    #[test]
    fn local_spacing_of_endpoints_uses_their_single_neighbour() {
        let track = track_with_points(vec![nav_point_at_meters(0.0), nav_point_at_meters(100.0)]);
        let first = spacing_at(&track, 0).expect("has a neighbour");
        let last = spacing_at(&track, 1).expect("has a neighbour");
        assert!((first - 100.0).abs() < 1.0, "got {first} px");
        assert!((last - 100.0).abs() < 1.0, "got {last} px");
    }

    #[test]
    fn local_spacing_keeps_cluster_boundary_visible() {
        // Three stacked fixes (parked), then a 100 m hop: the interior
        // parked fixes have zero spacing, but the departure fix sees its
        // far next-neighbour and must stay visible.
        let track = track_with_points(vec![
            nav_point_at_meters(0.0),
            nav_point_at_meters(0.0),
            nav_point_at_meters(0.0),
            nav_point_at_meters(100.0),
        ]);
        let interior = spacing_at(&track, 1).expect("has neighbours");
        let departure = spacing_at(&track, 2).expect("has neighbours");
        assert!(interior < f32::EPSILON, "got {interior} px");
        assert!((departure - 100.0).abs() < 1.0, "got {departure} px");
    }

    #[test]
    fn fix_icon_alpha_short_circuits_uniform_tracks() {
        let track = track_with_points(vec![nav_point_at_meters(0.0), nav_point_at_meters(0.0)]);
        let transform = unit_transform();
        let pos = transform.to_screen(track.points[0].merc);
        // AllHidden / AllVisible ignore local spacing entirely.
        let hidden = fix_icon_alpha(
            TrackIconFade::AllHidden,
            &track,
            0,
            pos,
            TEST_ICON_PX,
            &transform,
        );
        let visible = fix_icon_alpha(
            TrackIconFade::AllVisible,
            &track,
            0,
            pos,
            TEST_ICON_PX,
            &transform,
        );
        assert!(hidden <= 0.0);
        assert!(visible >= 1.0);
    }

    #[test]
    fn per_fix_alpha_handles_parked_highway_parked() {
        // The shape from the bug report: parked (stacked fixes), then
        // highway (100 m hops), then parked again. Parked interiors fade,
        // every highway fix and both cluster boundary fixes stay opaque.
        let track = track_with_points(vec![
            nav_point_at_meters(0.0),
            nav_point_at_meters(0.0),
            nav_point_at_meters(0.0), // departure: next neighbour is far
            nav_point_at_meters(100.0),
            nav_point_at_meters(200.0),
            nav_point_at_meters(300.0), // arrival: prev neighbour is far
            nav_point_at_meters(300.0),
            nav_point_at_meters(300.0),
        ]);
        let transform = unit_transform();
        let alpha_at = |pi: usize| {
            let pos = transform.to_screen(track.points[pi].merc);
            fix_icon_alpha(
                TrackIconFade::PerFix,
                &track,
                pi,
                pos,
                TEST_ICON_PX,
                &transform,
            )
        };
        // Parked interiors (including the track ends) are fully faded.
        assert!(alpha_at(0) <= 0.0);
        assert!(alpha_at(1) <= 0.0);
        assert!(alpha_at(6) <= 0.0);
        assert!(alpha_at(7) <= 0.0);
        // Departure, highway, and arrival fixes are fully opaque.
        for pi in 2..=5 {
            assert!(alpha_at(pi) >= 1.0, "fix {pi} should be opaque");
        }
    }

    #[test]
    fn line_alpha_buckets_quantize_the_crossfade() {
        assert_eq!(line_alpha_bucket(0.0), 0);
        assert_eq!(line_alpha_bucket(0.16), 0); // rounds down: still invisible
        assert_eq!(line_alpha_bucket(0.34), 1);
        assert_eq!(line_alpha_bucket(0.5), 2); // rounds half away from zero
        assert_eq!(line_alpha_bucket(1.0), QUALITY_LINE_ALPHA_STEPS);
        // Out-of-range inputs clamp instead of wrapping.
        assert_eq!(line_alpha_bucket(-1.0), 0);
        assert_eq!(line_alpha_bucket(2.0), QUALITY_LINE_ALPHA_STEPS);
        assert!((bucket_alpha(QUALITY_LINE_ALPHA_STEPS) - 1.0).abs() < f32::EPSILON);
        assert!(bucket_alpha(0) < f32::EPSILON);
    }

    #[test]
    fn quality_line_color_marks_ghost_fixes_red() {
        // No heading and no satellite report: tpv_point_color alone would say
        // blue, but the point is a ghost fix and must show as red.
        let tpv = make_tpv(51.5, -0.1, None);
        let point = NavPoint::new(tpv, None);
        assert_eq!(quality_line_color(&point), FIX_LOST_RED);
    }

    #[test]
    fn quality_line_color_follows_fix_quality_for_real_fixes() {
        let marginal = make_point(Some(sats_with_fix(4)));
        assert_eq!(quality_line_color(&marginal), FIX_MARGINAL_YELLOW);
        let strong = make_point(Some(sats_with_fix(12)));
        assert_eq!(quality_line_color(&strong), FIX_STRONG_BLUE);
    }

    #[test]
    fn split_spans_by_single_key_is_one_sub_span() {
        use egui::pos2;
        let span = [
            (Color32::BLUE, pos2(0.0, 0.0)),
            (Color32::BLUE, pos2(10.0, 0.0)),
            (Color32::BLUE, pos2(20.0, 0.0)),
        ];
        let subs = split_spans_by(&span, |k| k);
        assert_eq!(
            subs,
            vec![(
                Color32::BLUE,
                vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 0.0)]
            )]
        );
    }

    #[test]
    fn split_spans_by_edge_takes_key_of_its_starting_point() {
        use egui::pos2;
        let span = [
            (Color32::BLUE, pos2(0.0, 0.0)),
            (Color32::YELLOW, pos2(10.0, 0.0)),
            (Color32::YELLOW, pos2(20.0, 0.0)),
        ];
        let subs = split_spans_by(&span, |k| k);
        // The blue->yellow edge is blue (starting point's quality). The
        // boundary point is shared so the line stays continuous.
        assert_eq!(
            subs,
            vec![
                (Color32::BLUE, vec![pos2(0.0, 0.0), pos2(10.0, 0.0)]),
                (Color32::YELLOW, vec![pos2(10.0, 0.0), pos2(20.0, 0.0)]),
            ]
        );
    }

    #[test]
    fn split_spans_by_splits_on_alpha_bucket_within_one_color() {
        use egui::pos2;
        // Same quality color but different crossfade buckets: the line must
        // split so an opaque stretch (a parked cluster, bucket 3) and an
        // invisible stretch (well-spaced fixes, bucket 0) get separate
        // strokes - this is what localizes the quality line to the cluster.
        // Each edge takes its starting point's bucket, so the transition
        // edge still belongs to the cluster.
        let span = [
            ((Color32::BLUE, 3_u8), pos2(0.0, 0.0)),
            ((Color32::BLUE, 3_u8), pos2(10.0, 0.0)),
            ((Color32::BLUE, 0_u8), pos2(20.0, 0.0)),
            ((Color32::BLUE, 0_u8), pos2(30.0, 0.0)),
        ];
        let subs = split_spans_by(&span, |k| k);
        assert_eq!(
            subs,
            vec![
                (
                    (Color32::BLUE, 3_u8),
                    vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 0.0)]
                ),
                (
                    (Color32::BLUE, 0_u8),
                    vec![pos2(20.0, 0.0), pos2(30.0, 0.0)]
                ),
            ]
        );
    }

    #[test]
    fn split_spans_by_too_short_span_is_empty() {
        use egui::pos2;
        assert!(split_spans_by::<Color32, Color32>(&[], |k| k).is_empty());
        assert!(split_spans_by(&[(Color32::BLUE, pos2(0.0, 0.0))], |k| k).is_empty());
    }
}
