//! The channel read path: channel metadata, samples and component labels.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::optf64;
use crate::timestamp;
use crate::{GtdOptF64, GtdTimestamp};

/// Channel metadata returned by `gtd_nav_file_get_channel()`.
///
/// Sample timestamps, values, and component labels are fetched separately with
/// `gtd_nav_file_channel_times()`, `gtd_nav_file_channel_values()`, and
/// `gtd_nav_file_get_channel_component()`. A @ref component_count of zero marks
/// a scalar channel. All string fields are null-terminated and truncated to
/// their buffer size if longer. `gtd_nav_file_get_channel_unit()` reads the unit
/// without that limit and reports whether it is a recognized unit.
///
/// Only a channel with @ref period_deg set wraps: a `deg` channel without it
/// holds an unbounded angle.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdChannelInfo {
    /// Channel name.
    pub name: [c_char; 256],
    /// Non-zero if @ref unit is set.
    pub has_unit: u8,
    /// Unit of the values, when @ref has_unit.
    pub unit: [c_char; 64],
    /// Wrap period in degrees, or absent for a linear channel.
    pub period_deg: GtdOptF64,
    /// Non-zero if @ref description is set.
    pub has_description: u8,
    /// Description, when @ref has_description.
    pub description: [c_char; 1024],
    /// Number of vector components (0 = scalar channel).
    pub component_count: usize,
    /// Number of sample timestamps (value rows).
    pub sample_count: usize,
}

/// Return the number of channels in the file.
///
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    unsafe { (*file).file.channels().len() }
}

/// Fill @p out with metadata for the channel at @p index.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than `gtd_nav_file_channel_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if @p index is out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdChannelInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(ch) = handle.file.channels().get(index) else {
            error::set_last_error(format!("channel index {index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };

        // SAFETY: GtdChannelInfo is repr(C). Zeroing it is a valid initial state.
        *out = unsafe { std::mem::zeroed() };
        super::fill_c_str(&mut out.name, ch.name());
        if let Some(unit) = ch.unit() {
            out.has_unit = 1;
            super::fill_c_str(&mut out.unit, &unit.to_string());
        }
        out.period_deg = ch.period().map_or(optf64::opt_f64_none(), |a| {
            optf64::opt_f64_some(a.as_degrees())
        });
        if let Some(description) = ch.description() {
            out.has_description = 1;
            super::fill_c_str(&mut out.description, description);
        }
        out.component_count = ch.components().len();
        out.sample_count = ch.times().len();
        GtdStatus::GTD_OK
    })
}

/// Read a channel unit without the fixed-size @ref GtdChannelInfo buffer limit.
///
/// Pass NULL @p out and zero @p out_capacity to query the required byte length,
/// including the trailing null byte. A channel without a unit reports zero.
/// @p is_custom may be NULL when the recognized/custom distinction is not needed.
///
/// @p is_custom is non-zero for any label that is not a recognized unit. That
/// covers both a custom label and a legacy label an older writer stored, which
/// this SDK reports verbatim and rejects on the write path: passing such a label
/// to @ref gtd_builder_add_channel_with_unit_mode returns
/// `GTD_ERR_INVALID_CHANNEL`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel_unit(
    file: *const GtdNavFile,
    index: usize,
    out: *mut c_char,
    out_capacity: usize,
    required_length: *mut usize,
    is_custom: *mut u8,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let required_length = nonnull_mut!(required_length);
        let Some(ch) = handle.file.channels().get(index) else {
            error::set_last_error(format!("channel index {index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };
        let Some(unit) = ch.unit() else {
            *required_length = 0;
            if !is_custom.is_null() {
                // SAFETY: non-null output pointer is caller-owned.
                unsafe { *is_custom = 0 };
            }
            return GtdStatus::GTD_OK;
        };

        let label = unit.to_string();
        *required_length = label.len().saturating_add(1);
        if !is_custom.is_null() {
            // SAFETY: non-null output pointer is caller-owned.
            unsafe { *is_custom = u8::from(unit.as_recognized().is_none()) };
        }
        if out_capacity == 0 {
            return GtdStatus::GTD_OK;
        }
        if out.is_null() {
            error::set_last_error("out buffer is null but out_capacity > 0");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: out points to `out_capacity` writable bytes by the C API contract.
        let buffer = unsafe { std::slice::from_raw_parts_mut(out, out_capacity) };
        super::fill_c_str(buffer, &label);
        GtdStatus::GTD_OK
    })
}

/// Copy the label of a vector channel's component into @p out (null-terminated,
/// truncated to @p out_capacity bytes).
///
/// @param file            File handle.
/// @param channel_index   Channel index.
/// @param component_index Component index. Must be less than `GtdChannelInfo::component_count`.
/// @param out             Caller-allocated buffer of @p out_capacity bytes.
/// @param out_capacity    Capacity of @p out in bytes.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if an index is out of range or @p out is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel_component(
    file: *const GtdNavFile,
    channel_index: usize,
    component_index: usize,
    out: *mut c_char,
    out_capacity: usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        if out.is_null() {
            error::set_last_error("out buffer is null");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        if out_capacity == 0 {
            error::set_last_error("out buffer capacity is zero");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        let Some(ch) = handle.file.channels().get(channel_index) else {
            error::set_last_error(format!("channel index {channel_index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };
        let Some(label) = ch.components().get(component_index) else {
            error::set_last_error(format!("component index {component_index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };
        // SAFETY: out points to `out_capacity` writable bytes (caller contract).
        let buffer = unsafe { std::slice::from_raw_parts_mut(out, out_capacity) };
        super::fill_c_str(buffer, label);
        GtdStatus::GTD_OK
    })
}

/// Copy up to @p out_capacity sample timestamps of the channel at @p channel_index into @p out.
///
/// @return The channel's total sample count (independent of @p out_capacity). Pass a NULL
///         @p out or zero @p out_capacity to query the count without copying.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_times(
    file: *const GtdNavFile,
    channel_index: usize,
    out: *mut GtdTimestamp,
    out_capacity: usize,
) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    let Some(ch) = (unsafe { &(*file).file }).channels().get(channel_index) else {
        return 0;
    };
    let times = ch.times();
    if !out.is_null() && out_capacity > 0 {
        // SAFETY: out points to `out_capacity` writable elements (caller contract).
        let buffer = unsafe { std::slice::from_raw_parts_mut(out, out_capacity) };
        for (slot, &dt) in buffer.iter_mut().zip(times.iter()) {
            *slot = timestamp::ts_from_datetime(dt);
        }
    }
    times.len()
}

/// Copy up to @p out_capacity values of the channel at @p channel_index into @p out (row-major).
///
/// @return The channel's total value count, `sample_count * max(component_count, 1)`
///         (independent of @p out_capacity). Pass a NULL @p out or zero
///         @p out_capacity to query the count without copying.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_values(
    file: *const GtdNavFile,
    channel_index: usize,
    out: *mut f64,
    out_capacity: usize,
) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    let Some(ch) = (unsafe { &(*file).file }).channels().get(channel_index) else {
        return 0;
    };
    let values = ch.values();
    if !out.is_null() && out_capacity > 0 {
        // SAFETY: out points to `out_capacity` writable elements (caller contract).
        let buffer = unsafe { std::slice::from_raw_parts_mut(out, out_capacity) };
        for (slot, &v) in buffer.iter_mut().zip(values.iter()) {
            *slot = v;
        }
    }
    values.len()
}
