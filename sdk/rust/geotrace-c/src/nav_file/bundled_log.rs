//! The bundled raw-text log read path.

use std::ffi::c_char;

use geotrace_sdk::BundledLog;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

/// Bundled raw-text log returned by `gtd_nav_file_get_bundled_log()`.
///
/// @ref name and @ref text point into the file handle and remain valid until
/// `gtd_nav_file_destroy()`. The byte after @ref text at offset
/// @ref text_length is a nul byte. @ref text_length gives the exact UTF-8 byte
/// length, including any embedded nul bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdBundledLogInfo {
    /// HDF5-safe log name (nul-terminated UTF-8).
    pub name: *const c_char,
    /// Raw UTF-8 log text (nul-terminated after @ref text_length bytes).
    pub text: *const c_char,
    /// Byte length of @ref text, excluding the trailing nul byte.
    pub text_length: usize,
}

/// Return the number of bundled logs in the file.
///
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_bundled_log_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: `file` is non-null and points to a live handle (caller contract).
    unsafe { (*file).file.bundled_logs().len() }
}

/// Fill @p out with the name and text of the bundled log at @p index.
///
/// Bundled logs are sorted by name.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than
///              `gtd_nav_file_bundled_log_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if @p file or @p out is NULL.
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last bundled log.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_bundled_log(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdBundledLogInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(buffers) = handle.bundled_log_buffers.get(index) else {
            error::set_last_error(format!("bundled log index {index} is out of range"));
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };
        *out = buffers.info();
        GtdStatus::GTD_OK
    })
}

/// Nul-terminated byte buffers for one bundled log, which [`GtdBundledLogInfo`] points into.
pub(super) struct BundledLogBuffers {
    name_and_terminator: Box<[u8]>,
    text_and_terminator: Box<[u8]>,
}

impl BundledLogBuffers {
    pub(super) fn new(log: &BundledLog) -> Self {
        Self {
            name_and_terminator: [log.name().as_bytes(), &[0]].concat().into_boxed_slice(),
            text_and_terminator: [log.text().as_bytes(), &[0]].concat().into_boxed_slice(),
        }
    }

    fn info(&self) -> GtdBundledLogInfo {
        GtdBundledLogInfo {
            name: self.name_and_terminator.as_ptr().cast::<c_char>(),
            text: self.text_and_terminator.as_ptr().cast::<c_char>(),
            text_length: self.text_and_terminator.len().saturating_sub(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString, c_char};
    use std::fs;
    use std::ptr;
    use std::slice;

    use rstest::rstest;
    use tempfile::TempDir;

    use super::GtdBundledLogInfo;
    use crate::builder::{finish, ingest, lifecycle};
    use crate::error::{self, GtdStatus};
    use crate::nav_file::{GtdNavFile, read, write};

    fn last_error_string() -> String {
        let ptr = error::gtd_last_error();
        assert!(!ptr.is_null(), "expected an error message");
        // SAFETY: `gtd_last_error` returns a valid nul-terminated C string on this thread.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn read_bundled_log(file_ptr: *const GtdNavFile, index: usize) -> (String, Vec<u8>) {
        let mut info = GtdBundledLogInfo {
            name: ptr::null(),
            text: ptr::null(),
            text_length: 0,
        };
        // SAFETY: `file_ptr` is a live handle and `info` is writable.
        let status = unsafe { super::gtd_nav_file_get_bundled_log(file_ptr, index, &mut info) };
        assert_eq!(status, GtdStatus::GTD_OK);
        // SAFETY: `info.name` is a nul-terminated C string valid for `file_ptr`'s lifetime.
        let name = unsafe { CStr::from_ptr(info.name) }
            .to_str()
            .expect("bundled log name is UTF-8")
            .to_owned();
        // SAFETY: `info.text` points to `info.text_length` readable bytes.
        let text =
            unsafe { slice::from_raw_parts(info.text.cast::<u8>(), info.text_length) }.to_vec();
        (name, text)
    }

    #[test]
    fn bundled_logs_round_trip_sorted_with_embedded_nul_bytes() {
        let builder_ptr = lifecycle::gtd_builder_create();
        let name_b = c"zeta.log";
        let text_b = b"May 29 18:48:24 host proc: line1\0after_nul\n";
        let name_a = c"alpha.log";
        let text_a = b"2026-05-29T18:48:24Z boot complete\n";

        // SAFETY: pointers are valid for the duration of the FFI call.
        let status = unsafe {
            ingest::gtd_builder_add_bundled_log(
                builder_ptr,
                name_b.as_ptr(),
                text_b.as_ptr().cast::<c_char>(),
                text_b.len(),
            )
        };
        assert_eq!(status, GtdStatus::GTD_OK);

        // SAFETY: pointers are valid for the duration of the FFI call.
        let status = unsafe {
            ingest::gtd_builder_add_bundled_log(
                builder_ptr,
                name_a.as_ptr(),
                text_a.as_ptr().cast::<c_char>(),
                text_a.len(),
            )
        };
        assert_eq!(status, GtdStatus::GTD_OK);

        let mut file_ptr: *mut GtdNavFile = ptr::null_mut();
        // SAFETY: `builder_ptr` is a live builder and `file_ptr` is writable.
        let status = unsafe { finish::gtd_builder_finish(builder_ptr, &mut file_ptr) };
        assert_eq!(status, GtdStatus::GTD_OK);

        let mut bytes_ptr: *mut u8 = ptr::null_mut();
        let mut bytes_len: usize = 0;
        // SAFETY: `file_ptr` is live and output pointers are writable.
        let status =
            unsafe { write::gtd_nav_file_to_bytes(file_ptr, &mut bytes_ptr, &mut bytes_len) };
        assert_eq!(status, GtdStatus::GTD_OK);
        // SAFETY: `file_ptr` was produced by `gtd_builder_finish`.
        unsafe { write::gtd_nav_file_destroy(file_ptr) };

        let mut reloaded_ptr: *mut GtdNavFile = ptr::null_mut();
        // SAFETY: `bytes_ptr` points to `bytes_len` readable bytes from `gtd_nav_file_to_bytes`.
        let status =
            unsafe { read::gtd_nav_file_from_bytes(bytes_ptr, bytes_len, &mut reloaded_ptr) };
        assert_eq!(status, GtdStatus::GTD_OK);
        // SAFETY: `bytes_ptr` was allocated by `gtd_nav_file_to_bytes`.
        unsafe { write::gtd_free_bytes(bytes_ptr, bytes_len) };

        // SAFETY: `reloaded_ptr` is a live handle.
        let count = unsafe { super::gtd_nav_file_bundled_log_count(reloaded_ptr) };
        assert_eq!(count, 2);
        assert_eq!(
            read_bundled_log(reloaded_ptr, 0),
            ("alpha.log".to_owned(), text_a.to_vec())
        );
        assert_eq!(
            read_bundled_log(reloaded_ptr, 1),
            ("zeta.log".to_owned(), text_b.to_vec())
        );

        let mut info = GtdBundledLogInfo {
            name: ptr::null(),
            text: ptr::null(),
            text_length: 0,
        };
        // SAFETY: `reloaded_ptr` is live and `info` is writable.
        let status = unsafe { super::gtd_nav_file_get_bundled_log(reloaded_ptr, 2, &mut info) };
        assert_eq!(status, GtdStatus::GTD_ERR_OUT_OF_RANGE);

        // SAFETY: `reloaded_ptr` was produced by `gtd_nav_file_from_bytes`.
        unsafe { write::gtd_nav_file_destroy(reloaded_ptr) };
    }

    #[rstest]
    #[case::empty_text("device.log", b"", GtdStatus::GTD_ERR_INVALID_ARGUMENT)]
    #[case::unrecognised_format(
        "device.log",
        b"no timestamp on this line\n",
        GtdStatus::GTD_ERR_INVALID_ARGUMENT
    )]
    #[case::empty_name("", b"2026-05-29T18:48:24Z ok\n", GtdStatus::GTD_ERR_INVALID_ARGUMENT)]
    #[case::slash_in_name(
        "sub/device.log",
        b"2026-05-29T18:48:24Z ok\n",
        GtdStatus::GTD_ERR_INVALID_ARGUMENT
    )]
    #[case::dot_dot_name(
        "..",
        b"2026-05-29T18:48:24Z ok\n",
        GtdStatus::GTD_ERR_INVALID_ARGUMENT
    )]
    #[case::invalid_utf8("device.log", &[0xff, 0xfe], GtdStatus::GTD_ERR_UTF8)]
    fn invalid_bundled_log_is_rejected_with_error_message(
        #[case] name: &str,
        #[case] text: &[u8],
        #[case] expected_status: GtdStatus,
    ) {
        let builder_ptr = lifecycle::gtd_builder_create();
        let name_c = CString::new(name).expect("test name has no interior nul");

        // SAFETY: pointers are valid for the duration of the FFI call.
        let status = unsafe {
            ingest::gtd_builder_add_bundled_log(
                builder_ptr,
                name_c.as_ptr(),
                text.as_ptr().cast::<c_char>(),
                text.len(),
            )
        };
        assert_eq!(status, expected_status);
        assert!(!last_error_string().is_empty());

        // SAFETY: `builder_ptr` was created by `gtd_builder_create`.
        unsafe { lifecycle::gtd_builder_destroy(builder_ptr) };
    }

    #[test]
    fn add_bundled_log_from_file_reads_file_and_reports_io_error() {
        let dir = TempDir::new().expect("temp dir");
        let log_path = dir.path().join("daemon.log");
        let log_text = "2026-05-29 18:48:24 daemon started\n";
        fs::write(&log_path, log_text).expect("write log file");

        let builder_ptr = lifecycle::gtd_builder_create();
        let missing_path = CString::new(dir.path().join("missing.log").to_string_lossy().as_ref())
            .expect("utf-8 path");
        // SAFETY: pointers are valid for the call.
        let status = unsafe {
            ingest::gtd_builder_add_bundled_log_from_file(builder_ptr, missing_path.as_ptr())
        };
        assert_eq!(status, GtdStatus::GTD_ERR_IO);
        assert!(!last_error_string().is_empty());

        let valid_path = CString::new(log_path.to_string_lossy().as_ref()).expect("utf-8 path");
        // SAFETY: pointers are valid for the call.
        let status = unsafe {
            ingest::gtd_builder_add_bundled_log_from_file(builder_ptr, valid_path.as_ptr())
        };
        assert_eq!(status, GtdStatus::GTD_OK);

        let mut file_ptr: *mut GtdNavFile = ptr::null_mut();
        // SAFETY: `builder_ptr` is live and `file_ptr` is writable.
        let status = unsafe { finish::gtd_builder_finish(builder_ptr, &mut file_ptr) };
        assert_eq!(status, GtdStatus::GTD_OK);

        // SAFETY: `file_ptr` is live.
        let count = unsafe { super::gtd_nav_file_bundled_log_count(file_ptr) };
        assert_eq!(count, 1);
        assert_eq!(
            read_bundled_log(file_ptr, 0),
            ("daemon.log".to_owned(), log_text.as_bytes().to_vec())
        );

        // SAFETY: `file_ptr` was produced by `gtd_builder_finish`.
        unsafe { write::gtd_nav_file_destroy(file_ptr) };
    }

    #[test]
    fn duplicate_bundled_log_name_is_rejected_on_finish() {
        let builder_ptr = lifecycle::gtd_builder_create();
        let name = c"dup.log";
        let text = b"2026-05-29T18:48:24Z entry\n";

        for _ in 0..2 {
            // SAFETY: pointers are valid for the call.
            let status = unsafe {
                ingest::gtd_builder_add_bundled_log(
                    builder_ptr,
                    name.as_ptr(),
                    text.as_ptr().cast::<c_char>(),
                    text.len(),
                )
            };
            assert_eq!(status, GtdStatus::GTD_OK);
        }

        let mut file_ptr: *mut GtdNavFile = ptr::null_mut();
        // SAFETY: `builder_ptr` is live and `file_ptr` is writable.
        let status = unsafe { finish::gtd_builder_finish(builder_ptr, &mut file_ptr) };
        assert_eq!(status, GtdStatus::GTD_ERR_INVALID_ARGUMENT);
        assert!(file_ptr.is_null());
        assert!(last_error_string().contains("dup.log"));
    }
}
