//! Sizing constants for the sky plot, shared by every surface that renders
//! one so the plots read identically everywhere.

use gt_types::SignalQuality;

/// Diameter of the compact sky plot shown in the hover badge.
pub const COMPACT_DIAMETER_PX: f32 = 128.0;

/// Diameter of the full sky plot shown in the sticky popup.
pub const FULL_DIAMETER_PX: f32 = 256.0;

/// Elevations of the faint grid rings drawn between horizon and zenith.
pub const GRID_RING_ELEVATIONS_DEG: [f32; 2] = [30.0, 60.0];

/// Space between the horizon rim and the widget edge, reserving room for the
/// cardinal labels.
pub const FULL_RIM_MARGIN_PX: f32 = 16.0;
/// [`FULL_RIM_MARGIN_PX`] for the compact size.
pub const COMPACT_RIM_MARGIN_PX: f32 = 10.0;

/// Marks in the compact plot shrink by this factor.
pub const COMPACT_MARK_SCALE: f32 = 0.75;

/// Stroke width of the rim, grid rings, and cardinal spokes.
pub const GRID_STROKE_WIDTH_PX: f32 = 1.0;

/// Outline width around a filled (in fix) mark, in the panel color, keeping
/// overlapping marks separable.
pub const MARK_EDGE_STROKE_WIDTH_PX: f32 = 1.0;

/// Stroke width of a hollow (tracked-only) mark.
pub const HOLLOW_MARK_STROKE_WIDTH_PX: f32 = 1.5;

/// Pointer distance within which a mark shows its hover tooltip. Larger than
/// the biggest mark so small marks stay easy to hit.
pub const MARK_HOVER_RADIUS_PX: f32 = 6.0;

/// Alpha applied to marks outside the active highlight subset, so the
/// highlighted marks stand out while the rest stay faintly visible.
pub const DIMMED_MARK_ALPHA: f32 = 0.25;

/// Dash and gap lengths of the elevation-mask ring.
pub const MASK_RING_DASH_PX: f32 = 4.0;
/// See [`MASK_RING_DASH_PX`].
pub const MASK_RING_GAP_PX: f32 = 4.0;
/// Polyline segments approximating the dashed mask ring.
pub const MASK_RING_SEGMENTS: u32 = 90;

/// Font size of the N/E/S/W labels in the full plot.
pub const FULL_CARDINAL_FONT_SIZE: f32 = 10.5;
/// Font size of the north label in the compact plot.
pub const COMPACT_CARDINAL_FONT_SIZE: f32 = 9.0;
/// Font size of the elevation ring labels in the full plot.
pub const ELEVATION_LABEL_FONT_SIZE: f32 = 9.5;

/// Offset of an elevation ring label right of the north spoke.
pub const ELEVATION_LABEL_OFFSET_X_PX: f32 = 4.0;
/// Offset of an elevation ring label above its ring.
pub const ELEVATION_LABEL_OFFSET_Y_PX: f32 = 3.0;

/// Distance from the rim to the center of a cardinal label.
pub const FULL_CARDINAL_LABEL_OFFSET_PX: f32 = 9.0;
/// See [`FULL_CARDINAL_LABEL_OFFSET_PX`].
pub const COMPACT_CARDINAL_LABEL_OFFSET_PX: f32 = 6.0;
/// Length of the rim ticks marking E/S/W in the compact plot.
pub const COMPACT_CARDINAL_TICK_PX: f32 = 3.5;

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
