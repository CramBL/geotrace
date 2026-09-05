//! A nav file's metadata accessors.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::GtdTimestamp;
use crate::timestamp;

/// Return the file title, or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_title(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: file is non-null. `CString` is stored in the handle for its lifetime
    unsafe {
        (*file)
            .title
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the recording device name, or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_device(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*file)
            .device
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the notes string, or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_notes(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*file)
            .notes
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the identity string, or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_identity(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*file)
            .identity
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the travel mode wire name, or NULL if not set.
///
/// The value is the raw wire string (e.g. `"car"`). Pass it to
/// `gtd_travel_mode_from_name()` for the typed `GtdTravelMode`. A file written
/// by a newer SDK may carry a wire name that fails to parse - such values are
/// still returned here verbatim, never dropped.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_travel_mode(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*file)
            .travel_mode
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the version of the SDK build that wrote the file, or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_version(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*file)
            .sdk_version
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the commit of the `geotrace` repository the writing SDK was built from,
/// or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_git_commit(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*file)
            .sdk_git_commit
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

/// Return the committer timestamp of `gtd_nav_file_sdk_git_commit()`.
///
/// `gtd_ts_none()` if not set. Use `gtd_ts_is_none()` to check.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_commit_time(file: *const GtdNavFile) -> GtdTimestamp {
    if file.is_null() {
        return timestamp::gtd_ts_none();
    }
    // SAFETY: file is non-null
    unsafe { (*file).sdk_commit_time }
}
