mod plot_widget;
mod series;

pub use plot_widget::{PlotState, find_closest_tpv, show_trip_plot};

/// Precomputed mipmap series for all trips in one file.
///
/// This is an opaque wrapper so the internal `TripSeries` type stays private.
/// Build it on a background thread with [`prepare_file_series`] and hand the
/// result to [`PlotState::integrate_file`] on the UI thread.
pub struct PreparedSeries(pub(crate) Vec<series::TripSeries>);

/// Build mipmap series for all trips in `file` using `fi` as the file index.
///
/// This is the CPU-heavy work — call it from a background loader thread, not
/// from the render loop.
pub fn prepare_file_series(fi: usize, file: &gt_types::LoadedFile) -> PreparedSeries {
    PreparedSeries(series::build_file_series(fi, file))
}
