//! The icon a map marker is drawn with.

use std::ffi::c_char;

use strum::FromRepr;

use crate::error::{self, GtdStatus};

/// Icon for map markers. `GTD_ICON_AUTO` means the application picks the icon:
/// `gtd_builder_add_event_marker_style()` accepts it, and
/// `gtd_nav_file_get_event_marker_style()` returns it.
#[repr(C)]
#[derive(Clone, Copy, FromRepr)]
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
    /// Let the application pick the icon for an event marker variant.
    GTD_ICON_AUTO = 255,
}

impl GtdMarkerIcon {
    pub(crate) fn from_abi_value(value: u32) -> Option<Self> {
        Self::from_repr(usize::try_from(value).ok()?)
    }

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

/// Parse the wire name of a marker icon, e.g. `"satellite_lost"`.
///
/// @param name Wire name, NUL-terminated, lower `snake_case`.
/// @param out  Caller-allocated result, written on success.
///
/// @return `GTD_ERR_PARSE` if @p name is not a known marker icon,
///         `GTD_ICON_AUTO` included: it has no wire name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_marker_icon_from_name(
    name: *const c_char,
    out: *mut GtdMarkerIcon,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let name = cstr!(name);
        let out = nonnull_mut!(out);
        match geotrace_sdk::MarkerIcon::try_from_lower_case(name) {
            Ok(icon) => {
                *out = icon.into();
                GtdStatus::GTD_OK
            }
            Err(e) => {
                let status = error::status_for_error(&e);
                error::set_last_error(e);
                status
            }
        }
    })
}
