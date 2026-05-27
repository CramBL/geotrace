use crate::series::{TripSeries, build_all_series, closest_point_index};
use chrono::{DateTime, Utc};
use egui::Color32;
use egui_plot::{Line, PlotPoints, VLine};
use nav_egui_mipmap::{LevelSelection, MipMap};
use nav_types::{GlobalFilter, LoadedFile, TripDataVisibility};
use rayon::prelude::*;

/// Stable per-metric colors, in the same order as the `add` calls in
/// `add_series_lines`.  Every metric always gets the same color regardless of
/// which trip or file it came from, making it trivial to spot "velocity" or
/// "GPS fix" across multiple overlapping datasets.
const METRIC_COLORS: [Color32; 13] = [
    Color32::from_rgb(80, 200, 255), // Sats seen    — powder blue
    Color32::from_rgb(0, 100, 220),  // Sats fix     — deep blue
    Color32::from_rgb(0, 220, 80),   // GPS seen     — lime green
    Color32::from_rgb(0, 140, 40),   // GPS fix      — forest green
    Color32::from_rgb(255, 140, 30), // GLONASS seen — golden
    Color32::from_rgb(200, 80, 0),   // GLONASS fix  — amber
    Color32::from_rgb(255, 50, 110), // Galileo seen — hot pink
    Color32::from_rgb(155, 30, 255), // Galileo fix  — purple
    Color32::from_rgb(0, 230, 230),  // BeiDou seen  — cyan
    Color32::from_rgb(0, 160, 160),  // BeiDou fix   — teal
    Color32::from_rgb(255, 220, 0),  // Velocity     — bright yellow
    Color32::from_rgb(220, 20, 220), // EPH          — magenta
    Color32::from_rgb(255, 100, 50), // Heading      — red-orange
];

/// Global per-metric visibility flags.
///
/// Disabling a metric hides it for **all** trips at once, making it easy to
/// declutter the plot without touching per-trip settings.
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
        }
    }
}

impl MetricVisibility {
    /// Returns `true` when every metric is enabled.
    fn all_enabled(self) -> bool {
        self.sats_seen
            && self.sats_fix
            && self.gps_seen
            && self.gps_fix
            && self.glonass_seen
            && self.glonass_fix
            && self.galileo_seen
            && self.galileo_fix
            && self.beidou_seen
            && self.beidou_fix
            && self.velocity
            && self.eph
            && self.heading_deg
    }

    /// Set every metric to `enabled`.
    fn set_all(&mut self, enabled: bool) {
        self.sats_seen = enabled;
        self.sats_fix = enabled;
        self.gps_seen = enabled;
        self.gps_fix = enabled;
        self.glonass_seen = enabled;
        self.glonass_fix = enabled;
        self.galileo_seen = enabled;
        self.galileo_fix = enabled;
        self.beidou_seen = enabled;
        self.beidou_fix = enabled;
        self.velocity = enabled;
        self.eph = enabled;
        self.heading_deg = enabled;
    }
}

/// Cached level selections for all 12 metrics of one trip's series.
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
}

/// Persistent state for the trip plot panel.
///
/// Plot panel visibility is managed externally (via the tiles tree in the app),
/// so this struct only tracks the cursor hover time and the mipmap series cache.
#[derive(Debug, Clone)]
pub struct PlotState {
    /// Time currently hovered by the plot cursor, written each frame.
    /// `None` when the cursor is outside the plot area.
    pub hovered_time: Option<DateTime<Utc>>,
    /// Global per-metric visibility — toggled via the chip row above the plot.
    pub metric_vis: MetricVisibility,
    /// Whether the plot grid lines are visible.
    pub show_grid: bool,
    /// Mipmap cascade for every trip in every loaded file.
    pub(crate) series_cache: Vec<TripSeries>,
    /// Cached level selections, one entry per series.
    /// Invalidated when the effective plot bounds or target sample count changes.
    level_cache: Vec<TripLevelCache>,
    /// The `(eff_x_min, eff_x_max, target_count)` at which the current
    /// `level_cache` was computed.  `None` forces a recompute on the next frame.
    /// Used for hysteresis: the cache is reused as long as the view has not
    /// moved by more than 10 pixels since the last recompute.
    last_computed_bounds: Option<(f64, f64, usize)>,
    /// The map x-range (encoded as bit-pattern pairs) most recently applied to
    /// the plot via `set_plot_bounds_x`.  Used to detect changes and avoid
    /// re-applying the same range every frame (which would prevent manual zoom).
    applied_map_x_range: Option<(u64, u64)>,
}

impl Default for PlotState {
    fn default() -> Self {
        Self {
            hovered_time: None,
            metric_vis: MetricVisibility::default(),
            show_grid: true,
            series_cache: Vec::new(),
            level_cache: Vec::new(),
            last_computed_bounds: None,
            applied_map_x_range: None,
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
    /// Called after file deletion — runs on the UI thread since deletion is
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

/// Render the trip plot panel.
///
/// - `map_hover_time`: the timestamp of the TPV point currently hovered on
///   the map (if any).  The plot draws a vertical cursor line at this time so
///   the user can see the map-selected moment in context.
/// - `state.hovered_time` is written with the plot cursor's current time each
///   frame.  The caller should forward this to `MapHighlight::plot_hover_time`
///   before drawing the map so the renderer can cross-highlight the nearest
///   TPV arrow.
pub fn show_trip_plot(
    ui: &mut egui::Ui,
    files: &[LoadedFile],
    visibility: &TripDataVisibility,
    filter: &GlobalFilter,
    map_hover_time: Option<DateTime<Utc>>,
    // When map→plot sync is enabled, this carries the Unix-second x range
    // computed from TPV points visible in the current map viewport.
    // The plot will pan/zoom to this range the first frame it changes.
    map_sync_x_range: Option<(f64, f64)>,
    state: &mut PlotState,
) {
    // Compute the per-series visibility mask once so the three downstream
    // consumers — visible_count, the full-x-range loop, and the render loop —
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
                egui::RichText::new("Load a .nvd file to see trip metrics")
                    .weak()
                    .italics(),
            );
        });
        state.hovered_time = None;
        return;
    }

    let multi_trip = visible_count > 1;

    // Draw the per-metric filter row before the plot so it consumes vertical
    // space first; `ui.available_height()` below then gives the remainder.
    let hovered_chip = metric_filter_row(ui, &mut state.metric_vis, &mut state.show_grid);

    // Number of data points to request from the mipmap per frame.
    // Twice the pixel width gives ≥2 samples per screen pixel, which is enough
    // for faithful peak/trough rendering without over-sampling.
    // `available_width()` returns a positive f32; `.max(0.0)` makes this explicit
    // before casting, which the sign-loss lint cannot track statically.
    #[expect(
        clippy::cast_sign_loss,
        reason = "available_width is always ≥ 0 in practice; .max(0.0) makes it explicit"
    )]
    let target_count = (ui.available_width().max(0.0) as usize * 2).max(400);

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
    // Uses the precomputed `TripSeries::x_range` field — O(1) per series.
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

    // Encode the incoming map sync range as bit patterns so we can compare
    // without float equality warnings.
    let map_x_key = map_sync_x_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
    let need_map_sync = map_x_key.is_some() && map_x_key != state.applied_map_x_range;

    let mut new_hovered_time: Option<DateTime<Utc>> = None;
    let mut new_computed_bounds: Option<(f64, f64, usize)> = None;
    let mut new_level_cache: Option<Vec<TripLevelCache>> = None;
    let mut new_applied_map_x_range: Option<Option<(u64, u64)>> = None;

    let mut plot = egui_plot::Plot::new("trip_plot")
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
        let eff_x_min = filter_x_min.map_or(plot_x_min, |f| plot_x_min.max(f));
        let eff_x_max = filter_x_max.map_or(plot_x_max, |f| plot_x_max.min(f));

        // Hysteresis: skip recompute when the view has moved less than 10 px
        // since the last cache fill.  Converting to data space:
        //   10 px × (data_range / plot_width_px) = 20 × data_range / target_count
        // (target_count ≈ 2 × plot_width_px, always ≥ 400).
        let threshold = 20.0 * (eff_x_max - eff_x_min) / target_count as f64;
        let cache_valid = last_computed_bounds.is_some_and(|(lx_min, lx_max, lt_count)| {
            lt_count == target_count
                && level_cache.len() == series_cache.len()
                && (eff_x_min - lx_min).abs() <= threshold
                && (eff_x_max - lx_max).abs() <= threshold
        });

        // Recompute if the view changed enough since the last frame.
        // Uses rayon to parallelise across series — each is independent.
        let resolved: std::borrow::Cow<[TripLevelCache]> = if cache_valid {
            std::borrow::Cow::Borrowed(level_cache)
        } else {
            let fresh: Vec<TripLevelCache> = series_cache
                .par_iter()
                .map(|s| compute_level_cache(s, eff_x_min, eff_x_max, target_count))
                .collect();
            new_computed_bounds = Some((eff_x_min, eff_x_max, target_count));
            new_level_cache = Some(fresh.clone());
            std::borrow::Cow::Owned(fresh)
        };

        // Pan the plot to the map-visible time range when it changes.
        if need_map_sync {
            if let Some((x_min, x_max)) = map_sync_x_range {
                plot_ui.set_plot_bounds_x(x_min..=x_max);
            }
            new_applied_map_x_range = Some(map_x_key);
        }

        for (si, series) in series_cache.iter().enumerate() {
            if !visible.get(si).copied().unwrap_or(false) {
                continue;
            }
            // `resolved` has the same length as `series_cache` by construction.
            let Some(cache) = resolved.get(si) else {
                continue;
            };
            add_series_lines(plot_ui, series, multi_trip, cache, metric_vis, hovered_chip);
        }

        // Vertical cursor from map hover.
        if let Some(t) = map_hover_time {
            let x = t.timestamp() as f64;
            plot_ui.vline(
                VLine::new("Map position", x)
                    .color(Color32::from_rgba_unmultiplied(100, 200, 255, 200))
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

    // Clear the hovered time when the cursor leaves the plot area.
    state.hovered_time = if plot_response.response.hovered() {
        new_hovered_time
    } else {
        None
    };
}

/// Draw the per-metric filter controls above the trip plot.
///
/// Layout:
/// - A small controls bar: grid toggle and master hide/show button.
/// - A framed satellite group containing the ten constellation metrics (total
///   counts plus per-constellation GPS/GLONASS/Galileo/BeiDou seen and fix).
/// - A plain row for the three non-satellite metrics: velocity, EPH, heading.
///
/// Returns the index of the chip currently being hovered, or `None`.
/// The caller passes this to `add_series_lines` to highlight the hovered metric
/// and dim the rest, mirroring the standard egui-plot legend hover behaviour.
fn metric_filter_row(
    ui: &mut egui::Ui,
    vis: &mut MetricVisibility,
    show_grid: &mut bool,
) -> Option<u8> {
    let all_on = vis.all_enabled();

    // Controls bar: grid toggle and master hide/show.
    ui.horizontal(|ui| {
        let grid_label = if *show_grid { "Hide grid" } else { "Show grid" };
        if ui.small_button(grid_label).clicked() {
            *show_grid = !*show_grid;
        }
        ui.separator();
        let master_label = if all_on { "Hide all" } else { "Show all" };
        if ui.small_button(master_label).clicked() {
            vis.set_all(!all_on);
        }
    });

    // Satellite metrics — framed so they read as one coherent group.
    // `Frame::group` is constructed before the `.show` call so that the
    // immutable borrow of `ui` (for `ui.style()`) ends at the semicolon,
    // leaving `ui` free for the mutable `.show` call below.
    let sat_frame = egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(4));
    let sat = sat_frame.show(ui, |ui| {
        let mut so: Option<u8> = None;
        let mut hov: Option<u8> = None;
        ui.horizontal_wrapped(|ui| {
            macro_rules! sat_chip {
                ($field:expr, $name:expr, $color:expr, $idx:expr) => {{
                    let (s, h) = metric_chip(ui, &mut $field, $name, $color);
                    if s {
                        so = Some($idx);
                    }
                    if h {
                        hov = Some($idx);
                    }
                }};
            }
            sat_chip!(vis.sats_seen, "Sats seen", METRIC_COLORS[0], 0u8);
            sat_chip!(vis.sats_fix, "Sats fix", METRIC_COLORS[1], 1u8);
            sat_chip!(vis.gps_seen, "GPS seen", METRIC_COLORS[2], 2u8);
            sat_chip!(vis.gps_fix, "GPS fix", METRIC_COLORS[3], 3u8);
            sat_chip!(vis.glonass_seen, "GLONASS seen", METRIC_COLORS[4], 4u8);
            sat_chip!(vis.glonass_fix, "GLONASS fix", METRIC_COLORS[5], 5u8);
            sat_chip!(vis.galileo_seen, "Galileo seen", METRIC_COLORS[6], 6u8);
            sat_chip!(vis.galileo_fix, "Galileo fix", METRIC_COLORS[7], 7u8);
            sat_chip!(vis.beidou_seen, "BeiDou seen", METRIC_COLORS[8], 8u8);
            sat_chip!(vis.beidou_fix, "BeiDou fix", METRIC_COLORS[9], 9u8);
        });
        (so, hov)
    });
    let mut show_only = sat.inner.0;
    let mut hovered_chip = sat.inner.1;

    // Non-satellite metrics.
    ui.horizontal_wrapped(|ui| {
        macro_rules! chip {
            ($field:expr, $name:expr, $color:expr, $idx:expr) => {{
                let (s, h) = metric_chip(ui, &mut $field, $name, $color);
                if s {
                    show_only = Some($idx);
                }
                if h {
                    hovered_chip = Some($idx);
                }
            }};
        }
        chip!(vis.velocity, "Velocity", METRIC_COLORS[10], 10u8);
        chip!(vis.eph, "EPH", METRIC_COLORS[11], 11u8);
        chip!(vis.heading_deg, "Heading", METRIC_COLORS[12], 12u8);
    });

    // Apply "Show only this" — disable everything first, then re-enable the one.
    if let Some(idx) = show_only {
        vis.set_all(false);
        match idx {
            0 => vis.sats_seen = true,
            1 => vis.sats_fix = true,
            2 => vis.gps_seen = true,
            3 => vis.gps_fix = true,
            4 => vis.glonass_seen = true,
            5 => vis.glonass_fix = true,
            6 => vis.galileo_seen = true,
            7 => vis.galileo_fix = true,
            8 => vis.beidou_seen = true,
            9 => vis.beidou_fix = true,
            10 => vis.velocity = true,
            11 => vis.eph = true,
            12 => vis.heading_deg = true,
            _ => {}
        }
    }

    hovered_chip
}

/// A small colored toggle chip.  Left-click toggles the metric.  Right-click
/// opens a context menu with "Show only this".
///
/// Returns `(show_only, hovered)` — `show_only` is `true` when the user chose
/// "Show only this" from the context menu; `hovered` is `true` while the pointer
/// is over this chip.
fn metric_chip(ui: &mut egui::Ui, enabled: &mut bool, name: &str, color: Color32) -> (bool, bool) {
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
    let mut show_only = false;
    response.context_menu(|ui| {
        if ui.button("Show only this").clicked() {
            show_only = true;
            ui.close();
        }
    });
    (show_only, response.hovered())
}

/// Compute fresh level selections for all 12 metrics of one trip's series.
fn compute_level_cache(
    series: &TripSeries,
    x_min: f64,
    x_max: f64,
    target_count: usize,
) -> TripLevelCache {
    let sel = |mm: &MipMap| mm.select_indices(x_min, x_max, target_count);
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
    }
}

/// Returns `true` when the trip at `(fi, ti)` passes visibility and filter checks.
fn trip_is_visible(
    visibility: &TripDataVisibility,
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
    let Some(trip_vis) = file_vis.trips.get(ti) else {
        return false;
    };
    if !trip_vis.enabled {
        return false;
    }
    let Some(file) = files.get(fi) else {
        return false;
    };
    let Some(trip) = file.trips.get(ti) else {
        return false;
    };
    nav_types::filter::trip_passes_filter(&trip.metadata, global_filter)
}

/// Add all metric lines for one trip to the plot using pre-computed level selections.
///
/// When `hovered_chip` is `Some(idx)`, the line at `idx` is highlighted (double
/// stroke width, via egui-plot's built-in highlight mechanism) and every other
/// line is dimmed to 20 % brightness — mirroring the standard egui-plot legend
/// hover behaviour.
///
/// The `'a` lifetime ties both `plot_ui` and `series` together so that
/// [`egui_plot::PlotPoints::Borrowed`] can reference mipmap slices directly
/// without any per-frame allocation.
fn add_series_lines<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    series: &'a TripSeries,
    multi_trip: bool,
    cache: &TripLevelCache,
    metric_vis: &MetricVisibility,
    hovered_chip: Option<u8>,
) {
    let prefix = if multi_trip {
        format!("{}: ", series.label)
    } else {
        String::new()
    };

    let color_for = |color: Color32, idx: u8| -> (Color32, bool) {
        match hovered_chip {
            Some(h) if h == idx => (color, true),
            Some(_) => (color.gamma_multiply(0.2), false),
            None => (color, false),
        }
    };

    if metric_vis.sats_seen {
        let (c, h) = color_for(METRIC_COLORS[0], 0);
        add_line(
            plot_ui,
            series.total_seen.slice_at(cache.total_seen),
            format!("{prefix}Sats seen"),
            c,
            h,
        );
    }
    if metric_vis.sats_fix {
        let (c, h) = color_for(METRIC_COLORS[1], 1);
        add_line(
            plot_ui,
            series.total_fix.slice_at(cache.total_fix),
            format!("{prefix}Sats fix"),
            c,
            h,
        );
    }
    if metric_vis.gps_seen {
        let (c, h) = color_for(METRIC_COLORS[2], 2);
        add_line(
            plot_ui,
            series.gps_seen.slice_at(cache.gps_seen),
            format!("{prefix}GPS seen"),
            c,
            h,
        );
    }
    if metric_vis.gps_fix {
        let (c, h) = color_for(METRIC_COLORS[3], 3);
        add_line(
            plot_ui,
            series.gps_fix.slice_at(cache.gps_fix),
            format!("{prefix}GPS fix"),
            c,
            h,
        );
    }
    if metric_vis.glonass_seen {
        let (c, h) = color_for(METRIC_COLORS[4], 4);
        add_line(
            plot_ui,
            series.glonass_seen.slice_at(cache.glonass_seen),
            format!("{prefix}GLONASS seen"),
            c,
            h,
        );
    }
    if metric_vis.glonass_fix {
        let (c, h) = color_for(METRIC_COLORS[5], 5);
        add_line(
            plot_ui,
            series.glonass_fix.slice_at(cache.glonass_fix),
            format!("{prefix}GLONASS fix"),
            c,
            h,
        );
    }
    if metric_vis.galileo_seen {
        let (c, h) = color_for(METRIC_COLORS[6], 6);
        add_line(
            plot_ui,
            series.galileo_seen.slice_at(cache.galileo_seen),
            format!("{prefix}Galileo seen"),
            c,
            h,
        );
    }
    if metric_vis.galileo_fix {
        let (c, h) = color_for(METRIC_COLORS[7], 7);
        add_line(
            plot_ui,
            series.galileo_fix.slice_at(cache.galileo_fix),
            format!("{prefix}Galileo fix"),
            c,
            h,
        );
    }
    if metric_vis.beidou_seen {
        let (c, h) = color_for(METRIC_COLORS[8], 8);
        add_line(
            plot_ui,
            series.beidou_seen.slice_at(cache.beidou_seen),
            format!("{prefix}BeiDou seen"),
            c,
            h,
        );
    }
    if metric_vis.beidou_fix {
        let (c, h) = color_for(METRIC_COLORS[9], 9);
        add_line(
            plot_ui,
            series.beidou_fix.slice_at(cache.beidou_fix),
            format!("{prefix}BeiDou fix"),
            c,
            h,
        );
    }
    if metric_vis.velocity {
        let (c, h) = color_for(METRIC_COLORS[10], 10);
        add_line(
            plot_ui,
            series.velocity_kmh.slice_at(cache.velocity_kmh),
            format!("{prefix}Velocity (km/h)"),
            c,
            h,
        );
    }
    if metric_vis.eph {
        let (c, h) = color_for(METRIC_COLORS[11], 11);
        add_line(
            plot_ui,
            series.eph_m.slice_at(cache.eph_m),
            format!("{prefix}EPH (m)"),
            c,
            h,
        );
    }
    if metric_vis.heading_deg {
        let (c, h) = color_for(METRIC_COLORS[12], 12);
        add_line(
            plot_ui,
            series.heading_deg.slice_at(cache.heading_deg),
            format!("{prefix}Heading (°)"),
            c,
            h,
        );
    }
}

/// Submit one metric line to the plot, borrowing the point slice directly via
/// [`PlotPoints::Borrowed`] — no allocation.
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
    highlighted: bool,
) {
    if data.len() < 2 {
        return;
    }
    plot_ui.line(
        Line::new(name, PlotPoints::Borrowed(data))
            .color(color)
            .highlight(highlighted),
    );
}

/// Given a set of visible trips and a plot-hovered time (in seconds since
/// epoch), find the `(file_index, trip_index, point_index)` of the closest
/// TPV point across all visible trips.
///
/// Called by the app to set `MapHighlight::plot_hover_time` after the plot renders.
pub fn find_closest_tpv(
    files: &[LoadedFile],
    visibility: &TripDataVisibility,
    filter: &GlobalFilter,
    target: DateTime<Utc>,
) -> Option<(usize, usize, usize)> {
    let target_secs = target.timestamp() as f64;
    let mut best: Option<(usize, usize, usize, f64)> = None;

    for (fi, file) in files.iter().enumerate() {
        let Some(file_vis) = visibility.files.get(fi) else {
            continue;
        };
        if !file_vis.enabled {
            continue;
        }
        for (ti, trip) in file.trips.iter().enumerate() {
            let Some(trip_vis) = file_vis.trips.get(ti) else {
                continue;
            };
            if !trip_vis.enabled {
                continue;
            }
            if !nav_types::filter::trip_passes_filter(&trip.metadata, filter) {
                continue;
            }
            let Some(pi) = closest_point_index(&trip.points, target_secs) else {
                continue;
            };
            let Some(point) = trip.points.get(pi) else {
                continue;
            };
            let dist = (point.tpv.time().utc().timestamp() as f64 - target_secs).abs();
            if best.is_none_or(|(_, _, _, d)| dist < d) {
                best = Some((fi, ti, pi, dist));
            }
        }
    }

    best.map(|(fi, ti, pi, _)| (fi, ti, pi))
}
