use egui::Color32;

/// U+2014 EM DASH - used as a placeholder when a value is absent.
pub const EM_DASH: &str = "—";
/// U+2212 MINUS SIGN - visually distinct from the ASCII hyphen-minus.
pub const MINUS_SIGN: &str = "−";
/// U+0394 GREEK CAPITAL LETTER DELTA - used as a mathematical difference symbol.
pub const DELTA: &str = "Δ";
/// U+00B0 DEGREE SIGN.
pub const DEGREE_SIGN: &str = "°";

/// A colour with a dark-surface and a light-surface variant, so callers stay
/// legible on both themes. Themed foregrounds are contrast-checked against both
/// backgrounds by the crate's tests (see [`THEMED_FOREGROUNDS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemedColor {
    dark: Color32,
    light: Color32,
}

impl ThemedColor {
    /// A themed colour from its dark-surface and light-surface variants.
    pub const fn new(dark: Color32, light: Color32) -> Self {
        Self { dark, light }
    }

    /// The variant for the current theme. Pass `ui.visuals().dark_mode`.
    pub const fn resolve(self, dark_mode: bool) -> Color32 {
        if dark_mode { self.dark } else { self.light }
    }

    /// The dark-surface variant.
    pub const fn dark(self) -> Color32 {
        self.dark
    }

    /// The light-surface variant.
    pub const fn light(self) -> Color32 {
        self.light
    }
}

/// Highlight blue used for selected/hovered elements across map and panel.
pub const HIGHLIGHT_BLUE: Color32 = Color32::from_rgb(100, 200, 255);

/// Same hue as [`HIGHLIGHT_BLUE`] with reduced alpha - used for the plot seek-bar line.
///
/// Premultiplied equivalent of `(100, 200, 255, 200)`.
pub const HIGHLIGHT_BLUE_SEEK: Color32 = Color32::from_rgba_premultiplied(78, 157, 200, 200);

/// [`HIGHLIGHT_BLUE`] at query-halo alpha - the map's halo band for the match
/// hovered in the query results table, distinct from the draw-layer palette.
///
/// Premultiplied equivalent of `(100, 200, 255, 150)`.
pub const QUERY_MATCH_HOVER_HALO: Color32 = Color32::from_rgba_premultiplied(59, 118, 150, 150);

/// [`HIGHLIGHT_BLUE`] at low alpha - the plot's shaded time band for the match
/// hovered in the query results table.
///
/// Premultiplied equivalent of `(100, 200, 255, 40)`.
pub const HIGHLIGHT_BLUE_BAND: Color32 = Color32::from_rgba_premultiplied(16, 31, 40, 40);

/// Background fill for a danger button in its hovered state.
pub const DANGER_HOVER: Color32 = Color32::from_rgb(160, 35, 35);

/// Background fill for a danger button in its active (pressed) state.
pub const DANGER_ACTIVE: Color32 = Color32::from_rgb(130, 25, 25);

/// Foreground colour used on text/icons drawn over danger button backgrounds.
pub const DANGER_FG: Color32 = Color32::WHITE;

/// Colour used for inline load-error labels.
pub const ERROR_INDICATOR: Color32 = Color32::from_rgb(220, 70, 50);

/// Amber colour used for data quality warning icons and indicators. Bright
/// enough for dark backgrounds; use [`warning_amber`] where the surface may be
/// light.
pub const WARNING_AMBER: Color32 = Color32::from_rgb(255, 180, 0);

/// The dimmed amber for light backgrounds, where [`WARNING_AMBER`] glares.
pub const WARNING_AMBER_LIGHT: Color32 = Color32::from_rgb(176, 112, 0);

/// Warning amber for either theme: [`WARNING_AMBER`] on dark, [`WARNING_AMBER_LIGHT`] on light.
pub const WARNING: ThemedColor = ThemedColor::new(WARNING_AMBER, WARNING_AMBER_LIGHT);

/// The warning amber for the current theme. Pass `ui.visuals().dark_mode`.
pub const fn warning_amber(dark_mode: bool) -> Color32 {
    WARNING.resolve(dark_mode)
}

/// Colour for inline load-error labels, for the current theme. Pass
/// `ui.visuals().dark_mode`. Bright coral on dark; a deeper red on light,
/// where the bright variant is too pale to read.
pub const fn error_indicator(dark_mode: bool) -> Color32 {
    ERROR.resolve(dark_mode)
}

/// [`ERROR_INDICATOR`] paired with a deeper light-surface variant.
pub const ERROR: ThemedColor = ThemedColor::new(ERROR_INDICATOR, Color32::from_rgb(178, 40, 25));

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

/// The themed colour for a [`SignalQuality`](gt_types::SignalQuality) tier on a
/// green → red scale.
pub const fn snr_themed_color(quality: gt_types::SignalQuality) -> ThemedColor {
    use gt_types::SignalQuality;
    match quality {
        SignalQuality::Excellent => {
            ThemedColor::new(Color32::from_rgb(0, 200, 0), Color32::from_rgb(0, 120, 0))
        }
        SignalQuality::Good => ThemedColor::new(
            Color32::from_rgb(120, 200, 0),
            Color32::from_rgb(95, 120, 0),
        ),
        SignalQuality::Moderate => ThemedColor::new(
            Color32::from_rgb(220, 200, 0),
            Color32::from_rgb(150, 110, 0),
        ),
        SignalQuality::Weak => ThemedColor::new(
            Color32::from_rgb(255, 140, 0),
            Color32::from_rgb(186, 84, 0),
        ),
        SignalQuality::VeryWeak => {
            ThemedColor::new(Color32::from_rgb(220, 60, 0), Color32::from_rgb(188, 40, 8))
        }
    }
}

/// Map a [`SignalQuality`](gt_types::SignalQuality) tier to a colour on a green
/// → red gradient, for the current theme. Pass `ui.visuals().dark_mode`.
pub const fn snr_color(quality: gt_types::SignalQuality, dark_mode: bool) -> Color32 {
    snr_themed_color(quality).resolve(dark_mode)
}

/// Maps a GNSS fix-quality percentage to a colour on a green -> yellow -> red
/// scale, for the current theme. Pass `ui.visuals().dark_mode`.
///
/// `100%` is green, `95..=99%` is a flat yellow, and below `95%` the colour
/// gradually shifts from yellow toward red, reaching maximum red at `80%`
/// and staying there for any lower percentage. The three anchor hues each
/// have a light-surface variant so the label reads on a light theme too.
pub fn fix_quality_color(pct: u32, dark_mode: bool) -> Color32 {
    const YELLOW_FROM_PCT: u32 = 95;
    const RED_AT_PCT: u32 = 80;

    let green = FIX_QUALITY_GREEN.resolve(dark_mode);
    let yellow = FIX_QUALITY_YELLOW.resolve(dark_mode);
    let red = FIX_QUALITY_RED.resolve(dark_mode);

    if pct >= 100 {
        green
    } else if pct >= YELLOW_FROM_PCT {
        yellow
    } else if pct <= RED_AT_PCT {
        red
    } else {
        let num = i32::try_from(YELLOW_FROM_PCT - pct).unwrap_or(0);
        let den = i32::try_from(YELLOW_FROM_PCT - RED_AT_PCT).unwrap_or(1);
        Color32::from_rgb(
            lerp_channel(yellow.r(), red.r(), num, den),
            lerp_channel(yellow.g(), red.g(), num, den),
            lerp_channel(yellow.b(), red.b(), num, den),
        )
    }
}

/// The `100%` anchor of [`fix_quality_color`].
pub const FIX_QUALITY_GREEN: ThemedColor =
    ThemedColor::new(Color32::from_rgb(0, 200, 0), Color32::from_rgb(0, 120, 0));

/// The `95..=99%` anchor of [`fix_quality_color`].
pub const FIX_QUALITY_YELLOW: ThemedColor = ThemedColor::new(
    Color32::from_rgb(220, 200, 0),
    Color32::from_rgb(150, 110, 0),
);

/// The `<=80%` anchor of [`fix_quality_color`].
pub const FIX_QUALITY_RED: ThemedColor =
    ThemedColor::new(Color32::from_rgb(220, 60, 0), Color32::from_rgb(188, 40, 8));

/// Confidence tier for a satellite count shown in the point badge: more
/// satellites contributing reads as higher confidence, on a green → red scale.
///
/// The tier is a semantic step, decoupled from both the count thresholds
/// (see [`fix_count_tier`] / [`seen_count_tier`]) and the concrete hues (see
/// [`SatCountTier::themed_color`]), so neither can drift into hard-coded
/// colours at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumCount)]
pub enum SatCountTier {
    /// Plenty of satellites: a healthy fix.
    Good,
    /// Enough to fix, but with little margin.
    Fair,
    /// Marginal: a fix is degraded or fragile.
    Poor,
    /// Too few to trust: no fix or a lone satellite.
    Critical,
}

impl SatCountTier {
    /// The themed colour for this tier.
    pub const fn themed_color(self) -> ThemedColor {
        match self {
            Self::Good => {
                ThemedColor::new(Color32::from_rgb(0, 200, 0), Color32::from_rgb(0, 120, 0))
            }
            Self::Fair => ThemedColor::new(
                Color32::from_rgb(230, 200, 0),
                Color32::from_rgb(150, 110, 0),
            ),
            Self::Poor => ThemedColor::new(
                Color32::from_rgb(255, 140, 0),
                Color32::from_rgb(186, 84, 0),
            ),
            Self::Critical => ThemedColor::new(
                Color32::from_rgb(240, 60, 40),
                Color32::from_rgb(188, 40, 8),
            ),
        }
    }

    /// The colour for this tier in the current theme. Pass `ui.visuals().dark_mode`.
    pub const fn color(self, dark_mode: bool) -> Color32 {
        self.themed_color().resolve(dark_mode)
    }
}

/// The confidence tier for a "fix used" satellite count.
pub const fn fix_count_tier(count: u32) -> SatCountTier {
    if count == 0 {
        SatCountTier::Critical
    } else if count <= 2 {
        SatCountTier::Poor
    } else if count <= 4 {
        SatCountTier::Fair
    } else {
        SatCountTier::Good
    }
}

/// The confidence tier for a "total seen" satellite count.
pub const fn seen_count_tier(count: u32) -> SatCountTier {
    if count < 5 {
        SatCountTier::Poor
    } else if count < 8 {
        SatCountTier::Fair
    } else {
        SatCountTier::Good
    }
}

/// The standalone semantic foreground [`ThemedColor`]s, paired with a name for
/// diagnostics. The crate's contrast test iterates this to assert each variant
/// keeps enough contrast against its own theme's panel background, so a new
/// colour that is not legible on light (or dark) fails CI instead of shipping.
///
/// Enum-driven palettes ([`SatCountTier`], [`SignalQuality`](gt_types::SignalQuality))
/// are covered by the same test via `strum` iteration rather than being listed
/// here, so a newly added variant is contrast-checked automatically. Add new
/// standalone themed foreground colours here.
pub const THEMED_FOREGROUNDS: &[(&str, ThemedColor)] = &[
    ("WARNING", WARNING),
    ("ERROR", ERROR),
    ("FIX_QUALITY_GREEN", FIX_QUALITY_GREEN),
    ("FIX_QUALITY_YELLOW", FIX_QUALITY_YELLOW),
    ("FIX_QUALITY_RED", FIX_QUALITY_RED),
    (
        "QUERY_SYNTAX_IDENT",
        ThemedColor::new(QUERY_SYNTAX_IDENT, QUERY_SYNTAX_IDENT_LIGHT),
    ),
];

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

/// Halo colours for successive `draw` queries, so overlapping outlines stay
/// distinguishable. The first is [`QUERY_MATCH_HALO`], so a single draw query
/// looks unchanged. All semi-transparent and clear of the track blues.
pub const QUERY_MATCH_HALOS: [Color32; 5] = [
    QUERY_MATCH_HALO,
    // (70, 200, 120, 150) green
    Color32::from_rgba_premultiplied(41, 118, 71, 150),
    // (240, 150, 40, 150) amber
    Color32::from_rgba_premultiplied(141, 88, 24, 150),
    // (90, 160, 240, 150) blue
    Color32::from_rgba_premultiplied(53, 94, 141, 150),
    // (220, 90, 90, 150) red
    Color32::from_rgba_premultiplied(129, 53, 53, 150),
];

/// Halo colour for the `index`-th `draw` query, or the stale grey when the
/// visible data changed after the run. The palette cycles past its length.
pub fn query_halo_color(index: usize, stale: bool) -> Color32 {
    if stale {
        return QUERY_MATCH_HALO_STALE;
    }
    let palette = QUERY_MATCH_HALOS;
    palette
        .get(index % palette.len())
        .copied()
        .unwrap_or(QUERY_MATCH_HALO)
}

/// Query editor syntax highlighting: keywords (`points`, `where`, `and`, …).
pub const QUERY_SYNTAX_KEYWORD: Color32 = Color32::from_rgb(198, 120, 221);

/// Query editor syntax highlighting: numeric literals.
pub const QUERY_SYNTAX_NUMBER: Color32 = Color32::from_rgb(229, 192, 123);

/// Query editor syntax highlighting: metric, unit, and parameter names. Bright
/// for dark backgrounds; use [`query_syntax_ident`] to keep it legible on a
/// light editor too.
pub const QUERY_SYNTAX_IDENT: Color32 = Color32::from_rgb(120, 200, 255);

/// The identifier syntax colour, darkened for a light editor background where
/// [`QUERY_SYNTAX_IDENT`] is too pale to read.
pub const QUERY_SYNTAX_IDENT_LIGHT: Color32 = Color32::from_rgb(20, 110, 190);

/// The identifier syntax colour for the given theme. Pass `ui.visuals().dark_mode`.
pub fn query_syntax_ident(dark_mode: bool) -> Color32 {
    if dark_mode {
        QUERY_SYNTAX_IDENT
    } else {
        QUERY_SYNTAX_IDENT_LIGHT
    }
}

/// Query editor syntax highlighting: comments.
pub const QUERY_SYNTAX_COMMENT: Color32 = Color32::from_rgb(128, 148, 128);

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

    /// sRGB relative luminance per WCAG 2.x.
    fn relative_luminance(c: Color32) -> f64 {
        fn linearize(channel: u8) -> f64 {
            let s = f64::from(channel) / 255.0;
            if s <= 0.040_45 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linearize(c.r()) + 0.7152 * linearize(c.g()) + 0.0722 * linearize(c.b())
    }

    /// WCAG contrast ratio between two opaque colours (>= 1.0).
    fn contrast_ratio(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Contrast floor for status foreground colours. These are bold badge
    /// glyphs and small status labels drawn over the panel/window fill, i.e.
    /// graphical status indicators rather than body text, so the WCAG 1.4.11
    /// non-text / large-bold-text threshold of 3.0 is the right bar.
    const MIN_CONTRAST: f64 = 3.0;

    /// Every themed foreground the test checks: the standalone registry plus
    /// the enum-driven palettes, gathered via `strum` iteration so a newly
    /// added tier is contrast-checked without touching this test.
    fn all_themed_foregrounds() -> Vec<(String, ThemedColor)> {
        use strum::IntoEnumIterator;

        let mut all: Vec<(String, ThemedColor)> = THEMED_FOREGROUNDS
            .iter()
            .map(|(name, color)| ((*name).to_owned(), *color))
            .collect();
        for tier in SatCountTier::iter() {
            all.push((format!("SatCountTier::{tier:?}"), tier.themed_color()));
        }
        for quality in gt_types::SignalQuality::iter() {
            all.push((format!("snr::{quality:?}"), snr_themed_color(quality)));
        }
        all
    }

    #[test]
    fn themed_foregrounds_are_legible_on_their_own_theme() {
        let dark = egui::Visuals::dark();
        let light = egui::Visuals::light();
        // The two surfaces a foreground colour is drawn over: opaque panels and
        // tooltip/hover windows. Require legibility against the harder of the two.
        let dark_bgs = [dark.panel_fill, dark.window_fill];
        let light_bgs = [light.panel_fill, light.window_fill];

        let mut failures = Vec::new();
        for (name, color) in all_themed_foregrounds() {
            for bg in dark_bgs {
                let ratio = contrast_ratio(color.dark(), bg);
                if ratio < MIN_CONTRAST {
                    failures.push(format!(
                        "{name}.dark on {bg:?}: {ratio:.2} < {MIN_CONTRAST}"
                    ));
                }
            }
            for bg in light_bgs {
                let ratio = contrast_ratio(color.light(), bg);
                if ratio < MIN_CONTRAST {
                    failures.push(format!(
                        "{name}.light on {bg:?}: {ratio:.2} < {MIN_CONTRAST}"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "illegible themed colours:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn fix_quality_color_full_is_green() {
        assert_eq!(fix_quality_color(100, true), Color32::from_rgb(0, 200, 0));
    }

    #[test]
    fn fix_quality_color_near_full_is_flat_yellow() {
        assert_eq!(fix_quality_color(99, true), Color32::from_rgb(220, 200, 0));
        assert_eq!(fix_quality_color(95, true), Color32::from_rgb(220, 200, 0));
    }

    #[test]
    fn fix_quality_color_at_and_below_red_threshold_is_red() {
        assert_eq!(fix_quality_color(80, true), Color32::from_rgb(220, 60, 0));
        assert_eq!(fix_quality_color(0, true), Color32::from_rgb(220, 60, 0));
    }

    #[test]
    fn fix_quality_color_blends_between_yellow_and_red() {
        let c = fix_quality_color(88, true); // partway between 95% (yellow) and 80% (red)
        assert_eq!(c.r(), 220, "red channel constant across yellow/red");
        assert_eq!(c.b(), 0, "blue channel constant across yellow/red");
        assert!(
            c.g() > 60 && c.g() < 200,
            "green channel should blend: {c:?}"
        );
    }

    #[test]
    fn fix_quality_color_green_channel_decreases_toward_red() {
        let g95 = fix_quality_color(95, true).g();
        let g88 = fix_quality_color(88, true).g();
        let g80 = fix_quality_color(80, true).g();
        assert!(g95 > g88 && g88 > g80);
    }
}
