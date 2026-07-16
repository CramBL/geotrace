//! Line colors and styles: the metric hue table, the channel palette with
//! its per-component hue ladder and user overrides, and the per-file
//! lightness/line-style differentiation.

use std::collections::HashMap;

use egui::Color32;
use egui_plot::LineStyle;
use gt_types::MetricKind;

/// Per-file shade offsets applied to each metric's base colour.
///
/// Keeping hue fixed and only shifting value/lightness preserves metric identity
/// while still making overlapping lines from different files distinguishable.
///
/// Unlike [`gt_ui_theme::track_color`], which cycles a full colour palette per
/// (file, track) for the map (where hue *is* the identity signal), the plot
/// needs hue to stay tied to the metric - so files are distinguished by
/// lightness shift and line style ([`FILE_LINE_STYLES`]) instead.
const FILE_SHADE_FACTORS: [i16; 7] = [0, 22, -22, 12, -12, 32, -32];

/// File-level line styles to keep perfectly overlapping lines distinguishable.
///
/// Color still carries metric identity. Style only disambiguates file source.
pub(super) const FILE_LINE_STYLES: [LineStyle; 5] = [
    LineStyle::Solid,
    LineStyle::Dashed { length: 6.0 },
    LineStyle::Dotted { spacing: 5.0 },
    LineStyle::Dashed { length: 10.0 },
    LineStyle::Dotted { spacing: 8.0 },
];
/// The plot line color for `kind`, shaded by `file_index` so overlapping
/// lines from different files stay distinguishable (see
/// [`file_shade_factor`]).
pub(super) fn metric_line_color(kind: MetricKind, file_index: usize, dark_mode: bool) -> Color32 {
    shade_color(
        gt_ui_theme::metric_color(kind, dark_mode),
        file_shade_factor(file_index),
    )
}

/// The channel chip/line palette, cycled by a channel's index in the sorted
/// union of loaded channel names. Channels are dynamic, so unlike the metrics
/// they cannot carry a hardcoded per-variant color; hues are picked to avoid
/// the strong metric colors (velocity yellow, EPH magenta, heading orange).
pub(super) const CHANNEL_PALETTE: [Color32; 6] = [
    Color32::from_rgb(102, 204, 153), // spring green
    Color32::from_rgb(153, 128, 250), // lavender
    Color32::from_rgb(64, 175, 255),  // azure
    Color32::from_rgb(230, 126, 179), // rose
    Color32::from_rgb(181, 204, 92),  // olive
    Color32::from_rgb(94, 210, 217),  // teal
];

/// The chip color of the `index`-th channel (its position in the sorted name
/// union). The palette cycles past its length.
pub(super) fn channel_color(index: usize) -> Color32 {
    let palette = CHANNEL_PALETTE;
    palette
        .get(index % palette.len())
        .copied()
        .unwrap_or(Color32::GRAY)
}

pub(super) fn channel_line_color(index: usize, file_index: usize) -> Color32 {
    shade_color(channel_color(index), file_shade_factor(file_index))
}

/// Hue step between a vector channel's components, as a fraction of the
/// full hue circle. 25 degrees proved too close to tell apart in practice;
/// at 60 degrees x/y/z read as clearly different colors. The chip's bar
/// strip ties the rotated hues back to their channel, so staying near the
/// base hue matters less than being distinct.
const COMPONENT_HUE_STEP: f32 = 60.0 / 360.0;

/// The `component`-th color of a channel, honoring a user override from
/// the chip's right-click menu before falling back to the derived hue ladder.
pub(super) fn effective_component_color(
    overrides: &HashMap<String, Vec<Option<Color32>>>,
    channel: &str,
    base: Color32,
    component: usize,
) -> Color32 {
    overrides
        .get(channel)
        .and_then(|colors| colors.get(component).copied().flatten())
        .unwrap_or_else(|| component_color(base, component))
}

/// The `component`-th line color of a channel: the channel color with its
/// hue rotated in alternating steps (base, +25, -25, +50, ...), so a vector
/// channel's components separate without leaving its color family.
pub(super) fn component_color(base: Color32, component: usize) -> Color32 {
    if component == 0 {
        return base;
    }
    let steps = component.div_ceil(2) as f32;
    let sign = if component % 2 == 1 { 1.0 } else { -1.0 };
    let mut hsva = egui::ecolor::Hsva::from(base);
    hsva.h = (hsva.h + sign * steps * COMPONENT_HUE_STEP).rem_euclid(1.0);
    Color32::from(hsva)
}

pub(super) fn file_shade_factor(file_index: usize) -> i16 {
    let idx = file_index % FILE_SHADE_FACTORS.len();
    FILE_SHADE_FACTORS.get(idx).copied().unwrap_or(0)
}

pub(super) fn file_line_style(file_index: usize) -> LineStyle {
    let idx = file_index % FILE_LINE_STYLES.len();
    FILE_LINE_STYLES
        .get(idx)
        .copied()
        .unwrap_or(LineStyle::Solid)
}

/// Shifts a color toward white (positive `factor_pct`) or black (negative)
/// by the given percentage.
pub(super) fn shade_color(color: Color32, factor_pct: i16) -> Color32 {
    let (target, amount_pct) = if factor_pct >= 0 {
        (255, factor_pct)
    } else {
        (0, -factor_pct)
    };
    let num = i32::from(amount_pct.clamp(0, 100));
    Color32::from_rgb(
        gt_ui_theme::lerp_channel(color.r(), target, num, 100),
        gt_ui_theme::lerp_channel(color.g(), target, num, 100),
        gt_ui_theme::lerp_channel(color.b(), target, num, 100),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The component hue ladder: the first component keeps the channel
    /// color, later ones alternate around it in fixed hue steps and wrap
    /// cleanly around the hue circle - always distinct from the base.
    #[rstest::rstest]
    #[case::base(0, 0.0)]
    #[case::second_steps_up(1, 60.0 / 360.0)]
    #[case::third_steps_down(2, -60.0 / 360.0)]
    #[case::fourth_steps_further(3, 120.0 / 360.0)]
    #[case::fifth_steps_further_down(4, -120.0 / 360.0)]
    fn component_colors_ladder_around_the_base_hue(#[case] component: usize, #[case] offset: f32) {
        let base = CHANNEL_PALETTE[0];
        let base_hue = egui::ecolor::Hsva::from(base).h;
        let got = egui::ecolor::Hsva::from(component_color(base, component)).h;
        let want = (base_hue + offset).rem_euclid(1.0);
        assert!(
            (got - want).abs() < 0.01 || (got - want).abs() > 0.99,
            "component {component}: hue {got} != {want}"
        );
    }

    /// A user override wins over the derived hue; anything else - no entry,
    /// a `None` slot, an index past the stored vector - falls back to the
    /// ladder.
    #[test]
    fn effective_component_color_falls_back_without_an_override() {
        let base = CHANNEL_PALETTE[0];
        let red = Color32::from_rgb(255, 0, 0);
        let overrides = HashMap::from([("accel".to_owned(), vec![None, Some(red)])]);
        assert_eq!(
            effective_component_color(&overrides, "accel", base, 1),
            red,
            "the stored override wins"
        );
        assert_eq!(
            effective_component_color(&overrides, "accel", base, 0),
            component_color(base, 0),
            "a None slot falls back"
        );
        assert_eq!(
            effective_component_color(&overrides, "accel", base, 2),
            component_color(base, 2),
            "an index past the stored vector falls back"
        );
        assert_eq!(
            effective_component_color(&overrides, "incline", base, 0),
            component_color(base, 0),
            "a channel without overrides falls back"
        );
    }

    #[test]
    fn file_shading_distinguishes_adjacent_files() {
        let a = metric_line_color(MetricKind::SatsSeen, 0, true);
        let b = metric_line_color(MetricKind::SatsSeen, 1, true);
        assert_ne!(a, b);
    }

    #[test]
    fn sats_seen_and_sats_fix_stay_visually_separate_across_files() {
        // The seen line stays the lighter-green blue and fix the deeper one in
        // both themes, so the pair never collapses into one colour.
        for dark_mode in [true, false] {
            for fi in 0..FILE_SHADE_FACTORS.len() * 5 {
                let seen = metric_line_color(MetricKind::SatsSeen, fi, dark_mode);
                let fix = metric_line_color(MetricKind::SatsFix, fi, dark_mode);
                assert!(
                    seen.g() > fix.g(),
                    "seen should stay the lighter blue: dark={dark_mode}, fi={fi}, seen={seen:?}, fix={fix:?}"
                );
            }
        }
    }

    #[test]
    fn file_line_styles_are_pairwise_distinct() {
        for (i, a) in FILE_LINE_STYLES.iter().enumerate() {
            for (j, b) in FILE_LINE_STYLES.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    a, b,
                    "FILE_LINE_STYLES[{i}] duplicates FILE_LINE_STYLES[{j}]"
                );
            }
        }
    }
}
