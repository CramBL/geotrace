use geo::{ConvexHull, Distance, Haversine, InterpolatePoint as _};
use geo_types::{MultiPoint, Point};
use gt_types::NavPoint;
use gt_types::coordinates::{Latitude, Longitude};
use nalgebra::Vector3;
use smallvec::SmallVec;

/// The shorter great-circle arc between two positions, on the sphere
/// [`haversine_m`] measures on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GreatCircleArc {
    pub start: (Latitude, Longitude),
    pub end: (Latitude, Longitude),
}

impl GreatCircleArc {
    /// The position `ratio_from_start` of the way along the arc: 0.0 is
    /// [`GreatCircleArc::start`] and 1.0 is [`GreatCircleArc::end`]. An arc
    /// over the antimeridian stays over it, and one between two identical
    /// positions holds that position at every ratio.
    pub fn position_at_ratio(self, ratio_from_start: f64) -> (Latitude, Longitude) {
        let Self {
            start: (start_lat, start_lon),
            end: (end_lat, end_lon),
        } = self;
        let position = Haversine.point_at_ratio_between(
            Point::new(start_lon.as_degrees(), start_lat.as_degrees()),
            Point::new(end_lon.as_degrees(), end_lat.as_degrees()),
            ratio_from_start,
        );
        (Latitude::new(position.y()), Longitude::new(position.x()))
    }
}

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

/// Haversine length, in metres, of each segment between consecutive points,
/// in recording order. Empty for fewer than 2 points. The primitive behind
/// every along-track distance walk ([`path_distance_km`],
/// [`segment_length_range_m`], threshold crossings).
///
/// Measured over [`NavPoint::resolved_position`], so it walks the polyline
/// the map draws.
pub fn segment_distances_m(points: &[NavPoint]) -> impl Iterator<Item = f64> + '_ {
    points.windows(2).map(|w| match w {
        [a, b] => {
            let ((a_lat, a_lon), (b_lat, b_lon)) = (a.resolved_position(), b.resolved_position());
            haversine_m(a_lat, a_lon, b_lat, b_lon)
        }
        _ => 0.0,
    })
}

/// Sum of haversine distances along an ordered sequence of GPS points,
/// in kilometres. Returns `0.0` for fewer than 2 points.
pub fn path_distance_km(points: &[NavPoint]) -> f64 {
    segment_distances_m(points).sum::<f64>() / 1_000.0
}

/// Minimum and maximum haversine length, in metres, over the segments
/// between consecutive points. `None` for fewer than 2 points (no segments).
pub fn segment_length_range_m(points: &[NavPoint]) -> Option<(f64, f64)> {
    segment_distances_m(points).fold(None, |range: Option<(f64, f64)>, m| {
        Some(range.map_or((m, m), |(min, max)| (min.min(m), max.max(m))))
    })
}

/// Maximum haversine distance between any two [`NavPoint::resolved_position`]
/// in the set, in metres. Returns `0.0` for fewer than 2 points.
///
/// Searches the pairs of the set's convex hull vertices: O(k²) where k is the
/// hull vertex count, acceptable because GPS track hulls are small. The hull
/// is taken in a longitude/latitude grid rotated onto the set's mean
/// direction, so neither the ±180° discontinuity nor a pole falls inside the
/// set. A set spread over more than a hemisphere admits no such rotation and
/// is searched pair by pair instead.
pub fn point_set_diameter_m(points: &[NavPoint]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }

    let directions: Vec<Vector3<f64>> = points
        .iter()
        .map(|p| {
            let (lat, lon) = p.resolved_position();
            earth_centred_direction(lat, lon)
        })
        .collect();

    let Some(frame) = MeanDirectionFrame::covering(&directions) else {
        let positions: Vec<Point<f64>> = points
            .iter()
            .map(|p| {
                let (lat, lon) = p.resolved_position();
                Point::new(lon.as_degrees(), lat.as_degrees())
            })
            .collect();
        return max_pairwise_haversine_m(&positions);
    };

    let rotated: MultiPoint<f64> = directions
        .iter()
        .map(|d| frame.lon_lat_degrees(d))
        .collect();

    // `lines()` yields one segment per edge. Each segment's `.start` is a
    // unique hull vertex (the closing duplicate coord is excluded).
    let hull_vertices: SmallVec<[Point<f64>; 32]> = rotated
        .convex_hull()
        .exterior()
        .lines()
        .map(|line| Point::from(line.start))
        .collect();

    max_pairwise_haversine_m(&hull_vertices)
}

fn max_pairwise_haversine_m(positions: &[Point<f64>]) -> f64 {
    let mut max_m = 0.0_f64;
    for (i, a) in positions.iter().enumerate() {
        for b in positions.iter().skip(i + 1) {
            max_m = max_m.max(Haversine.distance(*a, *b));
        }
    }
    max_m
}

/// Direction from the earth's centre to a position, as a unit vector in the
/// cartesian frame with `x` at 0° N 0° E, `y` at 0° N 90° E and `z` at the
/// north pole.
fn earth_centred_direction(lat: Latitude, lon: Longitude) -> Vector3<f64> {
    let (sin_lat, cos_lat) = lat.as_degrees().to_radians().sin_cos();
    let (sin_lon, cos_lon) = lon.as_degrees().to_radians().sin_cos();
    Vector3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat)
}

/// A rotation of the longitude/latitude grid that puts a point set's mean
/// direction at 0° N 0° E, leaving the set clear of the ±180° discontinuity
/// and of both poles.
struct MeanDirectionFrame {
    origin: Vector3<f64>,
    east: Vector3<f64>,
    north: Vector3<f64>,
}

impl MeanDirectionFrame {
    /// How nearly parallel to the frame's origin the polar axis may be before
    /// the cross product that squares up the frame loses precision.
    const MAX_POLAR_AXIS_ALIGNMENT: f64 = 0.9;

    /// `None` when the directions cancel out, or when one of them is a quarter
    /// turn or more away from their mean: no rotation of the grid is
    /// continuous over a set that wide.
    fn covering(directions: &[Vector3<f64>]) -> Option<Self> {
        let origin = directions
            .iter()
            .sum::<Vector3<f64>>()
            .try_normalize(f64::EPSILON)?;

        if directions.iter().any(|d| d.dot(&origin) <= 0.0) {
            return None;
        }

        let reference_axis = if origin.z.abs() < Self::MAX_POLAR_AXIS_ALIGNMENT {
            Vector3::z()
        } else {
            Vector3::x()
        };
        let east = reference_axis.cross(&origin).try_normalize(f64::EPSILON)?;
        let north = origin.cross(&east);

        Some(Self {
            origin,
            east,
            north,
        })
    }

    fn lon_lat_degrees(&self, direction: &Vector3<f64>) -> Point<f64> {
        let Self {
            origin,
            east,
            north,
        } = self;
        let lon = direction.dot(east).atan2(direction.dot(origin));
        let lat = direction.dot(north).clamp(-1.0, 1.0).asin();
        Point::new(lon.to_degrees(), lat.to_degrees())
    }
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
        NavPoint::new(tpv, None).expect("coordinates in range")
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

    #[test]
    fn great_circle_arc_returns_its_endpoints_at_a_ratio_of_zero_and_one() {
        let arc = GreatCircleArc {
            start: (lat(55.68), lon(12.57)),
            end: (lat(-33.87), lon(151.21)),
        };

        assert_eq!(arc.position_at_ratio(0.0), arc.start);
        assert_eq!(arc.position_at_ratio(1.0), arc.end);
    }

    #[test]
    fn great_circle_arc_at_a_quarter_is_a_quarter_of_its_length_from_the_start() {
        let start = (lat(55.68), lon(12.57));
        let end = (lat(48.86), lon(2.35));
        let (quarter_lat, quarter_lon) = GreatCircleArc { start, end }.position_at_ratio(0.25);

        let arc_m = haversine_m(start.0, start.1, end.0, end.1);
        let walked_m = haversine_m(start.0, start.1, quarter_lat, quarter_lon);
        assert!(
            (walked_m - arc_m / 4.0).abs() < 0.1,
            "expected {} m from the start of a {arc_m} m arc, got {walked_m} m",
            arc_m / 4.0
        );
    }

    #[test]
    fn great_circle_arc_between_two_identical_positions_holds_that_position() {
        let position = (lat(-89.9), lon(120.0));

        let midpoint = GreatCircleArc {
            start: position,
            end: position,
        }
        .position_at_ratio(0.5);

        assert_eq!(midpoint, position);
    }
}
