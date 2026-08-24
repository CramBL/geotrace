pub mod buttons;
pub mod fonts;
pub mod labels;

use egui::Color32;

/// U+2026 HORIZONTAL ELLIPSIS - truncation marker, and the button-label
/// suffix for actions that need further input after the click (DESIGN.md).
/// Defined by [`gt_fmt`], which formats values for both the UI and non-UI
/// callers.
pub use gt_fmt::ELLIPSIS;
/// U+2014 EM DASH - used as a placeholder when a value is absent. Defined by
/// [`gt_fmt`], which formats values for both the UI and non-UI callers.
pub use gt_fmt::EM_DASH;
/// U+00B7 MIDDLE DOT - separates the fields of a one-line summary. Defined by
/// [`gt_fmt`], which formats values for both the UI and non-UI callers.
pub use gt_fmt::MIDDLE_DOT;
/// U+2212 MINUS SIGN - visually distinct from the ASCII hyphen-minus. Defined
/// by [`gt_fmt`], which formats values for both the UI and non-UI callers.
pub use gt_fmt::MINUS_SIGN;
/// U+2192 RIGHTWARDS ARROW - leads from where a span starts to where it ends.
/// Defined by [`gt_fmt`], which formats values for both the UI and non-UI
/// callers.
pub use gt_fmt::RIGHTWARDS_ARROW;
/// U+0394 GREEK CAPITAL LETTER DELTA - used as a mathematical difference symbol.
pub const DELTA: &str = "Δ";
/// U+00B0 DEGREE SIGN.
pub const DEGREE_SIGN: &str = "°";

/// Installs the font stack and the interaction defaults every GeoTrace window,
/// popup and modal inherits: labels do not select, and text worth copying opts
/// back in with [`egui::Label::selectable`] (DESIGN.md, "Text selection").
pub fn install_app_style(ctx: &egui::Context) {
    ctx.set_fonts(fonts::font_definitions());
    ctx.all_styles_mut(|style| style.interaction.selectable_labels = false);
}

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

    pub const fn dark(self) -> Color32 {
        self.dark
    }

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

/// Canonical themed display color for a GNSS constellation, so a constellation
/// reads the same hue wherever it appears (plot lines, marker tables, …).
/// Delegates to [`metric_themed_color`]'s per-constellation "seen" entries, so
/// the light variants carry that palette's contrast tuning and the two can
/// never drift apart.
pub const fn constellation_themed_color(
    constellation: gt_types::satellites::Constellation,
) -> ThemedColor {
    use gt_types::MetricKind;
    use gt_types::satellites::Constellation;
    metric_themed_color(match constellation {
        Constellation::Gps => MetricKind::GpsSeen,
        Constellation::Glonass => MetricKind::GlonassSeen,
        Constellation::Galileo => MetricKind::GalileoSeen,
        Constellation::Beidou => MetricKind::BeidouSeen,
        Constellation::Navic => MetricKind::NavicSeen,
        Constellation::Qzss => MetricKind::QzssSeen,
    })
}

/// The constellation colour for the current theme. Pass `ui.visuals().dark_mode`.
pub const fn constellation_color(
    constellation: gt_types::satellites::Constellation,
    dark_mode: bool,
) -> Color32 {
    constellation_themed_color(constellation).resolve(dark_mode)
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

/// Share of aircraft below which a cell reads as clear. gpsjam's own site
/// colours cells green under 2 %.
pub const INTERFERENCE_LOW_BREAKPOINT: f32 = 0.02;

/// Share at or above which a cell reads as heavily affected. Yellow between
/// the two breakpoints, red from here.
pub const INTERFERENCE_HIGH_BREAKPOINT: f32 = 0.10;

/// Fill opacity of an interference cell. Low enough that the track ink drawn
/// over it stays legible.
pub const INTERFERENCE_FILL_ALPHA: u8 = 70;

/// The themed fill for a cell where `bad_fraction` of aircraft reported low
/// navigation accuracy.
///
/// A continuous green → yellow → red ramp with the breakpoints as its
/// anchors, so a cell just past a breakpoint does not jump a whole tier.
pub fn interference_color(bad_fraction: f32) -> ThemedColor {
    const CLEAR: (Color32, Color32) =
        (Color32::from_rgb(0, 180, 80), Color32::from_rgb(0, 120, 55));
    const ELEVATED: (Color32, Color32) = (
        Color32::from_rgb(230, 200, 0),
        Color32::from_rgb(160, 130, 0),
    );
    const HEAVY: (Color32, Color32) = (
        Color32::from_rgb(230, 60, 40),
        Color32::from_rgb(175, 30, 20),
    );

    let fraction = bad_fraction.clamp(0.0, 1.0);
    let (from, to, t) = if fraction <= INTERFERENCE_LOW_BREAKPOINT {
        (CLEAR, ELEVATED, fraction / INTERFERENCE_LOW_BREAKPOINT)
    } else if fraction < INTERFERENCE_HIGH_BREAKPOINT {
        let span = INTERFERENCE_HIGH_BREAKPOINT - INTERFERENCE_LOW_BREAKPOINT;
        (
            ELEVATED,
            HEAVY,
            (fraction - INTERFERENCE_LOW_BREAKPOINT) / span,
        )
    } else {
        (HEAVY, HEAVY, 0.0)
    };
    ThemedColor::new(from.0.lerp_to_gamma(to.0, t), from.1.lerp_to_gamma(to.1, t))
}

/// Peak X-ray flux, in W/m², where each flare class begins. A flare marker
/// takes the colour of the class its flux falls in.
pub const FLARE_C_CLASS_FLUX: f64 = 1e-6;
pub const FLARE_M_CLASS_FLUX: f64 = 1e-5;
pub const FLARE_X_CLASS_FLUX: f64 = 1e-4;

/// A flare below the C class, weaker than anything the ionosphere registers.
pub const FLARE_BELOW_C_CLASS: ThemedColor = ThemedColor::new(
    Color32::from_rgb(205, 195, 130),
    Color32::from_rgb(140, 125, 70),
);

/// A C-class flare.
pub const FLARE_C_CLASS: ThemedColor = ThemedColor::new(
    Color32::from_rgb(240, 195, 65),
    Color32::from_rgb(170, 120, 20),
);

/// An M-class flare, the first class NOAA counts a radio blackout from. Also
/// the colour of the control offering the markers.
pub const FLARE_M_CLASS: ThemedColor = ThemedColor::new(
    Color32::from_rgb(245, 140, 45),
    Color32::from_rgb(185, 85, 15),
);

/// An X-class flare.
pub const FLARE_X_CLASS: ThemedColor = ThemedColor::new(
    Color32::from_rgb(250, 80, 60),
    Color32::from_rgb(190, 35, 25),
);

/// The themed stroke of a solar flare marker, by the class its peak flux
/// falls in.
pub const fn solar_flare_color(peak_flux_watts_per_square_meter: f64) -> ThemedColor {
    let flux = peak_flux_watts_per_square_meter;
    if flux >= FLARE_X_CLASS_FLUX {
        FLARE_X_CLASS
    } else if flux >= FLARE_M_CLASS_FLUX {
        FLARE_M_CLASS
    } else if flux >= FLARE_C_CLASS_FLUX {
        FLARE_C_CLASS
    } else {
        FLARE_BELOW_C_CLASS
    }
}

/// Opacity of the band marking how long a flare lasted, as a multiplier on
/// its class colour. Low enough that the metric lines stay readable through
/// a band covering a whole zoomed-in view.
const FLARE_SPAN_FILL_OPACITY: f32 = 0.22;

/// The themed fill of the band marking a solar flare's span: the marker
/// colour [`solar_flare_color`] gives its class, at the band's own opacity.
pub fn solar_flare_span_fill(peak_flux_watts_per_square_meter: f64) -> ThemedColor {
    let marker = solar_flare_color(peak_flux_watts_per_square_meter);
    ThemedColor::new(
        marker.dark().gamma_multiply(FLARE_SPAN_FILL_OPACITY),
        marker.light().gamma_multiply(FLARE_SPAN_FILL_OPACITY),
    )
}

/// Colour of a node holding no measurable content, and the bottom of the
/// scale.
const TEC_SCALE_BOTTOM: ThemedColor = ThemedColor::new(
    Color32::from_rgb(40, 50, 120),
    Color32::from_rgb(30, 40, 100),
);

/// The colour stops of the TEC heatmap, as (TEC units, themed colour) pairs in
/// ascending order.
///
/// The stops double, so the quiet range a mid-latitude recording sits in
/// (under 20 TECU) takes as much of the ramp as the disturbed range above it.
/// The top stop is roughly double the highest value ever measured, so a storm's
/// 175 TECU takes a colour of its own.
const TEC_SCALE: [(f64, ThemedColor); 8] = [
    (0.0, TEC_SCALE_BOTTOM),
    (
        5.0,
        ThemedColor::new(
            Color32::from_rgb(30, 110, 200),
            Color32::from_rgb(20, 80, 165),
        ),
    ),
    (
        10.0,
        ThemedColor::new(
            Color32::from_rgb(0, 170, 190),
            Color32::from_rgb(0, 125, 145),
        ),
    ),
    (
        20.0,
        ThemedColor::new(
            Color32::from_rgb(40, 175, 90),
            Color32::from_rgb(25, 130, 65),
        ),
    ),
    (
        40.0,
        ThemedColor::new(
            Color32::from_rgb(225, 200, 0),
            Color32::from_rgb(160, 135, 0),
        ),
    ),
    (
        80.0,
        ThemedColor::new(
            Color32::from_rgb(240, 140, 20),
            Color32::from_rgb(180, 95, 10),
        ),
    ),
    (
        160.0,
        ThemedColor::new(
            Color32::from_rgb(225, 55, 40),
            Color32::from_rgb(175, 30, 20),
        ),
    ),
    (
        320.0,
        ThemedColor::new(
            Color32::from_rgb(215, 60, 190),
            Color32::from_rgb(160, 35, 140),
        ),
    ),
];

/// Highest TEC value the heatmap's ramp distinguishes. Values above it take the
/// last stop's colour.
pub const TEC_SCALE_TOP_TECU: f64 = 320.0;

/// The values the heatmap legend labels, each of them a colour stop of the
/// scale.
pub const TEC_LEGEND_TICKS_TECU: [f64; 4] = [0.0, 20.0, 80.0, TEC_SCALE_TOP_TECU];

/// Opacity of the TEC heatmap, as the percentage the user sets. It scales the
/// fill of every grid node.
pub const TEC_OPACITY_PERCENT_MIN: f32 = 0.0;
pub const TEC_OPACITY_PERCENT_MAX: f32 = 100.0;
pub const TEC_OPACITY_PERCENT_DEFAULT: f32 = 40.0;

/// The fill alpha for an opacity percentage, clamped to
/// `TEC_OPACITY_PERCENT_MIN..=TEC_OPACITY_PERCENT_MAX` first.
pub fn tec_fill_alpha(percent: f32) -> u8 {
    unit_to_u8(percent.clamp(TEC_OPACITY_PERCENT_MIN, TEC_OPACITY_PERCENT_MAX) / 100.0)
}

/// Where `tecu` sits on the heatmap's ramp, from `0.0` at the first stop to
/// `1.0` at [`TEC_SCALE_TOP_TECU`]. Each stop takes an equal share of the ramp.
pub fn tec_scale_position(tecu: f64) -> f32 {
    let segments = TEC_SCALE.len().saturating_sub(1);
    for (index, stops) in TEC_SCALE.windows(2).enumerate() {
        let (Some(&(lower, _)), Some(&(upper, _))) = (stops.first(), stops.get(1)) else {
            continue;
        };
        if tecu < upper || index.saturating_add(1) >= segments {
            let span = upper - lower;
            let within = if span > 0.0 {
                ((tecu - lower) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let segment_count = if segments == 0 { 1.0 } else { segments as f64 };
            return ((index as f64 + within) / segment_count) as f32;
        }
    }
    0.0
}

/// The themed colour at `position` on the heatmap's ramp, as
/// [`tec_scale_position`] measures it.
pub fn tec_color_at_position(position: f32) -> ThemedColor {
    let segments = TEC_SCALE.len().saturating_sub(1);
    let scaled = f64::from(position.clamp(0.0, 1.0)) * segments as f64;
    let index = usize::try_from(scaled.floor() as i64)
        .unwrap_or(0)
        .min(segments.saturating_sub(1));
    let fraction = (scaled - index as f64) as f32;
    let (Some(&(_, from)), Some(&(_, to))) =
        (TEC_SCALE.get(index), TEC_SCALE.get(index.saturating_add(1)))
    else {
        return TEC_SCALE_BOTTOM;
    };
    ThemedColor::new(
        from.dark().lerp_to_gamma(to.dark(), fraction),
        from.light().lerp_to_gamma(to.light(), fraction),
    )
}

/// The themed fill for a grid node holding `tecu` TEC units.
pub fn tec_color(tecu: f64) -> ThemedColor {
    tec_color_at_position(tec_scale_position(tecu))
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
/// The tier is a semantic step, decoupled from both the count thresholds (see
/// [`fix_count_tier`] / [`seen_count_tier`]) and the concrete hues (see
/// [`SatCountTier::themed_color`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumCount)]
pub enum SatCountTier {
    /// Plenty of satellites: a healthy fix.
    Good,
    /// Enough to fix, but with little margin.
    Fair,
    /// Marginal: a fix is degraded or fragile.
    Poor,
    /// Too few to be reliable: no fix or a lone satellite.
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
    ("LOG_LIVE_FILTER", LOG_LIVE_FILTER),
];

/// The query editor's syntax-highlight colours. Checked against the editor
/// background (the theme's `extreme_bg_color`) rather than the panel fill.
pub const QUERY_SYNTAX_COLORS: &[(&str, ThemedColor)] = &[
    ("KEYWORD", QUERY_SYNTAX_KEYWORD),
    ("NUMBER", QUERY_SYNTAX_NUMBER),
    ("IDENT", QUERY_SYNTAX_IDENT),
    ("COMMENT", QUERY_SYNTAX_COMMENT),
];

/// Linearly interpolates a colour channel from `a` toward `b` by `num/den`
/// (where `0 <= num <= den`), clamped to `[0, 255]`.
pub fn lerp_channel(a: u8, b: u8, num: i32, den: i32) -> u8 {
    let a = i32::from(a);
    let b = i32::from(b);
    let value = a + (b - a) * num / den;
    u8::try_from(value.clamp(0, 255)).unwrap_or(0)
}

/// Maps a unit-interval value `[0, 1]` to a `u8` colour or alpha channel,
/// clamping out of range and rounding to the nearest integer. Routes through
/// `i32` like [`lerp_channel`] so the float never casts straight to an unsigned
/// integer.
pub fn unit_to_u8(value: f32) -> u8 {
    let scaled = (value.clamp(0.0, 1.0) * 255.0).round() as i32;
    u8::try_from(scaled.clamp(0, 255)).unwrap_or(0)
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

/// Alpha the match halos are drawn at while a new run's reveal animation is at
/// its brightest, before settling back to each layer colour's own alpha.
pub const QUERY_MATCH_REVEAL_PEAK_ALPHA: u8 = 230;

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

/// Alpha the query results table paints the magnitude bar behind a value cell
/// at: enough tint for the bar's length to read at a glance, faint enough for
/// the number over it to stay legible on either theme.
pub const QUERY_VALUE_BAR_ALPHA: u8 = 64;

/// `color` as the magnitude bar behind a value cell of the query results
/// table. Two draw queries paint bars of different colours: the caller passes
/// the halo colour of the query that matched, or the theme's text colour for a
/// query that draws no halo.
pub fn query_value_bar_color(color: Color32) -> Color32 {
    let [red, green, blue, _] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(red, green, blue, QUERY_VALUE_BAR_ALPHA)
}

/// Query editor syntax highlighting: keywords (`points`, `where`, `and`, …).
///
/// The code editor uses the theme's `extreme_bg_color` (near-black on dark,
/// white on light), so each token colour (this and its siblings below) carries
/// a light variant deepened enough to read on white, like the rest of the
/// themed palette.
pub const QUERY_SYNTAX_KEYWORD: ThemedColor = ThemedColor::new(
    Color32::from_rgb(198, 120, 221),
    Color32::from_rgb(137, 42, 168),
);

/// Numeric literals.
pub const QUERY_SYNTAX_NUMBER: ThemedColor = ThemedColor::new(
    Color32::from_rgb(229, 192, 123),
    Color32::from_rgb(140, 96, 18),
);

/// Metric, unit, and parameter names.
pub const QUERY_SYNTAX_IDENT: ThemedColor = ThemedColor::new(
    Color32::from_rgb(120, 200, 255),
    Color32::from_rgb(20, 110, 190),
);

/// Comments.
pub const QUERY_SYNTAX_COMMENT: ThemedColor = ThemedColor::new(
    Color32::from_rgb(128, 148, 128),
    Color32::from_rgb(92, 110, 92),
);

/// The identifier syntax colour for the given theme. Pass `ui.visuals().dark_mode`.
pub const fn query_syntax_ident(dark_mode: bool) -> Color32 {
    QUERY_SYNTAX_IDENT.resolve(dark_mode)
}

/// The keyword syntax colour for the given theme. Pass `ui.visuals().dark_mode`.
pub const fn query_syntax_keyword(dark_mode: bool) -> Color32 {
    QUERY_SYNTAX_KEYWORD.resolve(dark_mode)
}

/// The numeric-literal syntax colour for the given theme. Pass `ui.visuals().dark_mode`.
pub const fn query_syntax_number(dark_mode: bool) -> Color32 {
    QUERY_SYNTAX_NUMBER.resolve(dark_mode)
}

/// The comment syntax colour for the given theme. Pass `ui.visuals().dark_mode`.
pub const fn query_syntax_comment(dark_mode: bool) -> Color32 {
    QUERY_SYNTAX_COMMENT.resolve(dark_mode)
}

/// The themed plot-line colour for a metric series.
///
/// The dark variant is the vivid palette tuned for the near-black plot canvas.
/// On the light canvas every hue has to be dark enough to read, which flattens
/// the brightness axis that separates the families in dark mode, so the light
/// variant re-encodes that separation as *depth*: within a constellation the
/// `fix` line is darkest, `seen` mid, and `util`/`slip` sit at their own depths.
/// Hue is preserved, so the constellation colour-coding survives both themes.
/// The light values are contrast-tuned against [`PLOT_CANVAS_LIGHT`] and, with
/// the whole set, checked by the crate's contrast test via `MetricKind`
/// iteration.
pub const fn metric_themed_color(kind: gt_types::MetricKind) -> ThemedColor {
    use gt_types::MetricKind as M;
    // Hue names describe the dark variant. The light variant is the same hue
    // deepened for the light canvas. (dark, light):
    let (dark, light) = match kind {
        M::SatsSeen => ((80, 200, 255), (52, 131, 168)), // powder blue
        M::SatsFix => ((0, 100, 220), (0, 81, 180)),     // deep blue
        M::GpsSeen => ((0, 220, 80), (0, 140, 51)),      // lime green
        M::GpsFix => ((0, 140, 40), (0, 100, 28)),       // forest green
        M::GlonassSeen => ((255, 140, 30), (183, 100, 21)), // golden
        M::GlonassFix => ((200, 80, 0), (143, 57, 0)),   // amber
        M::GalileoSeen => ((255, 50, 110), (229, 44, 98)), // hot pink
        M::GalileoFix => ((155, 30, 255), (123, 23, 203)), // purple
        M::BeidouSeen => ((0, 230, 230), (0, 137, 137)), // cyan
        M::BeidouFix => ((0, 160, 160), (0, 95, 95)),    // teal
        M::NavicSeen => ((160, 120, 255), (137, 103, 219)), // violet
        M::NavicFix => ((110, 70, 210), (94, 60, 180)),  // deep violet
        M::QzssSeen => ((240, 110, 90), (196, 90, 73)),  // coral
        M::QzssFix => ((190, 60, 45), (151, 47, 35)),    // brick red
        M::Velocity => ((255, 220, 0), (142, 123, 0)),   // bright yellow
        M::Eph => ((220, 20, 220), (215, 19, 215)),      // magenta
        M::HeadingDeg => ((255, 100, 50), (209, 81, 40)), // red-orange
        M::ClockDeltaMs => ((200, 200, 200), (123, 123, 123)), // light gray
        // Utilization echoes each constellation's hue, lightened in dark mode.
        M::UtilAll => ((245, 245, 245), (122, 122, 122)), // near-white
        M::UtilGps => ((150, 255, 150), (74, 127, 74)),   // pale green
        M::UtilGlonass => ((255, 200, 130), (142, 111, 72)), // pale orange
        M::UtilGalileo => ((255, 150, 190), (163, 95, 121)), // pale pink
        M::UtilBeidou => ((150, 245, 245), (74, 122, 122)), // pale cyan
        M::UtilNavic => ((205, 185, 255), (122, 110, 152)), // pale violet
        M::UtilQzss => ((255, 185, 170), (147, 107, 98)), // pale coral
        // Slip rate uses a hot "warning" palette, distinct hue per constellation.
        M::SlipAll => ((255, 80, 80), (219, 68, 68)), // bright red
        M::SlipGps => ((255, 150, 60), (152, 89, 35)), // orange
        M::SlipGlonass => ((230, 200, 70), (119, 103, 36)), // amber-yellow
        M::SlipGalileo => ((235, 110, 200), (159, 74, 135)), // magenta
        M::SlipBeidou => ((120, 190, 235), (69, 110, 136)), // steel blue
        M::SlipNavic => ((170, 140, 245), (112, 92, 161)), // violet
        M::SlipQzss => ((245, 140, 110), (151, 86, 68)), // salmon
        // Mint green: distinct from Eph's magenta, which it is designed to
        // overlay (claimed accuracy vs. observed road deviation).
        M::SnapError => ((60, 255, 180), (0, 143, 94)),
        M::Jamming => ((255, 105, 60), (188, 62, 20)), // rust orange
        // One hue for the geomagnetic pair, split by depth the way a
        // constellation's seen/fix lines are: Hp30 is the brighter of the two.
        M::Hp30 => ((205, 245, 95), (92, 118, 20)), // chartreuse
        M::Kp => ((150, 190, 45), (60, 80, 8)),     // olive
        M::Tec => ((140, 180, 255), (58, 92, 190)), // periwinkle
    };
    ThemedColor::new(
        Color32::from_rgb(dark.0, dark.1, dark.2),
        Color32::from_rgb(light.0, light.1, light.2),
    )
}

/// The plot-line colour for a metric series in the current theme. Pass
/// `ui.visuals().dark_mode`.
pub const fn metric_color(kind: gt_types::MetricKind, dark_mode: bool) -> Color32 {
    metric_themed_color(kind).resolve(dark_mode)
}

/// The plot canvas fill on a light theme: a faint grey that keeps the deepened
/// light-variant series lines (see [`metric_themed_color`]) separated from the
/// background.
pub const PLOT_CANVAS_LIGHT: Color32 = Color32::from_gray(232);

/// The colour reserved for a log's live filter: the highlight over a matched
/// term in the viewer's table, and the glyphs its matches draw on the map.
///
/// Held apart from the layer slots below: a warm gold, which no filter already
/// added ever draws in.
pub const LOG_LIVE_FILTER: ThemedColor = ThemedColor::new(
    Color32::from_rgb(255, 212, 121),
    Color32::from_rgb(138, 109, 31),
);

/// The colours a log's layer chips draw in, indexed by the palette slot each
/// chip holds.
///
/// Magenta through rose. The hues are clear of the track blues and cyans and of
/// the semantic amber, red and green: a log layer never reads as a warning, an
/// error or a track.
pub const LOG_LAYER_SLOTS: [ThemedColor; 5] = [
    // Magenta.
    ThemedColor::new(
        Color32::from_rgb(230, 79, 200),
        Color32::from_rgb(164, 37, 138),
    ),
    // Orange.
    ThemedColor::new(
        Color32::from_rgb(255, 158, 44),
        Color32::from_rgb(178, 95, 0),
    ),
    // Leaf green, kept apart from the affirmative SUCCESS_GREEN.
    ThemedColor::new(
        Color32::from_rgb(139, 195, 74),
        Color32::from_rgb(78, 122, 27),
    ),
    // Violet.
    ThemedColor::new(
        Color32::from_rgb(179, 136, 255),
        Color32::from_rgb(106, 63, 196),
    ),
    // Rose, kept apart from the ERROR_INDICATOR red.
    ThemedColor::new(
        Color32::from_rgb(255, 90, 122),
        Color32::from_rgb(189, 34, 69),
    ),
];

/// Outline tone of the map's log hexagons. One dark tone separates a glyph
/// from the track line and the tiles on either theme: the background here is
/// the map's tiles, not the app's.
pub const LOG_HEXAGON_OUTLINE: Color32 = Color32::from_gray(24);

/// The colour of layer palette slot `slot_index`. Slots past the last one cycle
/// the palette, as the sixth and later layer chip of a session do.
#[expect(
    clippy::indexing_slicing,
    reason = "the slot is reduced modulo the palette length, so always in bounds"
)]
pub fn log_layer_slot_color(slot_index: usize) -> ThemedColor {
    LOG_LAYER_SLOTS[slot_index % LOG_LAYER_SLOTS.len()]
}

/// The colour one of a log's filters draws in, on the map and in the viewer
/// alike.
pub fn log_match_color(color: gt_ui_types::LogMatchColor, dark_mode: bool) -> Color32 {
    match color {
        gt_ui_types::LogMatchColor::LiveFilter => LOG_LIVE_FILTER.resolve(dark_mode),
        gt_ui_types::LogMatchColor::LayerSlot { index, .. } => {
            log_layer_slot_color(index).resolve(dark_mode)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A marker takes the colour of the class its flux falls in, so the
    /// colour steps exactly at the class boundaries.
    #[rstest::rstest]
    #[case::an_a_class(1e-8, FLARE_BELOW_C_CLASS)]
    #[case::just_below_c(9.9e-7, FLARE_BELOW_C_CLASS)]
    #[case::the_c_floor(FLARE_C_CLASS_FLUX, FLARE_C_CLASS)]
    #[case::the_m_floor(FLARE_M_CLASS_FLUX, FLARE_M_CLASS)]
    #[case::the_x_floor(FLARE_X_CLASS_FLUX, FLARE_X_CLASS)]
    #[case::an_x28(2.8e-3, FLARE_X_CLASS)]
    fn a_flare_marker_takes_the_colour_of_its_class(
        #[case] peak_flux_watts_per_square_meter: f64,
        #[case] expected: ThemedColor,
    ) {
        assert_eq!(
            solar_flare_color(peak_flux_watts_per_square_meter),
            expected
        );
    }

    /// How far a channel may move as a colour round-trips through the
    /// premultiplied storage [`Color32`] keeps: one step of the bar alpha,
    /// which is a quarter of full opacity.
    const BAR_TINT_ROUNDING: u8 = 4;

    /// A value bar keeps the hue of the halo it was tinted from, at the alpha
    /// the table paints its bars with.
    #[test]
    fn a_value_bar_keeps_its_halo_hue_at_the_bar_alpha() {
        let halo = query_halo_color(0, false);
        let [halo_red, halo_green, halo_blue, _] = halo.to_srgba_unmultiplied();
        let [red, green, blue, alpha] = query_value_bar_color(halo).to_srgba_unmultiplied();
        assert_eq!(alpha, QUERY_VALUE_BAR_ALPHA);
        for (channel, bar, halo) in [
            ("red", red, halo_red),
            ("green", green, halo_green),
            ("blue", blue, halo_blue),
        ] {
            assert!(
                bar.abs_diff(halo) <= BAR_TINT_ROUNDING,
                "{channel}: bar {bar}, halo {halo}"
            );
        }
    }

    /// Every legend tick is a stop of the ramp, so a label always sits on a
    /// colour the scale actually holds.
    #[test]
    fn every_legend_tick_is_a_colour_stop() {
        for tick in TEC_LEGEND_TICKS_TECU {
            assert!(
                TEC_SCALE
                    .iter()
                    .any(|&(tecu, _)| (tecu - tick).abs() < f64::EPSILON),
                "{tick} TECU is not a stop"
            );
        }
    }

    /// The ramp runs from the bottom stop to the top one, and a value past the
    /// top takes the top colour.
    #[rstest::rstest]
    #[case::the_bottom(0.0, 0.0)]
    #[case::the_second_stop(5.0, 1.0 / 7.0)]
    #[case::halfway_through_a_segment(15.0, 2.5 / 7.0)]
    #[case::the_top(320.0, 1.0)]
    #[case::past_the_top(1_000.0, 1.0)]
    #[case::below_the_bottom(-5.0, 0.0)]
    fn a_value_sits_where_the_stops_put_it(#[case] tecu: f64, #[case] expected: f32) {
        let position = tec_scale_position(tecu);
        assert!(
            (position - expected).abs() < 1e-6,
            "{tecu} TECU sits at {position}, expected {expected}"
        );
    }

    /// Storm values must stay apart from each other and from the quiet range,
    /// which a scale saturating below them would not do.
    #[test]
    fn storm_values_keep_their_own_colours() {
        let quiet = tec_color(15.0).dark();
        let elevated = tec_color(100.0).dark();
        let storm = tec_color(175.0).dark();
        let extreme = tec_color(250.0).dark();
        for (left, right) in [(quiet, elevated), (elevated, storm), (storm, extreme)] {
            assert_ne!(left, right);
        }
    }

    /// The ramp is continuous: neighbouring values cannot jump a whole stop.
    #[test]
    fn the_ramp_moves_smoothly_across_its_stops() {
        let channel_distance = |left: Color32, right: Color32| {
            i32::from(left.r()).abs_diff(i32::from(right.r()))
                + i32::from(left.g()).abs_diff(i32::from(right.g()))
                + i32::from(left.b()).abs_diff(i32::from(right.b()))
        };
        let mut previous = tec_color(0.0).dark();
        for step in 1..=320 {
            let color = tec_color(f64::from(step)).dark();
            assert!(
                channel_distance(previous, color) <= 120,
                "{step} TECU jumps from {previous:?} to {color:?}"
            );
            previous = color;
        }
    }

    #[rstest::rstest]
    #[case::transparent(0.0, 0)]
    #[case::the_default(TEC_OPACITY_PERCENT_DEFAULT, 102)]
    #[case::opaque(100.0, 255)]
    #[case::above_the_range(400.0, 255)]
    fn the_fill_alpha_follows_the_opacity_percentage(#[case] percent: f32, #[case] expected: u8) {
        assert_eq!(tec_fill_alpha(percent), expected);
    }

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
        for constellation in gt_types::satellites::Constellation::iter() {
            all.push((
                format!("constellation::{constellation:?}"),
                constellation_themed_color(constellation),
            ));
        }
        for (slot, color) in LOG_LAYER_SLOTS.iter().enumerate() {
            all.push((format!("LOG_LAYER_SLOTS[{slot}]"), *color));
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
    fn plot_metric_colors_are_legible_on_the_canvas() {
        use strum::IntoEnumIterator;

        // The plot canvas, not the panel: near-black in dark mode, faint grey
        // (PLOT_CANVAS_LIGHT) in light mode. Series lines are graphical objects,
        // so the 3.0 non-text bar applies.
        let dark_canvas = egui::Visuals::dark().extreme_bg_color;
        let mut failures = Vec::new();
        for kind in gt_types::MetricKind::iter() {
            let color = metric_themed_color(kind);
            let d = contrast_ratio(color.dark(), dark_canvas);
            if d < MIN_CONTRAST {
                failures.push(format!("{kind:?}.dark on canvas: {d:.2} < {MIN_CONTRAST}"));
            }
            let l = contrast_ratio(color.light(), PLOT_CANVAS_LIGHT);
            if l < MIN_CONTRAST {
                failures.push(format!("{kind:?}.light on canvas: {l:.2} < {MIN_CONTRAST}"));
            }
        }
        assert!(
            failures.is_empty(),
            "illegible plot series colours:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn query_syntax_colors_are_legible_in_the_editor() {
        // The code editor uses the theme's extreme_bg_color: near-black on dark,
        // white on light. Syntax highlighting is small text, so hold it to the
        // stricter 4.5 body-text bar.
        const EDITOR_MIN: f64 = 4.5;
        let dark_bg = egui::Visuals::dark().extreme_bg_color;
        let light_bg = egui::Visuals::light().extreme_bg_color;
        let mut failures = Vec::new();
        for (name, color) in QUERY_SYNTAX_COLORS {
            let d = contrast_ratio(color.dark(), dark_bg);
            if d < EDITOR_MIN {
                failures.push(format!("{name}.dark: {d:.2} < {EDITOR_MIN}"));
            }
            let l = contrast_ratio(color.light(), light_bg);
            if l < EDITOR_MIN {
                failures.push(format!("{name}.light: {l:.2} < {EDITOR_MIN}"));
            }
        }
        assert!(
            failures.is_empty(),
            "illegible query syntax colours:\n{}",
            failures.join("\n")
        );
    }

    /// The sixth layer chip of a session takes a slot past the palette, and
    /// draws in the first colour again.
    #[test]
    fn a_layer_slot_past_the_palette_cycles_it() {
        assert_eq!(log_layer_slot_color(0), log_layer_slot_color(5));
        assert_eq!(log_layer_slot_color(1), log_layer_slot_color(6));
        assert_ne!(log_layer_slot_color(0), log_layer_slot_color(1));
    }

    /// A layer colour must not be mistakable for a warning, an error or a
    /// track, all of which the map draws alongside the log's hexagons.
    #[test]
    fn no_layer_colour_repeats_a_semantic_or_track_colour() {
        for color in LOG_LAYER_SLOTS {
            assert!(!TRACK_COLORS.contains(&color.dark()));
            assert_ne!(color.dark(), WARNING_AMBER);
            assert_ne!(color.dark(), ERROR_INDICATOR);
            assert_ne!(color.dark(), SUCCESS_GREEN);
        }
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

    #[test]
    fn unit_to_u8_maps_and_clamps_the_unit_interval() {
        assert_eq!(unit_to_u8(0.0), 0);
        assert_eq!(unit_to_u8(1.0), 255);
        assert_eq!(unit_to_u8(0.5), 128, "rounds to nearest");
        // Out of range clamps rather than wrapping.
        assert_eq!(unit_to_u8(-1.0), 0);
        assert_eq!(unit_to_u8(2.0), 255);
    }
}
