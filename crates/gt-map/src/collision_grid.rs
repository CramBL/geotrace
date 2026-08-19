//! Viewport-stable decimation: keep one winner per Mercator-space grid cell.
//!
//! Cells are anchored at the Mercator origin, not the viewport, so which
//! candidate wins a cell is independent of panning - only zooming rebuckets.
//! This is what keeps decimated overlays (satellite labels, sky glyphs) from
//! shuffling while the user navigates.
//!
//! The map's log hexagons cluster on the same grid ([`cluster_positions`]),
//! using it to find the neighbours a position may collapse into instead of
//! keeping one winner per cell.

use gt_types::{MercBounds, MercPoint};
use rustc_hash::FxHashMap;

use crate::transform::MapScale;

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
        // Cell iteration order is arbitrary: sort so renderers get ascending
        // indices.
        for bucket in &mut self.selected {
            bucket.sort_unstable();
        }
        &self.selected
    }
}

/// Zoom-level step the decimation cell size snaps to. Rounding to the nearest
/// bucket leaves the bucketed zoom at most a quarter-level off the true value
/// (2^0.25 ≈ 1.19x scale drift at a bucket's edges, none at its centre), while
/// a smooth zoom still crosses only a few boundaries.
const ZOOM_DECIMATION_BUCKET: f64 = 0.5;

/// Zoom snapped to a coarse bucket, used only to size the collision-grid cell
/// that thins satellite labels and sky glyphs.
///
/// That grid keys its cells in Mercator space, so a cell size that slides
/// continuously with zoom re-partitions the world on every frame: during a
/// smooth zoom the winning point per cell keeps changing, and labels and
/// glyphs flicker as they are dropped and re-added. Snapping the cell's zoom
/// to a bucket holds the partition - and thus the selected set - steady until
/// zoom crosses a boundary. Rendering still uses the real zoom, so positions
/// and scale stay smooth.
///
/// Both inputs to the cell size must use this bucketed zoom: the label spacing
/// ([`tpv_renderer::label_cell_px`]) also varies with zoom, so pairing it with
/// the real scale would leave the cell sliding.
pub(crate) fn decimation_zoom(zoom: f64) -> f64 {
    (zoom / ZOOM_DECIMATION_BUCKET).round() * ZOOM_DECIMATION_BUCKET
}

/// Mercator cell size for a decimation pass whose points sit `spacing_px`
/// apart on screen, computed at the bucketed zoom (see [`decimation_zoom`]).
pub(crate) fn decimation_cell_merc(spacing_px: f32, zoom: f64) -> f64 {
    f64::from(spacing_px) / MapScale::from_zoom(decimation_zoom(zoom)).px_per_merc()
}

/// One screen-space cluster of log matches: where its hexagon draws, and how
/// many matches collapsed into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PositionCluster {
    pub(crate) merc: MercPoint,
    pub(crate) count: usize,
}

/// Collapses positions less than `spacing_merc` apart into one cluster each,
/// then drops the clusters further than one `spacing_merc` outside `viewport`.
///
/// Two clusters always end up more than `spacing_merc` apart, which is what
/// keeps the hexagons they draw off each other: a cluster sits on the first of
/// its positions and takes in every later position within `spacing_merc` of
/// that point.
///
/// Panning changes what is on screen, never how the positions grouped. They
/// collapse into clusters before the viewport culls, and the culling bounds
/// grow by the distance a cluster reaches beyond its own position.
///
/// The clusters come back in the order their positions arrived in, which is
/// the order they are drawn in.
pub(crate) fn cluster_positions(
    positions: &[MercPoint],
    spacing_merc: f64,
    viewport: MercBounds,
) -> Vec<PositionCluster> {
    let mut clusters: Vec<PositionCluster> = Vec::new();
    // Every cluster is registered under the cell of its position. A position
    // collapses only into a cluster within `spacing_merc`, and cells are that
    // size, so the candidates all sit in the position's own cell or the eight
    // around it.
    let mut clusters_by_cell: FxHashMap<(i64, i64), Vec<usize>> = FxHashMap::default();
    for &merc in positions {
        let cell = cell_key(merc.x, merc.y, spacing_merc);
        let nearest = neighbouring_cells(cell)
            .filter_map(|neighbour| clusters_by_cell.get(&neighbour))
            .flatten()
            .filter_map(|&index| {
                let cluster = clusters.get(index)?;
                let distance_sq =
                    (cluster.merc.x - merc.x).powi(2) + (cluster.merc.y - merc.y).powi(2);
                (distance_sq < spacing_merc.powi(2)).then_some((index, distance_sq))
            })
            .min_by(|&(_, a), &(_, b)| a.total_cmp(&b))
            .map(|(index, _)| index);
        match nearest.and_then(|index| clusters.get_mut(index)) {
            Some(cluster) => cluster.count = cluster.count.saturating_add(1),
            None => {
                clusters_by_cell
                    .entry(cell)
                    .or_default()
                    .push(clusters.len());
                clusters.push(PositionCluster { merc, count: 1 });
            }
        }
    }
    clusters.retain(|cluster| {
        cluster.merc.x >= viewport.x_min - spacing_merc
            && cluster.merc.x <= viewport.x_max + spacing_merc
            && cluster.merc.y >= viewport.y_min - spacing_merc
            && cluster.merc.y <= viewport.y_max + spacing_merc
    });
    clusters
}

/// The cell itself and the eight cells around it.
fn neighbouring_cells((x, y): (i64, i64)) -> impl Iterator<Item = (i64, i64)> {
    (-1..=1)
        .flat_map(move |dx| (-1..=1).map(move |dy| (x.saturating_add(dx), y.saturating_add(dy))))
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
    use super::{
        DecimationScratch, MercBounds, MercPoint, PositionCluster, ZOOM_DECIMATION_BUCKET,
        cluster_positions, decimation_zoom,
    };

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

    fn merc(x: f64, y: f64) -> MercPoint {
        MercPoint { x, y }
    }

    /// The whole Mercator square, so a test that is not about culling keeps
    /// every position it hands in.
    fn whole_world() -> MercBounds {
        MercBounds {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        }
    }

    #[test]
    fn positions_within_the_spacing_collapse_into_a_cluster_counting_them() {
        let positions = [
            merc(0.100, 0.100),
            merc(0.101, 0.101),
            merc(0.102, 0.102),
            merc(0.400, 0.400),
        ];

        let clusters = cluster_positions(&positions, 0.01, whole_world());

        assert_eq!(
            clusters,
            [
                PositionCluster {
                    merc: merc(0.100, 0.100),
                    count: 3,
                },
                PositionCluster {
                    merc: merc(0.400, 0.400),
                    count: 1,
                },
            ],
            "a cluster draws where the first of its positions is"
        );
    }

    /// The regression the hexagons showed: two matches a hair apart used to
    /// survive as two glyphs drawn on top of each other whenever the cell
    /// boundary happened to fall between them.
    #[test]
    fn positions_across_a_cell_boundary_collapse_too() {
        let positions = [merc(0.009_999, 0.1), merc(0.010_001, 0.1)];

        assert_eq!(
            cluster_positions(&positions, 0.01, whole_world()),
            [PositionCluster {
                merc: merc(0.009_999, 0.1),
                count: 2,
            }]
        );
    }

    /// A dense run of positions - a log matching line after line along the
    /// track it was recorded beside - collapses in equal steps, one cluster
    /// per spacing along the run.
    #[test]
    fn a_dense_run_collapses_in_equal_steps() {
        let positions: Vec<MercPoint> = (0..500)
            .map(|step| {
                let along = f64::from(step) * 0.000_2;
                merc(0.1 + along, 0.1 + along)
            })
            .collect();

        let counts: Vec<usize> = cluster_positions(&positions, 0.01, whole_world())
            .iter()
            .map(|cluster| cluster.count)
            .collect();

        // The run steps 0.000_2 in both axes, so a cluster takes in the 36
        // positions from its own up to the last one within 0.01 of it, and the
        // final cluster takes what is left of the 500.
        assert_eq!(
            counts,
            [36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 32]
        );
    }

    proptest::proptest! {
        /// However the positions fall, every one of them lands in exactly one
        /// cluster and no two clusters end up within the spacing - the
        /// invariant the hexagons' readability rests on, since a cluster any
        /// closer would draw over its neighbour's count.
        #[test]
        fn clusters_take_every_position_and_stay_a_spacing_apart(
            points in proptest::collection::vec((0.0_f64..1.0, 0.0_f64..1.0), 0..200),
            spacing in 0.000_1_f64..0.2,
        ) {
            let positions: Vec<MercPoint> = points.iter().map(|&(x, y)| merc(x, y)).collect();

            let clusters = cluster_positions(&positions, spacing, whole_world());

            proptest::prop_assert_eq!(
                clusters.iter().map(|cluster| cluster.count).sum::<usize>(),
                positions.len()
            );
            for (index, cluster) in clusters.iter().enumerate() {
                for other in clusters.iter().skip(index.saturating_add(1)) {
                    let distance =
                        (cluster.merc.x - other.merc.x).hypot(cluster.merc.y - other.merc.y);
                    proptest::prop_assert!(
                        distance >= spacing,
                        "clusters at {:?} and {:?} are {} apart, closer than the {} spacing",
                        cluster.merc,
                        other.merc,
                        distance,
                        spacing
                    );
                }
            }
        }
    }

    /// Zooming in shrinks the spacing, which is what dissolves a cluster into
    /// the matches it stood for.
    #[test]
    fn a_smaller_spacing_splits_a_cluster() {
        let positions = [merc(0.1000, 0.1), merc(0.1050, 0.1)];

        assert_eq!(cluster_positions(&positions, 0.01, whole_world()).len(), 1);
        assert_eq!(cluster_positions(&positions, 0.001, whole_world()).len(), 2);
    }

    /// Panning moves what is on screen without regrouping anything: positions
    /// collapse before the viewport culls, so a cluster that stays on screen
    /// keeps both its place and its count.
    #[test]
    fn panning_leaves_the_clusters_where_they_were() {
        let positions = [merc(0.100, 0.1), merc(0.104, 0.1), merc(0.200, 0.1)];
        let wide = MercBounds {
            x_min: 0.05,
            x_max: 0.30,
            y_min: 0.05,
            y_max: 0.30,
        };
        // Panned far enough that the first of the two collapsed positions is
        // off the left edge: the cluster it seeded must not move onto the
        // second one.
        let panned = MercBounds {
            x_min: 0.102,
            x_max: 0.35,
            y_min: 0.05,
            y_max: 0.30,
        };

        assert_eq!(
            cluster_positions(&positions, 0.01, wide),
            cluster_positions(&positions, 0.01, panned)
        );
    }

    #[test]
    fn a_position_outside_the_viewport_is_dropped() {
        let positions = [merc(0.1, 0.1), merc(0.9, 0.9)];
        let viewport = MercBounds {
            x_min: 0.0,
            x_max: 0.5,
            y_min: 0.0,
            y_max: 0.5,
        };

        assert_eq!(
            cluster_positions(&positions, 0.01, viewport),
            [PositionCluster {
                merc: merc(0.1, 0.1),
                count: 1,
            }]
        );
    }

    /// The decimation zoom is a step function: a fine zoom sweep the width of a
    /// bucket lands on a single value, so the collision-grid cell - and thus
    /// the selected labels and glyphs - hold steady instead of churning
    /// frame-to-frame during a smooth zoom.
    #[test]
    fn decimation_zoom_holds_steady_within_a_bucket() {
        let start = 12.0;
        let sweep: Vec<(f64, f64)> = (0u16..50)
            .map(|i| start + f64::from(i) / 50.0 * ZOOM_DECIMATION_BUCKET)
            .map(|real| (real, decimation_zoom(real)))
            .collect();
        // Every step across one bucket width maps to at most two distinct
        // bucketed zooms (the one boundary the sweep may cross), never a fresh
        // value each frame.
        let mut distinct: Vec<f64> = sweep.iter().map(|&(_, bucketed)| bucketed).collect();
        distinct.dedup();
        assert!(
            distinct.len() <= 2,
            "sweep churned across {} buckets: {distinct:?}",
            distinct.len()
        );
        // The bucketed zoom never drifts far from the real zoom, so on-screen
        // spacing stays close to the target.
        for &(real, bucketed) in &sweep {
            assert!((bucketed - real).abs() <= ZOOM_DECIMATION_BUCKET / 2.0);
        }
    }

    #[test]
    fn smallest_candidate_wins_a_cell() {
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
