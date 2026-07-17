use egui::Vec2;

use gt_types::satellites::Satellite;

/// Projects an azimuth/elevation pair onto the unit sky disc.
///
/// Equidistant polar projection: the radius is `(90° - elevation) / 90°`, so
/// the horizon lies on the rim (radius 1) and the zenith at the center. The
/// angle is the azimuth, clockwise from north, with north pointing up
/// (negative y, matching egui's screen coordinates).
///
/// Elevations are clamped to `[0°, 90°]`: a satellite reported slightly below
/// the horizon sits on the rim rather than outside the disc.
pub fn unit_disc_position(azimuth_deg: f32, elevation_deg: f32) -> Vec2 {
    let radius = (90.0 - elevation_deg.clamp(0.0, 90.0)) / 90.0;
    let azimuth = azimuth_deg.to_radians();
    Vec2::new(radius * azimuth.sin(), -radius * azimuth.cos())
}

/// The unit-disc position of a satellite mark, or `None` when the satellite
/// carries no azimuth or no elevation.
///
/// Unplaceable satellites are surfaced as a count/row next to the plot, never
/// dropped silently - the `None` tells the caller to do so.
pub fn mark_position(satellite: &Satellite) -> Option<Vec2> {
    Some(unit_disc_position(
        satellite.azimuth()?,
        satellite.elevation()?,
    ))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use gt_types::satellites::{Constellation, Satellite};

    use super::{mark_position, unit_disc_position};

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
    // Below-horizon elevations clamp to the rim instead of leaving the disc.
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

    #[test]
    fn wraparound_is_continuous_across_north() {
        let just_west = unit_disc_position(359.0, 30.0);
        let just_east = unit_disc_position(1.0, 30.0);
        assert!((just_west - just_east).length() < 0.05);
    }

    fn satellite(elevation: Option<f32>, azimuth: Option<f32>) -> Satellite {
        Satellite::new(Constellation::Gps, 1, elevation, azimuth, Some(40.0), true)
    }

    #[rstest]
    #[case::missing_azimuth(Some(45.0), None)]
    #[case::missing_elevation(None, Some(45.0))]
    #[case::missing_both(None, None)]
    fn satellite_without_sky_position_has_no_mark(
        #[case] elevation: Option<f32>,
        #[case] azimuth: Option<f32>,
    ) {
        assert_eq!(mark_position(&satellite(elevation, azimuth)), None);
    }

    #[test]
    fn satellite_with_sky_position_has_a_mark() {
        let mark = mark_position(&satellite(Some(45.0), Some(90.0)));
        assert_close(mark.unwrap_or_default(), egui::vec2(0.5, 0.0));
    }
}
