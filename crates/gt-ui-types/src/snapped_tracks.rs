use std::collections::HashMap;
use std::sync::Arc;

use gt_types::mercator::MercPoint;
use gt_types::{PointIdx, TrackRef};

/// Snapped-track geometry for the map: one entry per track with a completed,
/// currently shown snap run.
///
/// Prepared once per run by the app (runs are immutable, the map redraws
/// every frame) and shared via `Arc`. Plain data mirrored from the snap
/// machinery, so the map keeps no gt-snap dependency.
#[derive(Debug, Clone, Default)]
pub struct SnappedTracks {
    pub by_track: HashMap<TrackRef, Arc<SnappedTrackGeometry>>,
}

impl SnappedTracks {
    pub fn is_empty(&self) -> bool {
        self.by_track.is_empty()
    }
}

/// One track's snapped geometry plus the hover attributes of the road edges
/// it was matched to.
#[derive(Debug, Clone, Default)]
pub struct SnappedTrackGeometry {
    /// Polyline segments in normalized Mercator. Breaks between segments render
    /// as gaps: route discontinuities and unsnapped runs.
    pub segments: Vec<SnappedSegment>,
    /// Hover rows referenced by the segments' edge spans.
    pub edges: Vec<SnappedEdgeInfo>,
    /// One anchor per sent point with a snapped position, for the error
    /// whiskers (recorded point to snapped position at high zoom). The
    /// recorded end lives on the track itself, resolved at draw time.
    pub whiskers: Vec<WhiskerAnchor>,
}

/// The snapped end of one error whisker, addressed back to its recorded
/// point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WhiskerAnchor {
    /// The recorded point the whisker starts at.
    pub point: PointIdx,
    /// The snapped position the whisker ends at, normalized Mercator.
    pub snapped: MercPoint,
}

/// One unbroken snapped polyline with its per-vertex edge attribution.
#[derive(Debug, Clone, Default)]
pub struct SnappedSegment {
    pub points: Vec<MercPoint>,
    /// Which edge each vertex run was matched to. Sorted by start; spans of
    /// adjacent edges overlap at their shared boundary vertex (lookups take
    /// the first covering span); vertices without edge coverage are simply
    /// absent from every span.
    pub edge_spans: Vec<SnappedEdgeSpan>,
}

/// A run of segment vertices (`start..end`, exclusive) matched to the edge
/// at `edge` (an index into [`SnappedTrackGeometry::edges`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnappedEdgeSpan {
    pub start: usize,
    pub end: usize,
    pub edge: usize,
}

impl SnappedSegment {
    /// The edge info index covering the given vertex, if any (the first
    /// covering span wins at shared boundary vertices).
    pub fn edge_at(&self, vertex: usize) -> Option<usize> {
        self.edge_spans
            .iter()
            .find(|span| span.start <= vertex && vertex < span.end)
            .map(|span| span.edge)
    }
}

/// The map-matching costing choices, mirrored plainly (like
/// `SnapErrorKind`) so the panel needs no gt-snap dependency. The app maps
/// this onto the wire costing with exhaustive matches, and sources display
/// labels from the wire type's canonical spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, strum::EnumIter)]
pub enum SnapCosting {
    Auto,
    Bicycle,
    Pedestrian,
}

/// Hover attributes of one matched road edge, pre-rendered by the app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnappedEdgeInfo {
    /// The road name(s), joined for display. `None` for unnamed ways.
    pub name: Option<String>,
    /// Display name of the road classification.
    pub road_class: Option<String>,
    /// Pre-rendered speed limit, e.g. `120 km/h` or `Unlimited`.
    pub speed_limit: Option<String>,
    /// Display name of the surface classification.
    pub surface: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boundary vertices shared by two spans resolve to the first span;
    /// uncovered vertices resolve to nothing.
    #[test]
    fn edge_at_takes_the_first_covering_span() {
        let segment = SnappedSegment {
            points: Vec::new(),
            edge_spans: vec![
                SnappedEdgeSpan {
                    start: 0,
                    end: 3,
                    edge: 7,
                },
                SnappedEdgeSpan {
                    start: 2,
                    end: 5,
                    edge: 8,
                },
            ],
        };
        assert_eq!(segment.edge_at(0), Some(7));
        assert_eq!(segment.edge_at(2), Some(7), "boundary vertex: first span");
        assert_eq!(segment.edge_at(4), Some(8));
        assert_eq!(segment.edge_at(5), None, "past the last span");
    }
}
