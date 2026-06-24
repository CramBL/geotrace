/// Checks a raw pointer for null. On null, sets the thread-local error and
/// returns `GtdStatus::ErrNullArgument` from the enclosing closure.
macro_rules! nonnull_mut {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            $crate::error::set_last_error("null pointer argument");
            return $crate::error::GtdStatus::ErrNullArgument;
        }
        // SAFETY: checked non-null above. Caller is responsible for valid lifetime.
        unsafe { &mut *($ptr) }
    }};
}

/// Checks a const raw pointer for null. On null, sets the error and returns.
macro_rules! nonnull_ref {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            $crate::error::set_last_error("null pointer argument");
            return $crate::error::GtdStatus::ErrNullArgument;
        }
        // SAFETY: checked non-null above. Caller is responsible for valid lifetime.
        unsafe { &*($ptr) }
    }};
}

/// Converts a `*const c_char` to `&str`. On null or invalid UTF-8, sets the
/// error and returns the appropriate status from the enclosing closure.
macro_rules! cstr {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            $crate::error::set_last_error("null string argument");
            return $crate::error::GtdStatus::ErrNullArgument;
        }
        // SAFETY: checked non-null above. Caller guarantees null-terminated, valid lifetime.
        match unsafe { std::ffi::CStr::from_ptr($ptr) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                $crate::error::set_last_error("string argument is not valid UTF-8");
                return $crate::error::GtdStatus::ErrUtf8;
            }
        }
    }};
}

/// Converts a nullable `*const c_char` to `Option<&str>`.
/// Returns `None` for null, or the appropriate error on invalid UTF-8.
macro_rules! cstr_opt {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            None
        } else {
            // SAFETY: checked non-null above. Caller guarantees null-terminated, valid lifetime.
            match unsafe { std::ffi::CStr::from_ptr($ptr) }.to_str() {
                Ok(s) => Some(s),
                Err(_) => {
                    $crate::error::set_last_error("string argument is not valid UTF-8");
                    return $crate::error::GtdStatus::ErrUtf8;
                }
            }
        }
    }};
}
