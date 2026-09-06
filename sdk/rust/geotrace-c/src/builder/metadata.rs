//! The builder's metadata setters.

use std::ffi::c_char;

use super::GtdFileBuilder;
use crate::GtdTravelMode;
use crate::error::{self, GtdStatus};

/// Set the file title (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_CALL_ORDER` if data has already been added.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_title(
    builder: *mut GtdFileBuilder,
    title: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let s = cstr!(title);
        builder.set_title(s)
    })
}

/// Set the recording device name (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_CALL_ORDER` if data has already been added.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_device(
    builder: *mut GtdFileBuilder,
    device: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let s = cstr!(device);
        builder.set_device(s)
    })
}

/// Set free-form notes (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_CALL_ORDER` if data has already been added.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_notes(
    builder: *mut GtdFileBuilder,
    notes: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let s = cstr!(notes);
        builder.set_notes(s)
    })
}

/// Set a device/session identity string (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_CALL_ORDER` if data has already been added.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_identity(
    builder: *mut GtdFileBuilder,
    identity: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let s = cstr!(identity);
        builder.set_identity(s)
    })
}

/// Declare the platform the recording was made on (optional).
///
/// Must be called before the first `gtd_builder_add_*` call.
///
/// @return `GTD_ERR_CALL_ORDER` if data has already been added.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_travel_mode(
    builder: *mut GtdFileBuilder,
    mode: GtdTravelMode,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        builder.set_travel_mode(mode.into())
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
/// @return `GTD_ERR_CALL_ORDER` if data has already been added.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_lenient(builder: *mut GtdFileBuilder) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        builder.set_lenient()
    })
}
