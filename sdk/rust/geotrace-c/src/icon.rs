//! The `icon` group of `geotrace.h`: the icon a map marker is drawn with.

/// Icon for map markers. GTD_ICON_AUTO (255) uses the application default.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdMarkerIcon {
    Pin = 0,
    Cross = 1,
    Circle = 2,
    Lightning = 3,
    Warning = 4,
    Error = 5,
    Check = 6,
    Satellite = 7,
    SatelliteLost = 8,
    Gear = 9,
    Refresh = 10,
    Download = 11,
    Upload = 12,
    Wrench = 13,
    Auto = 255,
}

impl GtdMarkerIcon {
    pub(crate) fn to_marker_icon(self) -> Option<geotrace_sdk::MarkerIcon> {
        match self {
            Self::Pin => Some(geotrace_sdk::MarkerIcon::Pin),
            Self::Cross => Some(geotrace_sdk::MarkerIcon::Cross),
            Self::Circle => Some(geotrace_sdk::MarkerIcon::Circle),
            Self::Lightning => Some(geotrace_sdk::MarkerIcon::Lightning),
            Self::Warning => Some(geotrace_sdk::MarkerIcon::Warning),
            Self::Error => Some(geotrace_sdk::MarkerIcon::Error),
            Self::Check => Some(geotrace_sdk::MarkerIcon::Check),
            Self::Satellite => Some(geotrace_sdk::MarkerIcon::Satellite),
            Self::SatelliteLost => Some(geotrace_sdk::MarkerIcon::SatelliteLost),
            Self::Gear => Some(geotrace_sdk::MarkerIcon::Gear),
            Self::Refresh => Some(geotrace_sdk::MarkerIcon::Refresh),
            Self::Download => Some(geotrace_sdk::MarkerIcon::Download),
            Self::Upload => Some(geotrace_sdk::MarkerIcon::Upload),
            Self::Wrench => Some(geotrace_sdk::MarkerIcon::Wrench),
            Self::Auto => None,
        }
    }

    pub(crate) fn to_icon_choice(self) -> geotrace_sdk::EventMarkerIconChoice {
        match self.to_marker_icon() {
            Some(icon) => geotrace_sdk::EventMarkerIconChoice::Icon(icon),
            None => geotrace_sdk::EventMarkerIconChoice::Auto,
        }
    }
}

impl From<geotrace_sdk::MarkerIcon> for GtdMarkerIcon {
    fn from(icon: geotrace_sdk::MarkerIcon) -> Self {
        match icon {
            geotrace_sdk::MarkerIcon::Pin => Self::Pin,
            geotrace_sdk::MarkerIcon::Cross => Self::Cross,
            geotrace_sdk::MarkerIcon::Circle => Self::Circle,
            geotrace_sdk::MarkerIcon::Lightning => Self::Lightning,
            geotrace_sdk::MarkerIcon::Warning => Self::Warning,
            geotrace_sdk::MarkerIcon::Error => Self::Error,
            geotrace_sdk::MarkerIcon::Check => Self::Check,
            geotrace_sdk::MarkerIcon::Satellite => Self::Satellite,
            geotrace_sdk::MarkerIcon::SatelliteLost => Self::SatelliteLost,
            geotrace_sdk::MarkerIcon::Gear => Self::Gear,
            geotrace_sdk::MarkerIcon::Refresh => Self::Refresh,
            geotrace_sdk::MarkerIcon::Download => Self::Download,
            geotrace_sdk::MarkerIcon::Upload => Self::Upload,
            geotrace_sdk::MarkerIcon::Wrench => Self::Wrench,
        }
    }
}
