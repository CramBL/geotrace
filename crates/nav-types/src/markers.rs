use chrono::{DateTime, Duration, Utc};
use uom::si::angle::degree;
use uom::si::f64::Angle;

const EVENT_FALLBACK_COLORS: [(u8, u8, u8); 8] = [
    (230, 57, 70),
    (255, 149, 0),
    (255, 190, 11),
    (6, 214, 160),
    (46, 196, 182),
    (131, 56, 236),
    (255, 45, 85),
    (238, 66, 102),
];

/// Deterministic fallback color for an unstyled event marker variant.
///
/// Hashes `variant_path` into the `LOG_COLORS`-compatible palette so unstyled
/// variants still get visually distinct, consistent colors without configuration.
pub fn event_marker_fallback_color(variant_path: &str) -> (u8, u8, u8) {
    let mut hash: u64 = 5381;
    for b in variant_path.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(b));
    }
    #[expect(
        clippy::indexing_slicing,
        reason = "index is computed via modulo so always in bounds"
    )]
    EVENT_FALLBACK_COLORS[hash as usize % EVENT_FALLBACK_COLORS.len()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedMarkerKind {
    GpsFixLost,
    GpsFixRegained,
}

#[derive(Debug, Clone, Copy)]
pub struct GeneratedMarker {
    pub time: DateTime<Utc>,
    pub kind: GeneratedMarkerKind,
    pub lat: Angle,
    pub lon: Angle,
    /// For `GpsFixRegained`: how long the fix was lost. None for `GpsFixLost`.
    pub fix_lost_duration: Option<Duration>,
    /// Pre-computed normalized Mercator X, see [`crate::mercator`].
    pub merc_x: f64,
    /// Pre-computed normalized Mercator Y, see [`crate::mercator`].
    pub merc_y: f64,
}

impl GeneratedMarker {
    pub fn new(
        time: DateTime<Utc>,
        kind: GeneratedMarkerKind,
        lat: Angle,
        lon: Angle,
        fix_lost_duration: Option<Duration>,
    ) -> Self {
        let (merc_x, merc_y) = crate::mercator::normalize(lon.get::<degree>(), lat.get::<degree>());
        Self {
            time,
            kind,
            lat,
            lon,
            fix_lost_duration,
            merc_x,
            merc_y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerIcon {
    Pin,
    Cross,
    Circle,
    Lightning,
    Warning,
    Error,
    Check,
    Log,
}

#[derive(Debug, Clone)]
pub struct CustomMarker {
    pub time: DateTime<Utc>,
    pub label: String,
    pub icon: MarkerIcon,
    pub lat: Angle,
    pub lon: Angle,
    pub color_group: Option<u32>,
    /// Pre-computed normalized Mercator X, see [`crate::mercator`].
    pub merc_x: f64,
    /// Pre-computed normalized Mercator Y, see [`crate::mercator`].
    pub merc_y: f64,
}

impl CustomMarker {
    pub fn new(
        time: DateTime<Utc>,
        label: String,
        icon: MarkerIcon,
        lat: Angle,
        lon: Angle,
        color_group: Option<u32>,
    ) -> Self {
        let (merc_x, merc_y) = crate::mercator::normalize(lon.get::<degree>(), lat.get::<degree>());
        Self {
            time,
            label,
            icon,
            lat,
            lon,
            color_group,
            merc_x,
            merc_y,
        }
    }
}

/// File-level icon and color override for one event marker variant path.
#[derive(Debug, Clone)]
pub struct EventMarkerStyle {
    pub variant_path: String,
    /// Icon shape for this variant.
    pub icon: MarkerIcon,
    /// Fill color as `(R, G, B)`.
    pub color: (u8, u8, u8),
}

/// A single event marker instance placed on the map.
#[derive(Debug, Clone)]
pub struct EventMarker {
    pub time: DateTime<Utc>,
    pub variant_path: String,
    pub annotation: Option<String>,
    pub lat: Angle,
    pub lon: Angle,
    /// Pre-computed normalized Mercator X, see [`crate::mercator`].
    pub merc_x: f64,
    /// Pre-computed normalized Mercator Y, see [`crate::mercator`].
    pub merc_y: f64,
}

impl EventMarker {
    pub fn new(
        time: DateTime<Utc>,
        variant_path: String,
        annotation: Option<String>,
        lat: Angle,
        lon: Angle,
    ) -> Self {
        let (merc_x, merc_y) = crate::mercator::normalize(lon.get::<degree>(), lat.get::<degree>());
        Self {
            time,
            variant_path,
            annotation,
            lat,
            lon,
            merc_x,
            merc_y,
        }
    }
}
