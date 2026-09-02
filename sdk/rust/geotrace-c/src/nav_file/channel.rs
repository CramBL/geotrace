//! The `navfile_channels` group of `geotrace.h`: channel metadata, samples and component labels.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::optf64;
use crate::timestamp;
use crate::{GtdOptF64, GtdTimestamp};

/// Channel metadata returned by `gtd_nav_file_get_channel` (read path). Sample
/// timestamps, values, and component labels are fetched separately. A
/// `component_count` of zero marks a scalar channel.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdChannelInfo {
    pub name: [c_char; 256],
    pub has_unit: u8,
    pub unit: [c_char; 64],
    pub period_deg: GtdOptF64,
    pub has_description: u8,
    pub description: [c_char; 1024],
    pub component_count: usize,
    pub sample_count: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    unsafe { (*f).file.channels().len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut GtdChannelInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(ch) = f.file.channels().get(idx) else {
            error::set_last_error(format!("channel index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
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
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel_unit(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut c_char,
    cap: usize,
    required_len: *mut usize,
    is_custom: *mut u8,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let required_len = nonnull_mut!(required_len);
        let Some(ch) = f.file.channels().get(idx) else {
            error::set_last_error(format!("channel index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };
        let Some(unit) = ch.unit() else {
            *required_len = 0;
            if !is_custom.is_null() {
                // SAFETY: non-null output pointer is caller-owned.
                unsafe { *is_custom = 0 };
            }
            return GtdStatus::Ok;
        };

        let label = unit.to_string();
        *required_len = label.len().saturating_add(1);
        if !is_custom.is_null() {
            // SAFETY: non-null output pointer is caller-owned.
            unsafe { *is_custom = u8::from(unit.as_recognized().is_none()) };
        }
        if cap == 0 {
            return GtdStatus::Ok;
        }
        if out.is_null() {
            error::set_last_error("out buffer is null but cap > 0");
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: out points to cap writable bytes by the C API contract.
        let buffer = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        super::fill_c_str(buffer, &label);
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel_component(
    f: *const GtdNavFile,
    ch_idx: usize,
    comp_idx: usize,
    out: *mut c_char,
    cap: usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        if out.is_null() {
            error::set_last_error("out buffer is null");
            return GtdStatus::ErrNullArgument;
        }
        if cap == 0 {
            error::set_last_error("out buffer capacity is zero");
            return GtdStatus::ErrNullArgument;
        }
        let Some(ch) = f.file.channels().get(ch_idx) else {
            error::set_last_error(format!("channel index {ch_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };
        let Some(label) = ch.components().get(comp_idx) else {
            error::set_last_error(format!("component index {comp_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };
        // SAFETY: out points to `cap` writable bytes (caller contract).
        let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        super::fill_c_str(buf, label);
        GtdStatus::Ok
    })
}

/// Copy up to `cap` sample timestamps into `out`, returning the channel's total
/// sample count (independent of `cap`). Passing a null `out` or zero `cap`
/// queries the count without copying.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_times(
    f: *const GtdNavFile,
    ch_idx: usize,
    out: *mut GtdTimestamp,
    cap: usize,
) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    let Some(ch) = (unsafe { &(*f).file }).channels().get(ch_idx) else {
        return 0;
    };
    let times = ch.times();
    if !out.is_null() && cap > 0 {
        // SAFETY: out points to `cap` writable elements (caller contract).
        let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        for (slot, &dt) in buf.iter_mut().zip(times.iter()) {
            *slot = timestamp::ts_from_datetime(dt);
        }
    }
    times.len()
}

/// Copy up to `cap` values into `out`, returning the channel's total value count
/// (`sample_count * max(component_count, 1)`, row-major). Passing a null `out`
/// or zero `cap` queries the count without copying.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_values(
    f: *const GtdNavFile,
    ch_idx: usize,
    out: *mut f64,
    cap: usize,
) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    let Some(ch) = (unsafe { &(*f).file }).channels().get(ch_idx) else {
        return 0;
    };
    let values = ch.values();
    if !out.is_null() && cap > 0 {
        // SAFETY: out points to `cap` writable elements (caller contract).
        let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        for (slot, &v) in buf.iter_mut().zip(values.iter()) {
            *slot = v;
        }
    }
    values.len()
}
