use std::collections::BTreeMap;

use gt_geo_math::segment_distances_m;
use gt_types::sat_label::{SatLabelAnchor, SatLabelTier};
use gt_types::{FixQuality, NavPoint, PointIdx};

/// Along-track spacing of [`SatLabelTier::Fill`] anchors. Dense enough that
/// labels appear on every screen at street-level zoom; the renderer's
/// collision resolution thins them everywhere else.
const FILL_SPACING_M: f64 = 100.0;

/// A satellite-report point of the track, with the fields the anchor
/// selection passes reason about. Snapshotting the count and quality up
/// front keeps the windowed passes free of repeated `Option` plumbing.
struct SatPoint {
    index: usize,
    fix_count: u32,
    quality: FixQuality,
}

/// Select the satellite-label anchor candidates for a track's points.
///
/// Anchors mark the points whose satellite state is worth labeling, in
/// descending diagnostic priority (see [`SatLabelTier`]): fix-quality
/// transitions, local minima of the fix count, track endpoints and
/// ghost-stretch recoveries, and periodic fill along the track. Only points
/// carrying a satellite report qualify. Each point gets at most one anchor,
/// keeping the highest-priority tier; the result is ascending by point index.
pub fn build_sat_label_anchors(points: &[NavPoint]) -> Vec<SatLabelAnchor> {
    let sat_points: Vec<SatPoint> = points
        .iter()
        .enumerate()
        .filter(|(_, p)| p.satellites.is_some())
        .map(|(index, p)| SatPoint {
            index,
            fix_count: p.fix_count(),
            quality: p.fix_quality(),
        })
        .collect();
    let (Some(first), Some(last)) = (sat_points.first(), sat_points.last()) else {
        return Vec::new();
    };

    let mut anchors: BTreeMap<usize, SatLabelTier> = BTreeMap::new();
    add_anchor(&mut anchors, first.index, SatLabelTier::Endpoint);
    add_anchor(&mut anchors, last.index, SatLabelTier::Endpoint);

    // Quality transitions between consecutive satellite-report points,
    // anchored where the new tier takes effect.
    for w in sat_points.windows(2) {
        let [prev, cur] = w else { continue };
        if cur.quality != prev.quality {
            add_anchor(&mut anchors, cur.index, SatLabelTier::QualityTransition);
        }
    }

    add_fix_count_minima(&mut anchors, &sat_points);
    add_ghost_recoveries(&mut anchors, points);
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

/// Anchor the local minima of the fix count over the satellite-report
/// points: runs of equal counts whose neighboring runs are both higher,
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

/// Anchor the first real fix carrying a satellite report after each ghost
/// stretch. Quality transitions already cover recoveries from zero-fix
/// ghosts; this additionally catches heading-loss ghosts, where the fix
/// count (and therefore the quality tier) never changed.
fn add_ghost_recoveries(anchors: &mut BTreeMap<usize, SatLabelTier>, points: &[NavPoint]) {
    for (i, w) in points.windows(2).enumerate() {
        let [prev, cur] = w else { continue };
        if prev.is_ghost_fix() && !cur.is_ghost_fix() && cur.satellites.is_some() {
            add_anchor(anchors, i + 1, SatLabelTier::Endpoint);
        }
    }
}

/// Anchor a satellite-report point whenever the along-track distance since
/// the last anchored point reaches [`FILL_SPACING_M`]. Existing anchors of
/// any tier reset the accumulator, so fill only covers the gaps between
/// higher-priority anchors.
fn add_fill(anchors: &mut BTreeMap<usize, SatLabelTier>, points: &[NavPoint]) {
    let mut since_anchor_m = 0.0;
    for (i, segment_m) in segment_distances_m(points).enumerate() {
        since_anchor_m += segment_m;
        let cur_index = i + 1;
        if anchors.contains_key(&cur_index) {
            since_anchor_m = 0.0;
        } else if since_anchor_m >= FILL_SPACING_M
            && points
                .get(cur_index)
                .is_some_and(|p| p.satellites.is_some())
        {
            add_anchor(anchors, cur_index, SatLabelTier::Fill);
            since_anchor_m = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use rstest::rstest;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    /// ~1 m of longitude at the equator, in degrees.
    const DEG_PER_METER: f64 = 360.0 / 40_030_173.0;

    /// A point `x_m` meters east of the origin. `fix_count: None` means no
    /// satellite report attached; `heading: false` makes a ghost fix.
    fn point(x_m: f64, fix_count: Option<u32>, heading: bool) -> NavPoint {
        let sats = fix_count.map(|n| {
            let list: Vec<_> = (1..=n.max(1))
                .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, prn <= n))
                .collect();
            Satellites::new(None, None, list)
        });
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(Latitude::new(0.0))
            .lon(Longitude::new(x_m * DEG_PER_METER))
            .maybe_heading(heading.then(|| Angle::new::<degree>(90.0)))
            .build();
        NavPoint::new(tpv, sats)
    }

    /// Points spaced 10 m apart with the given fix counts, heading present.
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
        assert!(build_sat_label_anchors(&points).is_empty());
    }

    #[test]
    fn first_and_last_satellite_points_are_endpoints() {
        let points = track(&[None, Some(12), Some(12), Some(12), None]);
        let anchors = build_sat_label_anchors(&points);
        assert_eq!(tier_at(&anchors, 1), Some(SatLabelTier::Endpoint));
        assert_eq!(tier_at(&anchors, 3), Some(SatLabelTier::Endpoint));
        assert_eq!(anchors.len(), 2);
    }

    #[test]
    fn quality_transitions_anchor_where_the_tier_changes() {
        // Strong → Marginal at 2, Marginal → Strong at 4.
        let points = track(&[Some(12), Some(12), Some(4), Some(4), Some(12), Some(12)]);
        let anchors = build_sat_label_anchors(&points);
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
        let anchors = build_sat_label_anchors(&points);
        assert_eq!(
            tier_at(&anchors, expected),
            Some(SatLabelTier::FixCountMinimum)
        );
    }

    #[test]
    fn a_dip_touching_the_track_end_is_not_a_minimum() {
        let points = track(&[Some(8), Some(5), Some(5)]);
        let anchors = build_sat_label_anchors(&points);
        // The last point is anchored as an endpoint, not as a minimum.
        assert_eq!(tier_at(&anchors, 2), Some(SatLabelTier::Endpoint));
        assert_eq!(tier_at(&anchors, 1), None);
    }

    #[test]
    fn ghost_recovery_is_anchored() {
        // Heading loss in the middle: fix count never changes, so no quality
        // transition fires - the recovery anchor must come from the ghost pass.
        let points: Vec<NavPoint> = (0i32..)
            .zip([true, true, false, false, true, true])
            .map(|(i, heading)| point(f64::from(i) * 10.0, Some(12), heading))
            .collect();
        let anchors = build_sat_label_anchors(&points);
        assert_eq!(tier_at(&anchors, 4), Some(SatLabelTier::Endpoint));
    }

    #[test]
    fn fill_anchors_cover_stable_stretches() {
        // 1 km of stable strong fixes, 10 m spacing: fill anchors roughly
        // every 100 m between the two endpoint anchors.
        let points = track(&[Some(12); 101]);
        let anchors = build_sat_label_anchors(&points);
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
        let anchors = build_sat_label_anchors(&points);
        assert_eq!(tier_at(&anchors, 2), Some(SatLabelTier::QualityTransition));
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

            let anchors = build_sat_label_anchors(&points);

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

    #[test]
    fn anchors_are_ascending_and_carry_satellite_reports() {
        let points = track(&[
            None,
            Some(12),
            Some(4),
            None,
            Some(4),
            Some(2),
            Some(4),
            Some(12),
        ]);
        let anchors = build_sat_label_anchors(&points);
        assert!(anchors.windows(2).all(|w| match w {
            [a, b] => a.point < b.point,
            _ => true,
        }));
        assert!(
            anchors
                .iter()
                .all(|a| a.point.get(&points).is_some_and(|p| p.satellites.is_some()))
        );
    }
}
