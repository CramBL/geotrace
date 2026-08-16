use std::collections::HashMap;
use std::sync::Arc;

use gt_types::TrackRef;

/// One fix's interference value, resolved for plotting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JammingPoint {
    /// Plot x: the fix's time as Unix seconds (the plot's shared x-axis).
    pub x_secs: f64,
    /// Share of aircraft reporting low navigation integrity in the fix's
    /// cell, in percent. [`None`] where the fix's day is not archived, or
    /// its cell was not published, which breaks the line.
    pub percent: Option<f64>,
    /// Aircraft the share was computed over. Zero where `percent` is
    /// [`None`].
    pub aircraft: u32,
    /// Aircraft of those that reported low navigation integrity, for the
    /// hover's counts.
    pub bad: u32,
}

/// Interference values for the plot: one entry per track, one point per
/// fix, pre-resolved by the app from the archive.
///
/// Mirrors [`crate::SnapErrorSeries`], including the per-track [`Arc`] whose
/// identity drives the plot's cache invalidation.
#[derive(Debug, Clone, Default)]
pub struct JammingSeries {
    pub points_by_track: HashMap<TrackRef, Arc<Vec<JammingPoint>>>,
}

impl JammingSeries {
    pub fn is_empty(&self) -> bool {
        self.points_by_track.is_empty()
    }
}
