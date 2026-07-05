//! Query-match data handed from a query run to the map renderer.

use std::collections::HashMap;
use std::ops::Range;

use gt_types::TrackRef;

/// The composed display effect of a query pipeline, as point-index ranges per
/// track.
///
/// Produced by the app layer from the pipeline output and consumed by the map.
/// Rendering state only - per-query summaries and tables stay with the query
/// window. `hide`/`keep` queries contribute to `hidden` (the polyline breaks
/// there); each `draw` query is one entry in `draws`, painted in its own color.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryMatches {
    /// Per track: point ranges removed from the map (sorted, disjoint).
    pub hidden: HashMap<TrackRef, Vec<Range<usize>>>,
    /// One layer per `draw` query, in draw order.
    pub draws: Vec<DrawLayer>,
    /// True when the visible data changed after the run - the display grays
    /// out (or reverts) until the query runs again.
    pub stale: bool,
}

/// One `draw` query's halos: the points it matched that are still shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrawLayer {
    /// Palette index the renderer maps to a color (see `gt_ui_theme`).
    pub color: usize,
    /// Per track: sorted, disjoint, non-empty point-index ranges.
    pub ranges: HashMap<TrackRef, Vec<Range<usize>>>,
}

impl QueryMatches {
    /// Whether this run affects the map at all.
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty() && self.draws.iter().all(|d| d.ranges.is_empty())
    }

    /// The hidden ranges for one track, empty when none.
    ///
    /// Checks the sorted-and-disjoint invariant in debug builds: this is a
    /// per-track entry point, so the check runs once per track per frame
    /// rather than once per point.
    pub fn hidden_ranges(&self, track: TrackRef) -> &[Range<usize>] {
        track_ranges(&self.hidden, track)
    }

    /// Whether the point is hidden (removed from the map).
    pub fn is_hidden(&self, track: TrackRef, point_index: usize) -> bool {
        Self::range_at(self.hidden_ranges(track), point_index).is_some()
    }

    /// A bitmask of the `draw` layers covering the point (bit `i` for
    /// `draws[i]`). Few layers, so the caller stores it in the point key.
    pub fn draw_bits(&self, track: TrackRef, point_index: usize) -> u16 {
        let mut bits = 0u16;
        for (i, layer) in self.draws.iter().enumerate() {
            if Self::range_at(track_ranges(&layer.ranges, track), point_index).is_some() {
                bits |= 1u16 << (i % u16::BITS as usize);
            }
        }
        bits
    }

    /// The draw range containing the point, for the hover header: the first
    /// layer that covers it, since a header shows one match's extent.
    pub fn header_range(&self, track: TrackRef, point_index: usize) -> Option<&Range<usize>> {
        self.draws
            .iter()
            .find_map(|layer| Self::range_at(track_ranges(&layer.ranges, track), point_index))
    }

    /// The range containing the point within sorted, disjoint ranges.
    ///
    /// Binary search; associated so the renderer can reuse it on a slice it
    /// already holds. Deliberately without the invariant assert - this sits on
    /// the per-point hot path.
    pub fn range_at(ranges: &[Range<usize>], point_index: usize) -> Option<&Range<usize>> {
        let candidate = ranges.partition_point(|r| r.end <= point_index);
        ranges.get(candidate).filter(|r| r.contains(&point_index))
    }
}

impl DrawLayer {
    /// This layer's ranges for one track, empty when none.
    pub fn ranges_for(&self, track: TrackRef) -> &[Range<usize>] {
        track_ranges(&self.ranges, track)
    }
}

/// Look up a track's ranges, asserting the sorted-disjoint invariant in debug.
fn track_ranges(map: &HashMap<TrackRef, Vec<Range<usize>>>, track: TrackRef) -> &[Range<usize>] {
    let ranges: &[Range<usize>] = map.get(&track).map_or(&[], Vec::as_slice);
    debug_assert!(
        ranges_are_sorted_disjoint(ranges),
        "QueryMatches ranges for {track:?} must be sorted and disjoint"
    );
    ranges
}

fn ranges_are_sorted_disjoint(ranges: &[Range<usize>]) -> bool {
    ranges.iter().all(|r| !r.is_empty())
        && ranges.windows(2).all(|pair| match pair {
            [a, b] => a.end <= b.start,
            _ => true,
        })
}

#[cfg(test)]
mod tests {
    use gt_types::{FileIdx, TrackIdx};

    use super::*;

    fn track() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    /// A range built from arguments rather than a literal, so single-element
    /// `vec![rng(0, 1)]` does not trip clippy's `single_range_in_vec_init`.
    fn rng(start: usize, end: usize) -> Range<usize> {
        start..end
    }

    fn layer(color: usize, ranges: Vec<Range<usize>>) -> DrawLayer {
        DrawLayer {
            color,
            ranges: HashMap::from([(track(), ranges)]),
        }
    }

    #[test]
    fn draw_bits_report_each_covering_layer() {
        let matches = QueryMatches {
            draws: vec![layer(0, vec![rng(0, 3)]), layer(1, vec![rng(2, 6)])],
            ..QueryMatches::default()
        };
        // Point 1: only layer 0. Point 2: both. Point 5: only layer 1.
        assert_eq!(matches.draw_bits(track(), 1), 0b01);
        assert_eq!(matches.draw_bits(track(), 2), 0b11);
        assert_eq!(matches.draw_bits(track(), 5), 0b10);
        assert_eq!(matches.draw_bits(track(), 9), 0);
    }

    #[test]
    fn hidden_and_header_lookups() {
        let matches = QueryMatches {
            hidden: HashMap::from([(track(), vec![rng(2, 5), rng(9, 10)])]),
            draws: vec![layer(0, vec![rng(0, 3), rng(14, 20)])],
            ..QueryMatches::default()
        };
        assert!(matches.is_hidden(track(), 3));
        assert!(!matches.is_hidden(track(), 5));
        assert_eq!(matches.header_range(track(), 1), Some(&(0..3)));
        assert_eq!(matches.header_range(track(), 15), Some(&(14..20)));
        assert_eq!(matches.header_range(track(), 8), None);
    }

    #[test]
    fn empty_when_nothing_hidden_or_drawn() {
        assert!(QueryMatches::default().is_empty());
        let drawn = QueryMatches {
            draws: vec![layer(0, vec![rng(0, 1)])],
            ..QueryMatches::default()
        };
        assert!(!drawn.is_empty());
    }
}
