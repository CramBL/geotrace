//! The `navfile_read` group of `geotrace.h`: constructors of a `GtdNavFile` from a path or bytes.

use std::ffi::c_char;
use std::io::Cursor;

use geotrace_sdk::NavFile;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

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
                GtdStatus::Ok
            }
            Err(e) => {
                error::set_last_error(&e);
                error::status_for_error(&e)
            }
        }
    })
}

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
            return GtdStatus::ErrNullArgument;
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
                GtdStatus::Ok
            }
            Err(e) => {
                error::set_last_error(&e);
                error::status_for_error(&e)
            }
        }
    })
}
