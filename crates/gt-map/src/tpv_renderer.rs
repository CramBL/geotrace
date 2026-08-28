use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, PopupAnchor, Pos2, Stroke, Ui, Vec2};
use egui::{Grid, RichText, ScrollArea, Tooltip};
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use gt_filter::{self as filter, GlobalFilter};
use gt_sky::{SkyHighlight, SkyPlot, SkyPlotSize};
use gt_types::satellites::{Constellation, Satellite};
use gt_types::{
    DataCategory, FileIdx, LoadedFile, LoadedTrack, NavPoint, NearestSatelliteReport, PointIdx,
    SKY_REPORT_MAX_AGE_SECS, TrackIdx, TrackRef,
};
use gt_ui_theme::{DEGREE_SIGN, DELTA, EM_DASH};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight, PointWindowFolds};
use smallvec::SmallVec;
use strum::{EnumCount as _, IntoEnumIterator as _};
use uom::si::angle::{degree, radian};
use uom::si::f64::Angle;
use uom::si::length::meter;

use crate::icon_mesh::{IconId, IconInstance, IconMeshBatch, IconMeshLibrary};
use crate::recording_labels::RecordingLabels;
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

/// Side length, corner rounding, and inset of the constellation colour swatch
/// shown before a satellite-table header.
const SWATCH_SIZE_PX: f32 = 10.0;
const SWATCH_ROUNDING_PX: f32 = 2.0;
const SWATCH_MARGIN_PX: f32 = 1.0;

/// Size of the solid fold triangle. Big enough that its constellation tint
/// reads as the colour key to the plot's marks.
const FOLD_ARROW_SIZE_PX: f32 = 9.0;

/// Box the fold triangle is allocated in, leaving a little air around it.
const FOLD_ARROW_BOX_PX: f32 = 12.0;

/// Gap between the sticky popup's plot column and its satellite column, and
/// between the two satellite columns.
const STICKY_COLUMN_GAP_PX: f32 = 12.0;

/// Width a satellite column occupies: its PRN, SNR and fix-mark cells laid
/// out at the default text size. Both width thresholds below are derived from
/// it, so retuning a column only needs changing here.
const MIN_SATELLITE_COLUMN_WIDTH_PX: f32 = 140.0;

/// Width the satellite area needs before it splits into two columns - two
/// readable columns and the gap between them. Below it the constellations
/// stack in one column instead of being squeezed.
const MIN_TWO_COLUMN_WIDTH_PX: f32 = 2.0 * MIN_SATELLITE_COLUMN_WIDTH_PX + STICKY_COLUMN_GAP_PX;

/// Width the window needs to put the sky plot beside the satellite tables:
/// the full plot's own diameter, the column gap, and enough left for a
/// readable table. Narrower than this the plot stacks above them instead.
const MIN_SIDE_BY_SIDE_WIDTH_PX: f32 =
    SkyPlotSize::Full.diameter() + STICKY_COLUMN_GAP_PX + MIN_SATELLITE_COLUMN_WIDTH_PX;

/// Rows a constellation panel costs beyond its satellites: its own header plus
/// the PRN/SNR/Fix header row. Counted so a one-satellite constellation is not
/// treated as free when balancing the columns.
const PANEL_HEADER_ROWS: usize = 2;

/// Rows a folded constellation panel costs - just its own header.
const FOLDED_PANEL_ROWS: usize = 1;

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
    icon_meshes: Option<&IconMeshLibrary>,
) {
    // One batch for the whole track's icons. Painter primitives inside the
    // pass (accuracy circles, highlighted arrows) barrier the batch so
    // stacking matches immediate painting exactly; see [IconMeshBatch].
    let mut batch = IconMeshBatch::gpu_when_available(ui, icon_meshes);
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
            let screen_pos = transform.to_screen(point.merc());
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
            let (latitude, _) = point.resolved_position();
            let pixels_per_meter = if eph_m.is_some() {
                transform.pixels_per_meter(latitude)
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
                &mut batch,
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
        let screen_pos = transform.to_screen(point.merc());
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
        // - Otherwise derive from neighbouring Mercator positions (see
        //   [`ghost_direction`]).
        let direction = if let Some(h) = point.tpv.heading() {
            let angle_rad = h.get::<radian>() - std::f64::consts::FRAC_PI_2;
            egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32)
        } else {
            let merc_prev = pi
                .checked_sub(1)
                .and_then(|i| track.points.get(i))
                .map_or(point.merc(), |p| p.merc());
            let merc_next = track.points.get(pi + 1).map_or(point.merc(), |p| p.merc());
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
            &mut batch,
        );
    }
    batch.paint(ui.painter());
}

/// Show the hover tooltip for the given TPV point. When the point lies
/// inside a query match, `match_header` renders the match context above the
/// point table.
pub(crate) fn show_tooltip(
    ui: &Ui,
    files: &[LoadedFile],
    recording_labels: RecordingLabels<'_>,
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
    Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        tooltip_id,
        PopupAnchor::Pointer,
    )
    .show(|ui| {
        if let Some(header) = match_header {
            header(ui);
        }
        show_hover_table(
            ui,
            point,
            &SkySection::resolve(track, point_ref.point_index),
            recording_labels.name_when_several_files_loaded(point_ref.track.fi),
        );
    });
}

/// The sky column of the hover badge.
pub(crate) enum SkySection<'a> {
    /// The point's own satellite report, or one borrowed from a nearby point.
    Report(NearestSatelliteReport<'a>),
    /// The track has satellite reports, but none within the age window of
    /// this point.
    NoReportNearby,
    /// The track carries no satellite reports at all, so the badge has no
    /// sky column.
    TrackWithoutReports,
}

impl<'a> SkySection<'a> {
    pub(crate) fn resolve(track: &'a LoadedTrack, point_index: PointIdx) -> Self {
        if track.metadata.satellite_report_count == 0 {
            return Self::TrackWithoutReports;
        }
        match track.nearest_satellite_report(point_index) {
            Some(report) => Self::Report(report),
            None => Self::NoReportNearby,
        }
    }
}

/// The sky plot with its report-age line, or the no-report placeholder. The
/// full size is interactive (per-mark tooltips); the compact size lives
/// inside the hover badge, which is itself a tooltip.
fn sky_section_ui(
    ui: &mut Ui,
    sky: &SkySection<'_>,
    size: SkyPlotSize,
    highlight: Option<SkyHighlight>,
) {
    match sky {
        SkySection::TrackWithoutReports => {}
        SkySection::NoReportNearby => {
            ui.label(
                RichText::new(format!(
                    "No satellite report within {SKY_REPORT_MAX_AGE_SECS} s"
                ))
                .weak()
                .small(),
            );
        }
        SkySection::Report(report) => {
            let plot = SkyPlot::new(report.satellites, size).with_highlight(highlight);
            let plot = match size {
                SkyPlotSize::Full => plot.interactive(),
                SkyPlotSize::Compact => plot,
            };
            plot.ui(ui);
            if !report.age.is_zero() {
                ui.label(RichText::new(report_age_label(report.age)).weak().small());
            }
        }
    }
}

/// "Report 2.1 s earlier" / "Report 2.1 s later" for a borrowed report.
fn report_age_label(age: chrono::Duration) -> String {
    let seconds = age.num_milliseconds().abs() as f64 / 1000.0;
    let side = if age > chrono::Duration::zero() {
        "earlier"
    } else {
        "later"
    };
    format!("Report {seconds:.1} s {side}")
}

/// Zoom-derived visual parameters computed once per frame and shared
/// across all tracks: icon sizes scale with zoom (see [`base_arrow_size`])
/// and the outline alpha fades out below zoom 14 so dense clusters don't
/// blend into a white mass.
pub(crate) fn frame_style(zoom: f64) -> TpvDrawStyle {
    TpvDrawStyle {
        base_arrow_size: base_arrow_size(zoom),
        outline_alpha: ((zoom - 10.0) / 4.0).clamp(0.0, 1.0) as f32,
        icon_alpha: 1.0,
    }
}

/// The satellite-label collision-cell size in pixels: larger when zoomed out
/// so the label count doesn't explode at dense clusters (180 px at low zoom,
/// 60 px at high). Shared by [`frame_style`] and the map's zoom-debounced
/// label decimation.
pub(crate) fn label_cell_px(zoom: f64) -> f32 {
    60.0 + (1.0 - zoom_size_factor(zoom)) * 120.0
}

/// Cross-highlight: when the track plot cursor is active, draw a ring around
/// the pre-computed closest point, plus a sky disc for its report so
/// scrubbing the plot walks the sky along the track. The app layer computes
/// the point via find_closest_tpv and stores it in
/// `MapHighlight::plot_hover_point` - no O(n) scan needed here.
pub(crate) fn draw_plot_hover_overlay(
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
        let pos = transform.to_screen(point.merc());
        let painter = ui.painter();
        painter.circle_stroke(
            pos,
            style.base_arrow_size + 6.0,
            egui::Stroke::new(
                2.0_f32,
                egui::Color32::from_rgba_unmultiplied(100, 200, 255, 230),
            ),
        );
        painter.circle_stroke(
            pos,
            style.base_arrow_size + 3.0,
            egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_unmultiplied(100, 200, 255, 120),
            ),
        );
        // The detailed disc at the scrubbed point, independent of the sky
        // glyphs overlay's own variant or visibility.
        if let Some(satellites) = &point.satellites {
            crate::sky_glyph_renderer::draw_hover_disc(
                ui,
                pos,
                satellites,
                glyph_size_scale(style),
            );
        }
    }
}

/// `recording_name` fills the table's recording row. It is `None` while a
/// single file is loaded.
pub(crate) fn show_hover_table(
    ui: &mut Ui,
    p: &NavPoint,
    sky: &SkySection<'_>,
    recording_name: Option<&str>,
) {
    ui.horizontal_top(|ui| {
        hover_grid_ui(ui, p, recording_name);
        if !matches!(sky, SkySection::TrackWithoutReports) {
            ui.add_space(12.0);
            ui.vertical(|ui| sky_section_ui(ui, sky, SkyPlotSize::Compact, None));
        }
    });
}

/// The row naming the recording a fix came from, in the grid the caller has
/// open. Draws nothing while a single file is loaded.
fn recording_row_ui(ui: &mut Ui, recording_name: Option<&str>) {
    if let Some(name) = recording_name {
        ui.label("Recording");
        ui.label(name);
        ui.end_row();
    }
}

fn hover_grid_ui(ui: &mut Ui, p: &NavPoint, recording_name: Option<&str>) {
    Grid::new("hover_grid")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            recording_row_ui(ui, recording_name);

            ui.label("Time");
            ui.label(p.tpv.time().utc().format("%Y-%m-%d %H:%M:%S").to_string());
            ui.end_row();

            let lat = p.tpv.lat().as_written();
            let lon = p.tpv.lon().as_written();
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
                    ui.label(gt_fmt::format_signed_delta(sat_delta_ms));
                    ui.end_row();
                }
            }

            if let Some(offset) = p.tpv.gps_system_clock_offset() {
                let clock_delta_ms = offset.num_milliseconds();
                ui.label(format!("Clock {DELTA}t"));
                ui.label(format!(
                    "{} ({})",
                    gt_fmt::format_signed_delta(clock_delta_ms),
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
/// Returns whether the user asked to open the sky trails window at this
/// point's instant.
#[must_use]
pub(crate) fn show_sticky_tpv_content(
    ui: &mut Ui,
    p: &NavPoint,
    sky: &SkySection<'_>,
    folds: &mut PointWindowFolds,
    recording_name: Option<&str>,
) -> bool {
    // The sky plot is drawn beside the tables, so hovering a table row feeds
    // the plot through egui's per-frame data store: this frame's plot uses
    // last frame's highlight, and the tables set next frame's. A repaint on
    // change keeps the lag imperceptible.
    let highlight_id = sky_table_highlight_id(ui);
    let prev_highlight: Option<SkyHighlight> =
        ui.ctx().data(|d| d.get_temp(highlight_id)).flatten();
    let mut highlight: Option<SkyHighlight> = None;

    // Side by side when the plot and a satellite column both fit; stacked
    // when they do not, since squeezing the tables in beside a 256 px plot
    // leaves them too narrow to read.
    let side_by_side = ui.available_width() >= MIN_SIDE_BY_SIDE_WIDTH_PX;
    // `folds` is a parameter rather than a capture so both closures can take
    // it in turn without holding overlapping mutable borrows.
    // Returns whether the header's "open sky trails" button was pressed.
    let summary =
        |ui: &mut Ui, folds: &mut PointWindowFolds, highlight: &mut Option<SkyHighlight>| {
            let mut open_trails = false;
            if !matches!(sky, SkySection::TrackWithoutReports) {
                // Sized to the plot it titles: a right-aligned button in an
                // unbounded row would stretch the whole column to the window's
                // width and squeeze the satellite tables out.
                let header = ui.allocate_ui_with_layout(
                    egui::vec2(SkyPlotSize::Full.diameter(), ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let tint = ui.visuals().weak_text_color();
                        fold_arrow(ui, folds.plot_folded, tint);
                        ui.label(RichText::new("Sky").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.small_button(ICON_ARROW_SQUARE_OUT)
                                .on_hover_text("Open sky trails at this moment")
                        })
                        .inner
                    },
                );
                let button = header.inner;
                open_trails = button.clicked();
                // The fold covers the header up to the button, not through it:
                // an interaction registered after the button would sit on top
                // of it and swallow its clicks.
                let mut fold_rect = header.response.rect;
                fold_rect.max.x = button.rect.left() - ui.spacing().item_spacing.x;
                // Explicit id: `.interact` on the container response reuses an
                // auto-generated id, which collides with identically laid out
                // siblings.
                let fold = ui.interact(fold_rect, ui.id().with("sky_fold"), egui::Sense::click());
                crate::hover_labels::hover_affordance(ui, fold.rect);
                if fold.clicked() {
                    folds.plot_folded = !folds.plot_folded;
                }
                if !folds.plot_folded {
                    sky_section_ui(ui, sky, SkyPlotSize::Full, prev_highlight);
                }
                ui.add_space(6.0);
            }
            sticky_metrics(ui, p, highlight, recording_name);
            open_trails
        };
    // The satellite tables always scroll on their own, so the plot beside or
    // above them never scrolls out of view.
    let satellites =
        |ui: &mut Ui, folds: &mut PointWindowFolds, highlight: &mut Option<SkyHighlight>| {
            ScrollArea::vertical()
                .id_salt("sticky_sats_scroll")
                .show(ui, |ui| sticky_satellites(ui, p, folds, highlight));
        };

    let mut open_trails = false;
    if side_by_side {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| open_trails = summary(ui, folds, &mut highlight));
            ui.add_space(STICKY_COLUMN_GAP_PX);
            ui.vertical(|ui| satellites(ui, folds, &mut highlight));
        });
    } else {
        open_trails = summary(ui, folds, &mut highlight);
        ui.add_space(6.0);
        satellites(ui, folds, &mut highlight);
    }

    if highlight != prev_highlight {
        ui.ctx().request_repaint();
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(highlight_id, highlight));
    open_trails
}

/// The fix metrics beneath the plot: speed, heading, accuracy, the satellite
/// fix/seen counts, and the clock deltas. Hovering the fix count highlights
/// the in-fix satellites on the plot.
fn sticky_metrics(
    ui: &mut Ui,
    p: &NavPoint,
    highlight: &mut Option<SkyHighlight>,
    recording_name: Option<&str>,
) {
    Grid::new("sticky_tpv_basic").num_columns(2).show(ui, |ui| {
        recording_row_ui(ui, recording_name);

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

        if let Some(sats) = &p.satellites {
            let fix = sats.fix_count();
            let seen = sats.satellite_count();
            ui.label("Satellites");
            let dark_mode = ui.visuals().dark_mode;
            ui.horizontal(|ui| {
                let fix_resp = ui.colored_label(fix_count_color(fix, dark_mode), fix.to_string());
                if crate::hover_labels::hover_affordance(ui, fix_resp.rect) {
                    *highlight = Some(SkyHighlight::in_fix());
                }
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
                    ui.label(gt_fmt::format_signed_delta(sat_delta_ms));
                    ui.end_row();
                }
            }
        }

        // GPS/system-clock delta: how far the GPS clock leads the host clock.
        if let Some(offset) = p.tpv.gps_system_clock_offset() {
            let clock_delta_ms = offset.num_milliseconds();
            ui.label(format!("Clock {DELTA}t"));
            ui.label(gt_fmt::format_signed_delta(clock_delta_ms));
            ui.end_row();
        }
    });
}

/// A fold arrow, tinted with the colour of whatever it folds, so one element
/// shows both the fold state and the key to the plot's marks.
fn fold_arrow(ui: &mut Ui, folded: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(FOLD_ARROW_BOX_PX), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    // Painted, not set as a glyph: only the Regular phosphor weight is loaded,
    // and its caret is too fine to show the constellation colour.
    let c = rect.center();
    let half = FOLD_ARROW_SIZE_PX / 2.0;
    let points = if folded {
        // Pointing right: folded away.
        vec![
            egui::pos2(c.x - half, c.y - half),
            egui::pos2(c.x - half, c.y + half),
            egui::pos2(c.x + half, c.y),
        ]
    } else {
        // Pointing down: opened out.
        vec![
            egui::pos2(c.x - half, c.y - half),
            egui::pos2(c.x + half, c.y - half),
            egui::pos2(c.x, c.y + half),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// One constellation's satellites in the point window, carrying what both the
/// panel and the column packing need.
struct ConstellationGroup<'a> {
    /// Salts the panel's `Grid` id, so the two columns' grids stay distinct.
    grid_id: usize,
    constellation: Constellation,
    prn_prefix: &'a str,
    satellites: Vec<Satellite>,
}

impl ConstellationGroup<'_> {
    /// Rows this panel occupies: its own header plus, when it is not folded
    /// away, the table header and a row per satellite. Drives how the columns
    /// are balanced, so folding a constellation re-balances them rather than
    /// leaving a column sized for rows that are no longer drawn.
    fn weight(&self, folds: PointWindowFolds) -> usize {
        if folds.is_folded(self.constellation) {
            FOLDED_PANEL_ROWS
        } else {
            self.satellites.len() + PANEL_HEADER_ROWS
        }
    }
}

/// The per-constellation satellite tables. Hovering a constellation name, its
/// fix count, or a satellite row highlights the matching marks on the plot.
fn sticky_satellites(
    ui: &mut Ui,
    p: &NavPoint,
    folds: &mut PointWindowFolds,
    highlight: &mut Option<SkyHighlight>,
) {
    let Some(sats) = &p.satellites else {
        return;
    };
    ui.add_space(6.0);

    // Collect non-empty constellations up-front. `Satellite` is `Copy` so
    // we own the data and can borrow-free inside the layout closures.
    // Grouped in variant-declaration order, matching `Constellation`'s
    // `Ord` and the slip table's grouping.
    let groups: SmallVec<[ConstellationGroup<'_>; Constellation::COUNT]> = Constellation::iter()
        .enumerate()
        .filter_map(|(grid_id, constellation)| {
            let mut satellites: Vec<_> = sats.by_constellation(constellation).copied().collect();
            if satellites.is_empty() {
                return None;
            }
            satellites.sort_by_key(Satellite::prn);
            Some(ConstellationGroup {
                grid_id,
                constellation,
                prn_prefix: constellation.prn_prefix(),
                satellites,
            })
        })
        .collect();

    // Never more than two columns: a third would sit far enough from the plot
    // that correlating a row with its mark gets hard.
    let two_columns = groups.len() > 1 && ui.available_width() >= MIN_TWO_COLUMN_WIDTH_PX;
    if !two_columns {
        for group in &groups {
            constellation_panel(ui, group, folds, highlight);
        }
        return;
    }

    // Cut the ordered list where the two columns come out closest in height, so
    // uneven constellations pack tight.
    let weights: SmallVec<[usize; Constellation::COUNT]> =
        groups.iter().map(|group| group.weight(*folds)).collect();
    let (left, right) = groups.split_at(balanced_split(&weights));
    ui.horizontal_top(|ui| {
        for (column_i, column) in [left, right].into_iter().enumerate() {
            if column_i > 0 {
                ui.add_space(STICKY_COLUMN_GAP_PX);
            }
            ui.vertical(|ui| {
                for group in column {
                    constellation_panel(ui, group, folds, highlight);
                }
            });
        }
    });
}

/// The index at which to cut an ordered list of column weights into two
/// columns of as-equal height as possible.
///
/// One cut point, with the order preserved (GPS first, and so on).
fn balanced_split(weights: &[usize]) -> usize {
    if weights.len() < 2 {
        return weights.len();
    }
    let total: usize = weights.iter().sum();
    let mut left = 0;
    let mut best_index = 1;
    let mut best_imbalance = usize::MAX;
    for (i, weight) in weights.iter().enumerate().take(weights.len() - 1) {
        left += weight;
        let imbalance = left.abs_diff(total - left);
        if imbalance < best_imbalance {
            best_imbalance = imbalance;
            best_index = i + 1;
        }
    }
    best_index
}

/// One constellation's panel: the header (colour swatch, name, fix/seen count)
/// above its per-PRN table. Hovering the name, the fix count or a row drives
/// the matching sky-plot highlight.
fn constellation_panel(
    ui: &mut Ui,
    group: &ConstellationGroup<'_>,
    folds: &mut PointWindowFolds,
    highlight: &mut Option<SkyHighlight>,
) {
    let &ConstellationGroup {
        grid_id,
        constellation,
        prn_prefix,
        ..
    } = group;
    let satellites = group.satellites.as_slice();
    let folded = folds.is_folded(constellation);
    ui.vertical(|ui| {
        let dark_mode = ui.visuals().dark_mode;
        let const_fix = satellites.iter().filter(|s| s.in_fix()).count() as u32;
        // Header: the fold arrow in the constellation's plot colour, the name,
        // and the fix/seen count.
        //
        // The whole row folds on click, not just the arrow - the row is what
        // lights up on hover, so that is what has to be clickable. Which part
        // is under the pointer still picks the highlight: the fix count
        // highlights only the in-fix subset, anywhere else the whole
        // constellation.
        let header = ui.horizontal(|ui| {
            fold_arrow(
                ui,
                folded,
                gt_ui_theme::constellation_color(constellation, dark_mode),
            );
            ui.label(RichText::new(constellation.display_name()).strong());
            let fix_resp =
                ui.colored_label(fix_count_color(const_fix, dark_mode), const_fix.to_string());
            ui.label(RichText::new(format!("/{}", satellites.len())).weak());
            ui.rect_contains_pointer(fix_resp.rect)
        });
        let over_fix_count = header.inner;
        // Interact with an explicit id rather than re-sensing the container's
        // response: sibling panels lay out identically, so the auto-generated
        // container ids collide and the click lands on the wrong panel - or
        // nowhere at all.
        let header = ui.interact(
            header.response.rect,
            ui.id().with(("constellation_fold", constellation)),
            egui::Sense::click(),
        );
        if crate::hover_labels::hover_affordance(ui, header.rect) {
            *highlight = Some(if over_fix_count {
                SkyHighlight::constellation_in_fix(constellation)
            } else {
                SkyHighlight::constellation(constellation)
            });
        }
        if header.clicked() {
            folds.toggle(constellation);
        }
        if folded {
            // The header above keeps colour, name and counts on screen, so a
            // folded constellation still reads at a glance.
            return;
        }
        Grid::new(("sticky_sats", grid_id))
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("PRN").weak().small());
                ui.label(RichText::new("SNR").weak().small());
                ui.label(RichText::new("Fix").weak().small());
                ui.end_row();

                // A satellite contributing to the fix is shown in the
                // "good" tier green. An idle one is de-emphasised with
                // egui's own weak-text colour, which already tracks the
                // theme.
                let in_fix_color = gt_ui_theme::SatCountTier::Good.color(dark_mode);
                let muted_color = ui.visuals().weak_text_color();
                for sat in satellites {
                    let in_fix = sat.in_fix();
                    let prn_color = if in_fix { in_fix_color } else { muted_color };
                    let prn_resp = ui.label(
                        RichText::new(format!("{}{:02}", prn_prefix, sat.prn())).color(prn_color),
                    );
                    let snr_resp = match sat.snr() {
                        Some(snr) => ui.label(
                            RichText::new(format!("{:.1}", snr.value()))
                                .color(gt_ui_theme::snr_color(snr.quality(), dark_mode)),
                        ),
                        None => ui.label(RichText::new(EM_DASH).color(muted_color)),
                    };
                    let fix_resp = if in_fix {
                        ui.label(RichText::new(ICON_CHECK).color(in_fix_color))
                    } else {
                        ui.label("")
                    };
                    // Hovering anywhere across the row (the
                    // union spans the cells and the gaps
                    // between them) highlights just that
                    // satellite.
                    let row = prn_resp.union(snr_resp).union(fix_resp);
                    if crate::hover_labels::row_hover_affordance(ui, row.rect) {
                        *highlight = Some(SkyHighlight::satellite(constellation, sat.prn()));
                    }
                    ui.end_row();
                }
            });
    });
    ui.add_space(6.0);
}

/// The egui data-store key under which the sticky content stashes the sky
/// highlight driven by table hovers. Scoped to the containing window's `ui`,
/// so concurrent surfaces cannot collide. Shared with the tests that verify
/// the hover wiring.
pub(crate) fn sky_table_highlight_id(ui: &Ui) -> egui::Id {
    ui.id().with("sky_table_highlight")
}

/// A small colour swatch in the constellation's plot colour, drawn before a
/// table header or legend row so it reads as the key to the plot's marks.
pub(crate) fn constellation_swatch(ui: &mut Ui, constellation: Constellation) {
    let color = gt_ui_theme::constellation_color(constellation, ui.visuals().dark_mode);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(SWATCH_SIZE_PX), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect.shrink(SWATCH_MARGIN_PX), SWATCH_ROUNDING_PX, color);
}

fn show_satellite_rows(ui: &mut Ui, p: &NavPoint) {
    // A missing report does not mean there was no GPS fix, just that no
    // satellite data was captured or associated for this point.
    let Some(sats) = &p.satellites else {
        return;
    };

    let fix = sats.fix_count();
    let seen = sats.satellite_count();
    let dark_mode = ui.visuals().dark_mode;

    // Total summary row - bold to signal it is the aggregate.
    ui.label(RichText::new("Satellites").strong());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(fix.to_string())
                .color(fix_count_color(fix, dark_mode))
                .strong(),
        );
        ui.label(RichText::new("/").strong());
        ui.label(
            RichText::new(seen.to_string())
                .color(seen_count_color(seen, dark_mode))
                .strong(),
        );
    });
    ui.end_row();

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
        ui.horizontal(|ui| {
            constellation_swatch(ui, constellation);
            ui.label(constellation.display_name());
        });
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

/// Color for the "fix used" count in the satellite badge, legible on the
/// current theme. Pass `ui.visuals().dark_mode`.
pub(crate) fn fix_count_color(count: u32, dark_mode: bool) -> Color32 {
    gt_ui_theme::fix_count_tier(count).color(dark_mode)
}

/// Color for the "total seen" count in the satellite badge, legible on the
/// current theme. Pass `ui.visuals().dark_mode`.
pub(crate) fn seen_count_color(count: u32, dark_mode: bool) -> Color32 {
    gt_ui_theme::seen_count_tier(count).color(dark_mode)
}

/// Zoom-derived visual parameters computed once per frame and shared across
/// all points in all tracks.
#[derive(Clone, Copy)]
pub(crate) struct TpvDrawStyle {
    pub(crate) outline_alpha: f32,
    pub(crate) base_arrow_size: f32,
    /// Opacity of the fix icon currently being drawn, in (0.0, 1.0].
    /// Determined per fix from its local on-screen spacing (see
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

/// Zoom-adaptive size scale for overlay glyphs: 1.0 where the fix icons
/// reach full size, shrinking to the icons' minimum fraction below that, so
/// glyphs shrink in step with the heading arrows rather than staying a fixed
/// pixel size as the track shrinks.
pub(crate) fn glyph_size_scale(style: &TpvDrawStyle) -> f32 {
    style.base_arrow_size / MAX_ARROW_SIZE_PX
}

/// How a track's fix icons fade this frame, determined O(1) from the track's
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
    /// moving track): opacity is determined per fix from its local spacing.
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
    let ppm = scale.pixels_per_meter(track.metadata.bounding_box.lat.center());
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
            .map(|p| (transform.to_screen(p.merc()) - screen_pos).length())
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
#[expect(
    clippy::too_many_arguments,
    reason = "render context requires all parameters; a context struct would not add clarity"
)]
fn draw_tpv_point(
    ui: &Ui,
    screen_pos: Pos2,
    point_kind: &PointKind,
    eph_m: Option<f32>,
    pixels_per_meter: f64,
    highlighted: bool,
    style: &TpvDrawStyle,
    batch: &mut IconMeshBatch<'_>,
) {
    // Accuracy circle - rendered beneath the icon. Skipped when too small to
    // see at all, and when small enough to be entirely covered by the icon.
    if let Some(eph_m) = eph_m {
        let radius = (f64::from(eph_m) * pixels_per_meter) as f32;
        let min_visible_radius = (style.base_arrow_size * ACCURACY_CIRCLE_MIN_VISIBLE_FACTOR)
            .max(MIN_ACCURACY_CIRCLE_RADIUS_PX);
        if radius >= min_visible_radius {
            // The circle must sit above earlier icons and below this point's.
            batch.barrier(ui.painter());
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
                    1.0_f32,
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
                batch,
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
                batch,
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
/// Which anchors get a label this frame is determined by
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
        let screen_pos = transform.to_screen(point.merc());
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

/// Push a hollow chevron for a ghost fix into the track's icon batch.
///
/// The chevron tip points in `direction` (the inferred travel direction).
/// Stacking against interleaved painter primitives is the caller's job via
/// [IconMeshBatch::barrier]; see [draw_tpv_point].
fn draw_ghost_chevron(
    batch: &mut IconMeshBatch<'_>,
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
    batch.push(IconInstance {
        icon: IconId::GhostFix,
        center,
        half_extents: Vec2::splat(size),
        direction: Some(direction),
        tints: [tint; 2],
    });
}

/// Draw the car-GPS style navigation arrow for a real fix.
///
/// The hot path pushes one two-tint-slot mesh instance into the track's
/// icon batch (fill tinted with the fix-quality color, rim white with the
/// fade alpha); stacking against interleaved painter primitives is handled
/// by the caller's [IconMeshBatch::barrier] calls, see [draw_tpv_point].
/// Highlighted arrows (at most the hovered and the sticky point per frame)
/// keep the painter implementation: their thicker blue outline has a
/// different stroke width than the baked 1.5 px rim, and at that count the
/// per-frame tessellation cost is irrelevant. That painter path barriers
/// the batch itself, and is also what draws when the embedded meshes failed
/// to decode, so arrows never disappear.
#[expect(
    clippy::too_many_arguments,
    reason = "render context requires all parameters; a context struct would not add clarity"
)]
fn draw_navigation_arrow(
    ui: &Ui,
    batch: &mut IconMeshBatch<'_>,
    center: Pos2,
    heading: Angle,
    color: Color32,
    highlighted: bool,
    outline_alpha: f32,
    base_size: f32,
) {
    let angle_rad = heading.get::<radian>() - std::f64::consts::FRAC_PI_2;
    let dir = egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32);

    if !highlighted {
        // The painter faded the rim by scaling both its alpha and its stroke
        // width with `outline_alpha`; the baked rim has a fixed width, so
        // squaring the alpha matches that width x alpha ink.
        let rim_alpha = outline_alpha * outline_alpha;
        batch.push(IconInstance {
            icon: IconId::NavArrow,
            center,
            half_extents: Vec2::splat(base_size),
            direction: Some(dir),
            tints: [color, Color32::WHITE.gamma_multiply(rim_alpha)],
        });
        return;
    }

    // Highlighted arrows keep the painter implementation (thicker blue rim
    // with its own stroke width); flush so it stacks above earlier icons.
    batch.barrier(ui.painter());

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
mod tests;

#[cfg(test)]
mod arrow_snapshot_tests;
