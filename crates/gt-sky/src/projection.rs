use egui::Vec2;

use gt_types::satellites::Satellite;

/// Projects an azimuth/elevation pair onto the unit sky disc.
///
/// Equidistant polar projection: the radius is `(90° - elevation) / 90°`, so
/// the horizon lies on the rim (radius 1) and the zenith at the center. The
/// angle is the azimuth, clockwise from north, with north pointing up
/// (negative y, matching egui's screen coordinates).
///
/// Elevations are clamped to `[0°, 90°]`, so a below-horizon satellite sits on
/// the rim.
pub fn unit_disc_position(azimuth_deg: f32, elevation_deg: f32) -> Vec2 {
    let radius = unit_disc_radius(elevation_deg);
    let azimuth = azimuth_deg.to_radians();
    Vec2::new(radius * azimuth.sin(), -radius * azimuth.cos())
}

/// The unit-disc radius of an elevation: 1 at the horizon, 0 at the zenith,
/// clamped outside that range. The single definition of the equidistant
/// mapping, shared by satellite marks, grid rings, and the mask ring.
pub fn unit_disc_radius(elevation_deg: f32) -> f32 {
    (90.0 - elevation_deg.clamp(0.0, 90.0)) / 90.0
}

/// A satellite's sky position as `(azimuth, elevation)` in degrees, or `None`
/// when it carries no azimuth or no elevation. The single definition of
/// "placeable" on the sky, shared by the marks and the trails.
pub fn sky_position(satellite: &Satellite) -> Option<(f32, f32)> {
    Some((satellite.azimuth()?, satellite.elevation()?))
}

/// The unit-disc position of a satellite mark, or `None` when the satellite
/// has no sky position.
///
/// Callers surface unplaceable satellites as a count/row next to the plot,
/// never dropping them silently.
pub fn mark_position(satellite: &Satellite) -> Option<Vec2> {
    let (azimuth, elevation) = sky_position(satellite)?;
    Some(unit_disc_position(azimuth, elevation))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use gt_types::satellites::Constellation;

    use super::{mark_position, unit_disc_position};
    use crate::test_util::{self, Azimuth, Elevation};

    const EPSILON: f32 = 1e-5;

    fn assert_close(actual: egui::Vec2, expected: egui::Vec2) {
        assert!(
            (actual - expected).length() < EPSILON,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[rstest]
    // The four cardinal directions on the horizon: north up, east right.
    #[case::north_horizon(0.0, 0.0, egui::vec2(0.0, -1.0))]
    #[case::east_horizon(90.0, 0.0, egui::vec2(1.0, 0.0))]
    #[case::south_horizon(180.0, 0.0, egui::vec2(0.0, 1.0))]
    #[case::west_horizon(270.0, 0.0, egui::vec2(-1.0, 0.0))]
    // The zenith projects to the center regardless of azimuth.
    #[case::zenith(0.0, 90.0, egui::vec2(0.0, 0.0))]
    #[case::zenith_any_azimuth(123.0, 90.0, egui::vec2(0.0, 0.0))]
    // Equidistant: 45° elevation lands halfway between center and rim.
    #[case::mid_elevation(0.0, 45.0, egui::vec2(0.0, -0.5))]
    // A full turn wraps around to the same position.
    #[case::wraparound(360.0, 0.0, egui::vec2(0.0, -1.0))]
    // Below-horizon elevations clamp to the rim.
    #[case::below_horizon(90.0, -5.0, egui::vec2(1.0, 0.0))]
    // Above-zenith elevations clamp to the center.
    #[case::above_zenith(90.0, 95.0, egui::vec2(0.0, 0.0))]
    fn projects_to_expected_position(
        #[case] azimuth_deg: f32,
        #[case] elevation_deg: f32,
        #[case] expected: egui::Vec2,
    ) {
        assert_close(unit_disc_position(azimuth_deg, elevation_deg), expected);
    }

    #[rstest]
    #[case::missing_azimuth(None, Some(Elevation(45.0)))]
    #[case::missing_elevation(Some(Azimuth(45.0)), None)]
    #[case::missing_both(None, None)]
    fn satellite_without_sky_position_has_no_mark(
        #[case] azimuth: Option<Azimuth>,
        #[case] elevation: Option<Elevation>,
    ) {
        let satellite = test_util::sat(Constellation::Gps, 1, azimuth, elevation, true);

        assert_eq!(mark_position(&satellite), None);
    }

    #[test]
    fn satellite_with_sky_position_has_a_mark() {
        let satellite = test_util::sat(
            Constellation::Gps,
            1,
            Some(Azimuth(90.0)),
            Some(Elevation(45.0)),
            true,
        );

        assert_close(
            mark_position(&satellite).unwrap_or_default(),
            egui::vec2(0.5, 0.0),
        );
    }
}
