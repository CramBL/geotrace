//! A smooth signal-strength heat field over the sky disc.
//!
//! Places a soft glow at each of the current-instant in-fix satellites,
//! brighter with stronger signal, and sums them where satellites cluster. It is
//! rendered as one coloured triangle mesh: the field is sampled on a fixed grid
//! and egui interpolates the vertex colours between, so the result reads as a
//! continuous blur without a shader, at a cost fixed by the grid rather than by
//! the track's length.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Painter, Pos2, Shape, Vec2};

use gt_types::satellites::Snr;
use gt_ui_theme::{lerp_channel, unit_to_u8};

/// SNR band mapped onto the heat weight `[0, 1]`: at or below the floor a
/// satellite barely registers, at or above the ceiling it glows at full
/// strength. Brackets the useful GNSS carrier-to-noise range in dB-Hz.
const SNR_FLOOR_DBHZ: f32 = 20.0;
const SNR_CEIL_DBHZ: f32 = 48.0;

/// The faintest an in-fix satellite glows: even the weakest one used in the fix
/// still marks its place, so the field shows *where* the fix satellites are and
/// not only where the strong ones are.
const MIN_WEIGHT: f32 = 0.15;

/// Weight of an in-fix satellite whose report carries no SNR: it is used in the
/// fix, so it marks presence, but with no signal strength to justify a hotter
/// glow it sits low on the ramp.
const NO_SNR_WEIGHT: f32 = 0.3;

/// Standard deviation of a single satellite's glow, as a fraction of the disc
/// radius. Wide enough that a handful of satellites read as a smooth field
/// rather than isolated dots.
const GLOW_SIGMA_FRACTION: f32 = 0.22;

/// Cells per side of the square mesh laid over the disc's bounding box. The
/// field is sampled at each vertex and egui interpolates between them, so this
/// trades smoothness against per-frame triangles (fixed, independent of the
/// track's length).
const GRID_STEPS: usize = 48;

/// Peak alpha of the field, so even a saturated hot spot stays translucent and
/// the trails, markers, and grid read through it.
const MAX_ALPHA: f32 = 0.55;

/// Intensity at which the alpha reaches [`MAX_ALPHA`]; below it the field fades
/// toward transparent so faint tails melt into the background rather than
/// tinting the whole disc.
const ALPHA_FULL_AT: f32 = 0.5;

/// Fixed-point denominator for the ramp interpolation handed to [`lerp_channel`].
const RAMP_SCALE: i32 = 1000;

/// The warm colour ramp, low to high intensity: deep red, through orange, to a
/// warm yellow-white. Interpolated between the stops in [`heat_color`].
const RAMP: [(f32, [u8; 3]); 4] = [
    (0.0, [120, 25, 12]),
    (0.35, [214, 74, 16]),
    (0.7, [240, 152, 26]),
    (1.0, [252, 232, 140]),
];

/// The heat weight of an in-fix satellite from its SNR, normalized to
/// `[MIN_WEIGHT, 1]`. A satellite with no reported SNR gets [`NO_SNR_WEIGHT`].
pub fn snr_weight(snr: Option<Snr>) -> f32 {
    match snr {
        Some(snr) => {
            let t =
                ((snr.value() - SNR_FLOOR_DBHZ) / (SNR_CEIL_DBHZ - SNR_FLOOR_DBHZ)).clamp(0.0, 1.0);
            MIN_WEIGHT + (1.0 - MIN_WEIGHT) * t
        }
        None => NO_SNR_WEIGHT,
    }
}

/// The field colour for a normalized intensity `t`. Alpha rises from fully
/// transparent at zero to [`MAX_ALPHA`] at [`ALPHA_FULL_AT`] and holds, so the
/// faint tails of the field fade out instead of tinting the whole disc.
pub fn heat_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let [r, g, b] = ramp_rgb(t);
    let alpha = unit_to_u8(MAX_ALPHA * (t / ALPHA_FULL_AT).min(1.0));
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// The warm ramp's RGB at intensity `t` in `[0, 1]`, interpolating between the
/// two [`RAMP`] stops that bracket it.
fn ramp_rgb(t: f32) -> [u8; 3] {
    // The only caller pre-clamps, and RAMP's ends are pinned at 0.0/1.0, so the
    // scan below always finds a bracketing pair; this guards a future caller
    // that forgets to clamp rather than letting it fall through silently.
    debug_assert!(
        (0.0..=1.0).contains(&t),
        "ramp_rgb needs t in [0, 1], got {t}"
    );
    let [first, .., last] = RAMP;
    let (mut lo, mut hi) = (first, last);
    for pair in RAMP.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        if t >= a.0 && t <= b.0 {
            (lo, hi) = (*a, *b);
            break;
        }
    }
    let span = (hi.0 - lo.0).max(f32::EPSILON);
    let num = ((t - lo.0) / span * RAMP_SCALE as f32).round() as i32;
    let [lr, lg, lb] = lo.1;
    let [hr, hg, hb] = hi.1;
    [
        lerp_channel(lr, hr, num, RAMP_SCALE),
        lerp_channel(lg, hg, num, RAMP_SCALE),
        lerp_channel(lb, hb, num, RAMP_SCALE),
    ]
}

/// Paint the signal-strength heat field for `sources` (each a screen position
/// and a `[0, 1]` weight) onto the disc at `center`/`radius`.
///
/// Builds one coloured mesh: the field is sampled at a fixed grid and egui
/// interpolates the vertex colours between, so overlapping glows sum in
/// intensity and the result reads as a smooth blur. Clipped to the disc so it
/// never bleeds past the horizon rim. A no-op with no sources.
pub fn paint_signal_field(painter: &Painter, center: Pos2, radius: f32, sources: &[(Pos2, f32)]) {
    if sources.is_empty() || radius <= 0.0 {
        return;
    }
    let sigma = radius * GLOW_SIGMA_FRACTION;
    let inv_two_sigma_sq = 1.0 / (2.0 * sigma * sigma);
    let step = 2.0 * radius / GRID_STEPS as f32;
    let origin = center - Vec2::splat(radius);
    let cols = (GRID_STEPS + 1) as u32;

    let mut mesh = Mesh::default();
    for row in 0..=GRID_STEPS {
        for col in 0..=GRID_STEPS {
            let p = origin + Vec2::new(col as f32 * step, row as f32 * step);
            let intensity = field_intensity(p, center, radius, sources, inv_two_sigma_sq);
            mesh.vertices.push(Vertex {
                pos: p,
                uv: WHITE_UV,
                color: heat_color(intensity),
            });
        }
    }
    for row in 0..GRID_STEPS as u32 {
        for col in 0..GRID_STEPS as u32 {
            let top_left = row * cols + col;
            let top_right = top_left + 1;
            let bottom_left = top_left + cols;
            let bottom_right = bottom_left + 1;
            mesh.indices
                .extend_from_slice(&[top_left, top_right, bottom_right]);
            mesh.indices
                .extend_from_slice(&[top_left, bottom_right, bottom_left]);
        }
    }
    painter.add(Shape::Mesh(mesh.into()));
}

/// The field intensity at `p`: the sum over the sources of each one's weight
/// times a Gaussian of the distance to it. Zero outside the disc, so the field
/// is clipped to the horizon rim with a soft edge.
fn field_intensity(
    p: Pos2,
    center: Pos2,
    radius: f32,
    sources: &[(Pos2, f32)],
    inv_two_sigma_sq: f32,
) -> f32 {
    if center.distance_sq(p) > radius * radius {
        return 0.0;
    }
    sources
        .iter()
        .map(|&(pos, weight)| weight * (-p.distance_sq(pos) * inv_two_sigma_sq).exp())
        .sum()
}

#[cfg(test)]
mod tests {
    use egui::pos2;
    use rstest::rstest;

    use gt_types::satellites::Snr;

    use super::{
        ALPHA_FULL_AT, GLOW_SIGMA_FRACTION, MIN_WEIGHT, NO_SNR_WEIGHT, field_intensity, heat_color,
        snr_weight,
    };

    #[rstest]
    // Below the floor pins to the minimum, above the ceiling to full strength.
    #[case::below_floor(Some(10.0), MIN_WEIGHT)]
    #[case::at_ceiling(Some(48.0), 1.0)]
    #[case::above_ceiling(Some(60.0), 1.0)]
    fn snr_weight_clamps_to_the_band(#[case] snr: Option<f32>, #[case] expected: f32) {
        let weight = snr_weight(snr.map(Snr::new));
        assert!((weight - expected).abs() < 1e-6, "{weight} != {expected}");
    }

    #[test]
    fn snr_weight_is_monotonic_and_floored() {
        // A stronger signal never weighs less than a weaker one.
        let weak = snr_weight(Some(Snr::new(25.0)));
        let strong = snr_weight(Some(Snr::new(42.0)));
        assert!(strong > weak);
        // Every real reading stays at or above the floor, and a missing SNR
        // still marks presence rather than vanishing.
        assert!(weak >= MIN_WEIGHT);
        const { assert!(NO_SNR_WEIGHT >= MIN_WEIGHT) };
        assert!((snr_weight(None) - NO_SNR_WEIGHT).abs() < 1e-6);
    }

    #[test]
    fn heat_color_fades_in_from_transparent() {
        // Zero intensity is fully transparent, so bare disc shows through.
        assert_eq!(heat_color(0.0).a(), 0);
        // Alpha rises with intensity and saturates at the full-alpha point.
        let low = heat_color(ALPHA_FULL_AT / 2.0).a();
        let full = heat_color(ALPHA_FULL_AT).a();
        let over = heat_color(1.0).a();
        assert!(low > 0 && low < full, "low {low} not between 0 and {full}");
        assert_eq!(full, over, "alpha holds past the full-alpha point");
    }

    #[test]
    fn field_intensity_peaks_at_a_source_and_decays() {
        let center = pos2(100.0, 100.0);
        let radius = 100.0_f32;
        let source = pos2(120.0, 100.0);
        let sources = [(source, 1.0)];
        let inv_two_sigma_sq = 1.0 / (2.0 * (radius * GLOW_SIGMA_FRACTION).powi(2));

        let at_source = field_intensity(source, center, radius, &sources, inv_two_sigma_sq);
        let away = field_intensity(
            pos2(160.0, 100.0),
            center,
            radius,
            &sources,
            inv_two_sigma_sq,
        );
        assert!(at_source > away, "intensity must fall off from the source");
        // Outside the disc the field is clipped to nothing - an exact 0.0
        // early return, so a bit-exact comparison is right here.
        let outside = field_intensity(
            pos2(250.0, 100.0),
            center,
            radius,
            &sources,
            inv_two_sigma_sq,
        );
        #[expect(clippy::float_cmp, reason = "the clip is an exact 0.0 return")]
        {
            assert_eq!(outside, 0.0);
        }
    }
}
