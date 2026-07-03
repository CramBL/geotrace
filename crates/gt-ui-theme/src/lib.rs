use egui::Color32;

/// U+2014 EM DASH - used as a placeholder when a value is absent.
pub const EM_DASH: &str = "—";
/// U+2212 MINUS SIGN - visually distinct from the ASCII hyphen-minus.
pub const MINUS_SIGN: &str = "−";
/// U+0394 GREEK CAPITAL LETTER DELTA - used as a mathematical difference symbol.
pub const DELTA: &str = "Δ";
/// U+00B0 DEGREE SIGN.
pub const DEGREE_SIGN: &str = "°";

/// Highlight blue used for selected/hovered elements across map and panel.
pub const HIGHLIGHT_BLUE: Color32 = Color32::from_rgb(100, 200, 255);

/// Same hue as [`HIGHLIGHT_BLUE`] with reduced alpha - used for the plot seek-bar line.
///
/// Premultiplied equivalent of `(100, 200, 255, 200)`.
pub const HIGHLIGHT_BLUE_SEEK: Color32 = Color32::from_rgba_premultiplied(78, 157, 200, 200);

/// Background fill for a danger button in its hovered state.
pub const DANGER_HOVER: Color32 = Color32::from_rgb(160, 35, 35);

/// Background fill for a danger button in its active (pressed) state.
pub const DANGER_ACTIVE: Color32 = Color32::from_rgb(130, 25, 25);

/// Foreground colour used on text/icons drawn over danger button backgrounds.
pub const DANGER_FG: Color32 = Color32::WHITE;

/// Colour used for inline load-error labels.
pub const ERROR_INDICATOR: Color32 = Color32::from_rgb(220, 70, 50);

/// Amber colour used for data quality warning icons and indicators.
pub const WARNING_AMBER: Color32 = Color32::from_rgb(255, 180, 0);

/// Affirmative green for a primary call-to-action, e.g. the "Update and restart"
/// button in the update prompt.
pub const SUCCESS_GREEN: Color32 = Color32::from_rgb(46, 160, 67);

/// Background colour used to indicate that the corresponding map element is hovered.
///
/// Pass `ui.visuals().dark_mode` to select the appropriate variant.
pub fn map_hover_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgba_unmultiplied(210, 160, 0, 90)
    } else {
        Color32::from_rgba_unmultiplied(200, 140, 0, 55)
    }
}

/// Blue/cyan palette assigned to track polylines - chosen to stand out on both
/// OSM and satellite map backgrounds without implying error or warning semantics.
/// The palette cycles over (file_index, track_index) using a mixing function
/// so adjacent tracks get distinct shades.
pub const TRACK_COLORS: [Color32; 12] = [
    Color32::from_rgb(30, 160, 255),  // vivid blue
    Color32::from_rgb(0, 220, 220),   // cyan
    Color32::from_rgb(80, 200, 255),  // powder blue
    Color32::from_rgb(0, 180, 200),   // deep cyan
    Color32::from_rgb(60, 120, 255),  // royal blue
    Color32::from_rgb(0, 240, 180),   // cyan-green
    Color32::from_rgb(120, 220, 255), // ice blue
    Color32::from_rgb(0, 140, 255),   // azure
    Color32::from_rgb(40, 200, 160),  // teal
    Color32::from_rgb(160, 230, 255), // pale blue
    Color32::from_rgb(0, 200, 130),   // seafoam
    Color32::from_rgb(100, 180, 240), // cornflower
];

/// Canonical display color for a GNSS constellation, so a constellation reads
/// the same hue wherever it appears (plot lines, marker tables, …).  Mirrors the
/// per-constellation "seen" hues used by the time-series plot.
pub fn constellation_color(constellation: gt_types::satellites::Constellation) -> Color32 {
    use gt_types::satellites::Constellation;
    match constellation {
        Constellation::Gps => Color32::from_rgb(0, 220, 80), // lime green
        Constellation::Glonass => Color32::from_rgb(255, 140, 30), // golden
        Constellation::Galileo => Color32::from_rgb(255, 50, 110), // hot pink
        Constellation::Beidou => Color32::from_rgb(0, 230, 230), // cyan
        Constellation::Navic => Color32::from_rgb(160, 120, 255), // violet
        Constellation::Qzss => Color32::from_rgb(240, 110, 90), // coral
    }
}

/// Returns the track color for a (file_index, track_index) pair.
///
/// Coprime-factor mixing ensures adjacent tracks get distinct palette slots even
/// for moderate numbers of files and tracks.
pub fn track_color(fi: usize, ti: usize) -> Color32 {
    let idx = fi.wrapping_mul(7).wrapping_add(ti.wrapping_mul(3));
    #[expect(
        clippy::indexing_slicing,
        reason = "idx is reduced via modulo so always in bounds"
    )]
    TRACK_COLORS[idx % TRACK_COLORS.len()]
}

/// Map a [`SignalQuality`](gt_types::SignalQuality) tier to a colour on a green → red gradient.
pub fn snr_color(quality: gt_types::SignalQuality) -> Color32 {
    use gt_types::SignalQuality;
    match quality {
        SignalQuality::Excellent => Color32::from_rgb(0, 200, 0),
        SignalQuality::Good => Color32::from_rgb(120, 200, 0),
        SignalQuality::Moderate => Color32::from_rgb(220, 200, 0),
        SignalQuality::Weak => Color32::from_rgb(255, 140, 0),
        SignalQuality::VeryWeak => Color32::from_rgb(220, 60, 0),
    }
}

/// Maps a GNSS fix-quality percentage to a colour on a green -> yellow -> red
/// scale.
///
/// `100%` is green, `95..=99%` is a flat yellow, and below `95%` the colour
/// gradually shifts from yellow toward red, reaching maximum red at `80%`
/// and staying there for any lower percentage.
pub fn fix_quality_color(pct: u32) -> Color32 {
    const GREEN: Color32 = Color32::from_rgb(0, 200, 0);
    const YELLOW: Color32 = Color32::from_rgb(220, 200, 0);
    const RED: Color32 = Color32::from_rgb(220, 60, 0);
    const YELLOW_FROM_PCT: u32 = 95;
    const RED_AT_PCT: u32 = 80;

    if pct >= 100 {
        GREEN
    } else if pct >= YELLOW_FROM_PCT {
        YELLOW
    } else if pct <= RED_AT_PCT {
        RED
    } else {
        let num = i32::try_from(YELLOW_FROM_PCT - pct).unwrap_or(0);
        let den = i32::try_from(YELLOW_FROM_PCT - RED_AT_PCT).unwrap_or(1);
        Color32::from_rgb(
            lerp_channel(YELLOW.r(), RED.r(), num, den),
            lerp_channel(YELLOW.g(), RED.g(), num, den),
            lerp_channel(YELLOW.b(), RED.b(), num, den),
        )
    }
}

/// Linearly interpolates a colour channel from `a` toward `b` by `num/den`
/// (where `0 <= num <= den`), clamped to `[0, 255]`.
pub fn lerp_channel(a: u8, b: u8, num: i32, den: i32) -> u8 {
    let a = i32::from(a);
    let b = i32::from(b);
    let value = a + (b - a) * num / den;
    u8::try_from(value.clamp(0, 255)).unwrap_or(0)
}

/// Halo stroke for query matches, drawn beneath the track line.
///
/// Magenta reads as "annotation" - it is absent from the track palette
/// (blues/cyans), the quality gradient (green/yellow/red), and the hover
/// amber, so a match never blends into any of them. Semi-transparent so the
/// map background stays legible under wide strokes.
///
/// Premultiplied equivalent of `(235, 70, 220, 150)`.
pub const QUERY_MATCH_HALO: Color32 = Color32::from_rgba_premultiplied(138, 41, 129, 150);

/// [`QUERY_MATCH_HALO`] desaturated for stale results (the visible data
/// changed after the run). Still clearly present, per the "gray out, never
/// hide" rule.
///
/// Premultiplied equivalent of `(150, 120, 145, 110)`.
pub const QUERY_MATCH_HALO_STALE: Color32 = Color32::from_rgba_premultiplied(65, 52, 63, 110);

/// Colors used for log-entry markers, cycling over the marker's log index.
pub const LOG_COLORS: [Color32; 8] = [
    Color32::from_rgb(230, 57, 70),
    Color32::from_rgb(255, 149, 0),
    Color32::from_rgb(255, 190, 11),
    Color32::from_rgb(6, 214, 160),
    Color32::from_rgb(46, 196, 182),
    Color32::from_rgb(131, 56, 236),
    Color32::from_rgb(255, 45, 85),
    Color32::from_rgb(238, 66, 102),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_quality_color_full_is_green() {
        assert_eq!(fix_quality_color(100), Color32::from_rgb(0, 200, 0));
    }

    #[test]
    fn fix_quality_color_near_full_is_flat_yellow() {
        assert_eq!(fix_quality_color(99), Color32::from_rgb(220, 200, 0));
        assert_eq!(fix_quality_color(95), Color32::from_rgb(220, 200, 0));
    }

    #[test]
    fn fix_quality_color_at_and_below_red_threshold_is_red() {
        assert_eq!(fix_quality_color(80), Color32::from_rgb(220, 60, 0));
        assert_eq!(fix_quality_color(0), Color32::from_rgb(220, 60, 0));
    }

    #[test]
    fn fix_quality_color_blends_between_yellow_and_red() {
        let c = fix_quality_color(88); // partway between 95% (yellow) and 80% (red)
        assert_eq!(c.r(), 220, "red channel constant across yellow/red");
        assert_eq!(c.b(), 0, "blue channel constant across yellow/red");
        assert!(
            c.g() > 60 && c.g() < 200,
            "green channel should blend: {c:?}"
        );
    }

    #[test]
    fn fix_quality_color_green_channel_decreases_toward_red() {
        let g95 = fix_quality_color(95).g();
        let g88 = fix_quality_color(88).g();
        let g80 = fix_quality_color(80).g();
        assert!(g95 > g88 && g88 > g80);
    }
}
