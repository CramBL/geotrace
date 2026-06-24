use geo::{ConvexHull, Distance, Haversine};
use geo_types::{Coord, MultiPoint, Point};
use gt_types::NavPoint;
use gt_types::coordinates::{Latitude, Longitude};
use smallvec::SmallVec;

/// Great-circle distance between two positions, returned in kilometres.
pub fn haversine_km(lat1: Latitude, lon1: Longitude, lat2: Latitude, lon2: Longitude) -> f64 {
    haversine_m(lat1, lon1, lat2, lon2) / 1_000.0
}

/// Great-circle distance between two positions, returned in metres.
pub fn haversine_m(lat1: Latitude, lon1: Longitude, lat2: Latitude, lon2: Longitude) -> f64 {
    Haversine.distance(
        Point::new(lon1.as_degrees(), lat1.as_degrees()),
        Point::new(lon2.as_degrees(), lat2.as_degrees()),
    )
}

/// Sum of haversine distances along an ordered sequence of GPS points,
/// in kilometres. Returns `0.0` for fewer than 2 points.
pub fn path_distance_km(points: &[NavPoint]) -> f64 {
    points
        .windows(2)
        .map(|w| match w {
            [a, b] => haversine_km(a.tpv.lat(), a.tpv.lon(), b.tpv.lat(), b.tpv.lon()),
            _ => 0.0,
        })
        .sum()
}

/// Minimum and maximum haversine length, in metres, over the segments
/// between consecutive points. `None` for fewer than 2 points (no segments).
pub fn segment_length_range_m(points: &[NavPoint]) -> Option<(f64, f64)> {
    let mut range: Option<(f64, f64)> = None;
    for w in points.windows(2) {
        let [a, b] = w else { continue };
        let m = haversine_m(a.tpv.lat(), a.tpv.lon(), b.tpv.lat(), b.tpv.lon());
        range = Some(range.map_or((m, m), |(min, max)| (min.min(m), max.max(m))));
    }
    range
}

/// Maximum haversine distance between any two points in the set, in metres.
/// Returns `0.0` for fewer than 2 points.
///
/// Reduces the search space to the convex hull of the point set, then
/// iterates all pairs of hull vertices (O(k²) where k = hull vertex count;
/// acceptable because GPS track hulls are small).
pub fn point_set_diameter_m(points: &[NavPoint]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }

    let multi: MultiPoint<f64> = points
        .iter()
        .map(|p| Point::new(p.tpv.lon().as_degrees(), p.tpv.lat().as_degrees()))
        .collect();

    let hull = multi.convex_hull();

    // `lines()` yields one segment per edge. Each segment's `.start` is a
    // unique hull vertex (the closing duplicate coord is excluded).
    let hull_verts: SmallVec<[Coord<f64>; 32]> = hull.exterior().lines().map(|l| l.start).collect();

    let mut max_m = 0.0_f64;
    for (i, v1) in hull_verts.iter().enumerate() {
        for v2 in hull_verts.iter().skip(i + 1) {
            let d = Haversine.distance(Point::from(*v1), Point::from(*v2));
            if d > max_m {
                max_m = d;
            }
        }
    }
    max_m
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gt_types::NavPoint;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::time_types::GpsTime;

    const TOLERANCE_KM: f64 = 0.5;
    const TOLERANCE_M: f64 = 500.0;

    fn lat(v: f64) -> Latitude {
        Latitude::new(v)
    }

    fn lon(v: f64) -> Longitude {
        Longitude::new(v)
    }

    fn make_point(lat_deg: f64, lon_deg: f64) -> NavPoint {
        let tpv = gt_types::TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(lat(lat_deg))
            .lon(lon(lon_deg))
            .build();
        NavPoint::new(tpv, None)
    }

    #[test]
    fn haversine_m_zero_for_same_point() {
        let d = haversine_m(lat(51.5), lon(-0.1), lat(51.5), lon(-0.1));
        assert!(d < f64::EPSILON, "expected 0.0, got {d}");
    }

    #[test]
    fn haversine_km_one_degree_latitude_at_equator() {
        // 1 degree of latitude ≈ 111.195 km at the equator.
        let d = haversine_km(lat(0.0), lon(0.0), lat(1.0), lon(0.0));
        assert!((d - 111.195).abs() < TOLERANCE_KM, "got {d} km");
    }

    #[test]
    fn haversine_m_is_km_times_1000() {
        let km = haversine_km(lat(55.0), lon(12.0), lat(56.0), lon(12.0));
        let m = haversine_m(lat(55.0), lon(12.0), lat(56.0), lon(12.0));
        assert!((m - km * 1_000.0).abs() < 0.01, "km={km}, m={m}");
    }

    #[test]
    fn segment_length_range_none_without_segments() {
        assert_eq!(segment_length_range_m(&[]), None);
        assert_eq!(segment_length_range_m(&[make_point(55.0, 12.0)]), None);
    }

    #[test]
    fn segment_length_range_single_segment_has_equal_min_and_max() {
        let pts = [make_point(0.0, 0.0), make_point(0.0, 1.0)];
        let Some((min, max)) = segment_length_range_m(&pts) else {
            panic!("expected a range for 2 points");
        };
        assert!((min - max).abs() < f64::EPSILON, "min={min}, max={max}");
        // 1 degree of longitude ≈ 111.195 km at the equator.
        assert!((max - 111_195.0).abs() < TOLERANCE_M, "got {max} m");
    }

    #[test]
    fn segment_length_range_spans_shortest_and_longest_segment() {
        // Stationary pair (zero-length segment), then a ~111 km hop:
        // exactly the parked-then-highway shape the range must capture.
        let pts = [
            make_point(0.0, 0.0),
            make_point(0.0, 0.0),
            make_point(0.0, 1.0),
        ];
        let Some((min, max)) = segment_length_range_m(&pts) else {
            panic!("expected a range for 3 points");
        };
        assert!(min < f64::EPSILON, "expected 0.0 min, got {min}");
        assert!((max - 111_195.0).abs() < TOLERANCE_M, "got {max} m");
    }

    #[test]
    fn path_distance_km_empty() {
        let d = path_distance_km(&[]);
        assert!(d < f64::EPSILON, "expected 0.0, got {d}");
    }

    #[test]
    fn path_distance_km_single_point() {
        let d = path_distance_km(&[make_point(55.0, 12.0)]);
        assert!(d < f64::EPSILON, "expected 0.0, got {d}");
    }

    #[test]
    fn path_distance_km_two_points() {
        let d = path_distance_km(&[make_point(0.0, 0.0), make_point(1.0, 0.0)]);
        assert!((d - 111.195).abs() < TOLERANCE_KM, "got {d} km");
    }

    #[test]
    fn path_distance_km_three_points_sums_segments() {
        let leg = haversine_km(lat(0.0), lon(0.0), lat(1.0), lon(0.0));
        let total = path_distance_km(&[
            make_point(0.0, 0.0),
            make_point(1.0, 0.0),
            make_point(2.0, 0.0),
        ]);
        assert!((total - 2.0 * leg).abs() < TOLERANCE_KM, "got {total} km");
    }

    #[test]
    fn diameter_fewer_than_two_points() {
        assert!(point_set_diameter_m(&[]) < f64::EPSILON);
        assert!(point_set_diameter_m(&[make_point(55.0, 12.0)]) < f64::EPSILON);
    }

    #[test]
    fn diameter_identical_points_is_zero() {
        let d = point_set_diameter_m(&[
            make_point(55.0, 12.0),
            make_point(55.0, 12.0),
            make_point(55.0, 12.0),
        ]);
        assert!(d < 1.0, "expected ~0.0, got {d}");
    }

    #[test]
    fn diameter_two_points_matches_haversine() {
        let expected = haversine_m(lat(0.0), lon(0.0), lat(1.0), lon(0.0));
        let d = point_set_diameter_m(&[make_point(0.0, 0.0), make_point(1.0, 0.0)]);
        assert!(
            (d - expected).abs() < TOLERANCE_M,
            "expected {expected}, got {d}"
        );
    }

    #[test]
    fn diameter_collinear_points_is_endpoint_distance() {
        let expected = haversine_m(lat(0.0), lon(0.0), lat(1.0), lon(0.0));
        let d = point_set_diameter_m(&[
            make_point(0.0, 0.0),
            make_point(0.5, 0.0),
            make_point(1.0, 0.0),
        ]);
        assert!(
            (d - expected).abs() < TOLERANCE_M,
            "expected {expected}, got {d}"
        );
    }
}
