/// Checks a raw pointer for null. On null, sets the thread-local error and
/// returns `GtdStatus::GTD_ERR_NULL_ARGUMENT` from the enclosing closure.
macro_rules! nonnull_mut {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            $crate::error::set_last_error("null pointer argument");
            return $crate::error::GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: checked non-null above. Caller is responsible for valid lifetime.
        unsafe { &mut *($ptr) }
    }};
}

/// Checks a `const` raw pointer for null. On null, sets the error and returns.
macro_rules! nonnull_ref {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            $crate::error::set_last_error("null pointer argument");
            return $crate::error::GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: checked non-null above. Caller is responsible for valid lifetime.
        unsafe { &*($ptr) }
    }};
}

/// Converts a `*const c_char` to `&str`. Sets the error and returns
/// `GtdStatus::GTD_ERR_NULL_ARGUMENT` from the enclosing closure on null, and
/// `GtdStatus::GTD_ERR_UTF8` on invalid UTF-8.
macro_rules! cstr {
    ($ptr:expr) => {{
        if ($ptr).is_null() {
            $crate::error::set_last_error("null string argument");
            return $crate::error::GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: checked non-null above. Caller guarantees null-terminated, valid lifetime.
        match unsafe { std::ffi::CStr::from_ptr($ptr) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                $crate::error::set_last_error("string argument is not valid UTF-8");
                return $crate::error::GtdStatus::GTD_ERR_UTF8;
            }
        }
    }};
}

/// Converts a nullable `*const c_char` to `Option<&str>`.
/// Evaluates to `None` for null, and returns `GtdStatus::GTD_ERR_UTF8` from the
/// enclosing closure on invalid UTF-8.
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
                    return $crate::error::GtdStatus::GTD_ERR_UTF8;
                }
            }
        }
    }};
}
