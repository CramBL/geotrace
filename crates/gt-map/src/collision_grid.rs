//! Viewport-stable decimation: keep one winner per Mercator-space grid cell.
//!
//! Cells are anchored at the Mercator origin, not the viewport, so which
//! candidate wins a cell is independent of panning - only zooming rebuckets.
//! This is what keeps decimated overlays (satellite labels, sky glyphs) from
//! shuffling while the user navigates.

use rustc_hash::FxHashMap;

/// Reusable working set for one viewport-stable decimation pass, held across
/// frames so the candidate buffer, cell map, and per-geometry output lists
/// keep their allocations instead of being rebuilt every frame.
///
/// The cell map uses [`FxHashMap`]: keys are integer cell coordinates, so
/// SipHash's DoS resistance buys nothing over the per-frame rebuild.
pub(crate) struct DecimationScratch<C> {
    candidates: Vec<((f64, f64), C)>,
    cells: FxHashMap<(i64, i64), C>,
    selected: Vec<Vec<usize>>,
}

impl<C> Default for DecimationScratch<C> {
    fn default() -> Self {
        Self {
            candidates: Vec::new(),
            cells: FxHashMap::default(),
            selected: Vec::new(),
        }
    }
}

impl<C: Ord + Copy> DecimationScratch<C> {
    /// The per-geometry point-index lists from the last [`Self::resolve`],
    /// empty before the first call.
    pub(crate) fn selected(&self) -> &[Vec<usize>] {
        &self.selected
    }

    /// Empty the candidate buffer and hand it out for the caller to fill with
    /// this frame's `((merc_x, merc_y), candidate)` pairs.
    pub(crate) fn candidates(&mut self) -> &mut Vec<((f64, f64), C)> {
        self.candidates.clear();
        &mut self.candidates
    }

    /// Decimate the filled candidates into ascending per-geometry point-index
    /// lists, keeping the smallest candidate (by [`Ord`]) in each
    /// `cell_merc`-sized cell. `bucket_of` maps a surviving candidate to its
    /// `(geometry_index, point_index)`. The result always has exactly
    /// `geometry_count` buckets; the returned slice borrows the scratch.
    pub(crate) fn resolve(
        &mut self,
        cell_merc: f64,
        geometry_count: usize,
        bucket_of: impl Fn(C) -> (usize, usize),
    ) -> &[Vec<usize>] {
        self.cells.clear();
        for &((x, y), candidate) in &self.candidates {
            self.cells
                .entry(cell_key(x, y, cell_merc))
                .and_modify(|c| *c = (*c).min(candidate))
                .or_insert(candidate);
        }
        for bucket in &mut self.selected {
            bucket.clear();
        }
        self.selected.resize_with(geometry_count, Vec::new);
        for &candidate in self.cells.values() {
            let (geometry_index, point_index) = bucket_of(candidate);
            if let Some(bucket) = self.selected.get_mut(geometry_index) {
                bucket.push(point_index);
            }
        }
        // Cell iteration order is arbitrary; renderers get ascending indices.
        for bucket in &mut self.selected {
            bucket.sort_unstable();
        }
        &self.selected
    }
}

/// The grid cell containing a Mercator position. Cells are anchored at the
/// Mercator origin, which is what makes the bucketing pan-independent.
fn cell_key(x: f64, y: f64, cell_merc: f64) -> (i64, i64) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Mercator coords are in [0, 1] and cell sizes are bounded below by the max zoom, so the quotient is far inside i64 range"
    )]
    let key = |v: f64| (v / cell_merc).floor() as i64;
    (key(x), key(y))
}

#[cfg(test)]
mod tests {
    use super::DecimationScratch;

    /// Candidate carrying its cell winner value plus the geometry/point it
    /// maps to, so tests can check both the cell contest and the bucketing.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Cand {
        rank: i32,
        geometry: usize,
        point: usize,
    }

    /// Push `((x, y), rank, geometry, point)` candidates and resolve them.
    fn resolve(
        scratch: &mut DecimationScratch<Cand>,
        cell_merc: f64,
        geometry_count: usize,
        cands: &[((f64, f64), i32, usize, usize)],
    ) -> Vec<Vec<usize>> {
        let buf = scratch.candidates();
        for &(pos, rank, geometry, point) in cands {
            buf.push((
                pos,
                Cand {
                    rank,
                    geometry,
                    point,
                },
            ));
        }
        scratch
            .resolve(cell_merc, geometry_count, |c| (c.geometry, c.point))
            .to_vec()
    }

    #[test]
    fn smallest_candidate_wins_a_cell() {
        // Two candidates in one cell; the smaller by Ord (rank) survives.
        let mut scratch = DecimationScratch::default();
        let out = resolve(
            &mut scratch,
            1.0,
            1,
            &[((0.1, 0.1), 5, 0, 5), ((0.2, 0.2), 2, 0, 2)],
        );
        assert_eq!(out, vec![vec![2]]);
    }

    #[test]
    fn distant_candidates_land_in_distinct_cells() {
        let mut scratch = DecimationScratch::default();
        let out = resolve(
            &mut scratch,
            1.0,
            1,
            &[((0.0, 0.0), 1, 0, 1), ((5.0, 5.0), 2, 0, 2)],
        );
        assert_eq!(out, vec![vec![1, 2]]);
    }

    #[test]
    fn cells_are_anchored_at_the_origin_not_the_first_point() {
        // Points at 0.9 and 1.1 straddle the cell boundary at 1.0, so with a
        // unit cell they are in different cells and both survive - the
        // bucketing does not shift to wherever the first point happens to be.
        let mut scratch = DecimationScratch::default();
        let out = resolve(
            &mut scratch,
            1.0,
            1,
            &[((0.9, 0.5), 1, 0, 1), ((1.1, 0.5), 2, 0, 2)],
        );
        assert_eq!(out, vec![vec![1, 2]]);
    }

    #[test]
    fn buckets_are_grouped_and_sorted_per_geometry() {
        let mut scratch = DecimationScratch::default();
        let out = resolve(
            &mut scratch,
            1.0,
            3,
            &[
                ((0.0, 0.0), 0, 0, 3),
                ((5.0, 0.0), 0, 0, 1),
                ((0.0, 5.0), 0, 2, 5),
            ],
        );
        assert_eq!(out, vec![vec![1, 3], vec![], vec![5]]);
    }

    #[test]
    fn out_of_range_geometry_indices_are_dropped() {
        let mut scratch = DecimationScratch::default();
        let out = resolve(
            &mut scratch,
            1.0,
            1,
            &[((0.0, 0.0), 0, 0, 1), ((5.0, 0.0), 0, 9, 2)],
        );
        assert_eq!(out, vec![vec![1]]);
    }

    #[test]
    fn reused_scratch_does_not_leak_state_between_passes() {
        // A reused scratch must yield the same result as a fresh one: buffers
        // are cleared and the bucket count is resized to the new geometry
        // count, so a larger first pass cannot bleed into a smaller second.
        let mut scratch = DecimationScratch::default();
        let _first = resolve(
            &mut scratch,
            1.0,
            3,
            &[
                ((0.0, 0.0), 0, 0, 7),
                ((5.0, 0.0), 0, 1, 8),
                ((0.0, 5.0), 0, 2, 9),
            ],
        );
        let second = resolve(&mut scratch, 1.0, 1, &[((0.0, 0.0), 0, 0, 1)]);
        let fresh = resolve(
            &mut DecimationScratch::default(),
            1.0,
            1,
            &[((0.0, 0.0), 0, 0, 1)],
        );
        assert_eq!(second, fresh);
        assert_eq!(second, vec![vec![1]]);
    }
}
