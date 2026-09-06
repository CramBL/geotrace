//! Consuming a builder into a `GtdNavFile`.

use geotrace_sdk::BuildError;

use super::GtdFileBuilder;
use crate::GtdNavFile;
use crate::error::{self, GtdStatus};

/// Finalise the builder and produce a `GtdNavFile` handle.
///
/// The builder is **consumed** by this call regardless of success or failure.
/// Do not call `gtd_builder_destroy()` afterwards.
///
/// On success, `*out` is set to the new handle.
/// On failure, `*out` is set to NULL and `gtd_last_error()` describes the error.
///
/// @param builder Builder to finalise.
/// @param out     Output parameter for the resulting file handle.
///
/// @return `GTD_ERR_NO_NAV_FIXES` if no nav fixes were added.
/// @return `GTD_ERR_ANNOTATIONS_OOB` if annotations fall outside the time range (unless lenient).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_finish(
    builder: *mut GtdFileBuilder,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    error::run_catching_panics(|| {
        if builder.is_null() {
            error::set_last_error("null pointer argument (builder)");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        if out.is_null() {
            error::set_last_error("null pointer argument (out)");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        // SAFETY: builder is non-null, was created by `gtd_builder_create` via Box::into_raw
        let builder_box = unsafe { Box::from_raw(builder) };
        // SAFETY: out is non-null (checked above)
        let out_ref = unsafe { &mut *out };
        *out_ref = std::ptr::null_mut();

        let recorder = builder_box.into_recorder();

        match recorder.finish() {
            Ok(nav_file) => {
                let handle = Box::new(GtdNavFile::from_nav_file(nav_file));
                *out_ref = Box::into_raw(handle);
                GtdStatus::GTD_OK
            }
            Err(BuildError::NoNavFixes) => {
                error::set_last_error("no nav fixes were added; at least one is required");
                GtdStatus::GTD_ERR_NO_NAV_FIXES
            }
            Err(BuildError::AnnotationsOutsideRange { count }) => {
                error::set_last_error(format!(
                    "{count} annotation(s) fall outside the nav fix time range"
                ));
                GtdStatus::GTD_ERR_ANNOTATIONS_OOB
            }
            Err(BuildError::DuplicateChannelName { name }) => {
                error::set_last_error(format!("two channels share the name {name:?}"));
                GtdStatus::GTD_ERR_INVALID_CHANNEL
            }
            Err(error @ BuildError::GhostFixTimeOutOfRange { .. }) => {
                error::set_last_error(error);
                GtdStatus::GTD_ERR_INVALID_ARGUMENT
            }
        }
    })
}
