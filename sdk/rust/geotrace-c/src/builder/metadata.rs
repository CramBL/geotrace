//! The `builder` group of `geotrace.h`: the metadata setters.

use std::ffi::c_char;

use super::GtdFileBuilder;
use crate::GtdTravelMode;
use crate::error::{self, GtdStatus};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_title(
    b: *mut GtdFileBuilder,
    title: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(title);
        b.set_title(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_device(
    b: *mut GtdFileBuilder,
    device: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(device);
        b.set_device(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_notes(
    b: *mut GtdFileBuilder,
    notes: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(notes);
        b.set_notes(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_identity(
    b: *mut GtdFileBuilder,
    identity: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(identity);
        b.set_identity(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_travel_mode(
    b: *mut GtdFileBuilder,
    mode: GtdTravelMode,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        b.set_travel_mode(mode.into())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_lenient(b: *mut GtdFileBuilder) {
    if b.is_null() {
        error::set_last_error("null pointer argument");
        return;
    }
    // SAFETY: b is non-null and valid for the call duration
    unsafe { (*b).set_lenient() };
}
