use crate::series::{TrackSeries, build_all_series, closest_point_index};
use chrono::{DateTime, Utc};
use egui::Color32;
use egui_plot::{Line, LineStyle, PlotPoints, VLine};
use gt_egui_mipmap::{LevelSelection, MipMap};
use gt_filter::GlobalFilter;
use gt_types::{FileIdx, LoadedFile, MetricKind, PointIdx, TrackIdx};
use gt_ui_types::{HighlightScope, TrackDataVisibility};
use rayon::prelude::*;
use std::collections::BTreeSet;
use strum::IntoEnumIterator;

/// Chip color, label, and optional hover tooltip for each [`MetricKind`].
///
/// `MetricKind` lives in `gt_types` (shared with the persisted settings, see
/// `geotrace::settings::PlotSettings::metric`); these are presentation
/// details specific to this widget, so they live here as an extension trait
/// rather than on the type itself. Together with the `match` in
/// [`MetricVisibility::field`] and [`MetricVisibility::field_mut`], adding a
/// variant forces a compile error here until every arm is filled in — chip
/// interaction, color lookup, label lookup, and mipmap dispatch all go
/// through one type rather than parallel arrays and magic-number match arms.
trait MetricKindUi {
    fn color(self) -> Color32;
    fn label(self) -> &'static str;
    fn hover_text(self) -> Option<&'static str>;
}

impl MetricKindUi for MetricKind {
    fn color(self) -> Color32 {
        match self {
            Self::SatsSeen => Color32::from_rgb(80, 200, 255), // powder blue
            Self::SatsFix => Color32::from_rgb(0, 100, 220),   // deep blue
            Self::GpsSeen => Color32::from_rgb(0, 220, 80),    // lime green
            Self::GpsFix => Color32::from_rgb(0, 140, 40),     // forest green
            Self::GlonassSeen => Color32::from_rgb(255, 140, 30), // golden
            Self::GlonassFix => Color32::from_rgb(200, 80, 0), // amber
            Self::GalileoSeen => Color32::from_rgb(255, 50, 110), // hot pink
            Self::GalileoFix => Color32::from_rgb(155, 30, 255), // purple
            Self::BeidouSeen => Color32::from_rgb(0, 230, 230), // cyan
            Self::BeidouFix => Color32::from_rgb(0, 160, 160), // teal
            Self::Velocity => Color32::from_rgb(255, 220, 0),  // bright yellow
            Self::Eph => Color32::from_rgb(220, 20, 220),      // magenta
            Self::HeadingDeg => Color32::from_rgb(255, 100, 50), // red-orange
            Self::ClockDeltaMs => Color32::from_rgb(200, 200, 200), // light gray
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SatsSeen => "Sats seen",
            Self::SatsFix => "Sats fix",
            Self::GpsSeen => "GPS seen",
            Self::GpsFix => "GPS fix",
            Self::GlonassSeen => "GLONASS seen",
            Self::GlonassFix => "GLONASS fix",
            Self::GalileoSeen => "Galileo seen",
            Self::GalileoFix => "Galileo fix",
            Self::BeidouSeen => "BeiDou seen",
            Self::BeidouFix => "BeiDou fix",
            Self::Velocity => "Velocity (km/h)",
            Self::Eph => "EPH (m)",
            Self::HeadingDeg => "Heading (°)",
            Self::ClockDeltaMs => "Clock Δt (ms)",
        }
    }

    fn hover_text(self) -> Option<&'static str> {
        match self {
            Self::Eph => Some(
                "Estimated Horizontal Position error - the GPS receiver's own estimate of how \
                 far the reported position may be from the true position, in metres. \
                 Lower is more accurate.",
            ),
            Self::ClockDeltaMs => Some(
                "GPS clock lead over the host system clock, in milliseconds. \
                 Positive = GPS clock ahead of the system clock; negative = system clock ahead. \
                 Only shown when the receiver reports a system timestamp alongside the GPS fix.",
            ),
            _ => None,
        }
    }
}

/// Per-file shade offsets applied to each metric's base colour.
///
/// Keeping hue fixed and only shifting value/lightness preserves metric identity
/// while still making overlapping lines from different files distinguishable.
///
/// Unlike [`gt_ui_theme::track_color`], which cycles a full colour palette per
/// (file, track) for the map (where hue *is* the identity signal), the plot
/// needs hue to stay tied to the metric - so files are distinguished by
/// lightness shift and line style ([`FILE_LINE_STYLES`]) instead.
const FILE_SHADE_FACTORS: [i16; 7] = [0, 22, -22, 12, -12, 32, -32];

/// File-level line styles to keep perfectly overlapping lines distinguishable.
///
/// Color still carries metric identity; style only disambiguates file source.
const FILE_LINE_STYLES: [LineStyle; 5] = [
    LineStyle::Solid,
    LineStyle::Dashed { length: 6.0 },
    LineStyle::Dotted { spacing: 5.0 },
    LineStyle::Dashed { length: 10.0 },
    LineStyle::Dotted { spacing: 8.0 },
];
/// Default legend overlay position, anchored just inside the plot's top-left
/// corner.
pub const LEGEND_DOCK_OFFSET: egui::Vec2 = egui::vec2(10.0, 10.0);
/// Sub-pixel tolerance for [`legend_is_docked`]'s "is this exactly the dock
/// position" check - not to be confused with [`LEGEND_DOCK_SNAP_RADIUS`],
/// the much larger radius used to *move* the legend onto the dock position.
const LEGEND_DOCK_POSITION_TOLERANCE: f32 = 1.0;
/// Background opacity of the file-style legend overlay, matching the default
/// `background_alpha` of egui_plot's built-in legend.
const LEGEND_BACKGROUND_ALPHA: f32 = 0.75;
/// Minimum distance the dragged legend keeps from the plot edges.
const LEGEND_EDGE_MARGIN: f32 = 6.0;
/// Distance from the docked top-left position within which a dragged legend
/// snaps back to docking, so dropping it near the corner re-docks it without
/// requiring a click on the re-dock button.
const LEGEND_DOCK_SNAP_RADIUS: f32 = 32.0;
/// Dimensions of the line-style swatch painted next to each legend entry.
const SWATCH_SIZE: egui::Vec2 = egui::vec2(26.0, 10.0);
const SWATCH_STROKE_WIDTH: f32 = 2.0;
/// Gap between dashes as a fraction of the dash length.
const SWATCH_DASH_GAP_RATIO: f32 = 0.62;
const SWATCH_DOT_RADIUS: f32 = 1.7;

/// Overlap budget expressed as a multiple of the single-track target
/// (`≈ 2 × plot_width_px`).  Tracks that overlap in time each span the full
/// width; this many of them can do so at full resolution before [`budget_cap`]
/// starts sharing the budget between them.  See [`budget_cap`].
const BUDGET_TRACK_MULTIPLE: usize = 8;

fn metric_line_color(kind: MetricKind, file_index: usize) -> Color32 {
    shade_color(kind.color(), file_shade_factor(file_index))
}

/// Full-resolution sample target for a single track filling the plot width:
/// ~2 samples per pixel, floored so a very narrow plot still has usable detail.
fn single_target(available_width: f32) -> usize {
    #[expect(
        clippy::cast_sign_loss,
        reason = "available_width is always ≥ 0 in practice; .max(0.0) makes it explicit"
    )]
    let px = available_width.max(0.0) as usize;
    (px * 2).max(400)
}

/// Upper bound on any single track's sample target.
///
/// Tracks that overlap in time all span the full plot width, so without a cap
/// N of them would each request [`single_target`] points.  Sharing a budget of
/// `single × BUDGET_TRACK_MULTIPLE` across the visible tracks bounds the total
/// handed to egui_plot in that worst case.  Tracks that occupy only part of the
/// width get far less via [`track_target`]; this cap only bites when many tracks
/// pile up in the same time range.
fn budget_cap(available_width: f32, visible_count: usize) -> usize {
    let single = single_target(available_width);
    let count = visible_count.max(1);
    (single.saturating_mul(BUDGET_TRACK_MULTIPLE) / count).clamp(2, single)
}

/// Sample target for one track: ~2 points per pixel of the track's *visible*
/// width within the current view, capped by `cap` and floored at 2 (a single
/// segment).
///
/// A track that occupies only a few pixels when zoomed out therefore hands only
/// a few points to the plot.  Paired with the mipmap cascading down to 2 points
/// (so a coarse-enough level actually exists), this is what keeps many short
/// tracks cheap - the fixed per-track target it replaces always pulled hundreds
/// of points per track regardless of on-screen size.
fn track_target(
    x_range: Option<(f64, f64)>,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    cap: usize,
) -> usize {
    let view = x_max - x_min;
    let Some((lo, hi)) = x_range else { return 2 };
    if !view.is_finite() || view <= 0.0 {
        return cap;
    }
    let visible = (hi.min(x_max) - lo.max(x_min)).max(0.0);
    let pixels = f64::from(available_width) * (visible / view);
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "pixels is finite and ≥ 0; truncating a ~2-per-pixel count to an integer is intended"
    )]
    let want = (2.0 * pixels) as usize;
    want.clamp(2, cap)
}

fn file_shade_factor(file_index: usize) -> i16 {
    let idx = file_index % FILE_SHADE_FACTORS.len();
    FILE_SHADE_FACTORS.get(idx).copied().unwrap_or(0)
}

fn file_line_style(file_index: usize) -> LineStyle {
    let idx = file_index % FILE_LINE_STYLES.len();
    FILE_LINE_STYLES
        .get(idx)
        .copied()
        .unwrap_or(LineStyle::Solid)
}

fn paint_line_style_swatch(ui: &mut egui::Ui, style: LineStyle, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(SWATCH_SIZE, egui::Sense::hover());
    let y = rect.center().y;
    let start = egui::pos2(rect.left(), y);
    let end = egui::pos2(rect.right(), y);
    let painter = ui.painter();
    match style {
        LineStyle::Solid => {
            painter.line_segment([start, end], egui::Stroke::new(SWATCH_STROKE_WIDTH, color));
        }
        LineStyle::Dashed { length } => {
            painter.extend(egui::Shape::dashed_line(
                &[start, end],
                egui::Stroke::new(SWATCH_STROKE_WIDTH, color),
                length,
                length * SWATCH_DASH_GAP_RATIO,
            ));
        }
        LineStyle::Dotted { spacing } => {
            painter.extend(egui::Shape::dotted_line(
                &[start, end],
                color,
                spacing,
                SWATCH_DOT_RADIUS,
            ));
        }
    }
    response
}

/// Shifts a color toward white (positive `factor_pct`) or black (negative)
/// by the given percentage.
fn shade_color(color: Color32, factor_pct: i16) -> Color32 {
    let (target, amount_pct) = if factor_pct >= 0 {
        (255, factor_pct)
    } else {
        (0, -factor_pct)
    };
    let num = i32::from(amount_pct.clamp(0, 100));
    Color32::from_rgb(
        gt_ui_theme::lerp_channel(color.r(), target, num, 100),
        gt_ui_theme::lerp_channel(color.g(), target, num, 100),
        gt_ui_theme::lerp_channel(color.b(), target, num, 100),
    )
}

/// Global per-metric visibility flags.
///
/// Disabling a metric hides it for **all** tracks at once, making it easy to
/// declutter the plot without touching per-track settings.
#[derive(Debug, Clone, Copy)]
pub struct MetricVisibility {
    pub sats_seen: bool,
    pub sats_fix: bool,
    pub gps_seen: bool,
    pub gps_fix: bool,
    pub glonass_seen: bool,
    pub glonass_fix: bool,
    pub galileo_seen: bool,
    pub galileo_fix: bool,
    pub beidou_seen: bool,
    pub beidou_fix: bool,
    pub velocity: bool,
    pub eph: bool,
    pub heading_deg: bool,
    pub clock_delta_ms: bool,
}

impl Default for MetricVisibility {
    fn default() -> Self {
        Self {
            sats_seen: true,
            sats_fix: true,
            gps_seen: true,
            gps_fix: true,
            glonass_seen: true,
            glonass_fix: true,
            galileo_seen: true,
            galileo_fix: true,
            beidou_seen: true,
            beidou_fix: true,
            velocity: true,
            eph: true,
            heading_deg: true,
            clock_delta_ms: true,
        }
    }
}

impl MetricVisibility {
    /// Returns the current visibility for `kind`.
    fn field(&self, kind: MetricKind) -> bool {
        match kind {
            MetricKind::SatsSeen => self.sats_seen,
            MetricKind::SatsFix => self.sats_fix,
            MetricKind::GpsSeen => self.gps_seen,
            MetricKind::GpsFix => self.gps_fix,
            MetricKind::GlonassSeen => self.glonass_seen,
            MetricKind::GlonassFix => self.glonass_fix,
            MetricKind::GalileoSeen => self.galileo_seen,
            MetricKind::GalileoFix => self.galileo_fix,
            MetricKind::BeidouSeen => self.beidou_seen,
            MetricKind::BeidouFix => self.beidou_fix,
            MetricKind::Velocity => self.velocity,
            MetricKind::Eph => self.eph,
            MetricKind::HeadingDeg => self.heading_deg,
            MetricKind::ClockDeltaMs => self.clock_delta_ms,
        }
    }

    /// Returns a mutable reference to the visibility flag for `kind`.
    fn field_mut(&mut self, kind: MetricKind) -> &mut bool {
        match kind {
            MetricKind::SatsSeen => &mut self.sats_seen,
            MetricKind::SatsFix => &mut self.sats_fix,
            MetricKind::GpsSeen => &mut self.gps_seen,
            MetricKind::GpsFix => &mut self.gps_fix,
            MetricKind::GlonassSeen => &mut self.glonass_seen,
            MetricKind::GlonassFix => &mut self.glonass_fix,
            MetricKind::GalileoSeen => &mut self.galileo_seen,
            MetricKind::GalileoFix => &mut self.galileo_fix,
            MetricKind::BeidouSeen => &mut self.beidou_seen,
            MetricKind::BeidouFix => &mut self.beidou_fix,
            MetricKind::Velocity => &mut self.velocity,
            MetricKind::Eph => &mut self.eph,
            MetricKind::HeadingDeg => &mut self.heading_deg,
            MetricKind::ClockDeltaMs => &mut self.clock_delta_ms,
        }
    }

    /// Returns `true` when every metric is enabled.
    fn all_enabled(self) -> bool {
        MetricKind::iter().all(|k| self.field(k))
    }

    /// Set every metric to `enabled`.
    fn set_all(&mut self, enabled: bool) {
        for k in MetricKind::iter() {
            *self.field_mut(k) = enabled;
        }
    }
}

/// Cached level selections for all 12 metrics of one track's series.
#[derive(Debug, Clone, Copy, Default)]
struct TripLevelCache {
    total_seen: LevelSelection,
    total_fix: LevelSelection,
    gps_seen: LevelSelection,
    gps_fix: LevelSelection,
    glonass_seen: LevelSelection,
    glonass_fix: LevelSelection,
    galileo_seen: LevelSelection,
    galileo_fix: LevelSelection,
    beidou_seen: LevelSelection,
    beidou_fix: LevelSelection,
    velocity_kmh: LevelSelection,
    eph_m: LevelSelection,
    heading_deg: LevelSelection,
    clock_delta_ms: LevelSelection,
}

impl TripLevelCache {
    fn level_for(&self, kind: MetricKind) -> LevelSelection {
        match kind {
            MetricKind::SatsSeen => self.total_seen,
            MetricKind::SatsFix => self.total_fix,
            MetricKind::GpsSeen => self.gps_seen,
            MetricKind::GpsFix => self.gps_fix,
            MetricKind::GlonassSeen => self.glonass_seen,
            MetricKind::GlonassFix => self.glonass_fix,
            MetricKind::GalileoSeen => self.galileo_seen,
            MetricKind::GalileoFix => self.galileo_fix,
            MetricKind::BeidouSeen => self.beidou_seen,
            MetricKind::BeidouFix => self.beidou_fix,
            MetricKind::Velocity => self.velocity_kmh,
            MetricKind::Eph => self.eph_m,
            MetricKind::HeadingDeg => self.heading_deg,
            MetricKind::ClockDeltaMs => self.clock_delta_ms,
        }
    }
}

impl crate::series::TrackSeries {
    fn mipmap_for(&self, kind: MetricKind) -> &gt_egui_mipmap::MipMap {
        match kind {
            MetricKind::SatsSeen => &self.total_seen,
            MetricKind::SatsFix => &self.total_fix,
            MetricKind::GpsSeen => &self.gps_seen,
            MetricKind::GpsFix => &self.gps_fix,
            MetricKind::GlonassSeen => &self.glonass_seen,
            MetricKind::GlonassFix => &self.glonass_fix,
            MetricKind::GalileoSeen => &self.galileo_seen,
            MetricKind::GalileoFix => &self.galileo_fix,
            MetricKind::BeidouSeen => &self.beidou_seen,
            MetricKind::BeidouFix => &self.beidou_fix,
            MetricKind::Velocity => &self.velocity_kmh,
            MetricKind::Eph => &self.eph_m,
            MetricKind::HeadingDeg => &self.heading_deg,
            MetricKind::ClockDeltaMs => &self.clock_delta_ms,
        }
    }
}

/// Persistent state for the track plot panel.
///
/// Plot panel visibility is managed externally (via the tiles tree in the app),
/// so this struct only tracks the cursor hover time and the mipmap series cache.
#[derive(Debug, Clone)]
pub struct PlotState {
    /// Time currently hovered by the plot cursor, written each frame.
    /// `None` when the cursor is outside the plot area.
    pub hovered_time: Option<DateTime<Utc>>,
    /// Global per-metric visibility - toggled via the chip row above the plot.
    pub metric_vis: MetricVisibility,
    /// Whether the plot grid lines are visible.
    pub show_grid: bool,
    /// When true, the plot x-range tracks the map viewport.
    pub sync_to_map: bool,
    /// Whether the file-style legend body is collapsed.
    pub file_legend_collapsed: bool,
    /// Legend overlay position offset from the plot's top-left corner.
    pub file_legend_offset: egui::Vec2,
    /// Legend overlay size measured on the previous frame.
    /// Used to size this frame's drag-sensing background before the
    /// legend's content is laid out.
    file_legend_size: egui::Vec2,
    /// File index currently hovered in the legend overlay.
    pub legend_hover_file: Option<usize>,
    /// Mipmap cascade for every track in every loaded file.
    pub(crate) series_cache: Vec<TrackSeries>,
    /// Cached level selections, one entry per series.
    /// Invalidated when the effective plot bounds or target sample count changes.
    level_cache: Vec<TripLevelCache>,
    /// The `(eff_x_min, eff_x_max, plot_width_bits, sample_cap)` at which the
    /// current `level_cache` was computed.  `None` forces a recompute on the
    /// next frame.  Used for hysteresis: the cache is reused as long as the view
    /// has not moved by more than ~10 pixels and neither the plot width nor the
    /// per-track sample cap has changed since the last recompute.
    last_computed_bounds: Option<(f64, f64, u32, usize)>,
    /// The map x-range (encoded as bit-pattern pairs) most recently applied to
    /// the plot via `set_plot_bounds_x`.  Used to detect changes and avoid
    /// re-applying the same range every frame (which would prevent manual zoom).
    applied_map_x_range: Option<(u64, u64)>,
    /// Whether the plot cursor was snapped close to a data point on the most
    /// recently rendered frame.
    ///
    /// Set by [`show_track_plot`] using a 2-D screen-space distance check
    /// against every enabled metric line.  The app layer forwards this to
    /// [`gt_ui_types::MapHighlight::plot_hover_snapped`] so the map overlay
    /// activates only when the cursor is genuinely near a plotted value, not
    /// just anywhere inside the plot area.
    pub plot_cursor_snapped: bool,
}

impl Default for PlotState {
    fn default() -> Self {
        Self {
            hovered_time: None,
            metric_vis: MetricVisibility::default(),
            show_grid: true,
            sync_to_map: true,
            file_legend_collapsed: false,
            file_legend_offset: LEGEND_DOCK_OFFSET,
            file_legend_size: egui::Vec2::ZERO,
            legend_hover_file: None,
            series_cache: Vec::new(),
            level_cache: Vec::new(),
            last_computed_bounds: None,
            applied_map_x_range: None,
            plot_cursor_snapped: false,
        }
    }
}

impl PlotState {
    /// Incorporate pre-built mipmap series for file `fi`.
    ///
    /// Replaces any existing series for that file index and invalidates the
    /// level cache so the next frame recomputes level selections.
    pub fn integrate_file(&mut self, fi: usize, prepared: crate::PreparedSeries) {
        self.series_cache.retain(|s| s.fi != fi);
        // Re-insert at a stable position: find where fi would sit among existing
        // file indices so the ordering stays consistent with the loaded_files vec.
        let insert_pos = self
            .series_cache
            .iter()
            .position(|s| s.fi > fi)
            .unwrap_or(self.series_cache.len());
        for (offset, mut series) in prepared.0.into_iter().enumerate() {
            series.fi = fi;
            self.series_cache.insert(insert_pos + offset, series);
        }
        self.invalidate_level_cache();
    }

    /// Rebuild series for all currently loaded files from scratch.
    ///
    /// Called after file deletion - runs on the UI thread since deletion is
    /// cheap (files already parsed, just re-indexing the surviving files).
    pub fn rebuild_all(&mut self, files: &[LoadedFile]) {
        self.series_cache = build_all_series(files);
        self.invalidate_level_cache();
    }

    fn invalidate_level_cache(&mut self) {
        self.last_computed_bounds = None;
        self.level_cache.clear();
    }
}

/// Render the track plot panel.
///
/// - `map_hover_time`: the timestamp of the TPV point currently hovered on
///   the map (if any).  The plot draws a vertical cursor line at this time so
///   the user can see the map-selected moment in context.
/// - `state.hovered_time` is written with the plot cursor's current time each
///   frame.  The caller should forward this to `MapHighlight::plot_hover_time`
///   before drawing the map so the renderer can cross-highlight the nearest
///   TPV arrow.
#[expect(
    clippy::too_many_arguments,
    reason = "plot rendering needs files, visibility/filter state, cross-highlight inputs, map-sync range, and mutable plot state"
)]
pub fn show_track_plot(
    ui: &mut egui::Ui,
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    hover_scope: Option<HighlightScope>,
    map_hover_time: Option<DateTime<Utc>>,
    // When map→plot sync is enabled, this carries the Unix-second x range
    // computed from TPV points visible in the current map viewport.
    // The plot will pan/zoom to this range the first frame it changes.
    map_sync_x_range: Option<(f64, f64)>,
    state: &mut PlotState,
) {
    // Compute the per-series visibility mask once so the three downstream
    // consumers - visible_count, the full-x-range loop, and the render loop -
    // all share a single pass instead of calling trip_is_visible three times
    // per series per frame.
    let visible: Vec<bool> = state
        .series_cache
        .iter()
        .map(|s| trip_is_visible(visibility, filter, files, s.fi, s.ti))
        .collect();
    let visible_count = visible.iter().filter(|&&v| v).count();

    if visible_count == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("Load a .gtd file to see track metrics")
                    .weak()
                    .italics(),
            );
        });
        state.hovered_time = None;
        return;
    }

    // Per-series count, for the line-name prefix; distinct from the
    // per-file count below, which gates the file legend overlay.
    let multi_track = visible_count > 1;
    let visible_files: Vec<usize> = {
        let mut file_indices = BTreeSet::new();
        for (series, &is_vis) in state.series_cache.iter().zip(visible.iter()) {
            if is_vis {
                file_indices.insert(series.fi);
            }
        }
        file_indices.into_iter().collect()
    };

    // Draw the per-metric filter row before the plot so it consumes vertical
    // space first; `ui.available_height()` below then gives the remainder.
    let hovered_chip = metric_filter_row(
        ui,
        &mut state.metric_vis,
        &mut state.show_grid,
        &mut state.sync_to_map,
    );

    // Sample budgeting: each track requests ~2 points per pixel of its *visible*
    // width (`track_target`, computed per series below), so a track that only
    // occupies a few pixels when zoomed out hands over only a few points.  The
    // cap bounds the worst case where many tracks overlap in the same time range
    // and each spans the full width.  Together with the mipmap now cascading
    // down to 2 points, this is what keeps a screen full of short tracks cheap.
    let available_width = ui.available_width();
    let sample_cap = budget_cap(available_width, visible_count);

    // Filter time range → x-axis bounds for mipmap slice clamping.
    let filter_x_min: Option<f64> = filter.time_start.map(|t| t.timestamp() as f64);
    let filter_x_max: Option<f64> = filter.time_end.map(|t| t.timestamp() as f64);

    // Format an x-axis tick label from a Unix timestamp.
    let x_fmt = |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
        let ts = mark.value as i64;
        DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default()
    };

    // Format the hover tooltip: show time + value.
    let label_fmt = |name: &str, val: &egui_plot::PlotPoint| {
        let ts = val.x as i64;
        let time_str = DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        format!("{name}\n{time_str}\n{:.2}", val.y)
    };

    // Compute the full x range across all visible series so that double-click
    // (which triggers auto-bounds reset) zooms to fit the complete dataset.
    // Uses the precomputed `TrackSeries::x_range` field - O(1) per series.
    let mut full_x_min = f64::INFINITY;
    let mut full_x_max = f64::NEG_INFINITY;
    for (series, &is_vis) in state.series_cache.iter().zip(visible.iter()) {
        if !is_vis {
            continue;
        }

        if let Some((lo, hi)) = series.x_range {
            full_x_min = full_x_min.min(lo);
            full_x_max = full_x_max.max(hi);
        }
    }
    let has_full_range = full_x_min.is_finite() && full_x_max.is_finite();

    // Split borrows: extract immutable refs to the caches and metric visibility
    // before the closure so the borrow checker can see they are disjoint from
    // the mutable fields written after the closure (`hovered_time`, `level_cache`,
    // `last_computed_bounds`).
    let series_cache = &state.series_cache;
    let level_cache = &state.level_cache;
    let last_computed_bounds = state.last_computed_bounds;
    let metric_vis = &state.metric_vis;
    let effective_hover_scope = state
        .legend_hover_file
        .map(|fi| HighlightScope::File {
            file_index: FileIdx::new(fi),
        })
        .or(hover_scope);

    // Encode the incoming map sync range as bit patterns so we can compare
    // without float equality warnings.
    let map_x_key = map_sync_x_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
    let need_map_sync = map_x_key.is_some_and(|k| state.applied_map_x_range != Some(k));

    let mut new_hovered_time: Option<DateTime<Utc>> = None;
    let mut new_computed_bounds: Option<(f64, f64, u32, usize)> = None;
    let mut new_level_cache: Option<Vec<TripLevelCache>> = None;
    let mut new_applied_map_x_range: Option<Option<(u64, u64)>> = None;

    let mut plot = egui_plot::Plot::new("track_plot")
        .height(ui.available_height())
        .show_grid(state.show_grid)
        .x_axis_formatter(x_fmt)
        .label_formatter(label_fmt);

    // Tell egui_plot the full data extent so double-click reset zooms to fit.
    if has_full_range {
        plot = plot.include_x(full_x_min).include_x(full_x_max);
    }

    let plot_response = plot.show(ui, |plot_ui| {
        let bounds = plot_ui.plot_bounds();
        let plot_x_min = bounds.min()[0];
        let plot_x_max = bounds.max()[0];

        // Intersect the visible plot range with the active time filter.
        //
        // `eff_x_min`/`eff_x_max` may end up inverted when the active filter
        // and the visible viewport don't overlap; `MipMap` normalizes that
        // into an empty-range query (see `select_level_bounds`).
        let eff_x_min = filter_x_min.map_or(plot_x_min, |f| plot_x_min.max(f));
        let eff_x_max = filter_x_max.map_or(plot_x_max, |f| plot_x_max.min(f));

        // Hysteresis: skip recompute when the view has moved less than ~10 px
        // since the last cache fill.  Converting to data space:
        //   10 px × (data_range / plot_width_px) = 20 × data_range / single
        // (single ≈ 2 × plot_width_px, always ≥ 400).  The cache also depends on
        // the plot width and visible count (both feed the per-track targets), so
        // those are part of the validity check, not just the view bounds.
        let single = single_target(available_width);
        let threshold = 20.0 * (eff_x_max - eff_x_min) / single as f64;
        let cache_valid = last_computed_bounds.is_some_and(|(lx_min, lx_max, lw, lcap)| {
            lw == available_width.to_bits()
                && lcap == sample_cap
                && level_cache.len() == series_cache.len()
                && (eff_x_min - lx_min).abs() <= threshold
                && (eff_x_max - lx_max).abs() <= threshold
        });

        // Recompute if the view changed enough since the last frame.
        // Uses rayon to parallelise across series - each is independent.
        let resolved: std::borrow::Cow<[TripLevelCache]> = if cache_valid {
            std::borrow::Cow::Borrowed(level_cache)
        } else {
            let fresh: Vec<TripLevelCache> = series_cache
                .par_iter()
                .map(|s| compute_level_cache(s, eff_x_min, eff_x_max, available_width, sample_cap))
                .collect();
            new_computed_bounds =
                Some((eff_x_min, eff_x_max, available_width.to_bits(), sample_cap));
            std::borrow::Cow::Owned(fresh)
        };

        // Pan the plot to the map-visible time range when it changes.
        if need_map_sync {
            if let Some((x_min, x_max)) = map_sync_x_range {
                plot_ui.set_plot_bounds_x(x_min..=x_max);
            }
            new_applied_map_x_range = Some(map_x_key);
        }

        debug_assert_eq!(visible.len(), series_cache.len());
        for (si, (vis, series)) in visible.iter().zip(series_cache.iter()).enumerate() {
            if !vis {
                continue;
            }
            // `resolved` has the same length as `series_cache` by construction.
            let Some(cache) = resolved.get(si) else {
                continue;
            };
            add_series_lines(
                plot_ui,
                series,
                multi_track,
                cache,
                metric_vis,
                hovered_chip,
                effective_hover_scope,
            );
        }

        if let std::borrow::Cow::Owned(owned) = resolved {
            new_level_cache = Some(owned);
        }
        if let Some(t) = map_hover_time {
            let x = t.timestamp() as f64;
            plot_ui.vline(
                VLine::new("Map position", x)
                    .color(gt_ui_theme::HIGHLIGHT_BLUE_SEEK)
                    .width(1.5),
            );
        }

        new_hovered_time = plot_ui
            .pointer_coordinate()
            .and_then(|c| DateTime::from_timestamp(c.x as i64, 0));
    });

    // Persist the newly computed level cache (only when a recompute happened).
    if let Some(bounds) = new_computed_bounds {
        state.last_computed_bounds = Some(bounds);
    }
    if let Some(cache) = new_level_cache {
        state.level_cache = cache;
    }
    if let Some(applied) = new_applied_map_x_range {
        state.applied_map_x_range = applied;
    }
    // `hovered_plot_item` is set by egui_plot when the cursor is within its own
    // interact radius of a plotted item — the exact condition that causes
    // egui_plot to show a hover label.  Use it directly so the map overlay
    // activates at precisely the same moment, with no custom approximation.
    state.plot_cursor_snapped =
        plot_response.response.hovered() && plot_response.hovered_plot_item.is_some();
    state.legend_hover_file = show_file_legend_overlay(
        ui,
        files,
        &visible_files,
        plot_response.response.rect,
        state,
    );

    // Clear the hovered time when the cursor leaves the plot area.
    state.hovered_time = if plot_response.response.hovered() {
        new_hovered_time
    } else {
        None
    };
}

fn show_file_legend_overlay(
    ui: &egui::Ui,
    files: &[LoadedFile],
    visible_files: &[usize],
    plot_rect: egui::Rect,
    state: &mut PlotState,
) -> Option<usize> {
    if visible_files.len() < 2 {
        return None;
    }

    let mut redock_requested = false;
    let show_redock_icon = !legend_is_docked(state.file_legend_offset);
    let legend_id = ui.id().with("plot_file_legend_overlay");
    let drag_bg_size = state.file_legend_size;
    let area = egui::Area::new(legend_id)
        .order(egui::Order::Foreground)
        .movable(false)
        .current_pos(plot_rect.min + state.file_legend_offset)
        .show(ui.ctx(), |ui| {
            // Selectable labels also sense drag (for text selection) and
            // would win the hit-test over `drag_response` below.
            ui.style_mut().interaction.selectable_labels = false;

            // Drag-sense the whole body first (bottom z-order) so buttons
            // on top still get their clicks.
            let drag_rect = egui::Rect::from_min_size(ui.cursor().min, drag_bg_size);
            let drag_response =
                ui.interact(drag_rect, legend_id.with("drag_bg"), egui::Sense::drag());

            let hovered_file = egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(8, 4))
                .corner_radius(ui.visuals().window_corner_radius)
                .fill(ui.visuals().extreme_bg_color)
                .stroke(ui.visuals().window_stroke())
                .multiply_with_opacity(LEGEND_BACKGROUND_ALPHA)
                .show(ui, |ui| {
                    let mut hovered_file = None;
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let dock_btn_size = egui::vec2(
                                ui.spacing().interact_size.y,
                                ui.spacing().interact_size.y,
                            );
                            if show_redock_icon
                                && ui
                                    .add_sized(
                                        dock_btn_size,
                                        egui::Button::new(
                                            egui_phosphor::regular::ARROW_LINE_UP_LEFT,
                                        ),
                                    )
                                    .on_hover_text("Re-dock legend to top-left")
                                    .clicked()
                            {
                                redock_requested = true;
                            }
                            ui.add_sized(
                                dock_btn_size,
                                egui::Label::new(
                                    egui::RichText::new(egui_phosphor::regular::DOTS_SIX).weak(),
                                ),
                            )
                            .on_hover_cursor(egui::CursorIcon::Grab)
                            .on_hover_text("Drag to move legend");
                            let fold_icon = if state.file_legend_collapsed {
                                egui_phosphor::regular::CARET_RIGHT
                            } else {
                                egui_phosphor::regular::CARET_DOWN
                            };
                            if ui
                                .small_button(fold_icon)
                                .on_hover_text(if state.file_legend_collapsed {
                                    "Expand legend"
                                } else {
                                    "Collapse legend"
                                })
                                .clicked()
                            {
                                state.file_legend_collapsed = !state.file_legend_collapsed;
                            }
                        });
                        if !state.file_legend_collapsed {
                            for &fi in visible_files {
                                let row = ui.horizontal(|ui| {
                                    let style = file_line_style(fi);
                                    let file_name = files
                                        .get(fi)
                                        .map_or("Unknown file", |f| f.metadata.filename.as_str());
                                    let swatch = paint_line_style_swatch(
                                        ui,
                                        style,
                                        ui.visuals().text_color(),
                                    );
                                    let name = ui.label(egui::RichText::new(file_name).small());
                                    swatch.hovered() || name.hovered()
                                });
                                if row.response.hovered() || row.inner {
                                    hovered_file = Some(fi);
                                }
                            }
                        }
                    });
                    hovered_file
                })
                .inner;

            (hovered_file, drag_response)
        });

    let (hovered_file, drag_response) = area.inner;
    state.file_legend_size = area.response.rect.size();

    if drag_response.dragged() {
        state.file_legend_offset += ui.ctx().input(|i| i.pointer.delta());
    }

    state.file_legend_offset = resolve_legend_offset(
        state.file_legend_offset,
        state.file_legend_size,
        plot_rect,
        redock_requested,
        drag_response.drag_stopped(),
    );
    hovered_file
}

/// Clamps `offset` to the plot's edges, then snaps it to [`LEGEND_DOCK_OFFSET`]
/// if redocking was requested or the drag just ended near that corner.
fn resolve_legend_offset(
    offset: egui::Vec2,
    legend_size: egui::Vec2,
    plot_rect: egui::Rect,
    redock_requested: bool,
    drag_released: bool,
) -> egui::Vec2 {
    let max_x = (plot_rect.width() - legend_size.x - LEGEND_EDGE_MARGIN).max(LEGEND_EDGE_MARGIN);
    let max_y = (plot_rect.height() - legend_size.y - LEGEND_EDGE_MARGIN).max(LEGEND_EDGE_MARGIN);
    let clamped = egui::vec2(
        offset.x.clamp(LEGEND_EDGE_MARGIN, max_x),
        offset.y.clamp(LEGEND_EDGE_MARGIN, max_y),
    );
    // Snap only on release, so dragging away from the dock isn't pulled
    // straight back mid-drag.
    let near_dock =
        drag_released && (clamped - LEGEND_DOCK_OFFSET).length() < LEGEND_DOCK_SNAP_RADIUS;
    if redock_requested || near_dock {
        LEGEND_DOCK_OFFSET
    } else {
        clamped
    }
}

/// Whether `offset` is close enough to [`LEGEND_DOCK_OFFSET`] to be considered docked.
pub fn legend_is_docked(offset: egui::Vec2) -> bool {
    (offset.x - LEGEND_DOCK_OFFSET.x).abs() < LEGEND_DOCK_POSITION_TOLERANCE
        && (offset.y - LEGEND_DOCK_OFFSET.y).abs() < LEGEND_DOCK_POSITION_TOLERANCE
}

/// Draw the per-metric filter controls above the track plot.
///
/// All controls and metric chips share a single `horizontal_wrapped` row so they
/// fill available horizontal space before wrapping - no fixed-height satellite
/// group that forces other chips below it.
///
/// Returns the `MetricKind` currently being hovered, or `None`.
/// The caller passes this to `add_series_lines` to highlight the hovered metric
/// and dim the rest, mirroring the standard egui-plot legend hover behaviour.
fn metric_filter_row(
    ui: &mut egui::Ui,
    vis: &mut MetricVisibility,
    show_grid: &mut bool,
    sync_to_map: &mut bool,
) -> Option<MetricKind> {
    let all_on = vis.all_enabled();
    let mut show_only = None;
    let mut hovered_chip = None;

    ui.horizontal_wrapped(|ui| {
        // Sync toggle - placed first, to the left of the grid button.
        if ui
            .selectable_label(*sync_to_map, egui_phosphor::regular::LINK)
            .on_hover_text(if *sync_to_map {
                "Syncing plot time range to map viewport — click to disable"
            } else {
                "Sync plot time range to map viewport"
            })
            .clicked()
        {
            *sync_to_map = !*sync_to_map;
        }

        // Grid toggle - icon button with tooltip.
        if ui
            .small_button(egui_phosphor::regular::GRID_FOUR)
            .on_hover_text(if *show_grid { "Hide grid" } else { "Show grid" })
            .clicked()
        {
            *show_grid = !*show_grid;
        }

        // Show/hide all - icon button with tooltip.
        let eye_icon = if all_on {
            egui_phosphor::regular::EYE_SLASH
        } else {
            egui_phosphor::regular::EYE
        };
        if ui
            .small_button(eye_icon)
            .on_hover_text(if all_on {
                "Hide all metrics"
            } else {
                "Show all metrics"
            })
            .clicked()
        {
            vis.set_all(!all_on);
        }

        ui.separator();

        // Summary metrics (total satellite counts, velocity, EPH, heading, clock delta).
        for kind in [
            MetricKind::SatsSeen,
            MetricKind::SatsFix,
            MetricKind::Velocity,
            MetricKind::Eph,
            MetricKind::HeadingDeg,
            MetricKind::ClockDeltaMs,
        ] {
            let (s, h) = metric_chip(
                ui,
                vis.field_mut(kind),
                kind.label(),
                kind.color(),
                kind.hover_text(),
            );
            if s {
                show_only = Some(kind);
            }
            if h {
                hovered_chip = Some(kind);
            }
        }

        ui.separator();

        // Per-constellation chips grouped together.
        for kind in [
            MetricKind::GpsSeen,
            MetricKind::GpsFix,
            MetricKind::GlonassSeen,
            MetricKind::GlonassFix,
            MetricKind::GalileoSeen,
            MetricKind::GalileoFix,
            MetricKind::BeidouSeen,
            MetricKind::BeidouFix,
        ] {
            let (s, h) = metric_chip(
                ui,
                vis.field_mut(kind),
                kind.label(),
                kind.color(),
                kind.hover_text(),
            );
            if s {
                show_only = Some(kind);
            }
            if h {
                hovered_chip = Some(kind);
            }
        }
    });

    // Apply "Show only this" - disable everything, then re-enable the chosen one.
    if let Some(kind) = show_only {
        vis.set_all(false);
        *vis.field_mut(kind) = true;
    }

    hovered_chip
}

/// A small colored toggle chip.  Left-click toggles the metric.  Right-click
/// opens a context menu with "Show only this".
///
/// Returns `(show_only, hovered)` - `show_only` is `true` when the user chose
/// "Show only this" from the context menu; `hovered` is `true` while the pointer
/// is over this chip.
fn metric_chip(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    name: &str,
    color: Color32,
    tooltip: Option<&str>,
) -> (bool, bool) {
    let fill = if *enabled {
        color.gamma_multiply(0.75)
    } else {
        color.gamma_multiply(0.12)
    };
    let text_color = if *enabled {
        Color32::WHITE
    } else {
        Color32::from_gray(100)
    };
    let btn = egui::Button::new(egui::RichText::new(name).color(text_color).small())
        .fill(fill)
        .corner_radius(4.0);
    let response = ui.add(btn);
    if response.clicked() {
        *enabled = !*enabled;
    }
    if let Some(tip) = tooltip {
        response.clone().on_hover_text(tip);
    }
    let mut show_only = false;
    response.context_menu(|ui| {
        if ui.button("Show only this").clicked() {
            show_only = true;
            ui.close();
        }
    });
    (show_only, response.hovered())
}

/// Compute fresh level selections for all metrics of one track's series.
///
/// The sample target is derived per track from how many pixels the track
/// occupies in the current view ([`track_target`]), so a track that is only a
/// few pixels wide selects a coarse mipmap level with just a few points.
fn compute_level_cache(
    series: &TrackSeries,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    sample_cap: usize,
) -> TripLevelCache {
    let target = track_target(series.x_range, x_min, x_max, available_width, sample_cap);
    let sel = |mm: &MipMap| mm.select_indices(x_min, x_max, target);
    TripLevelCache {
        total_seen: sel(&series.total_seen),
        total_fix: sel(&series.total_fix),
        gps_seen: sel(&series.gps_seen),
        gps_fix: sel(&series.gps_fix),
        glonass_seen: sel(&series.glonass_seen),
        glonass_fix: sel(&series.glonass_fix),
        galileo_seen: sel(&series.galileo_seen),
        galileo_fix: sel(&series.galileo_fix),
        beidou_seen: sel(&series.beidou_seen),
        beidou_fix: sel(&series.beidou_fix),
        velocity_kmh: sel(&series.velocity_kmh),
        eph_m: sel(&series.eph_m),
        heading_deg: sel(&series.heading_deg),
        clock_delta_ms: sel(&series.clock_delta_ms),
    }
}

/// Returns `true` when the track at `(fi, ti)` passes visibility and filter checks.
fn trip_is_visible(
    visibility: &TrackDataVisibility,
    global_filter: &GlobalFilter,
    files: &[LoadedFile],
    fi: usize,
    ti: usize,
) -> bool {
    let Some(file_vis) = visibility.files.get(fi) else {
        return false;
    };
    if !file_vis.enabled {
        return false;
    }
    let Some(trip_vis) = file_vis.tracks.get(ti) else {
        return false;
    };
    if !trip_vis.enabled {
        return false;
    }
    let Some(file) = files.get(fi) else {
        return false;
    };
    let Some(track) = file.tracks.get(ti) else {
        return false;
    };
    gt_filter::track_passes_filter(&track.metadata, global_filter)
}

/// Add all metric lines for one track to the plot using pre-computed level selections.
///
/// When `hovered_chip` is `Some(kind)`, that metric is highlighted (double stroke
/// width) and every other line is dimmed to 20 % brightness - mirroring the
/// standard egui-plot legend hover behaviour.
///
/// The `'a` lifetime ties both `plot_ui` and `series` together so that
/// [`egui_plot::PlotPoints::Borrowed`] can reference mipmap slices directly
/// without any per-frame allocation.
fn add_series_lines<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    series: &'a TrackSeries,
    multi_track: bool,
    cache: &TripLevelCache,
    metric_vis: &MetricVisibility,
    hovered_chip: Option<MetricKind>,
    hover_scope: Option<HighlightScope>,
) {
    let prefix = if multi_track {
        format!("{}: ", series.label)
    } else {
        String::new()
    };
    let focused = series_matches_hover_scope(series, hover_scope);
    let has_track_focus = hover_scope.is_some();

    for kind in MetricKind::iter() {
        if !metric_vis.field(kind) {
            continue;
        }
        let line_style = file_line_style(series.fi);
        let (mut color, metric_highlighted) = match hovered_chip {
            Some(h) if h == kind => (metric_line_color(kind, series.fi), true),
            Some(_) => (
                metric_line_color(kind, series.fi).gamma_multiply(0.2),
                false,
            ),
            None => (metric_line_color(kind, series.fi), false),
        };
        if has_track_focus && !focused {
            color = color.gamma_multiply(0.2);
        }
        add_line(
            plot_ui,
            series.mipmap_for(kind).slice_at(cache.level_for(kind)),
            format!("{prefix}{}", kind.label()),
            color,
            line_style,
            metric_highlighted || (has_track_focus && focused),
        );
    }
}

fn series_matches_hover_scope(series: &TrackSeries, hover_scope: Option<HighlightScope>) -> bool {
    match hover_scope {
        Some(HighlightScope::File { file_index }) => file_index.as_usize() == series.fi,
        Some(HighlightScope::Track(track)) | Some(HighlightScope::TrackCategory { track, .. }) => {
            track.fi.as_usize() == series.fi && track.index.as_usize() == series.ti
        }
        Some(HighlightScope::Point(_)) | None => true,
    }
}

/// Submit one metric line to the plot, borrowing the point slice directly via
/// [`PlotPoints::Borrowed`] - no allocation.
///
/// Skips slices with fewer than two points: a single-point line produces no
/// visible geometry and would clutter the legend.
///
/// The shared lifetime `'a` ensures the borrowed slice lives at least as long
/// as the `PlotUi` that will consume it (required because `PlotUi<'a>` is
/// invariant over `'a`).
fn add_line<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    data: &'a [egui_plot::PlotPoint],
    name: String,
    color: Color32,
    style: LineStyle,
    highlighted: bool,
) {
    if data.len() < 2 {
        return;
    }
    plot_ui.line(
        Line::new(name, PlotPoints::Borrowed(data))
            .color(color)
            .style(style)
            .highlight(highlighted),
    );
}

/// Given a set of visible tracks and a plot-hovered time (in seconds since
/// epoch), find the `(file_index, track_index, point_index)` of the closest
/// TPV point across all visible tracks.
///
/// Called by the app to set `MapHighlight::plot_hover_time` after the plot renders.
pub fn find_closest_tpv(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    target: DateTime<Utc>,
) -> Option<(FileIdx, TrackIdx, PointIdx)> {
    let target_secs = target.timestamp() as f64;
    let mut best: Option<(FileIdx, TrackIdx, PointIdx, f64)> = None;

    for (fi, file) in files.iter().enumerate() {
        let fi = FileIdx::new(fi);
        let Some(file_vis) = fi.get(&visibility.files) else {
            continue;
        };
        if !file_vis.enabled {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            let ti = TrackIdx::new(ti);
            let Some(trip_vis) = ti.get(&file_vis.tracks) else {
                continue;
            };
            if !trip_vis.enabled {
                continue;
            }
            if !gt_filter::track_passes_filter(&track.metadata, filter) {
                continue;
            }
            let Some(pi) = closest_point_index(&track.points, target_secs) else {
                continue;
            };
            let pi = PointIdx::new(pi);
            let Some(point) = pi.get(&track.points) else {
                continue;
            };
            let dist = (point.tpv.time().as_secs_f64() - target_secs).abs();
            if best.is_none_or(|(_, _, _, d)| dist < d) {
                best = Some((fi, ti, pi, dist));
            }
        }
    }

    best.map(|(fi, ti, pi, _)| (fi, ti, pi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_target_scales_with_visible_pixels() {
        let width = 1000.0;
        let cap = single_target(width);

        // A track spanning the whole view gets ~2 points per pixel (width is
        // 1000 px, so a full-width track should exceed that).
        let full = track_target(Some((0.0, 100.0)), 0.0, 100.0, width, cap);
        assert!(full > 1000, "full-width track should be ~2 pts/pixel");
        assert!(full <= cap);

        // A track occupying ~1% of the view (~10 px) hands over only a handful
        // of points - the old fixed target pulled hundreds regardless.
        let tiny = track_target(Some((0.0, 1.0)), 0.0, 100.0, width, cap);
        assert!(tiny >= 2);
        assert!(
            tiny <= 32,
            "few-pixel track must hand over few points, got {tiny}"
        );

        // An empty track stays minimal; a degenerate (zero-width) view never
        // divides by zero and falls back to the cap.
        assert_eq!(track_target(None, 0.0, 100.0, width, cap), 2);
        assert_eq!(track_target(Some((0.0, 1.0)), 5.0, 5.0, width, cap), cap);
    }

    #[test]
    fn budget_cap_bounds_overlapping_tracks() {
        let width = 1000.0;
        let single = single_target(width);
        let budget = single * BUDGET_TRACK_MULTIPLE;

        // Up to BUDGET_TRACK_MULTIPLE overlapping tracks keep full resolution.
        assert_eq!(budget_cap(width, 1), single);
        assert_eq!(budget_cap(width, BUDGET_TRACK_MULTIPLE), single);

        // Beyond that the cap shares the budget so total full-width points stay
        // bounded (allowing the integer-division remainder).
        for count in [BUDGET_TRACK_MULTIPLE + 1, 50, 500] {
            let cap = budget_cap(width, count);
            assert!((2..=single).contains(&cap));
            assert!(
                cap * count <= budget + count,
                "cap {cap} × {count} exceeds budget {budget}"
            );
        }

        // Zero visible count must not divide by zero.
        assert_eq!(budget_cap(width, 0), single);
    }

    #[test]
    fn file_shading_distinguishes_adjacent_files() {
        let a = metric_line_color(MetricKind::SatsSeen, 0);
        let b = metric_line_color(MetricKind::SatsSeen, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn sats_seen_and_sats_fix_stay_visually_separate_across_files() {
        for fi in 0..FILE_SHADE_FACTORS.len() * 5 {
            let seen = metric_line_color(MetricKind::SatsSeen, fi);
            let fix = metric_line_color(MetricKind::SatsFix, fi);
            assert!(
                seen.g() > fix.g(),
                "Sats seen should remain the lighter blue family: fi={fi}, seen={seen:?}, fix={fix:?}"
            );
        }
    }

    #[test]
    fn file_line_styles_are_pairwise_distinct() {
        for (i, a) in FILE_LINE_STYLES.iter().enumerate() {
            for (j, b) in FILE_LINE_STYLES.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    a, b,
                    "FILE_LINE_STYLES[{i}] duplicates FILE_LINE_STYLES[{j}]"
                );
            }
        }
    }

    fn test_plot_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))
    }

    #[test]
    fn resolve_legend_offset_clamps_to_plot_edges() {
        let legend_size = egui::vec2(100.0, 50.0);
        let offset = resolve_legend_offset(
            egui::vec2(-50.0, 1000.0),
            legend_size,
            test_plot_rect(),
            false,
            false,
        );
        assert!((offset.x - LEGEND_EDGE_MARGIN).abs() < f32::EPSILON);
        assert!((offset.y - (300.0 - legend_size.y - LEGEND_EDGE_MARGIN)).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_legend_offset_redocks_on_explicit_request() {
        let offset = resolve_legend_offset(
            egui::vec2(200.0, 150.0),
            egui::vec2(100.0, 50.0),
            test_plot_rect(),
            true,
            false,
        );
        assert_eq!(offset, LEGEND_DOCK_OFFSET);
    }

    #[test]
    fn resolve_legend_offset_snaps_to_dock_on_release_near_corner_only() {
        let legend_size = egui::vec2(100.0, 50.0);
        let near_dock = LEGEND_DOCK_OFFSET + egui::vec2(LEGEND_DOCK_SNAP_RADIUS - 1.0, 0.0);

        let mid_drag =
            resolve_legend_offset(near_dock, legend_size, test_plot_rect(), false, false);
        assert_eq!(mid_drag, near_dock, "must not snap before drag release");

        let released = resolve_legend_offset(near_dock, legend_size, test_plot_rect(), false, true);
        assert_eq!(
            released, LEGEND_DOCK_OFFSET,
            "must snap once released near the dock"
        );
    }

    #[test]
    fn resolve_legend_offset_does_not_snap_when_far_from_dock() {
        let legend_size = egui::vec2(100.0, 50.0);
        let far = egui::vec2(200.0, 150.0);
        let offset = resolve_legend_offset(far, legend_size, test_plot_rect(), false, true);
        assert_eq!(offset, far);
    }
}
