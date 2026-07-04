//! Query-match data handed from a query run to the map renderer.

use std::collections::HashMap;
use std::ops::Range;

use gt_types::TrackRef;

/// Matches of a query run, as point-index ranges per track.
///
/// Produced by the app layer from a query run's output and consumed by the
/// map, which draws each range as a halo beneath the track line. Rendering
/// state only - the run summary and tables stay with the query window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryMatches {
    /// Per track: sorted, disjoint, non-empty point-index ranges.
    pub ranges: HashMap<TrackRef, Vec<Range<usize>>>,
    /// True when the visible data changed after the run - halos gray out
    /// until the query runs again.
    pub stale: bool,
}

impl QueryMatches {
    /// The ranges for one track, empty when the track has no matches.
    ///
    /// Checks the sorted-and-disjoint invariant in debug builds: this is the
    /// per-track entry point of the renderer, so the check runs once per
    /// track per frame instead of once per point.
    pub fn track_ranges(&self, track: TrackRef) -> &[Range<usize>] {
        let ranges: &[Range<usize>] = self.ranges.get(&track).map_or(&[], Vec::as_slice);
        debug_assert!(
            ranges_are_sorted_disjoint(ranges),
            "QueryMatches ranges for {track:?} must be sorted and disjoint"
        );
        ranges
    }

    /// The match range containing the given point, if any.
    pub fn match_at(&self, track: TrackRef, point_index: usize) -> Option<&Range<usize>> {
        Self::range_at(self.track_ranges(track), point_index)
    }

    /// The range containing the point within sorted, disjoint ranges.
    ///
    /// Binary search; associated so the renderer can reuse it on a
    /// [`Self::track_ranges`] slice it already holds. Deliberately without
    /// the invariant assert - this sits on the per-point hot path.
    pub fn range_at(ranges: &[Range<usize>], point_index: usize) -> Option<&Range<usize>> {
        let candidate = ranges.partition_point(|r| r.end <= point_index);
        ranges.get(candidate).filter(|r| r.contains(&point_index))
    }
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

    fn matches(ranges: Vec<Range<usize>>) -> QueryMatches {
        QueryMatches {
            ranges: HashMap::from([(track(), ranges)]),
            stale: false,
        }
    }

    #[test]
    fn match_at_finds_the_containing_range() {
        let m = matches(vec![2..5, 9..10, 14..20]);
        assert_eq!(m.match_at(track(), 2), Some(&(2..5)));
        assert_eq!(m.match_at(track(), 4), Some(&(2..5)));
        assert_eq!(m.match_at(track(), 5), None);
        assert_eq!(m.match_at(track(), 9), Some(&(9..10)));
        assert_eq!(m.match_at(track(), 13), None);
        assert_eq!(m.match_at(track(), 19), Some(&(14..20)));
    }

    #[test]
    fn unknown_track_has_no_matches() {
        let m = matches(vec![0..3]);
        let other = TrackRef::new(FileIdx::new(1), TrackIdx::new(0));
        assert!(m.track_ranges(other).is_empty());
        assert_eq!(m.match_at(other, 1), None);
    }

    #[test]
    #[should_panic(expected = "sorted and disjoint")]
    #[cfg(debug_assertions)]
    fn unsorted_ranges_fail_loudly_in_debug() {
        let m = matches(vec![9..10, 2..5]);
        let _ranges = m.track_ranges(track());
    }
}
