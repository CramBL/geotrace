//! The `navfile_write` group of `geotrace.h`: writing a nav file out, freeing a buffer or a handle.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_write_to_path(
    f: *const GtdNavFile,
    path: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let path_str = cstr!(path);
        match f.file.write_to_file(path_str) {
            Ok(()) => GtdStatus::Ok,
            Err(e) => {
                error::set_last_error(&e);
                error::status_for_error(&e)
            }
        }
    })
}

/// Serialises the file to a heap buffer. The buffer must be freed with
/// `gtd_free_bytes(buf, len)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_to_bytes(
    f: *const GtdNavFile,
    buf: *mut *mut u8,
    len: *mut usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let buf_out = nonnull_mut!(buf);
        let len_out = nonnull_mut!(len);

        let mut bytes: Vec<u8> = Vec::new();
        if let Err(e) = f.file.write(&mut bytes) {
            error::set_last_error(&e);
            return error::status_for_error(&e);
        }

        let mut boxed = bytes.into_boxed_slice();
        *len_out = boxed.len();
        *buf_out = boxed.as_mut_ptr();
        // Transfer ownership to the C caller. `gtd_free_bytes` reconstructs the Box.
        #[expect(
            clippy::mem_forget,
            reason = "intentionally leaking Box<[u8]> to transfer ownership to the C caller"
        )]
        std::mem::forget(boxed);
        GtdStatus::Ok
    })
}

/// Frees a buffer returned by `gtd_nav_file_to_bytes`.
/// `buf` and `len` must match the values written by `gtd_nav_file_to_bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_free_bytes(buf: *mut u8, len: usize) {
    if buf.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(buf, len);
    // SAFETY: slice reconstructs the Box<[u8]> allocated by `gtd_nav_file_to_bytes`
    unsafe { drop(Box::from_raw(slice)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_destroy(f: *mut GtdNavFile) {
    if f.is_null() {
        return;
    }
    // SAFETY: f was allocated by `gtd_builder_finish` or `gtd_nav_file_open` via Box::into_raw
    unsafe { drop(Box::from_raw(f)) };
}
