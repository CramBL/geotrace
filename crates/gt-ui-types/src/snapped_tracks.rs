use std::collections::HashMap;
use std::sync::Arc;

use gt_types::TrackRef;
use gt_types::mercator::MercPoint;

/// Snapped-track geometry for the map: one entry per track with a completed,
/// currently shown snap run.
///
/// Each segment is a polyline in normalized Mercator, projected once when the
/// run completes (runs are immutable, the map redraws every frame). Breaks
/// between segments render as gaps - route discontinuities and unsnapped
/// runs; the recorded track underneath is never painted over or hidden.
#[derive(Debug, Clone, Default)]
pub struct SnappedTracks {
    pub segments_by_track: HashMap<TrackRef, Arc<Vec<Vec<MercPoint>>>>,
}

impl SnappedTracks {
    pub fn is_empty(&self) -> bool {
        self.segments_by_track.is_empty()
    }
}
