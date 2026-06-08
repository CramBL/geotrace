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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedMarkerKind {
    GpsFixLost,
    GpsFixRegained,
}

impl std::fmt::Display for GeneratedMarkerKind {
    /// Canonical human-readable label - call sites should format through this
    /// rather than re-typing it, so "GPS"/"GNSS"/"Fix" wording can't drift
    /// out of sync across the side panel, map tooltips, sticky info card, and
    /// test fixtures the way it previously did (each had picked a different
    /// one of the three).
    ///
    /// `GpsFixRegained`'s tooltip additionally appends a measured duration
    /// ("... after 3.2s"); that's runtime data, so it stays a call-site
    /// concern layered on top of this base label rather than living here.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::GpsFixLost => "GNSS fix lost",
            Self::GpsFixRegained => "GNSS fix regained",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GeneratedMarker {
    pub time: DateTime<Utc>,
    pub kind: GeneratedMarkerKind,
    pub lat: Latitude,
    pub lon: Longitude,
    /// For `GpsFixRegained`: how long the fix was lost. None for `GpsFixLost`.
    pub fix_lost_duration: Option<Duration>,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

#[cfg(test)]
mod generated_marker_kind_tests {
    use super::*;

    /// Single source of truth for the "GPS"/"GNSS"/"Fix" wording question
    /// that every call site previously answered independently (and
    /// inconsistently - three different spellings across five copies). Pin
    /// it down so a future edit has to change it here, where every
    /// downstream label, tooltip, and test fixture will pick it up.
    #[test]
    fn label_is_canonical_wording() {
        assert_eq!(GeneratedMarkerKind::GpsFixLost.to_string(), "GNSS fix lost");
        assert_eq!(
            GeneratedMarkerKind::GpsFixRegained.to_string(),
            "GNSS fix regained"
        );
    }
}

impl GeneratedMarker {
    pub fn new(
        time: DateTime<Utc>,
        kind: GeneratedMarkerKind,
        lat: Latitude,
        lon: Longitude,
        fix_lost_duration: Option<Duration>,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            kind,
            lat,
            lon,
            fix_lost_duration,
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
