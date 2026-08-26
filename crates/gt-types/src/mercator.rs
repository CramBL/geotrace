/// Normalized Web Mercator projection helpers.
///
/// The formula here is identical to walkers' internal `mercator_normalized()`.
/// Pre-computing these coordinates at data-load time lets the per-frame
/// renderer replace the full trigonometric projection with a cheap affine
/// transform (two multiplies + two adds per point).
use crate::coordinates::{Latitude, Longitude};
use std::f64::consts::PI;

/// The latitude Web Mercator ends at, north and south: `asinh(tan(lat))`
/// reaches π there, which [`normalize`] places on the edge of the world. No
/// tile covers anything past it.
pub const MAX_LATITUDE_DEGREES: f64 = 85.051_128_779_806_59;

/// A pre-computed normalized Web Mercator position.
///
/// Both fields are in `[0.0, 1.0]`:
/// - `x` increases west → east (0 = 180° W, 0.5 = 0°, 1 = 180° E)
/// - `y` increases north → south (0 = top, 0.5 = equator, 1 = bottom)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MercPoint {
    pub x: f64,
    pub y: f64,
}

/// Convert a geographic position to a normalized Web Mercator [`MercPoint`].
///
/// The anchor point (lon = 0°, lat = 0°) maps to exactly `(0.5, 0.5)`, which
/// is used by the renderers to turn a single `projector.project(lat_lon(0,0))`
/// call into the per-frame screen → Mercator offset.
pub fn normalize(lat: Latitude, lon: Longitude) -> MercPoint {
    let x = lon.as_degrees().to_radians();
    let y = lat.as_degrees().to_radians().tan().asinh();
    MercPoint {
        x: (1.0 + x / PI) / 2.0,
        y: (1.0 - y / PI) / 2.0,
    }
}

/// Wrap a longitude in degrees into `[-180, 180]`.
///
/// A viewport can reach past the antimeridian, where the projection's own x
/// keeps growing.
pub fn wrap_longitude_degrees(deg: f64) -> f64 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// The inverse of [`normalize`]: a normalized Mercator position back to
/// degrees, with the longitude wrapped into `[-180, 180]`.
///
/// Latitude is not clamped: an x or y outside `[0.0, 1.0]` extrapolates past
/// the projection's limits, which the viewport does when zoomed out far
/// enough to show the poles.
pub fn denormalize(point: MercPoint) -> (f64, f64) {
    let lon = (point.x * 2.0 - 1.0) * PI;
    let lat = ((1.0 - point.y * 2.0) * PI).sinh().atan();
    (lat.to_degrees(), wrap_longitude_degrees(lon.to_degrees()))
}

#[cfg(test)]
mod tests {
    use super::{
        Latitude, Longitude, MAX_LATITUDE_DEGREES, MercPoint, denormalize, normalize,
        wrap_longitude_degrees,
    };

    proptest::proptest! {
        /// `normalize` must never return NaN or Inf for any geographically valid input.
        ///
        /// Web Mercator is undefined above ≈85.05°. Lat is clamped to [-85, 85]
        /// to stay within the finite range of `tan.asinh()`.
        #[test]
        fn normalize_is_finite_for_valid_inputs(
            lon in -180.0_f64..=180.0_f64,
            lat in -85.0_f64..=85.0_f64,
        ) {
            let pt = normalize(Latitude::new(lat), Longitude::new(lon));
            proptest::prop_assert!(pt.x.is_finite(), "x is not finite for lon={lon}, lat={lat}");
            proptest::prop_assert!(pt.y.is_finite(), "y is not finite for lon={lon}, lat={lat}");
        }

        /// x must lie in [0, 1]: west edge (lon = -180) → 0, east edge → 1.
        #[test]
        fn normalize_x_in_unit_range(lon in -180.0_f64..=180.0_f64) {
            let pt = normalize(Latitude::new(0.0), Longitude::new(lon));
            proptest::prop_assert!(pt.x >= 0.0 && pt.x <= 1.0, "x={} out of [0,1] for lon={lon}", pt.x);
        }

        /// y must lie in [0, 1]: north (lat = 85) → near 0, south → near 1.
        #[test]
        fn normalize_y_in_unit_range(lat in -85.0_f64..=85.0_f64) {
            let pt = normalize(Latitude::new(lat), Longitude::new(0.0));
            proptest::prop_assert!(pt.y >= 0.0 && pt.y <= 1.0, "y={} out of [0,1] for lat={lat}", pt.y);
        }
    }

    #[test]
    fn the_projection_limit_maps_to_the_edge_of_the_world() {
        let north = normalize(Latitude::new(MAX_LATITUDE_DEGREES), Longitude::new(0.0));
        let south = normalize(Latitude::new(-MAX_LATITUDE_DEGREES), Longitude::new(0.0));
        assert!(north.y.abs() < 1e-12, "north edge at y={}", north.y);
        assert!((south.y - 1.0).abs() < 1e-12, "south edge at y={}", south.y);
    }

    #[test]
    fn normalize_origin_maps_to_half() {
        let pt = normalize(Latitude::new(0.0), Longitude::new(0.0));
        assert!((pt.x - 0.5).abs() < 1e-12);
        assert!((pt.y - 0.5).abs() < 1e-12);
    }

    #[test]
    fn normalize_x_increases_east() {
        let pt1 = normalize(Latitude::new(0.0), Longitude::new(-90.0));
        let pt2 = normalize(Latitude::new(0.0), Longitude::new(90.0));
        assert!(pt1.x < pt2.x);
    }

    #[test]
    fn normalize_y_increases_south() {
        let pt_north = normalize(Latitude::new(60.0), Longitude::new(0.0));
        let pt_south = normalize(Latitude::new(-60.0), Longitude::new(0.0));
        assert!(pt_north.y < pt_south.y);
    }

    proptest::proptest! {
        /// `denormalize` inverts `normalize` to within floating-point noise.
        #[test]
        fn denormalize_inverts_normalize(
            lat in -85.0_f64..=85.0,
            lon in -180.0_f64..=180.0,
        ) {
            let point = normalize(Latitude::new(lat), Longitude::new(lon));
            let (back_lat, back_lon) = denormalize(point);
            proptest::prop_assert!((back_lat - lat).abs() < 1e-9, "lat {lat} -> {back_lat}");
            proptest::prop_assert!((back_lon - lon).abs() < 1e-9, "lon {lon} -> {back_lon}");
        }
    }

    #[test]
    fn wrap_longitude_degrees_wraps_past_the_antimeridian() {
        assert!((wrap_longitude_degrees(180.3) - -179.7).abs() < 1e-9);
        assert!((wrap_longitude_degrees(-180.3) - 179.7).abs() < 1e-9);
        assert!((wrap_longitude_degrees(12.5) - 12.5).abs() < 1e-9);
    }

    /// A viewport past the antimeridian denormalizes to a real longitude.
    #[test]
    fn denormalize_wraps_past_the_antimeridian() {
        let (_, lon) = denormalize(MercPoint { x: 1.001, y: 0.5 });
        assert!(
            lon < 0.0,
            "past the date line is a western longitude, got {lon}"
        );
        assert!((-180.0..=180.0).contains(&lon));
    }
}
