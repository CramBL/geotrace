use std::cell::RefCell;
use std::ffi::{CString, c_char};

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

pub(crate) fn set_last_error(msg: impl std::fmt::Display) {
    let s = CString::new(msg.to_string())
        .unwrap_or_else(|_| CString::new("(error message contained a null byte)").unwrap_or_default());
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
    Ok                = 0,
    ErrNullArgument   = 1,
    ErrInvalidPath    = 2,
    ErrNoNavFixes     = 3,
    ErrAnnotationsOob = 4,
    ErrIo             = 5,
    ErrHdf5           = 6,
    ErrVersion        = 7,
    ErrUtf8           = 8,
    ErrInternal       = 99,
}
