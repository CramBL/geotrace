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
            GtdStatus::ErrInternal
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtdStatus {
    Ok = 0,
    ErrNullArgument = 1,
    ErrInvalidPath = 2,
    ErrNoNavFixes = 3,
    ErrAnnotationsOob = 4,
    ErrIo = 5,
    ErrHdf5 = 6,
    ErrVersion = 7,
    ErrUtf8 = 8,
    ErrParse = 9,
    ErrInternal = 99,
}

// Pin every discriminant at compile time. These are the C ABI numbers and must
// match `GtdStatus` in sdk/c/geotrace.h exactly; the build script (build.rs)
// cross-checks the header so the two hand-written lists cannot drift.
const _: () = {
    assert!(GtdStatus::Ok as u32 == 0);
    assert!(GtdStatus::ErrNullArgument as u32 == 1);
    assert!(GtdStatus::ErrInvalidPath as u32 == 2);
    assert!(GtdStatus::ErrNoNavFixes as u32 == 3);
    assert!(GtdStatus::ErrAnnotationsOob as u32 == 4);
    assert!(GtdStatus::ErrIo as u32 == 5);
    assert!(GtdStatus::ErrHdf5 as u32 == 6);
    assert!(GtdStatus::ErrVersion as u32 == 7);
    assert!(GtdStatus::ErrUtf8 as u32 == 8);
    assert!(GtdStatus::ErrParse as u32 == 9);
    assert!(GtdStatus::ErrInternal as u32 == 99);
};

/// Map a core SDK error to its C status code. Decode failures (malformed or
/// corrupt file content) map to `ErrParse`, not `ErrInternal` (which means an
/// SDK bug). Exhaustive on purpose: a new `Error` variant must choose a code.
pub(crate) fn status_for_error(e: &geotrace_sdk::Error) -> GtdStatus {
    use geotrace_sdk::Error;
    match e {
        Error::Io(_) => GtdStatus::ErrIo,
        Error::Hdf5(_) => GtdStatus::ErrHdf5,
        Error::UnsupportedVersion { .. } => GtdStatus::ErrVersion,
        Error::UnknownConstellation { .. }
        | Error::ShapeMismatch { .. }
        | Error::UnknownConstellationName { .. }
        | Error::UnknownMarkerIcon { .. }
        | Error::ParseError { .. } => GtdStatus::ErrParse,
    }
}
