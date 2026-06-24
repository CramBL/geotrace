use crate::coordinates::{Latitude, Longitude};
use crate::mercator::MercPoint;
use chrono::{DateTime, Duration, Utc};

/// RGB fill color for an event marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl MarkerColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

const EVENT_FALLBACK_COLORS: [MarkerColor; 8] = [
    MarkerColor::new(230, 57, 70),
    MarkerColor::new(255, 149, 0),
    MarkerColor::new(255, 190, 11),
    MarkerColor::new(6, 214, 160),
    MarkerColor::new(46, 196, 182),
    MarkerColor::new(131, 56, 236),
    MarkerColor::new(255, 45, 85),
    MarkerColor::new(238, 66, 102),
];

/// Deterministic fallback color for an unstyled event marker variant.
///
/// Hashes `variant_path` into the `LOG_COLORS`-compatible palette so unstyled
/// variants still get visually distinct, consistent colors without configuration.
pub fn event_marker_fallback_color(variant_path: &str) -> MarkerColor {
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

/// An automatically-detected GNSS event, with the per-event payload carried in
/// the variant that needs it (so a `match` stays exhaustive and there are no
/// "valid only for kind X" optional fields hanging off the marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedMarkerKind {
    GnssFixLost,
    GnssFixRegained {
        /// How long the fix was lost before being regained.
        fix_lost_duration: Duration,
    },
    /// The GPS−system clock offset jumped abruptly at this sample relative to
    /// the previous one - e.g. a device resuming from suspend, where a stale
    /// pre-suspend GPS timestamp meets a post-wake system timestamp. Surfaced
    /// (never hidden) because such clock discontinuities are exactly the kind of
    /// anomaly engineers use GeoTrace to find.
    ClockDiscontinuity {
        /// Signed change in the GPS−system offset from the previous sample (the
        /// size of the jump).
        step: Duration,
    },
}

impl std::fmt::Display for GeneratedMarkerKind {
    /// Canonical human-readable label. Format through this rather than
    /// re-typing the wording at each call site.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::GnssFixLost => "GNSS fix lost",
            Self::GnssFixRegained { .. } => "GNSS fix regained",
            Self::ClockDiscontinuity { .. } => "Clock discontinuity",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GeneratedMarker {
    pub time: DateTime<Utc>,
    pub kind: GeneratedMarkerKind,
    pub lat: Latitude,
    pub lon: Longitude,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

#[cfg(test)]
mod generated_marker_kind_tests {
    use super::*;

    /// Single source of truth for the "GPS"/"GNSS"/"Fix" wording. Pin it down
    /// so a future edit has to change it here, where every downstream label,
    /// tooltip, and test fixture will pick it up.
    #[test]
    fn label_is_canonical_wording() {
        assert_eq!(
            GeneratedMarkerKind::GnssFixLost.to_string(),
            "GNSS fix lost"
        );
        assert_eq!(
            GeneratedMarkerKind::GnssFixRegained {
                fix_lost_duration: Duration::zero()
            }
            .to_string(),
            "GNSS fix regained"
        );
        assert_eq!(
            GeneratedMarkerKind::ClockDiscontinuity {
                step: Duration::zero()
            }
            .to_string(),
            "Clock discontinuity"
        );
    }
}

impl GeneratedMarker {
    pub fn new(
        time: DateTime<Utc>,
        kind: GeneratedMarkerKind,
        lat: Latitude,
        lon: Longitude,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            kind,
            lat,
            lon,
            merc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum MarkerIcon {
    Pin,
    Cross,
    Circle,
    Lightning,
    Warning,
    Error,
    Check,
    Log,
    Satellite,
    SatelliteLost,
    Gear,
    Refresh,
    Download,
    Upload,
    Wrench,
}

#[derive(Debug, Clone)]
pub struct CustomMarker {
    pub time: DateTime<Utc>,
    pub label: String,
    pub icon: MarkerIcon,
    pub lat: Latitude,
    pub lon: Longitude,
    pub color_group: Option<u32>,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

impl CustomMarker {
    pub fn new(
        time: DateTime<Utc>,
        label: String,
        icon: MarkerIcon,
        lat: Latitude,
        lon: Longitude,
        color_group: Option<u32>,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            label,
            icon,
            lat,
            lon,
            color_group,
            merc,
        }
    }
}

/// File-level icon and color override for one event marker variant path.
#[derive(Debug, Clone)]
pub struct EventMarkerStyle {
    pub variant_path: String,
    /// Icon shape for this variant.
    pub icon: MarkerIcon,
    /// Fill color.
    pub color: MarkerColor,
}

/// A single event marker instance placed on the map.
#[derive(Debug, Clone)]
pub struct EventMarker {
    pub time: DateTime<Utc>,
    pub variant_path: String,
    pub annotation: Option<String>,
    pub lat: Latitude,
    pub lon: Longitude,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

impl EventMarker {
    pub fn new(
        time: DateTime<Utc>,
        variant_path: String,
        annotation: Option<String>,
        lat: Latitude,
        lon: Longitude,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            variant_path,
            annotation,
            lat,
            lon,
            merc,
        }
    }
}
