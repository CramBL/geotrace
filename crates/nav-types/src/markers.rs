use chrono::{DateTime, Utc};
use uom::si::f64::Angle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerIcon {
    Pin,
    Cross,
    Circle,
    Lightning,
    Warning,
    Error,
    Check,
}

#[derive(Debug, Clone)]
pub struct CustomMarker {
    pub time: DateTime<Utc>,
    pub label: String,
    pub icon: MarkerIcon,
    pub lat: Angle,
    pub lon: Angle,
}
