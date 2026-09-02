//! The `travel_mode` group of `geotrace.h`: the recording platform and its wire name.

use std::ffi::{CStr, c_char};

use crate::error::{self, GtdStatus};

/// Platform a recording was made on, declared by the recorder.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdTravelMode {
    Car = 0,
    Motorcycle = 1,
    Bicycle = 2,
    Pedestrian = 3,
    Boat = 4,
    Rail = 5,
    Aircraft = 6,
}

impl From<GtdTravelMode> for geotrace_sdk::TravelMode {
    fn from(mode: GtdTravelMode) -> Self {
        match mode {
            GtdTravelMode::Car => geotrace_sdk::TravelMode::Car,
            GtdTravelMode::Motorcycle => geotrace_sdk::TravelMode::Motorcycle,
            GtdTravelMode::Bicycle => geotrace_sdk::TravelMode::Bicycle,
            GtdTravelMode::Pedestrian => geotrace_sdk::TravelMode::Pedestrian,
            GtdTravelMode::Boat => geotrace_sdk::TravelMode::Boat,
            GtdTravelMode::Rail => geotrace_sdk::TravelMode::Rail,
            GtdTravelMode::Aircraft => geotrace_sdk::TravelMode::Aircraft,
        }
    }
}

impl GtdTravelMode {
    /// The C `enum` cannot carry [`geotrace_sdk::TravelMode::Unknown`]'s
    /// preserved wire value, so unknown modes map to `None`. C callers read
    /// the raw value through `gtd_nav_file_travel_mode` instead.
    pub(crate) fn from_travel_mode(mode: &geotrace_sdk::TravelMode) -> Option<Self> {
        match mode {
            geotrace_sdk::TravelMode::Car => Some(Self::Car),
            geotrace_sdk::TravelMode::Motorcycle => Some(Self::Motorcycle),
            geotrace_sdk::TravelMode::Bicycle => Some(Self::Bicycle),
            geotrace_sdk::TravelMode::Pedestrian => Some(Self::Pedestrian),
            geotrace_sdk::TravelMode::Boat => Some(Self::Boat),
            geotrace_sdk::TravelMode::Rail => Some(Self::Rail),
            geotrace_sdk::TravelMode::Aircraft => Some(Self::Aircraft),
            geotrace_sdk::TravelMode::Unknown(_) => None,
        }
    }
}

/// Wire name of a travel mode, e.g. `GTD_TRAVEL_MODE_CAR` -> `"car"`.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_travel_mode_name(mode: GtdTravelMode) -> *const c_char {
    let name: &'static CStr = match mode {
        GtdTravelMode::Car => c"car",
        GtdTravelMode::Motorcycle => c"motorcycle",
        GtdTravelMode::Bicycle => c"bicycle",
        GtdTravelMode::Pedestrian => c"pedestrian",
        GtdTravelMode::Boat => c"boat",
        GtdTravelMode::Rail => c"rail",
        GtdTravelMode::Aircraft => c"aircraft",
    };
    name.as_ptr()
}

/// Parse a wire name (as produced by `gtd_travel_mode_name` or read from
/// `gtd_nav_file_travel_mode`) back into a travel mode.
///
/// # Safety
///
/// `name` must point to a NUL-terminated string and `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_travel_mode_from_name(
    name: *const c_char,
    out: *mut GtdTravelMode,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let name = cstr!(name);
        let out = nonnull_mut!(out);
        let mode = geotrace_sdk::TravelMode::from_lower_case(name);
        match GtdTravelMode::from_travel_mode(&mode) {
            Some(mode) => {
                *out = mode;
                GtdStatus::Ok
            }
            None => {
                error::set_last_error(format!("unknown travel mode name {name:?}"));
                GtdStatus::ErrParse
            }
        }
    })
}
