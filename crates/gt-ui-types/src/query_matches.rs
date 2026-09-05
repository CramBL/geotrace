//! Query-match data handed from a query run to the map renderer.

use std::fmt;
use std::ops::Range;

use gt_fmt::EM_DASH;
use gt_types::TrackRef;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// Point-index ranges per track, each track's ranges sorted and disjoint.
pub type TrackRanges = FxHashMap<TrackRef, Vec<Range<usize>>>;

/// The composed display effect of a query pipeline, as point-index ranges per
/// track.
///
/// Produced by the app layer from the pipeline output and consumed by the map.
/// Rendering state only - per-query summaries and tables stay with the query
/// window. `hide`/`keep` queries contribute to `hidden` (the polyline breaks
/// there). Each `draw` query is one entry in `draws`, painted in its own color.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryMatches {
    /// Per track: point ranges removed from the map (sorted, disjoint).
    pub hidden: TrackRanges,
    /// One layer per `draw` query, in draw order.
    pub draws: Vec<DrawLayer>,
    /// True when the visible data changed after the run - the display grays
    /// out (or reverts) until the query runs again.
    pub stale: bool,
    /// Monotonic sequence number of the run that produced these matches. Zero
    /// means no run produced them - a synthetic or hand-built value, which the
    /// map never animates.
    pub run: u64,
}

/// What every surface tells the user about a stale run, in one wording: the
/// note itself, and the reason a button disabled by the staleness gives, which
/// is the note plus what running again would let the button do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleRunNote {
    /// The note on its own, as the query window and the map tooltip show it.
    RunAgain,
    /// The reason the button framing the map on the whole run gives.
    RunAgainToFrameItsMatches,
    /// The reason the button framing the map on one match row gives.
    RunAgainToFrameThisMatch,
}

impl fmt::Display for StaleRunNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Data changed since this run {EM_DASH} run again")?;
        f.write_str(match self {
            Self::RunAgain => "",
            Self::RunAgainToFrameItsMatches => " to frame its matches",
            Self::RunAgainToFrameThisMatch => " to frame this match",
        })
    }
}

/// What the map frames and re-reveals for a map button press in the query
/// window: every match the run drew, or the one match on a name row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchRevealTarget {
    WholeRun,
    OneMatch {
        track: TrackRef,
        points: Range<usize>,
    },
}

/// Which `draw` layers cover one point, as a fixed-width bitset (bit `i` for
/// `draws[i]`).
///
/// The map renderer stores one of these in every point key and reads which
/// layer to paint from it. Layer indices past [`DrawLayerMask::MAX_LAYERS`]
/// cannot be represented: the app clamps the pipeline to that many draw
/// queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawLayerMask(u16);

impl DrawLayerMask {
    /// The most `draw` layers the mask distinguishes, set by its bit width.
    pub const MAX_LAYERS: usize = u16::BITS as usize;

    /// Records that layer `index` covers the point.
    pub fn insert(&mut self, index: usize) {
        debug_assert!(
            index < Self::MAX_LAYERS,
            "draw layer {index} exceeds the {}-bit mask",
            Self::MAX_LAYERS
        );
        self.0 |= 1u16 << (index % Self::MAX_LAYERS);
    }

    /// Whether layer `index` covers the point.
    pub fn contains(self, index: usize) -> bool {
        self.0 & (1u16 << (index % Self::MAX_LAYERS)) != 0
    }

    /// Whether no layer covers the point.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many layers cover the point.
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
}

/// One `draw` query's halos: the points it matched that are still shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrawLayer {
    /// Palette index the renderer maps to a color (see `gt_ui_theme`).
    pub color: usize,
    /// Per track: sorted, disjoint, non-empty point-index ranges.
    pub ranges: TrackRanges,
}

impl QueryMatches {
    /// Whether this run affects the map at all.
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty() && self.draws.iter().all(|d| d.ranges.is_empty())
    }

    /// Whether any `draw` layer matched a point, so the map paints at least
    /// one halo.
    pub fn has_halos(&self) -> bool {
        self.draws.iter().any(|layer| !layer.ranges.is_empty())
    }

    /// The hidden ranges for one track, empty when none.
    ///
    /// Checks the sorted-and-disjoint invariant in debug builds.
    pub fn hidden_ranges(&self, track: TrackRef) -> &[Range<usize>] {
        track_ranges(&self.hidden, track)
    }

    /// One track's hidden and per-layer ranges, with the hash lookups done
    /// once for a whole walk over that track's points.
    pub fn track_view(&self, track: TrackRef) -> TrackMatchView<'_> {
        TrackMatchView {
            hidden: self.hidden_ranges(track),
            layers: self
                .draws
                .iter()
                .take(DrawLayerMask::MAX_LAYERS)
                .map(|layer| layer.ranges_for(track))
                .collect(),
        }
    }

    /// Whether the point is hidden (removed from the map).
    pub fn is_hidden(&self, track: TrackRef, point_index: usize) -> bool {
        self.track_view(track).is_hidden(point_index)
    }

    /// Which `draw` layers cover the point. Few layers, so the caller stores
    /// the mask in the point key.
    pub fn draw_mask(&self, track: TrackRef, point_index: usize) -> DrawLayerMask {
        self.track_view(track).draw_mask(point_index)
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
    /// Binary search, associated so the renderer can reuse it on a slice it
    /// already holds. Without the invariant assert: this sits on the per-point
    /// hot path.
    pub fn range_at(ranges: &[Range<usize>], point_index: usize) -> Option<&Range<usize>> {
        let candidate = ranges.partition_point(|r| r.end <= point_index);
        ranges.get(candidate).filter(|r| r.contains(&point_index))
    }
}

/// One track's query ranges, looked up out of [`QueryMatches`] once so that
/// each per-point test over that track is the binary search alone, with no
/// hash lookup. The default value has no hidden range and no layer, which the
/// map uses for a track when no query has run.
#[derive(Debug, Default)]
pub struct TrackMatchView<'a> {
    hidden: &'a [Range<usize>],
    layers: SmallVec<[&'a [Range<usize>]; 4]>,
}

impl<'a> TrackMatchView<'a> {
    /// One track's view of a run's matches, or the default view when no run's
    /// matches are shown.
    pub fn for_track(query_matches: Option<&'a QueryMatches>, track: TrackRef) -> Self {
        query_matches.map_or_else(Self::default, |matches| matches.track_view(track))
    }

    /// Whether the point is hidden (removed from the map).
    pub fn is_hidden(&self, point_index: usize) -> bool {
        QueryMatches::range_at(self.hidden, point_index).is_some()
    }

    /// Whether the query hides any point of this track.
    pub fn hides_any_point(&self) -> bool {
        !self.hidden.is_empty()
    }

    /// Which `draw` layers cover the point.
    pub fn draw_mask(&self, point_index: usize) -> DrawLayerMask {
        let mut mask = DrawLayerMask::default();
        for (index, ranges) in self.layers.iter().copied().enumerate() {
            if QueryMatches::range_at(ranges, point_index).is_some() {
                mask.insert(index);
            }
        }
        mask
    }
}

impl DrawLayer {
    /// This layer's ranges for one track, empty when none.
    pub fn ranges_for(&self, track: TrackRef) -> &[Range<usize>] {
        track_ranges(&self.ranges, track)
    }
}

/// Look up a track's ranges, asserting the sorted-disjoint invariant in debug.
fn track_ranges(map: &TrackRanges, track: TrackRef) -> &[Range<usize>] {
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

    /// A range built from arguments, so single-element `vec![rng(0, 1)]` does
    /// not trip clippy's `single_range_in_vec_init`.
    fn rng(start: usize, end: usize) -> Range<usize> {
        start..end
    }

    fn layer(color: usize, ranges: Vec<Range<usize>>) -> DrawLayer {
        DrawLayer {
            color,
            ranges: TrackRanges::from_iter([(track(), ranges)]),
        }
    }

    #[test]
    fn draw_mask_reports_each_covering_layer() {
        let matches = QueryMatches {
            draws: vec![layer(0, vec![rng(0, 3)]), layer(1, vec![rng(2, 6)])],
            ..QueryMatches::default()
        };
        // Point 1: only layer 0.
        let at_1 = matches.draw_mask(track(), 1);
        assert!(at_1.contains(0) && !at_1.contains(1));
        // Point 2: both layers.
        let at_2 = matches.draw_mask(track(), 2);
        assert!(at_2.contains(0) && at_2.contains(1));
        // Point 5: only layer 1.
        let at_5 = matches.draw_mask(track(), 5);
        assert!(!at_5.contains(0) && at_5.contains(1));
        // Point 9: no layer.
        assert!(matches.draw_mask(track(), 9).is_empty());
    }

    #[test]
    fn hidden_and_header_lookups() {
        let matches = QueryMatches {
            hidden: TrackRanges::from_iter([(track(), vec![rng(2, 5), rng(9, 10)])]),
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
    fn a_track_view_reports_the_hidden_and_covering_ranges_of_its_own_track() {
        let matches = QueryMatches {
            hidden: TrackRanges::from_iter([(track(), vec![rng(2, 5)])]),
            draws: vec![layer(0, vec![rng(0, 3)]), layer(1, vec![rng(2, 6)])],
            ..QueryMatches::default()
        };
        let view = matches.track_view(track());
        assert!(view.hides_any_point());
        assert!(!view.is_hidden(1));
        assert!(view.is_hidden(4));
        assert!(!view.is_hidden(5));
        assert_eq!(view.draw_mask(1), DrawLayerMask(0b01));
        assert_eq!(view.draw_mask(2), DrawLayerMask(0b11));
        assert_eq!(view.draw_mask(5), DrawLayerMask(0b10));
        assert_eq!(view.draw_mask(9), DrawLayerMask::default());
    }

    #[test]
    fn a_track_with_no_matches_has_no_hidden_point_and_no_covering_layer() {
        let matches = QueryMatches {
            hidden: TrackRanges::from_iter([(track(), vec![rng(2, 5)])]),
            draws: vec![layer(0, vec![rng(0, 3)])],
            ..QueryMatches::default()
        };
        let other = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
        let view = matches.track_view(other);
        assert!(!view.hides_any_point());
        assert!(!view.is_hidden(3));
        assert!(view.draw_mask(1).is_empty());
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

    #[test]
    #[should_panic(expected = "sorted and disjoint")]
    fn unsorted_ranges_fail_loudly_in_debug() {
        let matches = QueryMatches {
            hidden: TrackRanges::from_iter([(track(), vec![rng(9, 10), rng(2, 5)])]),
            ..QueryMatches::default()
        };
        let _ = matches.hidden_ranges(track());
    }

    #[test]
    fn draw_mask_ignores_layers_beyond_the_mask_width() {
        // One layer per bit, plus one extra that the mask cannot hold.
        let draws = (0..=DrawLayerMask::MAX_LAYERS)
            .map(|i| layer(i, vec![rng(0, 1)]))
            .collect();
        let matches = QueryMatches {
            draws,
            ..QueryMatches::default()
        };
        // Every representable layer covers point 0. The overflow layer does not
        // alias back onto bit 0.
        assert_eq!(matches.draws.len(), DrawLayerMask::MAX_LAYERS + 1);
        assert_eq!(
            matches.draw_mask(track(), 0).count() as usize,
            DrawLayerMask::MAX_LAYERS
        );
    }

    /// Every button reason is the shared note plus its own suffix.
    #[rstest::rstest]
    #[case(StaleRunNote::RunAgain, "Data changed since this run — run again")]
    #[case(
        StaleRunNote::RunAgainToFrameItsMatches,
        "Data changed since this run — run again to frame its matches"
    )]
    #[case(
        StaleRunNote::RunAgainToFrameThisMatch,
        "Data changed since this run — run again to frame this match"
    )]
    fn a_stale_run_states_the_same_note_before_every_suffix(
        #[case] note: StaleRunNote,
        #[case] expected: &str,
    ) {
        assert_eq!(note.to_string(), expected);
    }
}
