use gt_types::extent::{DrawnFix, Extent};
use gt_types::placed_point::{PlacedPoint, PlacedPoints};
use gt_types::track::{LOD_BASE_TOLERANCE_MERC, LodChunk, LodLevel, TrackLod};

/// How many consecutive points of a [`LodLevel`], or of a track's full point
/// list, one [`LodChunk`] covers.
pub const LOD_CHUNK_POINTS: usize = 64;

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
/// the previously kept point or its [`gt_types::NavPoint::render_class`] (ghost flag,
/// fix quality) differs, so no styled transition is ever decimated away.
/// The first and last point always survive. Each level is built from the
/// previous one, so the total build cost is a geometric series in the point
/// count, capped by the shrink ratio a stored level must reach.
///
/// The first level's tolerance starts near the track's mean segment length, so
/// every level a sparse recording stores drops points from the full list.
pub fn build_track_lod(points: PlacedPoints<'_>) -> TrackLod {
    let full_point_chunks = chunks(
        &points
            .iter()
            .map(PlacedPoint::drawn_fix)
            .collect::<Vec<_>>(),
    );
    if points.len() < MIN_LEVEL_POINTS || u32::try_from(points.len()).is_err() {
        return TrackLod::new(0, Vec::new(), full_point_chunks);
    }

    let first_level_exp = first_useful_exponent(points);
    let mut levels: Vec<LodLevel> = Vec::new();
    let mut tolerance = LOD_BASE_TOLERANCE_MERC * 2_f64.powi(exponent_to_i32(first_level_exp));

    while levels.len() < MAX_LEVELS {
        let level = match levels.last() {
            None => decimate(points, 0..points.len(), tolerance),
            Some(prev) => decimate(
                points,
                prev.indices()
                    .iter()
                    .filter_map(|&i| usize::try_from(i).ok()),
                tolerance,
            ),
        };
        let prev_len = levels
            .last()
            .map_or(points.len(), |level| level.indices().len());
        if !levels.is_empty() && level.len() * MAX_KEPT_DENOMINATOR > prev_len * MAX_KEPT_NUMERATOR
        {
            // Tolerance doubled but barely anything merged (e.g. dense
            // key transitions everywhere): coarser levels won't help.
            break;
        }
        let done = level.len() < MIN_LEVEL_POINTS;
        let chunks = level_chunks(points, &level);
        levels.push(LodLevel::new(level, chunks));
        if done {
            break;
        }
        tolerance *= 2.0;
    }

    TrackLod::new(first_level_exp, levels, full_point_chunks)
}

/// The chunks of `level`, over the positions its points are drawn at and the
/// times the receiver stamped them. Empty when an entry of `level` addresses
/// no point of `points`, which [`decimate`] never emits: a renderer reading no
/// chunk walks the level whole.
fn level_chunks(points: PlacedPoints<'_>, level: &[u32]) -> Vec<LodChunk> {
    let drawn: Option<Vec<DrawnFix>> = level
        .iter()
        .map(|&i| Some(points.get(usize::try_from(i).ok()?)?.drawn_fix()))
        .collect();
    drawn.as_deref().map(chunks).unwrap_or_default()
}

/// One [`LodChunk`] per run of [`LOD_CHUNK_POINTS`] consecutive `drawn`
/// fixes, in the order they are drawn. A run either side of the antimeridian
/// gets a box across it, since [`gt_types::GeoBounds`] grows a longitude range
/// over the shorter of the two arcs.
///
/// Empty when a run's slots reach past [`u32::MAX`], which makes a renderer
/// walk the sequence whole.
fn chunks(drawn: &[DrawnFix]) -> Vec<LodChunk> {
    let chunks: Option<Vec<LodChunk>> = drawn
        .chunks(LOD_CHUNK_POINTS)
        .enumerate()
        .map(|(i, run)| {
            let (first, rest) = run.split_first()?;
            let start = u32::try_from(i.checked_mul(LOD_CHUNK_POINTS)?).ok()?;
            let end = start.checked_add(u32::try_from(run.len()).ok()?)?;
            Some(LodChunk::new(
                start..end,
                Extent::spanning(*first, rest.iter().copied()),
            ))
        })
        .collect();
    chunks.unwrap_or_default()
}

/// The tolerance exponent at which decimation starts paying off: the level
/// whose tolerance is roughly the track's mean Mercator segment length.
/// Finer levels would keep nearly every point.
fn first_useful_exponent(points: PlacedPoints<'_>) -> u32 {
    let segments = points.len().saturating_sub(1).max(1);
    let mercs: Vec<gt_types::MercPoint> = points.iter().map(|point| point.merc()).collect();
    let total: f64 = mercs
        .windows(2)
        .map(|w| match w {
            [a, b] => (b.x - a.x).hypot(b.y - a.y),
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
    points: PlacedPoints<'_>,
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
        let class = point.fix.render_class();
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
    use chrono::{DateTime, TimeDelta, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::nav_point::NavPoint;
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use gt_types::{FixQuality, MercBounds, TrackLod};
    use uom::si::angle::degree;
    use uom::si::f64::Angle;
    use vec1::Vec1;

    /// ~1 m of longitude at the equator, in degrees.
    const DEG_PER_METER: f64 = 360.0 / 40_030_173.0;

    /// The instant the first fix of every fixture track is stamped at.
    const FIRST_FIX_TIME: DateTime<Utc> = DateTime::<Utc>::UNIX_EPOCH;

    /// Every fixture fix records a position, so it is drawn where it was
    /// recorded.
    fn merc_at(point: &NavPoint) -> gt_types::MercPoint {
        let (latitude, longitude) = point.tpv.position().expect("a recorded position");
        gt_types::mercator::normalize(latitude, longitude)
    }

    /// The LOD of `points` taken as a track of their own.
    fn lod_of(points: &[NavPoint]) -> TrackLod {
        let geometry = crate::segment::measure_track_geometry(
            points,
            crate::segment::FixPlacementRule::default(),
        );
        let placed = geometry
            .measured()
            .and_then(|measured| PlacedPoints::new(points, &measured.resolved_positions))
            .expect("every fixture fix has a recorded position");
        build_track_lod(placed)
    }

    fn point_at_meters(x_m: f64, fix_count: u32, time: DateTime<Utc>) -> NavPoint {
        point_at_longitude(Longitude::new(x_m * DEG_PER_METER), fix_count, time)
    }

    fn point_at_longitude(longitude: Longitude, fix_count: u32, time: DateTime<Utc>) -> NavPoint {
        let sats: Vec<_> = (1..=fix_count.max(1))
            .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, prn <= fix_count))
            .collect();
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(time))
            .lat(Latitude::new(0.0))
            .lon(longitude)
            .heading(Angle::new::<degree>(90.0))
            .build();
        NavPoint::new(tpv, Some(Satellites::new(None, None, sats)))
    }

    /// One second per fix from [`FIRST_FIX_TIME`].
    fn fix_time(i: i32) -> DateTime<Utc> {
        FIRST_FIX_TIME + TimeDelta::seconds(i.into())
    }

    /// 1024 strong-fix points spaced 10 m apart.
    fn uniform_track() -> Vec<NavPoint> {
        (0..1024)
            .map(|i| point_at_meters(f64::from(i) * 10.0, 12, fix_time(i)))
            .collect()
    }

    /// 512 strong-fix points running east from 179°E over the antimeridian,
    /// 0.01° apart.
    fn track_across_the_antimeridian() -> Vec<NavPoint> {
        (0..512)
            .map(|i| {
                let degrees = 179.0 + f64::from(i) * 0.01;
                let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
                point_at_longitude(Longitude::new(wrapped), 12, fix_time(i))
            })
            .collect()
    }

    /// Neither end of the second chunk is the earliest or the latest fix of
    /// that chunk. The 100th fix of the uniform track is stamped an hour
    /// before the fix that opens the track.
    fn track_with_a_backward_time_step() -> Vec<NavPoint> {
        let mut points = uniform_track();
        points[100] = point_at_meters(1000.0, 12, FIRST_FIX_TIME - TimeDelta::hours(1));
        points
    }

    /// Whether `bounds` holds `merc`, reading a box across the antimeridian
    /// as its two pieces the way [`gt_types::MercBounds::intersects`] does.
    fn holds(bounds: gt_types::MercBounds, merc: gt_types::MercPoint) -> bool {
        bounds.intersects(gt_types::MercBounds {
            x_min: merc.x,
            x_max: merc.x,
            y_min: merc.y,
            y_max: merc.y,
        })
    }

    #[test]
    fn tiny_tracks_get_no_lod() {
        let points: Vec<_> = (0..10)
            .map(|i| point_at_meters(f64::from(i), 12, fix_time(i)))
            .collect();
        let lod = lod_of(&points);
        assert!(lod.select(f64::MAX, f32::MAX).is_none());
    }

    #[test]
    fn sub_base_tolerance_spacing_starts_at_exponent_zero() {
        // 0.1 m spacing is below the ~0.6 m base tolerance, so the first
        // useful exponent clamps to zero and the finest level already
        // merges aggressively.
        let points: Vec<_> = (0..512)
            .map(|i| point_at_meters(f64::from(i) * 0.1, 12, fix_time(i)))
            .collect();
        let lod = lod_of(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("expected a usable level for a dense sub-meter track");
        assert!(
            level.indices().len() < points.len() / 4,
            "expected strong reduction, got {} of {} points",
            level.indices().len(),
            points.len()
        );
    }

    #[test]
    fn coarse_scales_get_coarse_levels() {
        let points = uniform_track();
        let lod = lod_of(&points);
        // At a scale where the whole ~10 km track is a few hundred pixels,
        // a sub-pixel-error level must exist and be much smaller than the
        // full point list.
        let px_per_merc = 26_000.0; // whole world at ~26k px: track ≈ 6 px
        let level = lod
            .select(px_per_merc, 0.75)
            .expect("expected a usable level");
        assert!(
            level.indices().len() < points.len() / 4,
            "expected strong reduction, got {} of {} points",
            level.indices().len(),
            points.len()
        );
    }

    #[test]
    fn fine_scales_fall_back_to_full_points() {
        let points = uniform_track();
        let lod = lod_of(&points);
        // Zoomed in so far that even the finest level's tolerance is more
        // than a pixel: the renderer must use the full point list.
        let px_per_merc = 2_f64.powi(40);
        assert!(lod.select(px_per_merc, 0.75).is_none());
    }

    #[test]
    fn selected_level_error_stays_sub_pixel() {
        // For every stored level reachable via select, verify the actual
        // screen-space deviation of dropped points from the kept polyline
        // anchor stays below the 0.75 px bound passed to `select`.
        let points = uniform_track();
        let lod = lod_of(&points);
        for exp in 10..30 {
            let px_per_merc = 2_f64.powi(exp);
            let Some(level) = lod.select(px_per_merc, 0.75) else {
                continue;
            };
            for w in level.indices().windows(2) {
                let [a, b] = w else { continue };
                // All original points between two kept points must lie
                // within the error bound of the kept anchor `a`.
                let anchor = &points[*a as usize];
                for pi in (*a + 1)..*b {
                    let p = &points[pi as usize];
                    let dx = (merc_at(p).x - merc_at(anchor).x) * px_per_merc;
                    let dy = (merc_at(p).y - merc_at(anchor).y) * px_per_merc;
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
        let mut points: Vec<_> = (0..512)
            .map(|i| point_at_meters(0.0, 12, fix_time(i)))
            .collect();
        for (i, p) in points.iter_mut().enumerate().take(260).skip(200) {
            *p = point_at_meters(0.0, 4, fix_time(i.try_into().unwrap_or(0)));
        }
        let lod = lod_of(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("coarsest level exists");
        let marginal_kept = level
            .indices()
            .iter()
            .any(|&i| points[i as usize].fix_quality() == FixQuality::Marginal);
        assert!(marginal_kept, "marginal stretch was decimated away");
    }

    #[test]
    fn select_level_index_matches_selected_slice() {
        // The diagnostics index and the rendering slice must always agree -
        // strategy-transition logs would otherwise misreport the level.
        let points = uniform_track();
        let lod = lod_of(&points);
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
        let lod = lod_of(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("coarsest level exists");
        assert_eq!(level.indices().first(), Some(&0));
        assert_eq!(level.indices().last(), Some(&1023));
    }

    #[test]
    fn stacked_points_collapse_to_endpoints() {
        // A parked recording: hundreds of identical points reduce to the
        // two mandatory endpoints at every level.
        let points: Vec<_> = (0..512)
            .map(|i| point_at_meters(0.0, 12, fix_time(i)))
            .collect();
        let lod = lod_of(&points);
        let level = lod
            .select(f64::MIN_POSITIVE, 0.75)
            .expect("coarsest level exists");
        assert_eq!(level.indices(), &[0, 511]);
    }

    /// Every chunked sequence the LOD stores: the track's full point list
    /// first, then each stored level, each with the indices its chunks cover.
    fn chunked_sequences<'a>(
        lod: &'a TrackLod,
        full_indices: &'a [u32],
    ) -> Vec<(&'a [u32], &'a [LodChunk])> {
        let mut sequences = vec![(full_indices, lod.full_point_chunks())];
        for i in 0.. {
            let Some(level) = lod.level(i) else { break };
            sequences.push((level.indices(), level.chunks()));
        }
        sequences
    }

    #[rstest::rstest]
    #[case::uniform(uniform_track())]
    #[case::across_the_antimeridian(track_across_the_antimeridian())]
    #[case::with_a_backward_time_step(track_with_a_backward_time_step())]
    fn every_chunk_holds_the_positions_and_the_times_of_the_points_it_covers(
        #[case] points: Vec<NavPoint>,
    ) {
        let lod = lod_of(&points);
        let full_indices: Vec<u32> = (0..points.len())
            .filter_map(|i| u32::try_from(i).ok())
            .collect();

        for (indices, chunks) in chunked_sequences(&lod, &full_indices) {
            assert_eq!(chunks.len(), indices.len().div_ceil(LOD_CHUNK_POINTS));
            for chunk in chunks {
                let covered = indices
                    .get(chunk.slots())
                    .expect("a chunk covers stored slots");
                for point in covered.iter().filter_map(|&i| points.get(i as usize)) {
                    let extent = chunk.extent();
                    let merc = merc_at(point);
                    assert!(
                        holds(extent.merc(), merc),
                        "{:?} misses {merc:?}",
                        extent.merc()
                    );
                    let time = point.tpv.time().utc();
                    assert!(
                        extent.time().contains(time),
                        "{:?} misses {time}",
                        extent.time()
                    );
                }
            }
        }
    }

    /// The union of the full point list's chunks is what the track measures.
    /// The chunk extents and the track's own measures are two folds over the
    /// same fixes, and the fixture tracks encircle no pole.
    #[rstest::rstest]
    #[case::uniform(uniform_track())]
    #[case::across_the_antimeridian(track_across_the_antimeridian())]
    #[case::with_a_backward_time_step(track_with_a_backward_time_step())]
    fn the_full_point_chunks_union_to_the_track_bounds_and_time_range(
        #[case] points: Vec<NavPoint>,
    ) {
        let lod = lod_of(&points);
        let geometry = crate::segment::measure_track_geometry(
            &points,
            crate::segment::FixPlacementRule::default(),
        );
        let measured = geometry.measured().expect("every fixture fix is placed");
        let metadata = crate::segment::compute_track_metadata(
            1,
            &Vec1::try_from_vec(points).expect("a non-empty fixture"),
            &[],
            &[],
        );

        let union = lod
            .full_point_chunks()
            .iter()
            .map(LodChunk::extent)
            .reduce(Extent::union)
            .expect("a chunk per 64 fixes");

        assert_eq!(union.time(), metadata.time_range);
        assert_merc_bounds_close(union.merc(), measured.merc_bounds);
    }

    /// Asserts two boxes agree to within a millimetre at the equator. Two
    /// orders of the same fold round differently in the last few bits.
    fn assert_merc_bounds_close(left: MercBounds, right: MercBounds) {
        const TOLERANCE_MERC: f64 = 1e-11;
        let edges = [
            (left.x_min, right.x_min),
            (left.x_max, right.x_max),
            (left.y_min, right.y_min),
            (left.y_max, right.y_max),
        ];
        for (a, b) in edges {
            assert!(
                (a - b).abs() <= TOLERANCE_MERC,
                "{left:?} against {right:?}"
            );
        }
    }
}
