//! The icon a map marker is drawn with.

/// Icon for map markers. Use `GTD_ICON_AUTO` to let the application choose.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdMarkerIcon {
    /// Map pin.
    GTD_ICON_PIN = 0,
    /// Cross / X mark.
    GTD_ICON_CROSS = 1,
    /// Circle.
    GTD_ICON_CIRCLE = 2,
    /// Lightning bolt.
    GTD_ICON_LIGHTNING = 3,
    /// Warning triangle.
    GTD_ICON_WARNING = 4,
    /// Error indicator.
    GTD_ICON_ERROR = 5,
    /// Check mark.
    GTD_ICON_CHECK = 6,
    /// Satellite with signal.
    GTD_ICON_SATELLITE = 7,
    /// Satellite without signal.
    GTD_ICON_SATELLITE_LOST = 8,
    /// Gear / settings.
    GTD_ICON_GEAR = 9,
    /// Refresh / reload.
    GTD_ICON_REFRESH = 10,
    /// Download arrow.
    GTD_ICON_DOWNLOAD = 11,
    /// Upload arrow.
    GTD_ICON_UPLOAD = 12,
    /// Wrench / tool.
    GTD_ICON_WRENCH = 13,
    /// Use the application default for this variant.
    GTD_ICON_AUTO = 255,
}

impl GtdMarkerIcon {
    pub(crate) fn to_marker_icon(self) -> Option<geotrace_sdk::MarkerIcon> {
        match self {
            Self::GTD_ICON_PIN => Some(geotrace_sdk::MarkerIcon::Pin),
            Self::GTD_ICON_CROSS => Some(geotrace_sdk::MarkerIcon::Cross),
            Self::GTD_ICON_CIRCLE => Some(geotrace_sdk::MarkerIcon::Circle),
            Self::GTD_ICON_LIGHTNING => Some(geotrace_sdk::MarkerIcon::Lightning),
            Self::GTD_ICON_WARNING => Some(geotrace_sdk::MarkerIcon::Warning),
            Self::GTD_ICON_ERROR => Some(geotrace_sdk::MarkerIcon::Error),
            Self::GTD_ICON_CHECK => Some(geotrace_sdk::MarkerIcon::Check),
            Self::GTD_ICON_SATELLITE => Some(geotrace_sdk::MarkerIcon::Satellite),
            Self::GTD_ICON_SATELLITE_LOST => Some(geotrace_sdk::MarkerIcon::SatelliteLost),
            Self::GTD_ICON_GEAR => Some(geotrace_sdk::MarkerIcon::Gear),
            Self::GTD_ICON_REFRESH => Some(geotrace_sdk::MarkerIcon::Refresh),
            Self::GTD_ICON_DOWNLOAD => Some(geotrace_sdk::MarkerIcon::Download),
            Self::GTD_ICON_UPLOAD => Some(geotrace_sdk::MarkerIcon::Upload),
            Self::GTD_ICON_WRENCH => Some(geotrace_sdk::MarkerIcon::Wrench),
            Self::GTD_ICON_AUTO => None,
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
            geotrace_sdk::MarkerIcon::Pin => Self::GTD_ICON_PIN,
            geotrace_sdk::MarkerIcon::Cross => Self::GTD_ICON_CROSS,
            geotrace_sdk::MarkerIcon::Circle => Self::GTD_ICON_CIRCLE,
            geotrace_sdk::MarkerIcon::Lightning => Self::GTD_ICON_LIGHTNING,
            geotrace_sdk::MarkerIcon::Warning => Self::GTD_ICON_WARNING,
            geotrace_sdk::MarkerIcon::Error => Self::GTD_ICON_ERROR,
            geotrace_sdk::MarkerIcon::Check => Self::GTD_ICON_CHECK,
            geotrace_sdk::MarkerIcon::Satellite => Self::GTD_ICON_SATELLITE,
            geotrace_sdk::MarkerIcon::SatelliteLost => Self::GTD_ICON_SATELLITE_LOST,
            geotrace_sdk::MarkerIcon::Gear => Self::GTD_ICON_GEAR,
            geotrace_sdk::MarkerIcon::Refresh => Self::GTD_ICON_REFRESH,
            geotrace_sdk::MarkerIcon::Download => Self::GTD_ICON_DOWNLOAD,
            geotrace_sdk::MarkerIcon::Upload => Self::GTD_ICON_UPLOAD,
            geotrace_sdk::MarkerIcon::Wrench => Self::GTD_ICON_WRENCH,
        }
    }
}
