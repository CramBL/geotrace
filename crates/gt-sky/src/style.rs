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

/// Stroke width of a satellite trail in the whole-track plot.
pub const TRAIL_WIDTH_PX: f32 = 2.0;

/// A trail draws as a comet's tail: [`TRAIL_MAX_ALPHA`] at the satellite's
/// current position, fading back over [`TRAIL_TAIL_SECS`] of *past* travel to
/// [`TRAIL_MIN_ALPHA`], where the rest of the path stays as a faint ghost. The
/// path ahead of the satellite - where it has not been yet - is held at the
/// floor, so the bright end always points the way it is moving and the
/// direction of travel reads at a glance without arrows or other clutter.
/// The floor is the ghost's strength: high enough that the whole path stays
/// readable on a long recording, low enough that the head still reads as much
/// brighter - roughly a sevenfold ratio.
pub const TRAIL_MIN_ALPHA: f32 = 0.13;
pub const TRAIL_MAX_ALPHA: f32 = 0.95;

/// Length of the bright tail behind the satellite, in seconds: the trail is at
/// [`TRAIL_MAX_ALPHA`] at the current instant and has faded to
/// [`TRAIL_MIN_ALPHA`] this far back along the path it already travelled. Ten
/// minutes.
pub const TRAIL_TAIL_SECS: f32 = 600.0;

/// Alpha steps the trail fade is quantized into. Each maximal run of one step
/// is drawn as a single connected polyline, so translucent segments never stack
/// opacity where the path is dense or doubles back. Enough steps that the
/// banding is imperceptible while the count of drawn shapes stays small.
pub const TRAIL_FADE_STEPS: u32 = 24;

/// The trail opacity control, as a percentage the user types. It scales the
/// whole trail - tail and ghost alike - so a busy multi-constellation sky can
/// be quietened and a sparse one turned right up.
///
/// [`TRAIL_OPACITY_PERCENT_DEFAULT`] is the calibrated look: the tuned alphas
/// above are exactly what the plot is designed to show, so it maps to a scale
/// of `1.0` ([`trail_opacity_multiplier`]). The scale is left deliberately
/// short of the `0..=100 %` range's top so there is real headroom to push the
/// trails bolder than the default, per the request to be able to turn them "up
/// much more" - at 100 % the whole trail is drawn at
/// `100 / DEFAULT` times its tuned strength.
pub const TRAIL_OPACITY_PERCENT_MIN: f32 = 0.0;
pub const TRAIL_OPACITY_PERCENT_MAX: f32 = 100.0;
pub const TRAIL_OPACITY_PERCENT_DEFAULT: f32 = 40.0;

/// The alpha scale for an opacity percentage: `1.0` at
/// [`TRAIL_OPACITY_PERCENT_DEFAULT`] (the calibrated look), rising to
/// `100 / DEFAULT` at full and falling to `0` (invisible) at nothing. The
/// percentage is clamped first so a stray value can never overdrive or invert
/// the trails.
pub fn trail_opacity_multiplier(percent: f32) -> f32 {
    percent.clamp(TRAIL_OPACITY_PERCENT_MIN, TRAIL_OPACITY_PERCENT_MAX)
        / TRAIL_OPACITY_PERCENT_DEFAULT
}

/// Alpha applied to trails of the constellations not currently focused
/// (hovered), so the focused constellation stands out.
pub const TRAIL_DIMMED_ALPHA: f32 = 0.12;

/// Radius of the marker dropped on each trail at the scrubbed time.
pub const TRAIL_MARKER_RADIUS_PX: f32 = 4.0;

/// Outline width of a scrub marker, in the panel colour, so it reads over
/// the trail beneath it.
pub const TRAIL_MARKER_EDGE_PX: f32 = 1.5;

/// Ring width of a hollow scrub marker - a satellite tracked but not in the
/// fix at the scrubbed instant, drawn as an outline rather than a filled dot.
pub const TRAIL_MARKER_HOLLOW_EDGE_PX: f32 = 1.6;

/// Half-length of a slip mark's arms (an "×" drawn on the trail where a
/// satellite slipped).
pub const SLIP_MARK_RADIUS_PX: f32 = 4.5;

/// Stroke width of a slip mark's arms.
pub const SLIP_MARK_WIDTH_PX: f32 = 1.6;

/// Pointer distance within which a slip mark shows its hover tooltip.
pub const SLIP_MARK_HOVER_RADIUS_PX: f32 = 6.0;

/// Dash and gap lengths of the elevation-mask ring.
pub const MASK_RING_DASH_PX: f32 = 4.0;
/// See [`MASK_RING_DASH_PX`].
pub const MASK_RING_GAP_PX: f32 = 4.0;
/// Polyline segments approximating the dashed mask ring.
pub const MASK_RING_SEGMENTS: u32 = 90;

/// Pointer distance from the mask ring, in either radial direction, within
/// which the ring reads as hovered and shows its tooltip.
pub const MASK_RING_HOVER_BAND_PX: f32 = 6.0;

/// Stroke width of the mask ring while hovered, thicker than the resting
/// [`GRID_STROKE_WIDTH_PX`] so it reads as picked out.
pub const MASK_RING_HOVER_WIDTH_PX: f32 = 2.0;

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

    /// The opacity percentage maps to an alpha scale of `1.0` at the calibrated
    /// default, `0` at nothing, and above `1.0` at the top of the range so the
    /// trails can be turned up past their tuned strength. Out-of-range percents
    /// clamp rather than inverting or overdriving.
    #[test]
    fn trail_opacity_multiplier_is_one_at_the_default_and_scales_the_range() {
        use super::{
            TRAIL_OPACITY_PERCENT_DEFAULT, TRAIL_OPACITY_PERCENT_MAX, TRAIL_OPACITY_PERCENT_MIN,
            trail_opacity_multiplier as mult,
        };
        let approx = |a: f32, b: f32| (a - b).abs() < 1e-6;

        assert!(approx(mult(TRAIL_OPACITY_PERCENT_DEFAULT), 1.0));
        assert!(approx(mult(TRAIL_OPACITY_PERCENT_MIN), 0.0));
        assert!(
            mult(TRAIL_OPACITY_PERCENT_MAX) > 1.0,
            "the top of the range must exceed the default so trails can be bolder"
        );
        // Clamped both ways: nothing below the min or above the max.
        assert!(approx(mult(-50.0), mult(TRAIL_OPACITY_PERCENT_MIN)));
        assert!(approx(mult(1000.0), mult(TRAIL_OPACITY_PERCENT_MAX)));
    }
}
