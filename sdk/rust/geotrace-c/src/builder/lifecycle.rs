//! Creating and destroying a builder handle.

use super::GtdFileBuilder;

/// Create a new file builder.
///
/// @return A new builder handle, or NULL on allocation failure.
///         Destroy with `gtd_builder_destroy()` on error, or consume with `gtd_builder_finish()`.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_builder_create() -> *mut GtdFileBuilder {
    Box::into_raw(Box::new(GtdFileBuilder::new()))
}

/// Free a builder without writing a file.
///
/// Do **not** call this after a successful `gtd_builder_finish()`: that call
/// already consumes the builder.
///
/// @param builder Builder to destroy. No-op if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_destroy(builder: *mut GtdFileBuilder) {
    if builder.is_null() {
        return;
    }
    // SAFETY: builder was allocated by `gtd_builder_create` via Box::into_raw
    unsafe { drop(Box::from_raw(builder)) };
}
