//! The channel read path: channel metadata, samples and component labels.

use std::ffi::{CStr, CString, c_char};
use std::fmt;

use geotrace_sdk::Channel;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::optf64;
use crate::timestamp;
use crate::{GtdOptF64, GtdTimestamp};

/// Channel metadata returned by `gtd_nav_file_get_channel()`.
///
/// Every string pointer points into the file handle and is valid until
/// `gtd_nav_file_destroy()`. Sample timestamps and values are fetched separately
/// with `gtd_nav_file_channel_times()` and `gtd_nav_file_channel_values()`.
///
/// Only a channel with @ref period_deg set wraps: a `deg` channel without it
/// holds an unbounded angle.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdChannelInfo {
    /// Channel name.
    pub name: *const c_char,
    /// Unit of the values, or NULL for a channel without a unit.
    /// `gtd_nav_file_get_channel_unit()` reports whether it is a recognized unit.
    pub unit: *const c_char,
    /// Wrap period in degrees, or absent for a linear channel.
    pub period_deg: GtdOptF64,
    /// Description, or NULL for a channel without one.
    pub description: *const c_char,
    /// The @ref component_count component labels of a vector channel, or NULL
    /// for a scalar channel.
    pub components: *const *const c_char,
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
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last channel.
/// @return `GTD_ERR_INVALID_CHANNEL` if a string of the channel has a nul byte.
///         `gtd_last_error()` states the string and the byte offset.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdChannelInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let (Some(channel), Some(c_strings)) = (
            handle.file.channels().get(index),
            handle.channel_c_strings.get(index),
        ) else {
            error::set_last_error(format!("channel index {index} is out of range"));
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };
        match c_strings {
            Ok(c_strings) => {
                *out = c_strings.info(channel);
                GtdStatus::GTD_OK
            }
            Err(string_with_nul) => {
                error::set_last_error(format!("channel {index}: {string_with_nul}"));
                GtdStatus::GTD_ERR_INVALID_CHANNEL
            }
        }
    })
}

/// Copy the unit label of the channel at @p index into @p out, and report whether
/// it is a recognized unit.
///
/// Pass NULL @p out and zero @p out_capacity to query the required byte length,
/// including the trailing null byte. A channel without a unit reports zero.
/// With a non-zero @p out_capacity below the required length, the SDK leaves
/// @p out unwritten and returns `GTD_ERR_OUT_OF_RANGE`.
///
/// @p is_custom is non-zero for any label that is not a recognized unit. That
/// covers both a custom label and a legacy label an older writer stored, which
/// this SDK reports verbatim and rejects on the write path: passing such a label
/// to @ref gtd_builder_add_channel_with_unit_mode returns
/// `GTD_ERR_INVALID_CHANNEL`.
///
/// @param file            File handle.
/// @param index           Zero-based channel index.
/// @param out             Buffer for the unit label, or NULL to size it.
/// @param out_capacity    Bytes writable at @p out.
/// @param required_length Receives the label's byte length including the null byte.
/// @param is_custom       Receives the recognized/custom distinction. May be NULL.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last channel or
///         @p out_capacity is below the required length.
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
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };
        let Some(unit) = ch.unit() else {
            *required_length = 0;
            if !is_custom.is_null() {
                // SAFETY: non-null output pointer is caller-owned.
                unsafe { *is_custom = 0 };
            }
            return GtdStatus::GTD_OK;
        };

        let label = unit.label();
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
        match super::fill_c_str(buffer, label) {
            Ok(()) => GtdStatus::GTD_OK,
            Err(error) => {
                error::set_last_error(format!("channel unit: {error}"));
                GtdStatus::GTD_ERR_OUT_OF_RANGE
            }
        }
    })
}

/// Copy up to @p out_capacity sample timestamps of the channel at @p channel_index into @p out.
///
/// @param file          File handle. Returns 0 if NULL.
/// @param channel_index Zero-based channel index. Returns 0 if past the last channel.
/// @param out           Caller-allocated array of @p out_capacity timestamps, or NULL.
/// @param out_capacity  Capacity of @p out in elements.
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
/// @param file          File handle. Returns 0 if NULL.
/// @param channel_index Zero-based channel index. Returns 0 if past the last channel.
/// @param out           Caller-allocated array of @p out_capacity values, or NULL.
/// @param out_capacity  Capacity of @p out in elements.
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

/// The C strings of one channel, which [`GtdChannelInfo`] points into.
pub(super) struct ChannelCStrings {
    name: CString,
    unit: Option<CString>,
    description: Option<CString>,
    components: Vec<CString>,
    component_pointers: Vec<*const c_char>,
}

impl ChannelCStrings {
    pub(super) fn new(channel: &Channel) -> Result<Self, ChannelStringWithNul> {
        let name = ChannelString::Name.to_c_string(channel.name())?;
        let unit = channel
            .unit()
            .map(|unit| ChannelString::Unit.to_c_string(unit.label()))
            .transpose()?;
        let description = channel
            .description()
            .map(|description| ChannelString::Description.to_c_string(description))
            .transpose()?;
        let components = channel
            .components()
            .iter()
            .enumerate()
            .map(|(index, label)| ChannelString::Component(index).to_c_string(label))
            .collect::<Result<Vec<CString>, ChannelStringWithNul>>()?;
        let component_pointers = components.iter().map(|label| label.as_ptr()).collect();
        Ok(Self {
            name,
            unit,
            description,
            components,
            component_pointers,
        })
    }

    fn info(&self, channel: &Channel) -> GtdChannelInfo {
        GtdChannelInfo {
            name: self.name.as_ptr(),
            unit: self.unit.as_deref().map_or(std::ptr::null(), CStr::as_ptr),
            period_deg: channel.period().map_or(optf64::opt_f64_none(), |period| {
                optf64::opt_f64_some(period.as_degrees())
            }),
            description: self
                .description
                .as_deref()
                .map_or(std::ptr::null(), CStr::as_ptr),
            // `Vec::as_ptr` returns a dangling pointer for an empty vector.
            components: if self.components.is_empty() {
                std::ptr::null()
            } else {
                self.component_pointers.as_ptr()
            },
            component_count: self.components.len(),
            sample_count: channel.times().len(),
        }
    }
}

pub(super) struct ChannelStringWithNul {
    string: ChannelString,
    offset: usize,
}

impl fmt::Display for ChannelStringWithNul {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { string, offset } = self;
        write!(f, "the {string} has a nul byte at offset {offset}")
    }
}

#[derive(Clone, Copy)]
enum ChannelString {
    Component(usize),
    Description,
    Name,
    Unit,
}

impl ChannelString {
    fn to_c_string(self, value: &str) -> Result<CString, ChannelStringWithNul> {
        CString::new(value).map_err(|error| ChannelStringWithNul {
            string: self,
            offset: error.nul_position(),
        })
    }
}

impl fmt::Display for ChannelString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Component(index) => write!(f, "label of component {index}"),
            Self::Description => f.write_str("description"),
            Self::Name => f.write_str("name"),
            Self::Unit => f.write_str("unit"),
        }
    }
}
