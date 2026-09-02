//! Writing a nav file out, freeing a buffer or a handle.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

/// Write the navigation file to disk.
///
/// The `.gtd` extension is appended automatically if @p path has no extension.
///
/// @param f    File handle (not consumed, the caller must still call `gtd_nav_file_destroy()`).
/// @param path Destination file path.
///
/// @return `GTD_ERR_FIELD_TOO_LONG` if an event marker style holds a variant path
///         or color longer than its field.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_write_to_path(
    f: *const GtdNavFile,
    path: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let path_str = cstr!(path);
        match f.file.write_to_file(path_str) {
            Ok(()) => GtdStatus::GTD_OK,
            Err(e) => {
                error::set_last_error(&e);
                error::status_for_error(&e)
            }
        }
    })
}

/// Serialise the navigation file into a heap-allocated byte buffer.
///
/// On success, `*buf` points to a buffer of `*len` bytes that the caller must
/// free with `gtd_free_bytes(*buf, *len)`.
///
/// @param f   File handle (not consumed).
/// @param buf Output: pointer to the allocated buffer.
/// @param len Output: number of bytes in the buffer.
///
/// @return `GTD_ERR_FIELD_TOO_LONG` if an event marker style holds a variant path
///         or color longer than its field.
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
        GtdStatus::GTD_OK
    })
}

/// Free a byte buffer returned by `gtd_nav_file_to_bytes()`.
///
/// @p buf and @p len must match the values written by `gtd_nav_file_to_bytes()`.
/// No-op if @p buf is NULL.
///
/// @param buf Pointer to the buffer.
/// @param len Number of bytes in the buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_free_bytes(buf: *mut u8, len: usize) {
    if buf.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(buf, len);
    // SAFETY: slice reconstructs the Box<[u8]> allocated by `gtd_nav_file_to_bytes`
    unsafe { drop(Box::from_raw(slice)) };
}

/// Destroy a navigation file handle and free all associated memory.
///
/// @param f Handle to destroy. No-op if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_destroy(f: *mut GtdNavFile) {
    if f.is_null() {
        return;
    }
    // SAFETY: f was allocated by `gtd_builder_finish` or `gtd_nav_file_open` via Box::into_raw
    unsafe { drop(Box::from_raw(f)) };
}
