//! The status codes and the last error message.

use std::cell::RefCell;
use std::ffi::{CString, c_char};

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

pub(crate) fn set_last_error(msg: impl std::fmt::Display) {
    let s = CString::new(msg.to_string()).unwrap_or_else(|_| {
        CString::new("(error message contained a null byte)").unwrap_or_default()
    });
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(s));
}

pub(crate) fn last_error_ptr() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    })
}

pub(crate) fn run_catching_panics<F: FnOnce() -> GtdStatus>(f: F) -> GtdStatus {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(s) => s,
        Err(_) => {
            set_last_error("internal panic in geotrace-c");
            GtdStatus::GTD_ERR_INTERNAL
        }
    }
}

/// Return code for all fallible SDK functions.
///
/// On failure, call `gtd_last_error()` for a human-readable description.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtdStatus {
    /// Success.
    GTD_OK = 0,
    /// A required pointer argument was NULL.
    GTD_ERR_NULL_ARGUMENT = 1,
    /// Malformed event-marker variant path.
    GTD_ERR_INVALID_PATH = 2,
    /// Builder finished with no nav fixes.
    GTD_ERR_NO_NAV_FIXES = 3,
    /// Annotation(s) outside the nav fix time range.
    GTD_ERR_ANNOTATIONS_OOB = 4,
    /// I/O error (file not found, permission denied, etc.).
    GTD_ERR_IO = 5,
    /// HDF5 library error.
    GTD_ERR_HDF5 = 6,
    /// Unsupported file format version.
    GTD_ERR_VERSION = 7,
    /// String argument contained invalid UTF-8.
    GTD_ERR_UTF8 = 8,
    /// Malformed or corrupt .gtd file (decode failed).
    GTD_ERR_PARSE = 9,
    /// Malformed channel (bad name/component or length mismatch).
    GTD_ERR_INVALID_CHANNEL = 10,
    /// A string is longer than the `.gtd` field that holds it.
    GTD_ERR_FIELD_TOO_LONG = 11,
    /// An argument's value is not allowed.
    GTD_ERR_INVALID_ARGUMENT = 12,
    /// Internal error (bug in the SDK).
    GTD_ERR_INTERNAL = 99,
}

// Pin every discriminant at compile time. These are the C ABI numbers, which a
// reordering of the variants must not change.
const _: () = {
    assert!(GtdStatus::GTD_OK as u32 == 0);
    assert!(GtdStatus::GTD_ERR_NULL_ARGUMENT as u32 == 1);
    assert!(GtdStatus::GTD_ERR_INVALID_PATH as u32 == 2);
    assert!(GtdStatus::GTD_ERR_NO_NAV_FIXES as u32 == 3);
    assert!(GtdStatus::GTD_ERR_ANNOTATIONS_OOB as u32 == 4);
    assert!(GtdStatus::GTD_ERR_IO as u32 == 5);
    assert!(GtdStatus::GTD_ERR_HDF5 as u32 == 6);
    assert!(GtdStatus::GTD_ERR_VERSION as u32 == 7);
    assert!(GtdStatus::GTD_ERR_UTF8 as u32 == 8);
    assert!(GtdStatus::GTD_ERR_PARSE as u32 == 9);
    assert!(GtdStatus::GTD_ERR_INVALID_CHANNEL as u32 == 10);
    assert!(GtdStatus::GTD_ERR_FIELD_TOO_LONG as u32 == 11);
    assert!(GtdStatus::GTD_ERR_INVALID_ARGUMENT as u32 == 12);
    assert!(GtdStatus::GTD_ERR_INTERNAL as u32 == 99);
};

/// Map a core SDK error to its C status code. Decode failures (malformed or
/// corrupt file content) map to `ErrParse`, not `ErrInternal` (which means an
/// SDK bug). Exhaustive on purpose: a new `Error` variant must choose a code.
pub(crate) fn status_for_error(e: &geotrace_sdk::Error) -> GtdStatus {
    use geotrace_sdk::Error;
    match e {
        Error::Io(_) => GtdStatus::GTD_ERR_IO,
        Error::Hdf5(_) => GtdStatus::GTD_ERR_HDF5,
        Error::UnsupportedVersion { .. } => GtdStatus::GTD_ERR_VERSION,
        Error::UnknownConstellation { .. }
        | Error::ShapeMismatch { .. }
        | Error::UnknownConstellationName { .. }
        | Error::UnknownMarkerIcon { .. }
        | Error::ParseError { .. }
        | Error::UnreadableField { .. }
        | Error::FixWithoutTimestamp { .. }
        | Error::ReportWithoutTimestamp { .. }
        | Error::DatasetSizePastFileLength { .. } => GtdStatus::GTD_ERR_PARSE,
        Error::UnwritableField { .. } => GtdStatus::GTD_ERR_FIELD_TOO_LONG,
    }
}

/// Map an event marker build error to its C status code: a value past the
/// capacity of the field that holds it gets `ErrFieldTooLong`, a malformed
/// variant path `ErrInvalidPath`.
pub(crate) fn status_for_event_marker_error(e: &geotrace_sdk::EventMarkerError) -> GtdStatus {
    use geotrace_sdk::EventMarkerError;
    match e {
        EventMarkerError::TooLong { .. } | EventMarkerError::UnwritableAnnotation { .. } => {
            GtdStatus::GTD_ERR_FIELD_TOO_LONG
        }
        EventMarkerError::Empty { .. }
        | EventMarkerError::LeadingSlash { .. }
        | EventMarkerError::TrailingSlash { .. }
        | EventMarkerError::EmptySegment { .. }
        | EventMarkerError::InvalidChars { .. } => GtdStatus::GTD_ERR_INVALID_PATH,
    }
}

/// Returns the last error message for the current thread, or NULL if none.
///
/// The pointer is valid until the next SDK call on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_last_error() -> *const c_char {
    last_error_ptr()
}
