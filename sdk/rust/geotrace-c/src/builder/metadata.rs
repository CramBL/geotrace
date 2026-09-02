//! The builder's metadata setters.

use std::ffi::c_char;

use super::GtdFileBuilder;
use crate::GtdTravelMode;
use crate::error::{self, GtdStatus};

/// Set the file title (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_INTERNAL` if data has already been added.
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

/// Set the recording device name (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_INTERNAL` if data has already been added.
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

/// Set free-form notes (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_INTERNAL` if data has already been added.
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

/// Set a device/session identity string (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_INTERNAL` if data has already been added.
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

/// Declare the platform the recording was made on (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_INTERNAL` if data has already been added.
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

/// Enable lenient mode.
///
/// By default `gtd_builder_finish()` returns `GTD_ERR_ANNOTATIONS_OOB` when any
/// annotation falls outside the nav fix time range. Calling this function
/// downgrades that error to a warning and lets the build succeed.
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @param b Builder handle. No-op if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_lenient(b: *mut GtdFileBuilder) {
    if b.is_null() {
        error::set_last_error("null pointer argument");
        return;
    }
    // SAFETY: b is non-null and valid for the call duration
    unsafe { (*b).set_lenient() };
}
