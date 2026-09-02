//! The channel write path: the unit mode, the channel input, and unit parsing.

use std::ffi::{CStr, c_char};

use geotrace_sdk::{ChannelUnit, UnitParseError};

use crate::error::{self, GtdStatus};
use crate::{GtdOptF64, GtdTimestamp};

/// How a channel unit label should be interpreted on the write path.
///
/// A recognized unit has a physical quantity and a conversion factor, so a
/// GeoTrace query compares it against literals in any unit of the same
/// quantity. A custom unit is a label the catalog does not cover. It is stored
/// and shown verbatim, and its values stay unitless in queries. A file may also
/// hold a legacy label that is neither (see @ref gtd_nav_file_get_channel_unit):
/// it is readable but not writable, so neither mode accepts it.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdChannelUnitMode {
    /// Validate as a recognized, convertible unit.
    GTD_CHANNEL_UNIT_RECOGNIZED = 0,
    /// Preserve as display-only: queries treat values as unitless.
    GTD_CHANNEL_UNIT_CUSTOM = 1,
}

impl GtdChannelUnitMode {
    pub(crate) fn from_abi_value(unit_mode: u32) -> Option<Self> {
        match unit_mode {
            0 => Some(Self::GTD_CHANNEL_UNIT_RECOGNIZED),
            1 => Some(Self::GTD_CHANNEL_UNIT_CUSTOM),
            _ => None,
        }
    }

    pub(crate) fn parse_label(self, label: &str) -> Result<ChannelUnit, UnitParseError> {
        match self {
            Self::GTD_CHANNEL_UNIT_RECOGNIZED => label.parse(),
            Self::GTD_CHANNEL_UNIT_CUSTOM => ChannelUnit::custom(label),
        }
    }
}

/// A scalar or vector channel to add via `gtd_builder_add_channel()`.
///
/// A scalar channel leaves @ref components NULL and @ref n_components zero. A
/// vector channel points @ref components at @ref n_components label strings.
/// @ref values is row-major: @ref n_times rows of one column (scalar) or
/// @ref n_components columns (vector), so @ref n_values must equal
/// `n_times * (n_components > 0 ? n_components : 1)`.
///
/// Only a channel with @ref period_deg set wraps: a `deg` channel without it
/// holds an unbounded angle.
#[repr(C)]
pub struct GtdChannel {
    /// Channel name (a lowercase identifier).
    pub name: *const c_char,
    /// Unit of the values, or NULL. See @ref GtdChannelUnitMode.
    pub unit: *const c_char,
    /// Wrap period in degrees for an angular channel, or `GTD_NONE_F64`.
    pub period_deg: GtdOptF64,
    /// Human-readable description, or NULL.
    pub description: *const c_char,
    /// Component labels for a vector channel, or NULL for scalar.
    pub components: *const *const c_char,
    /// Number of component labels (0 = scalar channel).
    pub n_components: usize,
    /// Sample timestamps, one per row.
    pub times: *const GtdTimestamp,
    /// Number of timestamps.
    pub n_times: usize,
    /// Row-major values, `n_times * max(n_components, 1)` of them.
    pub values: *const f64,
    /// Number of values.
    pub n_values: usize,
}

/// Validate and canonicalize a channel unit label.
///
/// Call with @p out NULL and @p out_capacity zero to query the required byte
/// length, including the terminating NUL, then call again with a large enough
/// buffer. Validation and Unicode handling are identical to the Rust SDK.
///
/// Under `GTD_CHANNEL_UNIT_RECOGNIZED` the label is trimmed and aliases are
/// resolved, so `"kph"`, `"degrees"` and `"m/s²"` come back as `"km/h"`,
/// `"deg"` and `"m/s2"`. Under `GTD_CHANNEL_UNIT_CUSTOM` the label is only
/// trimmed, and a label that names a recognized unit is rejected: it belongs in
/// `GTD_CHANNEL_UNIT_RECOGNIZED`, which keeps its conversion factor.
///
/// @param label        Unit label to validate, NUL-terminated UTF-8.
/// @param unit_mode    A @ref GtdChannelUnitMode value.
/// @param out          Buffer for the canonical label, or NULL to size it.
/// @param out_capacity Bytes writable at @p out.
/// @param required_len Receives the canonical byte length including the NUL.
///
/// @return `GTD_ERR_INVALID_CHANNEL` if the label is invalid for @p unit_mode or
///         @p unit_mode is not a @ref GtdChannelUnitMode, `GTD_ERR_UTF8` if
///         @p label is not UTF-8, `GTD_ERR_NULL_ARGUMENT` if @p out is too small
///         for the canonical label.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_channel_unit_parse(
    label: *const c_char,
    unit_mode: u32,
    out: *mut c_char,
    out_capacity: usize,
    required_len: *mut usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        if label.is_null() || required_len.is_null() {
            error::set_last_error("null pointer argument");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: label is non-null and must point to a NUL-terminated string.
        let label = match unsafe { CStr::from_ptr(label) }.to_str() {
            Ok(label) => label,
            Err(error) => {
                error::set_last_error(error);
                return GtdStatus::GTD_ERR_UTF8;
            }
        };
        let Some(unit_mode) = GtdChannelUnitMode::from_abi_value(unit_mode) else {
            error::set_last_error("unit_mode is not a valid GtdChannelUnitMode");
            return GtdStatus::GTD_ERR_INVALID_CHANNEL;
        };
        let unit = match unit_mode.parse_label(label) {
            Ok(unit) => unit,
            Err(error) => {
                error::set_last_error(error);
                return GtdStatus::GTD_ERR_INVALID_CHANNEL;
            }
        };
        let bytes = unit.label().as_bytes();
        // SAFETY: `required_len` is non-null and writable by the caller.
        unsafe { *required_len = bytes.len().saturating_add(1) };
        if out.is_null() || out_capacity == 0 {
            return GtdStatus::GTD_OK;
        }
        if out_capacity <= bytes.len() {
            error::set_last_error("channel unit output buffer is too small");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: out points to `out_capacity` bytes and the capacity was checked.
        // SAFETY: out points to at least bytes.len() writable bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len()) };
        // SAFETY: `out_capacity` is greater than bytes.len().
        let terminator = unsafe { out.add(bytes.len()) };
        // SAFETY: terminator points within the caller's writable buffer.
        unsafe { *terminator = 0 };
        GtdStatus::GTD_OK
    })
}
