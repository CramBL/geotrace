//! Narrow entry points used by the Criterion baselines for map policy.
//!
//! Benchmarks need to exercise the production implementation without making
//! its internal plan types public.

use std::hint;

use gt_filter::GlobalFilter;
use gt_types::{LoadedFile, SpatialPoint};
use gt_ui_types::{DisplayMask, MapPresence, TrackDataVisibility};

/// Compile the production per-track frame plan and keep the result opaque to
/// the optimizer.
pub fn compile_track_plan(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    display_mask: DisplayMask,
    zoom: f64,
) {
    hint::black_box(super::viewport::TrackPlan::compute(
        files,
        visibility,
        filter,
        display_mask,
        zoom,
    ));
}

/// The production candidate-local visibility check used by hover and the
/// marker renderers.
pub fn spatial_point_visible(point: &SpatialPoint, scope: MapPresence<'_>) -> bool {
    super::viewport::is_spatial_point_visible(point, scope)
}
