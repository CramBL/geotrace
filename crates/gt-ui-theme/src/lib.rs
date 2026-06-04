use egui::Color32;

// UI symbol constants

/// U+2014 EM DASH - used as a placeholder when a value is absent.
pub const EM_DASH: &str = "—";
/// U+2212 MINUS SIGN - visually distinct from the ASCII hyphen-minus.
pub const MINUS_SIGN: &str = "−";
/// U+0394 GREEK CAPITAL LETTER DELTA - used as a mathematical difference symbol.
pub const DELTA: &str = "Δ";
/// U+00B0 DEGREE SIGN.
pub const DEGREE_SIGN: &str = "°";

// Semantic single-use tokens

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

// Track palette

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

// Signal quality colors

/// Map a [`SignalQuality`] tier to a colour on a green → red gradient.
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

// Log marker palette

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
