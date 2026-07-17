//! Viewport-stable decimation: keep one winner per Mercator-space grid cell.
//!
//! Cells are anchored at the Mercator origin, not the viewport, so which
//! candidate wins a cell is independent of panning - only zooming rebuckets.
//! This is what keeps decimated overlays (satellite labels, sky glyphs) from
//! shuffling while the user navigates.

use std::collections::HashMap;

/// Keep the smallest candidate (by [`Ord`]) in each `cell_merc`-sized cell.
/// Winners come back in arbitrary order, so callers that need determinism
/// sort afterwards.
pub(crate) fn winners_per_cell<C: Ord + Copy>(
    candidates: impl IntoIterator<Item = ((f64, f64), C)>,
    cell_merc: f64,
) -> impl Iterator<Item = C> {
    let mut cells: HashMap<(i64, i64), C> = HashMap::new();
    for ((x, y), candidate) in candidates {
        cells
            .entry(cell_key(x, y, cell_merc))
            .and_modify(|c| *c = (*c).min(candidate))
            .or_insert(candidate);
    }
    cells.into_values()
}

/// Group per-geometry winners into ascending point-index lists, one bucket
/// per geometry index. The shared shape both decimated overlays render from.
pub(crate) fn group_by_geometry(
    winners: impl Iterator<Item = (usize, usize)>,
    geometry_count: usize,
) -> Vec<Vec<usize>> {
    let mut selected: Vec<Vec<usize>> = vec![Vec::new(); geometry_count];
    for (geometry_index, point_index) in winners {
        if let Some(bucket) = selected.get_mut(geometry_index) {
            bucket.push(point_index);
        }
    }
    // Cell iteration order is arbitrary; renderers get ascending indices.
    for bucket in &mut selected {
        bucket.sort_unstable();
    }
    selected
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
    use super::{group_by_geometry, winners_per_cell};

    #[test]
    fn smallest_candidate_wins_a_cell() {
        // Two candidates in one cell; the smaller by Ord survives.
        let winners: Vec<i32> = winners_per_cell([((0.1, 0.1), 5), ((0.2, 0.2), 2)], 1.0).collect();
        assert_eq!(winners, vec![2]);
    }

    #[test]
    fn distant_candidates_land_in_distinct_cells() {
        let mut winners: Vec<i32> =
            winners_per_cell([((0.0, 0.0), 1), ((5.0, 5.0), 2)], 1.0).collect();
        winners.sort_unstable();
        assert_eq!(winners, vec![1, 2]);
    }

    #[test]
    fn cells_are_anchored_at_the_origin_not_the_first_point() {
        // Points at 0.9 and 1.1 straddle the cell boundary at 1.0, so with a
        // unit cell they are in different cells and both survive - the
        // bucketing does not shift to wherever the first point happens to be.
        let mut winners: Vec<i32> =
            winners_per_cell([((0.9, 0.5), 1), ((1.1, 0.5), 2)], 1.0).collect();
        winners.sort_unstable();
        assert_eq!(winners, vec![1, 2]);
    }

    #[test]
    fn group_by_geometry_buckets_and_sorts() {
        let grouped = group_by_geometry([(0, 3), (0, 1), (2, 5)].into_iter(), 3);
        assert_eq!(grouped, vec![vec![1, 3], vec![], vec![5]]);
    }

    #[test]
    fn group_by_geometry_drops_out_of_range_indices() {
        let grouped = group_by_geometry([(0, 1), (9, 2)].into_iter(), 1);
        assert_eq!(grouped, vec![vec![1]]);
    }
}
