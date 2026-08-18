use std::collections::HashMap;
use std::sync::Arc;

use gt_types::TrackRef;

/// One fix's ionospheric total electron content, resolved for plotting.
///
/// The value is interpolated from the archived maps of the fix's own UTC day,
/// over the fix's position and time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TecPoint {
    /// Plot x: the fix's time as Unix seconds (the plot's shared x-axis).
    pub x_secs: f64,
    /// Vertical TEC in TEC units. [`None`] where the fix's day is not
    /// archived, its position lies outside the grid, or a contributing node
    /// is a gap, which breaks the line.
    pub tecu: Option<f64>,
}

/// TEC values for the plot: one entry per track, one point per fix,
/// pre-resolved by the app from the archive.
///
/// Mirrors [`crate::GeomagneticSeries`], including the per-track [`Arc`] whose
/// identity drives the plot's cache invalidation.
#[derive(Debug, Clone, Default)]
pub struct TecSeries {
    pub points_by_track: HashMap<TrackRef, Arc<Vec<TecPoint>>>,
}

impl TecSeries {
    pub fn is_empty(&self) -> bool {
        self.points_by_track.is_empty()
    }
}
