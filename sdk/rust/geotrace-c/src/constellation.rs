//! The GNSS constellation identifier.

use std::ffi::c_char;

use strum::FromRepr;

use crate::error::{self, GtdStatus};

/// GNSS constellation identifier.
/// cbindgen:rename-all=QualifiedScreamingSnakeCase
#[repr(C)]
#[derive(Clone, Copy, FromRepr)]
pub enum GtdConstellation {
    /// GPS (USA).
    Gps = 0,
    /// GLONASS (Russia).
    Glonass = 1,
    /// Galileo (EU).
    Galileo = 2,
    /// BeiDou (China).
    Beidou = 3,
    /// NavIC / IRNSS (India).
    Navic = 4,
    /// QZSS (Japan).
    Qzss = 5,
}

impl GtdConstellation {
    pub(crate) fn from_abi_value(value: u32) -> Option<Self> {
        Self::from_repr(usize::try_from(value).ok()?)
    }
}

impl From<GtdConstellation> for geotrace_sdk::Constellation {
    fn from(c: GtdConstellation) -> Self {
        match c {
            GtdConstellation::Gps => geotrace_sdk::Constellation::Gps,
            GtdConstellation::Glonass => geotrace_sdk::Constellation::Glonass,
            GtdConstellation::Galileo => geotrace_sdk::Constellation::Galileo,
            GtdConstellation::Beidou => geotrace_sdk::Constellation::Beidou,
            GtdConstellation::Navic => geotrace_sdk::Constellation::Navic,
            GtdConstellation::Qzss => geotrace_sdk::Constellation::Qzss,
        }
    }
}

impl From<geotrace_sdk::Constellation> for GtdConstellation {
    fn from(c: geotrace_sdk::Constellation) -> Self {
        match c {
            geotrace_sdk::Constellation::Gps => GtdConstellation::Gps,
            geotrace_sdk::Constellation::Glonass => GtdConstellation::Glonass,
            geotrace_sdk::Constellation::Galileo => GtdConstellation::Galileo,
            geotrace_sdk::Constellation::Beidou => GtdConstellation::Beidou,
            geotrace_sdk::Constellation::Navic => GtdConstellation::Navic,
            geotrace_sdk::Constellation::Qzss => GtdConstellation::Qzss,
        }
    }
}

/// Parse the wire name of a constellation, e.g. `"beidou"`.
///
/// @param name Wire name, NUL-terminated, lower case.
/// @param out  Caller-allocated result, written on success.
///
/// @return `GTD_ERR_PARSE` if @p name is not a known constellation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_constellation_from_name(
    name: *const c_char,
    out: *mut GtdConstellation,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let name = cstr!(name);
        let out = nonnull_mut!(out);
        match geotrace_sdk::Constellation::try_from_lower_case(name) {
            Ok(constellation) => {
                *out = constellation.into();
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
