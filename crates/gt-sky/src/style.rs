//! Sizing constants for the sky plot, shared by every surface that renders
//! one so the plots read identically everywhere.

use gt_types::SignalQuality;

/// Diameter of the compact sky plot shown in the hover badge.
pub const COMPACT_DIAMETER_PX: f32 = 128.0;

/// Diameter of the full sky plot shown in the sticky popup.
pub const FULL_DIAMETER_PX: f32 = 256.0;

/// Elevations of the faint grid rings drawn between horizon and zenith.
pub const GRID_RING_ELEVATIONS_DEG: [f32; 2] = [30.0, 60.0];

/// Mark radius at full size for a satellite's signal quality, so weak
/// satellites read as small at a glance. `None` (no reported SNR) gets the
/// smallest radius rather than a made-up middle tier.
pub const fn mark_radius(quality: Option<SignalQuality>) -> f32 {
    match quality {
        Some(SignalQuality::Excellent) => 4.5,
        Some(SignalQuality::Good) => 4.0,
        Some(SignalQuality::Moderate) => 3.5,
        Some(SignalQuality::Weak) => 3.0,
        Some(SignalQuality::VeryWeak) | None => 2.5,
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use gt_types::SignalQuality;

    use super::mark_radius;

    /// Better quality never renders smaller - the size encoding stays
    /// monotonic even if the radii are retuned.
    #[test]
    fn mark_radius_is_monotonic_in_quality() {
        let radii: Vec<f32> = SignalQuality::iter()
            .map(|quality| mark_radius(Some(quality)))
            .collect();
        for pair in radii.windows(2) {
            let [better, worse] = pair else {
                continue;
            };
            assert!(better > worse, "radii not monotonic: {radii:?}");
        }
    }

    #[test]
    fn missing_snr_gets_the_smallest_radius() {
        let smallest = SignalQuality::iter()
            .map(|quality| mark_radius(Some(quality)))
            .fold(f32::INFINITY, f32::min);
        assert!(mark_radius(None) <= smallest);
    }
}
