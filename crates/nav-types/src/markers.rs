use chrono::{DateTime, Duration, Utc};
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
}
