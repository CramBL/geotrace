use std::sync::Arc;

use gt_types::TrackRef;
use rustc_hash::FxHashMap;

/// One fix's geomagnetic index values, resolved for plotting.
///
/// Both indices are planetary averages over a period, so each is the value of
/// the period the fix's own UTC time falls in. They are on one scale, which
/// Hp30 climbs past 9 on during an extreme storm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeomagneticPoint {
    /// Plot x: the fix's time as Unix seconds (the plot's shared x-axis).
    pub x_secs: f64,
    /// The 30-minute period's Hp30 value. [`None`] where the fix's day is not
    /// archived or the service published no value for that period, which
    /// breaks the line.
    pub hp30: Option<f64>,
    /// The 3-hour period's Kp value, missing under the same conditions as
    /// [`Self::hp30`].
    pub kp: Option<f64>,
}

/// Geomagnetic index values for the plot: one entry per track, one point per
/// fix, pre-resolved by the app from the archive.
///
/// Mirrors [`crate::JammingSeries`], including the per-track [`Arc`] whose
/// identity drives the plot's cache invalidation.
#[derive(Debug, Clone, Default)]
pub struct GeomagneticSeries {
    pub points_by_track: FxHashMap<TrackRef, Arc<Vec<GeomagneticPoint>>>,
}

impl GeomagneticSeries {
    pub fn is_empty(&self) -> bool {
        self.points_by_track.is_empty()
    }
}
