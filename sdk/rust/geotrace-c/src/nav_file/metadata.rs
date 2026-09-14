//! A nav file's metadata accessors.

use std::ffi::{CStr, c_char};

use super::GtdNavFile;
use crate::GtdTimestamp;
use crate::timestamp;

/// Return the file title, or NULL if not set.
///
/// Returns NULL for a title with a nul byte. `gtd_nav_file_title_with_length()` returns the whole
/// title.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_title(file: *const GtdNavFile) -> *const c_char {
    // SAFETY: `file` is NULL or a live handle (caller contract).
    unsafe { meta_value_as_c_string(file, |file| file.title.as_ref()) }
}

/// Return the file title and write its byte length to @p length, or return NULL if not set.
///
/// Returns the whole title, nul bytes included. The byte after the title is 0.
///
/// @param file   File handle. Returns NULL if NULL.
/// @param length Receives the byte length of the title, or 0 with a NULL return. May be NULL.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_title_with_length(
    file: *const GtdNavFile,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: `file` is NULL or a live handle, and `length` is NULL or points to a writable
    // `size_t` (caller contract).
    unsafe { meta_value_with_length(file, |file| file.title.as_ref(), length) }
}

/// Return the recording device name, or NULL if not set.
///
/// Returns NULL for a device name with a nul byte. `gtd_nav_file_device_with_length()` returns
/// the whole device name.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_device(file: *const GtdNavFile) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title`
    unsafe { meta_value_as_c_string(file, |file| file.device.as_ref()) }
}

/// Return the recording device name and write its byte length to @p length, or return NULL if
/// not set.
///
/// Returns the whole device name, nul bytes included. The byte after the device name is 0.
///
/// @param file   File handle. Returns NULL if NULL.
/// @param length Receives the byte length of the device name, or 0 with a NULL return. May be
///               NULL.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_device_with_length(
    file: *const GtdNavFile,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title_with_length`
    unsafe { meta_value_with_length(file, |file| file.device.as_ref(), length) }
}

/// Return the notes string, or NULL if not set.
///
/// Returns NULL for notes with a nul byte. `gtd_nav_file_notes_with_length()` returns the whole
/// notes string.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_notes(file: *const GtdNavFile) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title`
    unsafe { meta_value_as_c_string(file, |file| file.notes.as_ref()) }
}

/// Return the notes string and write its byte length to @p length, or return NULL if not set.
///
/// Returns the whole notes string, nul bytes included. The byte after the notes string is 0.
///
/// @param file   File handle. Returns NULL if NULL.
/// @param length Receives the byte length of the notes string, or 0 with a NULL return. May be
///               NULL.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_notes_with_length(
    file: *const GtdNavFile,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title_with_length`
    unsafe { meta_value_with_length(file, |file| file.notes.as_ref(), length) }
}

/// Return the identity string, or NULL if not set.
///
/// Returns NULL for an identity with a nul byte. `gtd_nav_file_identity_with_length()` returns
/// the whole identity.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_identity(file: *const GtdNavFile) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title`
    unsafe { meta_value_as_c_string(file, |file| file.identity.as_ref()) }
}

/// Return the identity string and write its byte length to @p length, or return NULL if not set.
///
/// Returns the whole identity, nul bytes included. The byte after the identity is 0.
///
/// @param file   File handle. Returns NULL if NULL.
/// @param length Receives the byte length of the identity, or 0 with a NULL return. May be NULL.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_identity_with_length(
    file: *const GtdNavFile,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title_with_length`
    unsafe { meta_value_with_length(file, |file| file.identity.as_ref(), length) }
}

/// Return the travel mode wire name, or NULL if not set.
///
/// The value is the raw wire string (e.g. `"car"`). Pass it to
/// `gtd_travel_mode_from_name()` for the typed `GtdTravelMode`. A file written
/// by a newer SDK may carry a wire name that fails to parse - such values are
/// still returned here verbatim, never dropped.
///
/// Returns NULL for a wire name with a nul byte. `gtd_nav_file_travel_mode_with_length()`
/// returns the whole wire name.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_travel_mode(file: *const GtdNavFile) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title`
    unsafe { meta_value_as_c_string(file, |file| file.travel_mode.as_ref()) }
}

/// Return the travel mode wire name and write its byte length to @p length, or return NULL if not
/// set.
///
/// Returns the whole wire name, nul bytes included. The byte after the wire name is 0.
///
/// @param file   File handle. Returns NULL if NULL.
/// @param length Receives the byte length of the wire name, or 0 with a NULL return. May be NULL.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_travel_mode_with_length(
    file: *const GtdNavFile,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: same as `gtd_nav_file_title_with_length`
    unsafe { meta_value_with_length(file, |file| file.travel_mode.as_ref(), length) }
}

/// Return the version of the SDK build that wrote the file, or NULL if not set.
///
/// The returned pointer is valid for the lifetime of @p file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_version(file: *const GtdNavFile) -> *const c_char {
    if file.is_null() {
        return std::ptr::null();
    }
    // SAFETY: file is non-null. `CString` is stored in the handle for its lifetime
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
    // SAFETY: file is non-null. `CString` is stored in the handle for its lifetime
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
///
/// @param file File handle. Returns `gtd_ts_none()` if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_commit_time(file: *const GtdNavFile) -> GtdTimestamp {
    if file.is_null() {
        return timestamp::gtd_ts_none();
    }
    // SAFETY: file is non-null
    unsafe { (*file).sdk_commit_time }
}

/// A metadata value and a nul byte after it. The value may contain nul bytes of its own.
pub(super) struct TerminatedMetaValue {
    value_and_terminator: Box<[u8]>,
}

impl TerminatedMetaValue {
    pub(super) fn new(value: &str) -> Self {
        Self {
            value_and_terminator: [value.as_bytes(), &[0]].concat().into_boxed_slice(),
        }
    }

    /// `None` for a value with a nul byte.
    fn as_c_str(&self) -> Option<&CStr> {
        CStr::from_bytes_with_nul(&self.value_and_terminator).ok()
    }

    fn value_length(&self) -> usize {
        self.value_and_terminator.len().saturating_sub(1)
    }
}

/// # Safety
///
/// `file` is NULL or a live handle.
unsafe fn meta_value_as_c_string(
    file: *const GtdNavFile,
    meta_value: fn(&GtdNavFile) -> Option<&TerminatedMetaValue>,
) -> *const c_char {
    // SAFETY: `file` is NULL or a live handle, by the contract of this function.
    unsafe { file.as_ref() }
        .and_then(meta_value)
        .and_then(TerminatedMetaValue::as_c_str)
        .map_or(std::ptr::null(), CStr::as_ptr)
}

/// # Safety
///
/// `file` is NULL or a live handle, and `length` is NULL or points to a writable `usize`.
unsafe fn meta_value_with_length(
    file: *const GtdNavFile,
    meta_value: fn(&GtdNavFile) -> Option<&TerminatedMetaValue>,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: `file` is NULL or a live handle, by the contract of this function.
    let value = unsafe { file.as_ref() }.and_then(meta_value);
    // SAFETY: `length` is NULL or points to a writable `usize`, by the contract of this function.
    if let Some(length) = unsafe { length.as_mut() } {
        *length = value.map_or(0, TerminatedMetaValue::value_length);
    }
    value.map_or(std::ptr::null(), |value| {
        value.value_and_terminator.as_ptr().cast::<c_char>()
    })
}
