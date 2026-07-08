//! Viewport-stable placement of the satellite-count labels.
//!
//! Which points are worth labeling is decided at load time
//! (`LoadedTrack::sat_label_anchors`, see [`gt_track_builder::sat_label`]).
//! This module resolves per-frame which anchors actually get a label: a
//! collision grid keyed in Mercator space keeps the highest-priority anchor
//! per cell, across all tracks at once. Keying in Mercator units rather
//! than screen pixels makes the labeled set independent of panning - only
//! zooming rebuckets - so labels never shuffle while navigating.

use std::collections::HashMap;

use gt_types::sat_label::SatLabelTier;
use gt_types::{LoadedTrack, MercBounds, NavPoint, TrackRef};

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
    let mut cells: HashMap<(i64, i64), Candidate> = HashMap::new();
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
            let candidate = Candidate {
                tier: anchor.tier,
                track: track_ref,
                point_index: anchor.point.as_usize(),
                geometry_index,
            };
            cells
                .entry(cell_key(x, y, cell_merc))
                .and_modify(|c| *c = (*c).min(candidate))
                .or_insert(candidate);
        }
    }

    let mut selected: SelectedLabels = vec![Vec::new(); geometry_count];
    for c in cells.into_values() {
        if let Some(track_labels) = selected.get_mut(c.geometry_index) {
            track_labels.push(c.point_index);
        }
    }
    // Cell iteration order is arbitrary; renderers get ascending indices.
    for track_labels in &mut selected {
        track_labels.sort_unstable();
    }
    selected
}

/// The grid cell containing a Mercator position. Cells are anchored at the
/// Mercator origin, not the viewport, which is what makes the bucketing
/// pan-independent.
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
    use super::*;
    use gt_types::sat_label::SatLabelAnchor;
    use gt_types::{FileIdx, PointIdx, TrackIdx};

    /// ~1 m of longitude at the equator, in degrees.
    const DEG_PER_METER: f64 = 360.0 / 40_030_173.0;

    fn track_with_anchors(
        positions_m: &[(f64, f64)],
        anchors: &[(usize, SatLabelTier)],
    ) -> LoadedTrack {
        use gt_types::time_types::GpsTime;
        let points = positions_m
            .iter()
            .map(|&(x_m, y_m)| {
                let tpv = gt_types::TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(chrono::Utc::now()))
                    .lat(gt_types::Latitude::new(y_m * DEG_PER_METER))
                    .lon(gt_types::Longitude::new(x_m * DEG_PER_METER))
                    .build();
                NavPoint::new(tpv, None)
            })
            .collect();
        LoadedTrack {
            metadata: gt_types::TrackMetadata::default(),
            points,
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: anchors
                .iter()
                .map(|&(i, tier)| SatLabelAnchor {
                    point: PointIdx::new(i),
                    tier,
                })
                .collect(),
            custom_markers: Vec::new(),
            generated_markers: Vec::new(),
            event_markers: Vec::new(),
            channels: Vec::new(),
        }
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
