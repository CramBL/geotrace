//! The track plot widget: [`PlotState`], the frame loop
//! ([`show_track_plot`]), and the submodules it orchestrates.

mod chips;
mod clock_excursion;
mod jamming;
mod legend;
mod levels;
mod lines;
mod snap_error;
mod style;

pub use chips::{ChannelVisibility, MetricVisibility};
pub use legend::{LEGEND_DOCK_OFFSET, legend_is_docked};

use chips::{MetricAvailability, SectionGates, loaded_channels, metric_filter_row};
use clock_excursion::{ExcursionViewport, add_clock_excursions};
use jamming::{JammingPlotCache, JammingViewport, jamming_available, sync_jamming_cache};
use legend::show_file_legend_overlay;
use levels::{TripLevelCache, budget_cap, compute_level_cache, single_target};
use lines::{
    NearestHoverLabel, add_series_lines, add_util_anomalies, series_track_ref,
    show_nearest_hover_label,
};
use snap_error::{
    SnapErrorPlotCache, SnapErrorViewport, snap_error_available, sync_snap_error_cache,
};

use crate::AnalysisConfig;
use crate::series::{TrackSeries, build_all_series};
use chrono::{DateTime, Utc};
use egui::Color32;
use egui::RichText;
use egui_plot::{Span, VLine};
use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingNames;
use gt_types::satellites::ConstellationSet;
use gt_types::{FileIdx, LoadedFile, MetricKind, PointIdx, TrackIdx, TrackRef};
use gt_ui_types::{HighlightScope, JammingSeries, SnapErrorSeries, TrackDataVisibility};
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};

/// Grid base-color intensity, as a multiplier on the theme text color.
/// egui_plot's grid stroke width is fixed at 1.0, so with the thinner default
/// data lines the grid at full text-color brightness would dominate; dimming
/// it restores the lines as the visually strongest element.
const GRID_COLOR_STRENGTH: f32 = 0.5;
/// Default stroke width of the metric and channel plot lines.  Slightly below
/// egui_plot's 1.0 default: many lines are enabled by default, and a thinner
/// stroke keeps overlapping lines readable.
pub const DEFAULT_PLOT_LINE_WIDTH: f32 = 0.75;
/// Allowed plot line width, shared by the display-settings slider and the
/// clamp applied to persisted settings on load.
pub const PLOT_LINE_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 0.5..=5.0;
/// Stroke width of the vertical seek lines (hovered match, map position). Above
/// the data lines so the marker stays findable across a crowded plot.
const SEEK_LINE_WIDTH: f32 = 1.5;

/// Fallback label for a file index with no loaded recording behind it, so a
/// stale index still shows something readable.
const UNKNOWN_RECORDING: &str = "Unknown file";

/// The recording's display name, as the side panel shows it.
fn recording_name(names: &RecordingNames, fi: usize) -> &str {
    names.get(FileIdx::new(fi)).unwrap_or(UNKNOWN_RECORDING)
}

/// The plot's cursor label: the hovered line's name, the time, and the value.
///
/// Away from any line the name line is left out rather than drawn blank. The
/// label stays away entirely while a custom hover label draws
/// (see [`lines::show_nearest_hover_label`]): both sit at the cursor, and the
/// custom one already carries the time and the value.
fn cursor_label(
    custom_hover_label_shown: &Cell<bool>,
    pos: &egui_plot::HoverPosition<'_>,
) -> Option<String> {
    if custom_hover_label_shown.get() {
        return None;
    }
    let (name, point) = match pos {
        egui_plot::HoverPosition::NearDataPoint {
            plot_name,
            position,
            ..
        } => (*plot_name, position),
        egui_plot::HoverPosition::Elsewhere { position } => ("", position),
    };
    let time = DateTime::from_timestamp(point.x as i64, 0)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_default();
    let reading = format!("{time}\n{:.2}", point.y);
    if name.is_empty() {
        Some(reading)
    } else {
        Some(format!("{name}\n{reading}"))
    }
}

/// One track's plot label: the recording's display `name`, with the track
/// number appended when the recording split into several tracks.
fn track_label(name: &str, ti: usize, track_count: usize) -> String {
    if track_count > 1 {
        format!("{name} T{}", ti + 1)
    } else {
        name.to_owned()
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
    /// Stroke width of the metric and channel lines, adjusted via the plot
    /// display popup (the gear button in the chip row).
    pub line_width: f32,
    /// When true, the plot x-range tracks the map viewport.
    pub sync_to_map: bool,
    /// Whether to draw the masked-satellite anomaly markers (a used satellite
    /// below the elevation mask).  Toggled in Settings.
    pub mark_masked_fix: bool,
    /// Whether the advanced analysis chips (satellite utilization) are revealed.
    /// Off by default - these metrics are hidden until the user opts in.
    pub show_advanced_metrics: bool,
    /// Whether the ad-hoc channel chips and lines are revealed. Off by
    /// default, like the advanced section; the toggle only renders when a
    /// loaded track carries channels.
    pub show_channels: bool,
    /// Global per-channel visibility - toggled via the channel chips.
    pub channel_vis: ChannelVisibility,
    /// Analysis parameters the cached series were built with (elevation mask).
    /// Changing it via [`PlotState::set_analysis`] re-derives the affected series.
    pub analysis: AnalysisConfig,
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
    /// Per-track snap error mipmaps and marker lists, rebuilt only when a
    /// track's series `Arc` changes (see [`sync_snap_error_cache`]).
    snap_error_cache: HashMap<TrackRef, SnapErrorPlotCache>,
    /// Per-track interference line caches, rebuilt when a track's series
    /// `Arc` changes (see [`sync_jamming_cache`]).
    jamming_cache: HashMap<TrackRef, JammingPlotCache>,
    /// User-chosen component colors, keyed by channel name: one optional
    /// override per component, `None` = the derived hue. Edited through the
    /// chip's right-click menu; persisted with the plot settings.
    pub channel_component_colors: HashMap<String, Vec<Option<Color32>>>,
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
            line_width: DEFAULT_PLOT_LINE_WIDTH,
            sync_to_map: true,
            mark_masked_fix: true,
            show_advanced_metrics: false,
            show_channels: false,
            channel_vis: ChannelVisibility::default(),
            analysis: AnalysisConfig::default(),
            file_legend_collapsed: false,
            file_legend_offset: LEGEND_DOCK_OFFSET,
            file_legend_size: egui::Vec2::ZERO,
            legend_hover_file: None,
            series_cache: Vec::new(),
            level_cache: Vec::new(),
            last_computed_bounds: None,
            applied_map_x_range: None,
            snap_error_cache: HashMap::new(),
            jamming_cache: HashMap::new(),
            channel_component_colors: HashMap::new(),
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
        self.series_cache = build_all_series(files, self.analysis);
        self.invalidate_level_cache();
    }

    /// Apply new analysis parameters (elevation mask), re-deriving only the
    /// mask-dependent series in place.  A no-op when `analysis` is unchanged, so
    /// it is cheap to call every frame while a settings control is open.
    pub fn set_analysis(&mut self, files: &[LoadedFile], analysis: AnalysisConfig) {
        if self.analysis == analysis {
            return;
        }
        self.analysis = analysis;
        for series in &mut self.series_cache {
            if let Some(track) = files
                .get(series.fi)
                .and_then(|file| file.tracks.get(series.ti))
            {
                series.apply_analysis(track, analysis);
            }
        }
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
    // Per-file display names, resolved from the user's recording-name template
    // (see [`RecordingNames`]) so the legend, line names and hover labels all
    // read the same name the side panel shows.
    names: &RecordingNames,
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    hover_scope: Option<HighlightScope>,
    map_hover_time: Option<DateTime<Utc>>,
    // Time span of the match hovered in the query results table, drawn as a
    // shaded band beneath the series so the match shows in metric context.
    match_hover_time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    // When map→plot sync is enabled, this carries the Unix-second x range
    // computed from TPV points visible in the current map viewport.
    // The plot will pan/zoom to this range the first frame it changes.
    map_sync_x_range: Option<(f64, f64)>,
    // Snap error per track, resolved by the app from completed snap runs
    // (see `gt_ui_types::SnapErrorSeries`).
    snap_error: &SnapErrorSeries,
    // Interference per fix, resolved by the app from the archive
    // (see `gt_ui_types::JammingSeries`).
    jamming: &JammingSeries,
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
                RichText::new("Load a .gtd file to see track metrics")
                    .weak()
                    .italics(),
            );
        });
        state.hovered_time = None;
        return;
    }

    // Per-series count, for the line-name prefix.  Distinct from the
    // per-file count below, which gates the file legend overlay.
    let multi_track = visible_count > 1;
    // Per-series labels, `None` while a single track is visible and nothing
    // needs naming. Resolved every frame so a template change lands right
    // away, unlike the mipmaps these sit beside.
    let series_labels: Vec<Option<String>> = state
        .series_cache
        .iter()
        .map(|s| {
            multi_track.then(|| {
                let track_count = files.get(s.fi).map_or(1, |file| file.tracks.len());
                track_label(recording_name(names, s.fi), s.ti, track_count)
            })
        })
        .collect();
    let visible_files: Vec<usize> = {
        let mut file_indices = BTreeSet::new();
        for (series, &is_vis) in state.series_cache.iter().zip(visible.iter()) {
            if is_vis {
                file_indices.insert(series.fi);
            }
        }
        file_indices.into_iter().collect()
    };

    // Constellations present anywhere in the loaded data.  Per-constellation
    // chips and lines are gated on this so a constellation with no data (e.g.
    // NavIC or QZSS in a GPS-only recording) never clutters the UI.
    let present = state
        .series_cache
        .iter()
        .fold(ConstellationSet::empty(), |acc, s| acc.union(s.present));

    // Channels present anywhere in the loaded data, unioned like the
    // constellations: the Channels toggle and chips render only when a track
    // actually carries channels.
    let channels = loaded_channels(state.series_cache.iter().flat_map(|s| s.channels.iter()));

    // Whether any visible track has a completed snap run: gates the snap
    // error chip (disabled with hover text until a run completes) and the
    // per-point hover hit-testing.
    let snap_error_available = snap_error_available(&state.series_cache, &visible, snap_error);
    let jamming_available = jamming_available(&state.series_cache, &visible, jamming);

    // Draw the per-metric filter row before the plot so it consumes vertical
    // space first.  `ui.available_height()` below then gives the remainder.
    let hovered_chip = metric_filter_row(
        ui,
        &mut state.metric_vis,
        present,
        &channels,
        &mut state.channel_vis,
        &mut state.channel_component_colors,
        &mut state.show_grid,
        &mut state.line_width,
        &mut state.sync_to_map,
        &mut state.show_advanced_metrics,
        &mut state.show_channels,
        MetricAvailability {
            snap_error: snap_error_available,
            jamming: jamming_available,
        },
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

    sync_snap_error_cache(&mut state.snap_error_cache, snap_error);
    sync_jamming_cache(&mut state.jamming_cache, jamming);

    // Split borrows: extract immutable refs to the caches and metric visibility
    // before the closure so the borrow checker can see they are disjoint from
    // the mutable fields written after the closure (`hovered_time`, `level_cache`,
    // `last_computed_bounds`).
    let series_cache = &state.series_cache;
    let snap_error_cache = &state.snap_error_cache;
    let jamming_cache = &state.jamming_cache;
    let channel_component_colors = &state.channel_component_colors;
    let level_cache = &state.level_cache;
    let last_computed_bounds = state.last_computed_bounds;
    let metric_vis = &state.metric_vis;
    let channel_vis = &state.channel_vis;
    let show_channels = state.show_channels;
    let line_width = state.line_width;
    // Anomaly markers ride on the "Util all" line, so they show only when that
    // metric is visible and the settings toggle is on.
    let show_advanced = state.show_advanced_metrics;
    let show_anomalies =
        show_advanced && state.mark_masked_fix && state.metric_vis.field(MetricKind::UtilAll);
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
    // The custom hover label to draw: the closest candidate offered by any
    // series of any recording inside the plot closure, turned into a tooltip
    // after it returns.
    let mut hovered_label = NearestHoverLabel::default();
    // The custom label and the cursor label never draw in the same frame:
    // egui_plot runs the plot closure before it formats the cursor label, and
    // this flag is raised at the end of that closure.
    let custom_hover_label_shown = Cell::new(false);
    let show_snap_error = snap_error_available && state.metric_vis.field(MetricKind::SnapError);

    let mut plot = egui_plot::Plot::new("track_plot")
        .height(ui.available_height())
        .show_grid(state.show_grid)
        .grid_color(
            ui.visuals()
                .text_color()
                .gamma_multiply(GRID_COLOR_STRENGTH),
        )
        .x_axis_formatter(x_fmt)
        .label_formatter(|pos| cursor_label(&custom_hover_label_shown, pos));

    // Tell egui_plot the full data extent so double-click reset zooms to fit.
    if has_full_range {
        plot = plot.include_x(full_x_min).include_x(full_x_max);
    }

    let dark_mode = ui.visuals().dark_mode;
    // On a light theme, give the plot a faint-grey canvas rather than egui's
    // pure white so the deepened light-variant series lines keep a little
    // separation from the background. Scoped to this plot: restored right after.
    let saved_extreme_bg = ui.visuals().extreme_bg_color;
    if !dark_mode {
        ui.visuals_mut().extreme_bg_color = gt_ui_theme::PLOT_CANVAS_LIGHT;
    }
    let plot_response = plot.show(ui, |plot_ui| {
        let bounds = plot_ui.plot_bounds();
        let plot_x_min = bounds.min()[0];
        let plot_x_max = bounds.max()[0];

        // Intersect the visible plot range with the active time filter.
        //
        // `eff_x_min`/`eff_x_max` may end up inverted when the active filter
        // and the visible viewport don't overlap.  `MipMap` normalizes that
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

        // Pointer position for anomaly-marker hit-testing, sampled once.
        let anomaly_pointer = if show_anomalies {
            plot_ui.response().hover_pos()
        } else {
            None
        };
        let snap_pointer = if show_snap_error {
            plot_ui.response().hover_pos()
        } else {
            None
        };
        // Unconditional: the excursion overlay gates itself on its metric's
        // chip, and reading the hover position is a field access.
        let excursion_pointer = plot_ui.response().hover_pos();

        // The hovered match's time band, before the series so the lines stay
        // on top. A `Span` rather than a polygon: it fills the plot's full
        // height on its own and contributes nothing to the auto-bounds, so
        // the view never re-fits to the band (a polygon sized to the current
        // bounds fed back into the next frame's bounds and made the plot
        // oscillate). A single-point match has no width; a cursor line marks
        // it.
        if let Some((start, end)) = match_hover_time_range {
            let (x0, x1) = (start.timestamp() as f64, end.timestamp() as f64);
            if x0 < x1 {
                // Unnamed: the span draws its name inside the band, and the
                // highlight needs no caption.
                plot_ui.span(Span::new("", x0..=x1).fill(gt_ui_theme::HIGHLIGHT_BLUE_BAND));
            } else {
                plot_ui.vline(
                    VLine::new("Hovered match", x0)
                        .color(gt_ui_theme::HIGHLIGHT_BLUE_SEEK)
                        .width(SEEK_LINE_WIDTH),
                );
            }
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
            let track_label = series_labels.get(si).and_then(Option::as_deref);
            add_series_lines(
                plot_ui,
                series,
                track_label,
                cache,
                metric_vis,
                channel_vis,
                channel_component_colors,
                present,
                &channels,
                hovered_chip.as_ref(),
                effective_hover_scope,
                SectionGates {
                    show_advanced,
                    show_channels,
                },
                line_width,
                dark_mode,
                snap_error_cache.get(&series_track_ref(series)),
                SnapErrorViewport {
                    x_min: eff_x_min,
                    x_max: eff_x_max,
                    width: available_width,
                    cap: sample_cap,
                },
                snap_pointer,
                jamming_cache.get(&series_track_ref(series)),
                JammingViewport {
                    x_min: eff_x_min,
                    x_max: eff_x_max,
                    width: available_width,
                    cap: sample_cap,
                },
                &mut hovered_label,
            );
            add_clock_excursions(
                plot_ui,
                series,
                track_label,
                ExcursionViewport {
                    x_min: eff_x_min,
                    x_max: eff_x_max,
                    metric_vis,
                    dark_mode,
                },
                excursion_pointer,
                &mut hovered_label,
            );
            if show_anomalies {
                add_util_anomalies(
                    plot_ui,
                    series,
                    track_label,
                    eff_x_min..=eff_x_max,
                    anomaly_pointer,
                    &mut hovered_label,
                    dark_mode,
                );
            }
        }

        if let std::borrow::Cow::Owned(owned) = resolved {
            new_level_cache = Some(owned);
        }
        if let Some(t) = map_hover_time {
            let x = t.timestamp() as f64;
            plot_ui.vline(
                VLine::new("Map position", x)
                    .color(gt_ui_theme::HIGHLIGHT_BLUE_SEEK)
                    .width(SEEK_LINE_WIDTH),
            );
        }

        custom_hover_label_shown.set(hovered_label.has_candidate());
        new_hovered_time = plot_ui
            .pointer_coordinate()
            .and_then(|c| DateTime::from_timestamp(c.x as i64, 0));
    });
    if !dark_mode {
        ui.visuals_mut().extreme_bg_color = saved_extreme_bg;
    }

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
    // interact radius of a plotted item, the exact condition that causes
    // egui_plot to show a hover label.  Use it directly so the map overlay
    // activates at precisely the same moment, with no custom approximation.
    state.plot_cursor_snapped =
        plot_response.response.hovered() && plot_response.hovered_plot_item.is_some();

    show_nearest_hover_label(ui, &plot_response.response, hovered_label);

    state.legend_hover_file = show_file_legend_overlay(
        ui,
        names,
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
            // Only points inside the filter's time window are drawn on the map,
            // so the cross-highlight must pick the nearest *visible* point - never
            // one filtered out of view.
            let nearest = track
                .points
                .iter()
                .enumerate()
                .filter(|(_, p)| gt_filter::point_passes_time_filter(p.tpv.time().utc(), filter))
                .map(|(i, p)| (i, (p.tpv.time().as_secs_f64() - target_secs).abs()))
                .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let Some((pi, dist)) = nearest else {
                continue;
            };
            if best.is_none_or(|(_, _, _, d)| dist < d) {
                best = Some((fi, ti, PointIdx::new(pi), dist));
            }
        }
    }

    best.map(|(fi, ti, pi, _)| (fi, ti, pi))
}

#[cfg(test)]
mod label_tests {
    use std::cell::Cell;

    use super::{cursor_label, track_label};
    use egui_plot::PlotPoint;

    /// For the tests that are not about suppression, so they need no `Cell`
    /// of their own.
    fn cursor_label_alone(pos: &egui_plot::HoverPosition<'_>) -> Option<String> {
        cursor_label(&Cell::new(false), pos)
    }

    #[test]
    fn a_single_track_recording_is_labelled_by_name_alone() {
        assert_eq!(track_label("Morning ride", 0, 1), "Morning ride");
    }

    #[test]
    fn a_split_recording_numbers_its_tracks() {
        assert_eq!(track_label("Morning ride", 1, 3), "Morning ride T2");
    }

    /// 2024-01-15 12:00:00 UTC.
    const T: f64 = 1_705_320_000.0;

    #[test]
    fn a_snapped_point_is_captioned_by_its_line() {
        let pos = egui_plot::HoverPosition::NearDataPoint {
            plot_name: "Morning ride: Satellites seen",
            position: PlotPoint::new(T, 12.0),
            index: 0,
        };
        assert_eq!(
            cursor_label_alone(&pos).as_deref(),
            Some("Morning ride: Satellites seen\n12:00:00\n12.00")
        );
    }

    #[test]
    fn an_unsnapped_cursor_leaves_out_the_name_line() {
        let pos = egui_plot::HoverPosition::Elsewhere {
            position: PlotPoint::new(T, 12.0),
        };
        assert_eq!(
            cursor_label_alone(&pos).as_deref(),
            Some("12:00:00\n12.00"),
            "an empty name must not leave a blank first line"
        );
    }

    /// A custom hover label and the cursor label would both sit at the
    /// cursor, so the cursor label stays away while a custom one draws.
    #[test]
    fn the_cursor_label_yields_to_a_custom_hover_label() {
        let pos = egui_plot::HoverPosition::Elsewhere {
            position: PlotPoint::new(T, 12.0),
        };
        assert_eq!(cursor_label(&Cell::new(true), &pos), None);
    }
}
