#![cfg(test)]
//! Fixtures shared between the test modules of gt-track-builder.

use gt_types::{NavPoint, PlacedPoints};

use crate::segment::{self, FixPlacementRule};

/// Calls `read` with `points` taken as a track of their own, each fix beside
/// where the builder places it. `read` takes `None` for a track whose every
/// fix the builder leaves unplaced. The placement borrows the geometry, which
/// lives for the call alone.
pub fn with_placed_points_of<R>(
    points: &[NavPoint],
    read: impl FnOnce(Option<PlacedPoints<'_>>) -> R,
) -> R {
    let geometry = segment::measure_track_geometry(points, FixPlacementRule::default());
    read(
        geometry
            .measured()
            .and_then(|measured| PlacedPoints::new(points, &measured.resolved_positions)),
    )
}
