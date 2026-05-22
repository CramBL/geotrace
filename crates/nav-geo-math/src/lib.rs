use geo::{ConvexHull, Distance, Haversine};
use geo_types::{Coord, MultiPoint, Point};
use smallvec::SmallVec;

/// Great-circle distance between two (lat, lon) points in decimal degrees,
/// returned in kilometres.
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    haversine_m(lat1, lon1, lat2, lon2) / 1_000.0
}

/// Great-circle distance between two (lat, lon) points in decimal degrees,
/// returned in metres.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    Haversine.distance(Point::new(lon1, lat1), Point::new(lon2, lat2))
}

/// Sum of haversine distances along an ordered sequence of `(lat, lon)` pairs,
/// in kilometres. Returns `0.0` for fewer than 2 points.
pub fn path_distance_km(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|w| match w {
            [(lat1, lon1), (lat2, lon2)] => haversine_km(*lat1, *lon1, *lat2, *lon2),
            _ => 0.0,
        })
        .sum()
}

/// Maximum haversine distance between any two points in the set, in metres.
/// Returns `0.0` for fewer than 2 points.
///
/// Reduces the search space to the convex hull of the point set, then
/// iterates all pairs of hull vertices (O(k²) where k = hull vertex count;
/// acceptable because GPS track hulls are small).
pub fn point_set_diameter_m(points: &[(f64, f64)]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }

    let multi: MultiPoint<f64> = points
        .iter()
        .map(|&(lat, lon)| Point::new(lon, lat))
        .collect();

    let hull = multi.convex_hull();

    // `lines()` yields one segment per edge; each segment's `.start` is a
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

    const TOLERANCE_KM: f64 = 0.5;
    const TOLERANCE_M: f64 = 500.0;

    #[test]
    fn haversine_m_zero_for_same_point() {
        let d = haversine_m(51.5, -0.1, 51.5, -0.1);
        assert!(d < f64::EPSILON, "expected 0.0, got {d}");
    }

    #[test]
    fn haversine_km_one_degree_latitude_at_equator() {
        // 1 degree of latitude ≈ 111.195 km at the equator.
        let d = haversine_km(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111.195).abs() < TOLERANCE_KM, "got {d} km");
    }

    #[test]
    fn haversine_m_is_km_times_1000() {
        let km = haversine_km(55.0, 12.0, 56.0, 12.0);
        let m = haversine_m(55.0, 12.0, 56.0, 12.0);
        assert!((m - km * 1_000.0).abs() < 0.01, "km={km}, m={m}");
    }

    #[test]
    fn path_distance_km_empty() {
        let d = path_distance_km(&[]);
        assert!(d < f64::EPSILON, "expected 0.0, got {d}");
    }

    #[test]
    fn path_distance_km_single_point() {
        let d = path_distance_km(&[(55.0, 12.0)]);
        assert!(d < f64::EPSILON, "expected 0.0, got {d}");
    }

    #[test]
    fn path_distance_km_two_points() {
        let d = path_distance_km(&[(0.0, 0.0), (1.0, 0.0)]);
        assert!((d - 111.195).abs() < TOLERANCE_KM, "got {d} km");
    }

    #[test]
    fn path_distance_km_three_points_sums_segments() {
        let leg = haversine_km(0.0, 0.0, 1.0, 0.0);
        let total = path_distance_km(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]);
        assert!((total - 2.0 * leg).abs() < TOLERANCE_KM, "got {total} km");
    }

    #[test]
    fn diameter_fewer_than_two_points() {
        assert!(point_set_diameter_m(&[]) < f64::EPSILON);
        assert!(point_set_diameter_m(&[(55.0, 12.0)]) < f64::EPSILON);
    }

    #[test]
    fn diameter_identical_points_is_zero() {
        let d = point_set_diameter_m(&[(55.0, 12.0), (55.0, 12.0), (55.0, 12.0)]);
        assert!(d < 1.0, "expected ~0.0, got {d}");
    }

    #[test]
    fn diameter_two_points_matches_haversine() {
        let expected = haversine_m(0.0, 0.0, 1.0, 0.0);
        let d = point_set_diameter_m(&[(0.0, 0.0), (1.0, 0.0)]);
        assert!(
            (d - expected).abs() < TOLERANCE_M,
            "expected {expected}, got {d}"
        );
    }

    #[test]
    fn diameter_collinear_points_is_endpoint_distance() {
        let expected = haversine_m(0.0, 0.0, 1.0, 0.0);
        let d = point_set_diameter_m(&[(0.0, 0.0), (0.5, 0.0), (1.0, 0.0)]);
        assert!(
            (d - expected).abs() < TOLERANCE_M,
            "expected {expected}, got {d}"
        );
    }
}
