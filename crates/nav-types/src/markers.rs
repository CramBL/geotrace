use chrono::{DateTime, Duration, Utc};
use uom::si::angle::degree;
use uom::si::f64::Angle;

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
