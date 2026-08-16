use std::collections::HashMap;
use std::sync::Arc;

use gt_types::TrackRef;

/// Per-point match kind of a snap run, as the plot shows it. A plain mirror
/// of gt-snap's wire enum so the plot stays decoupled from the snap machinery
/// (like [`crate::SnappedTracks`] for the map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapErrorKind {
    Snapped,
    /// The common case on slow recordings, not an anomaly: matched by
    /// interpolation between independently matched neighbors. Carries a full
    /// error value and gets no special styling, only its kind in hover text.
    Interpolated,
    /// The road network rejected this point: no error value, the series line
    /// breaks, and the plot marks the point.
    Unsnapped,
}

/// One sent point of a snap run, resolved for plotting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapErrorPoint {
    /// Plot x: the point's time as Unix seconds (the plot's shared x-axis).
    pub x_secs: f64,
    /// Snap error in meters; `None` exactly for unsnapped points.
    pub error_m: Option<f64>,
    pub kind: SnapErrorKind,
    /// True when the run holds no snap data for the points right before
    /// this one (the receiver was dead reckoning there, or a chunk failed). The
    /// series line breaks here.
    pub follows_gap: bool,
}

/// Snap error series for the plot: one entry per track with a completed snap
/// run. Points are the run's sent points in track order, pre-resolved from
/// `PointIdx` to plot time by the app. Runs are immutable, so the per-run `Arc`
/// is shared, not rebuilt per frame.
#[derive(Debug, Clone, Default)]
pub struct SnapErrorSeries {
    pub points_by_track: HashMap<TrackRef, Arc<Vec<SnapErrorPoint>>>,
}

impl SnapErrorSeries {
    pub fn is_empty(&self) -> bool {
        self.points_by_track.is_empty()
    }
}
