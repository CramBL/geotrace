//! Constructors of a `GtdNavFile` from a path or from bytes.

use std::ffi::c_char;
use std::io::Cursor;

use geotrace_sdk::NavFile;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

/// Open and parse a `.gtd` navigation file.
///
/// On success, `*out` is set to a new handle.
/// On failure, `*out` is NULL and `gtd_last_error()` describes the error.
///
/// @param path File path to open.
/// @param out  Output parameter for the file handle.
///
/// @return `GTD_ERR_VERSION` if the file uses an unsupported format version.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_open(
    path: *const c_char,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let path_str = cstr!(path);
        let out_ref = nonnull_mut!(out);
        *out_ref = std::ptr::null_mut();

        match NavFile::open(path_str) {
            Ok(file) => {
                *out_ref = Box::into_raw(Box::new(GtdNavFile::from_nav_file(file)));
                GtdStatus::GTD_OK
            }
            Err(e) => {
                error::set_last_error(&e);
                error::status_for_error(&e)
            }
        }
    })
}

/// Parse a navigation file from an in-memory buffer.
///
/// The caller retains ownership of @p data. It may be freed after this call returns.
///
/// @param data Pointer to the serialised file data.
/// @param len  Length of the data in bytes.
/// @param out  Output parameter for the file handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_from_bytes(
    data: *const u8,
    len: usize,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let out_ref = nonnull_mut!(out);
        *out_ref = std::ptr::null_mut();
        if data.is_null() && len > 0 {
            error::set_last_error("data is null but len > 0");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }

        let slice = if len == 0 {
            &[][..]
        } else {
            // SAFETY: data is non-null (checked above), `len` is the byte count
            unsafe { std::slice::from_raw_parts(data, len) }
        };

        match NavFile::read(Cursor::new(slice)) {
            Ok(file) => {
                *out_ref = Box::into_raw(Box::new(GtdNavFile::from_nav_file(file)));
                GtdStatus::GTD_OK
            }
            Err(e) => {
                error::set_last_error(&e);
                error::status_for_error(&e)
            }
        }
    })
}
