//! Viewport-stable placement of the satellite-count labels.
//!
//! Which points are worth labeling is decided at load time
//! (`LoadedTrack::sat_label_anchors`, see [`gt_track_builder::sat_label`]).
//! This module resolves per-frame which anchors actually get a label,
//! decimating them through the shared [`crate::collision_grid`] so the
//! highest-priority anchor wins each Mercator cell across all tracks at
//! once.

use gt_types::sat_label::SatLabelTier;
use gt_types::{LoadedTrack, MercBounds, NavPoint, TrackRef};

use crate::collision_grid;

/// One selected label: the anchor's point index into its track.
///
/// Grouped per geometry index by [`select_sat_labels`] so the renderer can
/// draw a track's labels in the same phase as its other layers.
pub(crate) type SelectedLabels = Vec<Vec<usize>>;

/// The candidate occupying a grid cell. Ordered comparison implements the
/// deterministic winner rule: highest tier first, then the stable
/// track/point key so ties cannot flicker between frames.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    tier: SatLabelTier,
    track: TrackRef,
    point_index: usize,
    /// Index into the caller's geometry list, carried through so the
    /// winners can be grouped per track for the phased paint passes.
    geometry_index: usize,
}

/// Resolve which satellite-label anchors get a label this frame.
///
/// `tracks` yields each visible track with its geometry index and ref;
/// `point_passes` applies the per-point conditions the caller already
/// knows about (time filter, query hiding). Anchors outside `viewport`
/// are skipped. Within each `cell_merc`-sized grid cell the
/// highest-priority candidate wins ([`Candidate`]'s ordering).
pub(crate) fn select_sat_labels<'a>(
    tracks: impl Iterator<Item = (usize, TrackRef, &'a LoadedTrack)>,
    geometry_count: usize,
    viewport: MercBounds,
    cell_merc: f64,
    mut point_passes: impl FnMut(TrackRef, usize, &NavPoint) -> bool,
) -> SelectedLabels {
    let mut candidates: Vec<((f64, f64), Candidate)> = Vec::new();
    for (geometry_index, track_ref, track) in tracks {
        for anchor in &track.sat_label_anchors {
            let Some(point) = anchor.point.get(&track.points) else {
                continue;
            };
            let (x, y) = (point.merc.x, point.merc.y);
            if x < viewport.x_min || x > viewport.x_max || y < viewport.y_min || y > viewport.y_max
            {
                continue;
            }
            if !point_passes(track_ref, anchor.point.as_usize(), point) {
                continue;
            }
            candidates.push((
                (x, y),
                Candidate {
                    tier: anchor.tier,
                    track: track_ref,
                    point_index: anchor.point.as_usize(),
                    geometry_index,
                },
            ));
        }
    }
    let winners = collision_grid::winners_per_cell(candidates, cell_merc)
        .map(|c| (c.geometry_index, c.point_index));
    collision_grid::group_by_geometry(winners, geometry_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gt_types::sat_label::SatLabelAnchor;
    use gt_types::{FileIdx, PointIdx, TrackIdx};

    fn track_with_anchors(
        positions_m: &[(f64, f64)],
        anchors: &[(usize, SatLabelTier)],
    ) -> LoadedTrack {
        let points = positions_m
            .iter()
            .map(|&(x_m, y_m)| gt_test_utils::nav_point_at_meters(x_m, y_m, None))
            .collect();
        let mut track = gt_test_utils::loaded_track_with_points(points);
        track.sat_label_anchors = anchors
            .iter()
            .map(|&(i, tier)| SatLabelAnchor {
                point: PointIdx::new(i),
                tier,
            })
            .collect();
        track
    }

    const WORLD: MercBounds = MercBounds {
        x_min: 0.0,
        x_max: 1.0,
        y_min: 0.0,
        y_max: 1.0,
    };

    fn select(tracks: &[LoadedTrack], viewport: MercBounds, cell_merc: f64) -> SelectedLabels {
        select_sat_labels(
            tracks
                .iter()
                .enumerate()
                .map(|(i, t)| (i, TrackRef::new(FileIdx::new(0), TrackIdx::new(i)), t)),
            tracks.len(),
            viewport,
            cell_merc,
            |_, _, _| true,
        )
    }

    #[test]
    fn higher_tier_wins_the_cell() {
        // Two anchors within one cell: the quality transition beats the fill.
        let track = track_with_anchors(
            &[(0.0, 0.0), (1.0, 0.0)],
            &[
                (0, SatLabelTier::Fill),
                (1, SatLabelTier::QualityTransition),
            ],
        );
        let selected = select(std::slice::from_ref(&track), WORLD, 1.0);
        assert_eq!(selected, vec![vec![1]]);
    }

    #[test]
    fn distant_anchors_land_in_distinct_cells() {
        // ~1100 km apart in a ~55 m cell: both must survive.
        let track = track_with_anchors(
            &[(0.0, 0.0), (1_100_000.0, 0.0)],
            &[(0, SatLabelTier::Fill), (1, SatLabelTier::Fill)],
        );
        let selected = select(std::slice::from_ref(&track), WORLD, 55.0 / 40_000_000.0);
        assert_eq!(selected, vec![vec![0, 1]]);
    }

    #[test]
    fn collisions_resolve_across_tracks() {
        // Same spot on two tracks: exactly one label, on the higher tier.
        let a = track_with_anchors(&[(0.0, 0.0)], &[(0, SatLabelTier::Fill)]);
        let b = track_with_anchors(&[(0.0, 0.0)], &[(0, SatLabelTier::Endpoint)]);
        let selected = select(&[a, b], WORLD, 1.0);
        assert_eq!(selected, vec![vec![], vec![0]]);
    }

    #[test]
    fn selection_is_pan_independent() {
        // The same anchors through two viewports that both contain them:
        // panning must not change which anchors are labeled.
        let track = track_with_anchors(
            &[(0.0, 0.0), (30.0, 0.0), (500.0, 0.0)],
            &[
                (0, SatLabelTier::Endpoint),
                (1, SatLabelTier::Fill),
                (2, SatLabelTier::Endpoint),
            ],
        );
        let cell = 60.0 / 40_000_000.0;
        let panned = MercBounds {
            x_min: 0.5 - 0.1,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        };
        // Both viewports contain every anchor (points sit near merc 0.5).
        let full = select(std::slice::from_ref(&track), WORLD, cell);
        let shifted = select(std::slice::from_ref(&track), panned, cell);
        assert_eq!(full, shifted);
        // And the dense pair actually collided (0 beat 1 by tier).
        assert_eq!(full, vec![vec![0, 2]]);
    }

    #[test]
    fn zoomed_out_only_the_top_anomaly_survives() {
        // A cell spanning the whole world (the far-zoom limit): the single
        // surviving label is the highest-priority anomaly, not whichever
        // anchor came first.
        let track = track_with_anchors(
            &[(0.0, 0.0), (50_000.0, 0.0), (100_000.0, 0.0)],
            &[
                (0, SatLabelTier::Endpoint),
                (1, SatLabelTier::QualityTransition),
                (2, SatLabelTier::Fill),
            ],
        );
        let selected = select(std::slice::from_ref(&track), WORLD, 1.0);
        assert_eq!(selected, vec![vec![1]]);
    }

    #[test]
    fn viewport_culls_and_filter_rejects() {
        let track = track_with_anchors(
            &[(0.0, 0.0), (1_000.0, 0.0)],
            &[(0, SatLabelTier::Endpoint), (1, SatLabelTier::Endpoint)],
        );
        let nothing = MercBounds {
            x_min: 0.0,
            x_max: 0.1,
            y_min: 0.0,
            y_max: 0.1,
        };
        assert_eq!(
            select(std::slice::from_ref(&track), nothing, 1.0),
            vec![Vec::<usize>::new()]
        );
        let selected = select_sat_labels(
            [(0, TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), &track)].into_iter(),
            1,
            WORLD,
            1e-9,
            |_, pi, _| pi != 0,
        );
        assert_eq!(selected, vec![vec![1]]);
    }
}
