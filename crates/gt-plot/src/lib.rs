mod plot_widget;
mod series;

pub use plot_widget::{
    DEFAULT_PLOT_LINE_WIDTH, LEGEND_DOCK_OFFSET, PLOT_LINE_WIDTH_RANGE, PlotState,
    find_closest_tpv, legend_is_docked, show_track_plot,
};

/// Default elevation mask, in degrees, shared by the satellite utilization rate
/// and the slip rate.
///
/// 15 deg is the conventional GNSS baseline: below it atmospheric delay and
/// multipath dominate, so receivers routinely ignore those satellites.
pub const DEFAULT_ELEVATION_MASK_DEG: f32 = 15.0;

/// Default SNR drop, in dB-Hz between consecutive epochs, that counts as a
/// loss-of-lock slip.
pub const DEFAULT_SNR_DROP_DB: f32 = 10.0;

/// Default averaging window, in minutes, over which the slip rate is computed.
pub const DEFAULT_SLIP_WINDOW_MIN: f32 = 10.0;

/// Default deviation from a track's baseline clock offset, in seconds, above
/// which a sample is treated as a clock offset excursion.  Re-exported from the
/// detector so the plot and the generated markers share one default.
pub const DEFAULT_CLOCK_EXCURSION_THRESHOLD_S: f32 =
    gt_analysis::clock_offset::DEFAULT_EXCURSION_THRESHOLD_S;

/// Range the clock-excursion threshold may be set to, in seconds.  Shared by
/// the settings control and the clamp applied to persisted settings on load.
pub const CLOCK_EXCURSION_THRESHOLD_RANGE_S: std::ops::RangeInclusive<f32> = 1.0..=3600.0;

/// Tunable parameters for the derived satellite-analysis series (utilization
/// rate and slip rate).  Threaded into series building so a change re-derives
/// the affected mipmaps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisConfig {
    /// Elevation mask applied to the "in view" baseline of the utilization rate
    /// and to slip detection.
    pub elevation_mask_deg: f32,
    /// SNR drop, in dB-Hz between consecutive epochs, above which a still-tracked
    /// satellite is counted as having slipped.
    pub snr_drop_db: f32,
    /// Trailing window, in minutes, over which the slip rate is averaged.
    pub slip_window_min: f32,
    /// Deviation from a track's baseline clock offset, in seconds, above which a
    /// sample counts as a clock offset excursion: kept off the shared y-axis and
    /// marked at the edge of the view instead.
    pub clock_excursion_threshold_s: f32,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            elevation_mask_deg: DEFAULT_ELEVATION_MASK_DEG,
            snr_drop_db: DEFAULT_SNR_DROP_DB,
            slip_window_min: DEFAULT_SLIP_WINDOW_MIN,
            clock_excursion_threshold_s: DEFAULT_CLOCK_EXCURSION_THRESHOLD_S,
        }
    }
}

/// Precomputed mipmap series for all tracks in one file.
///
/// This is an opaque wrapper so the internal `TrackSeries` type stays private.
/// Build it on a background thread with [`prepare_file_series`] and hand the
/// result to [`PlotState::integrate_file`] on the UI thread.
pub struct PreparedSeries(pub(crate) Vec<series::TrackSeries>);

/// Build mipmap series for all tracks in `file` using `fi` as the file index.
///
/// This is the CPU-heavy work - call it from a background loader thread, not
/// from the render loop.  `analysis` parameterizes the derived satellite series;
/// pass the same value the [`PlotState`] is using so newly loaded files match
/// the rest of the plot.
pub fn prepare_file_series(
    fi: usize,
    file: &gt_types::LoadedFile,
    analysis: AnalysisConfig,
) -> PreparedSeries {
    PreparedSeries(series::build_file_series(fi, file, analysis))
}
