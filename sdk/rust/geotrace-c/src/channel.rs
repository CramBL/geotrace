//! The channel write path of `geotrace.h`, declared in its `satellite` group.

use std::ffi::{CStr, c_char};

use geotrace_sdk::ChannelUnit;

use crate::error::{self, GtdStatus};
use crate::{GtdOptF64, GtdTimestamp};

/// A channel to add via `gtd_builder_add_channel` (write path, input from C).
///
/// A scalar channel leaves `components` NULL and `n_components` zero. A vector
/// channel points `components` at `n_components` label strings. `values` is
/// row-major: `n_times` rows of one column (scalar) or `n_components` columns
/// (vector), so `n_values` must equal `n_times * max(n_components, 1)`.
#[repr(C)]
pub struct GtdChannel {
    pub name: *const c_char,
    pub unit: *const c_char,
    pub period_deg: GtdOptF64,
    pub description: *const c_char,
    pub components: *const *const c_char,
    pub n_components: usize,
    pub times: *const GtdTimestamp,
    pub n_times: usize,
    pub values: *const f64,
    pub n_values: usize,
}

/// Validate and canonicalize a channel unit label.
///
/// # Safety
///
/// `label` must point to a NUL-terminated string, `required_len` must be
/// writable, and a non-null `out` must point to `out_capacity` writable bytes.
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
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: label is non-null and must point to a NUL-terminated string.
        let label = match unsafe { CStr::from_ptr(label) }.to_str() {
            Ok(label) => label,
            Err(error) => {
                error::set_last_error(error);
                return GtdStatus::ErrUtf8;
            }
        };
        let parsed = match unit_mode {
            0 => label.parse::<ChannelUnit>(),
            1 => ChannelUnit::custom(label),
            _ => {
                error::set_last_error("unit_mode is not a valid GtdChannelUnitMode");
                return GtdStatus::ErrInvalidChannel;
            }
        };
        let unit = match parsed {
            Ok(unit) => unit,
            Err(error) => {
                error::set_last_error(error);
                return GtdStatus::ErrInvalidChannel;
            }
        };
        let bytes = unit.label().as_bytes();
        // SAFETY: `required_len` is non-null and writable by the caller.
        unsafe { *required_len = bytes.len().saturating_add(1) };
        if out.is_null() || out_capacity == 0 {
            return GtdStatus::Ok;
        }
        if out_capacity <= bytes.len() {
            error::set_last_error("channel unit output buffer is too small");
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: out points to `out_capacity` bytes and the capacity was checked.
        // SAFETY: out points to at least bytes.len() writable bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len()) };
        // SAFETY: `out_capacity` is greater than bytes.len().
        let terminator = unsafe { out.add(bytes.len()) };
        // SAFETY: terminator points within the caller's writable buffer.
        unsafe { *terminator = 0 };
        GtdStatus::Ok
    })
}
