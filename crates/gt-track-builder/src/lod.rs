use gt_types::nav_point::NavPoint;
use gt_types::track::{LOD_BASE_TOLERANCE_MERC, TrackLod};

/// Stop building coarser levels once one has this few points - drawing them
/// costs nothing, and coarser levels would only erase the track's shape.
const MIN_LEVEL_POINTS: usize = 64;

/// Hard cap on stored levels. With doubling tolerances this spans a zoom
/// range far beyond what the map can express.
const MAX_LEVELS: usize = 24;

/// A level must shrink to at most 3/4 of its predecessor to be worth
/// storing. Otherwise its tolerance is below the recording's own point
/// spacing and the predecessor (or the full list) serves that scale.
/// Expressed as a ratio of integers to avoid float comparisons.
const MAX_KEPT_NUMERATOR: usize = 3;
const MAX_KEPT_DENOMINATOR: usize = 4;

/// Build the multi-resolution render LOD for a track's points.
///
/// Levels apply a Mercator-space radial-distance filter with doubling
/// tolerances: a point survives when it is at least the tolerance away from
/// the previously kept point or its [`NavPoint::render_class`] (ghost flag,
/// fix quality) differs, so no styled transition is ever decimated away.
/// The first and last point always survive. Each level is built from the
/// previous one, so the total build cost is a geometric series in the point
/// count, capped by the shrink ratio a stored level must reach.
///
/// The first level's tolerance starts near the track's mean segment length so
/// sparse recordings store no full-length levels.
pub fn build_track_lod(points: &[NavPoint]) -> TrackLod {
    if points.len() < MIN_LEVEL_POINTS || u32::try_from(points.len()).is_err() {
        return TrackLod::default();
    }

    let first_level_exp = first_useful_exponent(points);
    let mut levels: Vec<Vec<u32>> = Vec::new();
    let mut tolerance = LOD_BASE_TOLERANCE_MERC * 2_f64.powi(exponent_to_i32(first_level_exp));

    while levels.len() < MAX_LEVELS {
        let level = match levels.last() {
            None => decimate(points, 0..points.len(), tolerance),
            Some(prev) => decimate(
                points,
                prev.iter().filter_map(|&i| usize::try_from(i).ok()),
                tolerance,
            ),
        };
        let prev_len = levels.last().map_or(points.len(), Vec::len);
        if !levels.is_empty() && level.len() * MAX_KEPT_DENOMINATOR > prev_len * MAX_KEPT_NUMERATOR
        {
            // Tolerance doubled but barely anything merged (e.g. dense
            // key transitions everywhere): coarser levels won't help.
            break;
        }
        let done = level.len() < MIN_LEVEL_POINTS;
        levels.push(level);
        if done {
            break;
        }
        tolerance *= 2.0;
    }

    TrackLod::new(first_level_exp, levels)
}

/// The tolerance exponent at which decimation starts paying off: the level
/// whose tolerance is roughly the track's mean Mercator segment length.
/// Finer levels would keep nearly every point.
fn first_useful_exponent(points: &[NavPoint]) -> u32 {
    let segments = points.len().saturating_sub(1).max(1);
    let total: f64 = points
        .windows(2)
        .map(|w| match w {
            [a, b] => (b.merc().x - a.merc().x).hypot(b.merc().y - a.merc().y),
            _ => 0.0,
        })
        .sum();
    let mean = total / segments as f64;
    if mean <= LOD_BASE_TOLERANCE_MERC {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "mean > base so the log2 ratio is positive, and Mercator space caps it far below u32::MAX"
    )]
    let exp = (mean / LOD_BASE_TOLERANCE_MERC).log2().floor() as u32;
    exp
}

fn exponent_to_i32(exp: u32) -> i32 {
    i32::try_from(exp).unwrap_or(i32::MAX)
}

/// One radial-distance decimation pass over `candidates` (indices into
/// `points`, ascending). Keeps a candidate when it is at least `tolerance`
/// (Mercator units) from the last kept point or its render class changed.
/// The first and last candidate are always kept.
fn decimate(
    points: &[NavPoint],
    candidates: impl Iterator<Item = usize>,
    tolerance: f64,
) -> Vec<u32> {
    let tol_sq = tolerance * tolerance;
    let mut kept: Vec<u32> = Vec::new();
    let mut last_kept: Option<(gt_types::MercPoint, (bool, gt_types::FixQuality))> = None;
    let mut last_candidate: Option<u32> = None;

    for pi in candidates {
        let Some(point) = points.get(pi) else {
            continue;
        };
        let Ok(idx) = u32::try_from(pi) else {
            continue;
        };
        last_candidate = Some(idx);
        let class = point.render_class();
        let keep = match last_kept {
            None => true,
            Some((merc, last_class)) => {
                let dx = point.merc().x - merc.x;
                let dy = point.merc().y - merc.y;
                class != last_class || dx * dx + dy * dy >= tol_sq
            }
        };
        if keep {
            kept.push(idx);
            last_kept = Some((point.merc(), class));
        }
    }
    if let Some(last) = last_candidate
        && kept.last() != Some(&last)
    {
        kept.push(last);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gt_types::FixQuality;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    /// ~1 m of longitude at the equator, in degrees.
    const DEG_PER_METER: f64 = 360.0 / 40_030_173.0;

    fn point_at_meters(x_m: f64, fix_count: u32) -> NavPoint {
        let sats: Vec<_> = (1..=fix_count.max(1))
            .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, prn <= fix_count))
            .collect();
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(Latitude::new(0.0))
            .lon(Longitude::new(x_m * DEG_PER_METER))
            .heading(Angle::new::<degree>(90.0))
            .build();
        NavPoint::new(tpv, Some(Satellites::new(None, None, sats))).expect("coordinates in range")
    }

    /// 1024 strong-fix points spaced 10 m apart.
    fn uniform_track() -> Vec<NavPoint> {
        (0..1024)
            .map(|i| point_at_meters(f64::from(i) * 10.0, 12))
            .collect()
    }

    #[test]
    fn tiny_tracks_get_no_lod() {
        let points: Vec<_> = (0..10).map(|i| point_at_meters(f64::from(i), 12)).collect();
        let lod = build_track_lod(&points);
        assert!(lod.select(f64::MAX, f32::MAX).is_none());
    }

    #[test]
    fn sub_base_tolerance_spacing_starts_at_exponent_zero() {
        // 0.1 m spacing is below the ~0.6 m base tolerance, so the first
        // useful exponent clamps to zero and the finest level already
        // merges aggressively.
        let points: Vec<_> = (0..512)
            .map(|i| point_at_meters(f64::from(i) * 0.1, 12))
            .collect();
        let lod = build_track_lod(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("expected a usable level for a dense sub-meter track");
        assert!(
            level.len() < points.len() / 4,
            "expected strong reduction, got {} of {} points",
            level.len(),
            points.len()
        );
    }

    #[test]
    fn coarse_scales_get_coarse_levels() {
        let points = uniform_track();
        let lod = build_track_lod(&points);
        // At a scale where the whole ~10 km track is a few hundred pixels,
        // a sub-pixel-error level must exist and be much smaller than the
        // full point list.
        let px_per_merc = 26_000.0; // whole world at ~26k px: track ≈ 6 px
        let level = lod
            .select(px_per_merc, 0.75)
            .expect("expected a usable level");
        assert!(
            level.len() < points.len() / 4,
            "expected strong reduction, got {} of {} points",
            level.len(),
            points.len()
        );
    }

    #[test]
    fn fine_scales_fall_back_to_full_points() {
        let points = uniform_track();
        let lod = build_track_lod(&points);
        // Zoomed in so far that even the finest level's tolerance is more
        // than a pixel: the renderer must use the full point list.
        let px_per_merc = 2_f64.powi(40);
        assert!(lod.select(px_per_merc, 0.75).is_none());
    }

    #[test]
    fn selected_level_error_stays_sub_pixel() {
        // For every stored level reachable via select, verify the actual
        // screen-space deviation of dropped points from the kept polyline
        // anchor stays below the requested error bound.
        let points = uniform_track();
        let lod = build_track_lod(&points);
        for exp in 10..30 {
            let px_per_merc = 2_f64.powi(exp);
            let Some(level) = lod.select(px_per_merc, 0.75) else {
                continue;
            };
            for w in level.windows(2) {
                let [a, b] = w else { continue };
                // All original points between two kept points must lie
                // within the error bound of the kept anchor `a`.
                let anchor = &points[*a as usize];
                for pi in (*a + 1)..*b {
                    let p = &points[pi as usize];
                    let dx = (p.merc().x - anchor.merc().x) * px_per_merc;
                    let dy = (p.merc().y - anchor.merc().y) * px_per_merc;
                    let err_px = dx.hypot(dy);
                    assert!(
                        err_px <= 0.75,
                        "dropped point {pi} deviates {err_px} px at scale 2^{exp}"
                    );
                }
            }
        }
    }

    #[test]
    fn quality_transitions_survive_every_level() {
        // A marginal-fix stretch in the middle of stacked points: decimation
        // would merge the whole cluster, but the quality transitions must
        // survive so the yellow stretch stays visible at any zoom.
        let mut points: Vec<_> = (0..512).map(|_| point_at_meters(0.0, 12)).collect();
        for p in points.iter_mut().take(260).skip(200) {
            *p = point_at_meters(0.0, 4);
        }
        let lod = build_track_lod(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("coarsest level exists");
        let marginal_kept = level
            .iter()
            .any(|&i| points[i as usize].fix_quality() == FixQuality::Marginal);
        assert!(marginal_kept, "marginal stretch was decimated away");
    }

    #[test]
    fn select_level_index_matches_selected_slice() {
        // The diagnostics index and the rendering slice must always agree -
        // strategy-transition logs would otherwise misreport the level.
        let points = uniform_track();
        let lod = build_track_lod(&points);
        for exp in 0..40 {
            let px_per_merc = 2_f64.powi(exp);
            let by_index = lod
                .select_level(px_per_merc, 0.75)
                .and_then(|i| lod.level(i));
            assert_eq!(by_index, lod.select(px_per_merc, 0.75));
        }
    }

    #[test]
    fn first_and_last_points_survive_every_level() {
        let points = uniform_track();
        let lod = build_track_lod(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("coarsest level exists");
        assert_eq!(level.first(), Some(&0));
        assert_eq!(level.last(), Some(&1023));
    }

    #[test]
    fn stacked_points_collapse_to_endpoints() {
        // A parked recording: hundreds of identical points reduce to the
        // two mandatory endpoints at every level.
        let points: Vec<_> = (0..512).map(|_| point_at_meters(0.0, 12)).collect();
        let lod = build_track_lod(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("coarsest level exists");
        assert_eq!(level, &[0, 511]);
    }
}
