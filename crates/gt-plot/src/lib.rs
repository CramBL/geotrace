mod plot_widget;
mod series;

pub use plot_widget::{
    LEGEND_DOCK_OFFSET, PlotState, find_closest_tpv, legend_is_docked, show_track_plot,
};

/// Default elevation mask, in degrees, for the satellite utilization rate.
///
/// 15 deg is the conventional GNSS baseline: below it atmospheric delay and
/// multipath dominate, so receivers routinely ignore those satellites.
pub const DEFAULT_ELEVATION_MASK_DEG: f32 = 15.0;

/// Tunable parameters for the derived satellite-analysis series (currently the
/// utilization rate).  Threaded into series building so a change re-derives the
/// affected mipmaps.
///
/// New parameterized analyses (slip rate, etc.) add their fields here rather
/// than widening every build signature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisConfig {
    /// Elevation mask applied to the "in view" baseline of the utilization rate.
    pub elevation_mask_deg: f32,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            elevation_mask_deg: DEFAULT_ELEVATION_MASK_DEG,
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
