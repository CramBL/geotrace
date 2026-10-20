use std::collections::BTreeMap;

use gt_types::sat_label::{SatLabelAnchor, SatLabelTier};
use gt_types::{
    FixQuality, PlacedPoints, PointIdx, PositionProvenance, SatelliteQueryScope, SolutionState,
};

struct SatPoint {
    index: usize,
    fix_count: u32,
    quality: (SolutionState, FixQuality),
}

/// Select the satellite-label anchor candidates for a track's points.
///
/// Anchors mark the points whose satellite state is worth labeling, in
/// descending diagnostic priority (see [`SatLabelTier`]): fix-quality
/// transitions, local minima of the fix count, track endpoints and
/// navigation recoveries, and periodic fill along the track. Points with a
/// known global used count qualify. Each point gets at most one anchor,
/// keeping the highest-priority tier. The result is ascending by point index.
///
/// A label is drawn beside the fix it belongs to, so only a track that has a
/// geometry has anchors: the fill spacing is measured along where its fixes
/// are drawn.
pub fn build_sat_label_anchors(points: PlacedPoints<'_>) -> Vec<SatLabelAnchor> {
    let sat_points: Vec<SatPoint> = points
        .fixes()
        .iter()
        .enumerate()
        .filter_map(|(index, p)| {
            let count = p.resolved_used_count(SatelliteQueryScope::Global)?.count;
            let state = p.resolved_navigation_state().state;
            Some(SatPoint {
                index,
                fix_count: count,
                quality: (state, p.resolved_fix_quality()),
            })
        })
        .collect();
    let (Some(first), Some(last)) = (sat_points.first(), sat_points.last()) else {
        return Vec::new();
    };

    let mut anchors: BTreeMap<usize, SatLabelTier> = BTreeMap::new();
    add_anchor(&mut anchors, first.index, SatLabelTier::Endpoint);
    add_anchor(&mut anchors, last.index, SatLabelTier::Endpoint);

    for w in sat_points.windows(2) {
        let [prev, cur] = w else { continue };
        if cur.quality != prev.quality {
            add_anchor(&mut anchors, cur.index, SatLabelTier::QualityTransition);
        }
    }

    add_fix_count_minima(&mut anchors, &sat_points);
    add_navigation_recoveries(&mut anchors, points);
    add_fill(&mut anchors, points);

    anchors
        .into_iter()
        .map(|(index, tier)| SatLabelAnchor {
            point: PointIdx::new(index),
            tier,
        })
        .collect()
}

/// Insert an anchor, keeping the higher-priority (smaller) tier when the
/// point is already anchored.
fn add_anchor(anchors: &mut BTreeMap<usize, SatLabelTier>, index: usize, tier: SatLabelTier) {
    anchors
        .entry(index)
        .and_modify(|t| *t = (*t).min(tier))
        .or_insert(tier);
}

/// Anchor the local minima over points with a known global used count:
/// runs of equal counts whose neighboring runs are both higher,
/// anchored at the run's first point. Runs touching either end of the
/// track are not minima - the endpoint anchors cover them.
fn add_fix_count_minima(anchors: &mut BTreeMap<usize, SatLabelTier>, sat_points: &[SatPoint]) {
    // (first point index of the run, fix count of the run)
    let mut runs: Vec<(usize, u32)> = Vec::new();
    for sp in sat_points {
        if runs.last().is_none_or(|&(_, count)| count != sp.fix_count) {
            runs.push((sp.index, sp.fix_count));
        }
    }
    for w in runs.windows(3) {
        let [(_, prev), (index, count), (_, next)] = w else {
            continue;
        };
        if count < prev && count < next {
            add_anchor(anchors, *index, SatLabelTier::FixCountMinimum);
        }
    }
}

fn add_navigation_recoveries(
    anchors: &mut BTreeMap<usize, SatLabelTier>,
    points: PlacedPoints<'_>,
) {
    for (i, w) in points.fixes().windows(2).enumerate() {
        let [prev, cur] = w else { continue };
        let previous_is_synthesized_or_without_fix = prev.navigation.position_provenance
            == Some(PositionProvenance::SdkSynthesized)
            || matches!(
                prev.resolved_navigation_state().state,
                SolutionState::NoFix | SolutionState::EstimatedOnly
            );
        let current_is_recorded_fix = cur.navigation.position_provenance
            != Some(PositionProvenance::SdkSynthesized)
            && cur.resolved_navigation_state().state == SolutionState::Fix;
        if previous_is_synthesized_or_without_fix
            && current_is_recorded_fix
            && cur
                .resolved_used_count(SatelliteQueryScope::Global)
                .is_some()
        {
            add_anchor(anchors, i + 1, SatLabelTier::Endpoint);
        }
    }
}

/// Anchor a point with a known global used count when the along-track distance since
/// the last anchored point reaches [`FILL_SPACING_M`]. Existing anchors of
/// any tier reset the accumulator, so fill only covers the gaps between
/// higher-priority anchors.
fn add_fill(anchors: &mut BTreeMap<usize, SatLabelTier>, points: PlacedPoints<'_>) {
    let positions: Vec<(gt_types::Latitude, gt_types::Longitude)> = points.positions().collect();
    let mut since_anchor_m = 0.0;
    for (i, segment_m) in gt_geo_math::segment_distances_m(&positions).enumerate() {
        since_anchor_m += segment_m;
        let cur_index = i + 1;
        if anchors.contains_key(&cur_index) {
            since_anchor_m = 0.0;
        } else if since_anchor_m >= FILL_SPACING_M
            && points.get(cur_index).is_some_and(|p| {
                p.fix
                    .resolved_used_count(SatelliteQueryScope::Global)
                    .is_some()
            })
        {
            add_anchor(anchors, cur_index, SatLabelTier::Fill);
            since_anchor_m = 0.0;
        }
    }
}

/// Along-track spacing of [`SatLabelTier::Fill`] anchors. Dense enough that
/// labels appear on every screen at street-level zoom. The renderer's
/// collision resolution thins them everywhere else.
const FILL_SPACING_M: f64 = 100.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util;
    use chrono::Utc;
    use gt_types::fixtures::{self, FixKind, MetricOffset};
    use gt_types::nav_point::NavPoint;
    use rstest::rstest;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    /// The anchors of `points` taken as a track of their own, none for a track
    /// of no fixes.
    fn anchors_of(points: &[NavPoint]) -> Vec<SatLabelAnchor> {
        test_util::with_placed_points_of(points, |placed| {
            placed.map_or_else(Vec::new, build_sat_label_anchors)
        })
    }

    /// A point `x_m` meters east of the origin. `fix_count: None` means no
    /// satellite report attached. `heading: false` makes a ghost fix.
    fn point(x_m: f64, fix_count: Option<u32>, heading: bool) -> NavPoint {
        let (latitude, longitude) = MetricOffset {
            east_m: x_m,
            north_m: 0.0,
        }
        .to_latlon();
        let without_a_report = fixtures::nav_point_heading(
            Utc::now(),
            latitude,
            longitude,
            heading.then(|| Angle::new::<degree>(90.0)),
            FixKind::GhostWithoutHeading,
        );
        NavPoint::new(
            without_a_report.tpv,
            fix_count.map(test_util::satellite_report_of),
        )
    }

    /// Points spaced 10 m apart with the fix counts in `fix_counts`, heading
    /// present.
    fn track(fix_counts: &[Option<u32>]) -> Vec<NavPoint> {
        (0i32..)
            .zip(fix_counts)
            .map(|(i, &n)| point(f64::from(i) * 10.0, n, true))
            .collect()
    }

    fn tier_at(anchors: &[SatLabelAnchor], index: usize) -> Option<SatLabelTier> {
        anchors
            .iter()
            .find(|a| a.point == PointIdx::new(index))
            .map(|a| a.tier)
    }

    #[test]
    fn no_satellite_reports_no_anchors() {
        let points = track(&[None, None, None]);
        assert!(anchors_of(&points).is_empty());
    }

    #[test]
    fn first_and_last_satellite_points_are_endpoints() {
        let points = track(&[None, Some(12), Some(12), Some(12), None]);
        let anchors = anchors_of(&points);
        assert_eq!(tier_at(&anchors, 1), Some(SatLabelTier::Endpoint));
        assert_eq!(tier_at(&anchors, 3), Some(SatLabelTier::Endpoint));
        assert_eq!(anchors.len(), 2);
    }

    #[test]
    fn quality_transitions_anchor_where_the_tier_changes() {
        // Strong → Marginal at 2, Marginal → Strong at 4.
        let points = track(&[Some(12), Some(12), Some(4), Some(4), Some(12), Some(12)]);
        let anchors = anchors_of(&points);
        assert_eq!(tier_at(&anchors, 2), Some(SatLabelTier::QualityTransition));
        assert_eq!(tier_at(&anchors, 4), Some(SatLabelTier::QualityTransition));
    }

    #[rstest]
    // Strict minimum inside marginal territory: no tier change 8→5→8.
    #[case(&[Some(12), Some(8), Some(5), Some(8), Some(12)], 2)]
    // Plateau minimum anchors at the plateau's first point.
    #[case(&[Some(12), Some(8), Some(5), Some(5), Some(5), Some(8), Some(12)], 2)]
    fn fix_count_minima_anchor_the_dip(#[case] counts: &[Option<u32>], #[case] expected: usize) {
        let points = track(counts);
        let anchors = anchors_of(&points);
        assert_eq!(
            tier_at(&anchors, expected),
            Some(SatLabelTier::FixCountMinimum)
        );
    }

    #[test]
    fn a_dip_touching_the_track_end_is_not_a_minimum() {
        let points = track(&[Some(8), Some(5), Some(5)]);
        let anchors = anchors_of(&points);
        // The last point is anchored as an endpoint, not as a minimum.
        assert_eq!(tier_at(&anchors, 2), Some(SatLabelTier::Endpoint));
        assert_eq!(tier_at(&anchors, 1), None);
    }

    #[test]
    fn synthesized_position_recovery_is_anchored() {
        let points: Vec<NavPoint> = (0i32..)
            .zip([true, true, false, false, true, true])
            .map(|(i, heading)| {
                let mut point = point(f64::from(i) * 10.0, Some(12), heading);
                if !heading {
                    point.navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
                }
                point
            })
            .collect();
        let anchors = anchors_of(&points);
        assert_eq!(tier_at(&anchors, 4), Some(SatLabelTier::Endpoint));
    }

    #[test]
    fn fill_anchors_cover_stable_stretches() {
        // 1 km of stable strong fixes, 10 m spacing: fill anchors roughly
        // every 100 m between the two endpoint anchors.
        let points = track(&[Some(12); 101]);
        let anchors = anchors_of(&points);
        let fills = anchors
            .iter()
            .filter(|a| a.tier == SatLabelTier::Fill)
            .count();
        assert!(
            (8..=10).contains(&fills),
            "expected ~9 fill anchors over 1 km, got {fills}"
        );
    }

    #[test]
    fn higher_priority_tier_wins_on_a_shared_point() {
        // The last satellite point is both an endpoint and a quality
        // transition - the transition tier must win.
        let points = track(&[Some(12), Some(12), Some(4)]);
        let anchors = anchors_of(&points);
        assert_eq!(tier_at(&anchors, 2), Some(SatLabelTier::QualityTransition));
    }

    #[test]
    fn navigation_summaries_supply_label_counts_without_observations() {
        let points: Vec<_> = [Some(8), None, Some(5), Some(8), Some(0)]
            .into_iter()
            .enumerate()
            .map(|(i, count)| {
                let mut point = point(i as f64 * 10.0, None, false);
                point.navigation.used_count = count;
                point.navigation.state = Some(SolutionState::Fix);
                point
            })
            .collect();
        let anchors = anchors_of(&points);
        assert_eq!(tier_at(&anchors, 0), Some(SatLabelTier::Endpoint));
        assert_eq!(tier_at(&anchors, 1), None);
        assert_eq!(tier_at(&anchors, 2), Some(SatLabelTier::FixCountMinimum));
        assert_eq!(tier_at(&anchors, 4), Some(SatLabelTier::QualityTransition));
    }

    #[test]
    fn headingless_recorded_positions_do_not_create_recovery_labels() {
        let points: Vec<_> = [true, false, true, true]
            .into_iter()
            .enumerate()
            .map(|(i, heading)| point(i as f64 * 10.0, Some(8), heading))
            .collect();
        assert_eq!(tier_at(&anchors_of(&points), 2), None);
    }

    #[rstest]
    #[case::fix(SolutionState::Fix, Some(SatLabelTier::Endpoint))]
    #[case::estimated_only(SolutionState::EstimatedOnly, None)]
    #[case::unknown(SolutionState::Unknown, None)]
    fn recovery_labels_require_a_recorded_fix(
        #[case] state: SolutionState,
        #[case] expected: Option<SatLabelTier>,
    ) {
        let mut points = track(&[None; 4]);
        for point in &mut points {
            point.navigation.state = Some(state);
            point.navigation.used_count = Some(8);
        }
        points[1].navigation.state = Some(SolutionState::NoFix);
        points[1].navigation.used_count = None;
        assert_eq!(tier_at(&anchors_of(&points), 2), expected);
    }

    proptest::proptest! {
        /// Invariants under arbitrary track shapes (fix counts, heading
        /// dropouts, point spacing): anchors ascend without duplicates,
        /// never outnumber the points, and always sit on a point that
        /// carries a satellite report.
        #[test]
        fn anchor_invariants_hold_for_arbitrary_tracks(
            specs in proptest::collection::vec(
                (
                    proptest::option::of(0u32..20),
                    proptest::bool::ANY,
                    0.0f64..300.0,
                ),
                0..100usize,
            )
        ) {
            let mut x_m = 0.0;
            let points: Vec<NavPoint> = specs
                .iter()
                .map(|&(fix_count, heading, step_m)| {
                    x_m += step_m;
                    point(x_m, fix_count, heading)
                })
                .collect();

            let anchors = anchors_of(&points);

            proptest::prop_assert!(
                anchors
                    .windows(2)
                    .all(|w| matches!(w, [a, b] if a.point < b.point))
            );
            proptest::prop_assert!(anchors.len() <= points.len());
            for a in &anchors {
                proptest::prop_assert!(
                    a.point.get(&points).is_some_and(|p| p.satellites.is_some())
                );
            }
        }
    }
}
