//! Writing a nav file out, freeing a buffer or a handle.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

/// Write the navigation file to disk.
///
/// The `.gtd` extension is appended automatically if @p path has no extension.
///
/// @param file File handle (not consumed, the caller must still call `gtd_nav_file_destroy()`).
/// @param path Destination file path.
///
/// @return `GTD_ERR_FIELD_TOO_LONG` if an event marker style holds a variant path
///         or color longer than its field.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_write_to_path(
    file: *const GtdNavFile,
    path: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let path_str = cstr!(path);
        match handle.file.write_to_file(path_str) {
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
/// On success, `*buffer` points to a buffer of `*length` bytes that the caller must
/// free with `gtd_free_bytes(*buffer, *length)`.
///
/// @param file   File handle (not consumed).
/// @param buffer Output: pointer to the allocated buffer.
/// @param length Output: number of bytes in the buffer.
///
/// @return `GTD_ERR_FIELD_TOO_LONG` if an event marker style holds a variant path
///         or color longer than its field.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_to_bytes(
    file: *const GtdNavFile,
    buffer: *mut *mut u8,
    length: *mut usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let buffer_out = nonnull_mut!(buffer);
        let length_out = nonnull_mut!(length);

        let mut bytes: Vec<u8> = Vec::new();
        if let Err(e) = handle.file.write(&mut bytes) {
            error::set_last_error(&e);
            return error::status_for_error(&e);
        }

        let mut boxed = bytes.into_boxed_slice();
        *length_out = boxed.len();
        *buffer_out = boxed.as_mut_ptr();
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
/// @p buffer and @p length must match the values written by `gtd_nav_file_to_bytes()`.
/// No-op if @p buffer is NULL.
///
/// @param buffer Pointer to the buffer.
/// @param length Number of bytes in the buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_free_bytes(buffer: *mut u8, length: usize) {
    if buffer.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(buffer, length);
    // SAFETY: slice reconstructs the Box<[u8]> allocated by `gtd_nav_file_to_bytes`
    unsafe { drop(Box::from_raw(slice)) };
}

/// Destroy a navigation file handle and free all associated memory.
///
/// @param file Handle to destroy. No-op if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_destroy(file: *mut GtdNavFile) {
    if file.is_null() {
        return;
    }
    // SAFETY: file was allocated by `gtd_builder_finish` or `gtd_nav_file_open` via Box::into_raw
    unsafe { drop(Box::from_raw(file)) };
}
