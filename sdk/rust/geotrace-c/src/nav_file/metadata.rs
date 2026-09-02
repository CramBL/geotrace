//! The metadata accessors of the `navfile_read` group of `geotrace.h`.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::GtdTimestamp;
use crate::timestamp;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_title(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: f is non-null. `CString` is stored in the handle for its lifetime
    unsafe {
        (*f).title
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_device(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*f).device
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_notes(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*f).notes
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_identity(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*f).identity
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_travel_mode(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*f).travel_mode
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_version(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*f).sdk_version
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_git_commit(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as `gtd_nav_file_title`
    unsafe {
        (*f).sdk_git_commit
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_sdk_commit_time(f: *const GtdNavFile) -> GtdTimestamp {
    if f.is_null() {
        return timestamp::gtd_ts_none();
    }
    // SAFETY: f is non-null
    unsafe { (*f).sdk_commit_time }
}
