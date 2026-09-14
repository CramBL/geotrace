//! Consuming a builder into a `GtdNavFile`.

use std::ptr;

use geotrace_sdk::BuildError;

use super::GtdFileBuilder;
use crate::GtdNavFile;
use crate::error::{self, GtdStatus};

/// Finalise the builder and produce a `GtdNavFile` handle.
///
/// The call **consumes** a non-null `builder` whatever status it returns, including when `out`
/// is NULL. Do not call `gtd_builder_destroy()` afterwards.
///
/// On success, `*out` is set to the new handle.
/// On failure, `*out` is set to NULL for a non-null `out`, including when `builder` is NULL, and
/// `gtd_last_error()` describes the error.
///
/// @param builder Builder to finalise.
/// @param out     Output parameter for the resulting file handle.
///
/// On a builder without nav fixes, the call returns `GTD_OK` and a file with zero nav points,
/// unless the builder has a satellite report, an annotation or an event marker.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if `builder` or `out` is NULL. `gtd_last_error()` states which.
/// @return `GTD_ERR_NO_NAV_FIXES` if the builder has a satellite report, an annotation or an
///         event marker and no nav fix, in lenient mode too. `gtd_last_error()` states the
///         number of each.
/// @return `GTD_ERR_ANNOTATIONS_OOB` if annotations fall outside the time range (unless lenient).
/// @return `GTD_ERR_EVENT_MARKERS_OOB` if event markers fall outside the time range (unless lenient).
/// @return `GTD_ERR_INVALID_CHANNEL` if two channels share a name.
/// @return `GTD_ERR_INVALID_ARGUMENT` if the timestamp the builder computes for a ghost nav fix
///         is past the range a timestamp covers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_finish(
    builder: *mut GtdFileBuilder,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    error::run_catching_panics(|| {
        // SAFETY: a non-null `out` points to a writable `GtdNavFile *` (caller contract).
        if let Some(out) = unsafe { out.as_mut() } {
            *out = ptr::null_mut();
        }
        // SAFETY: a non-null `builder` comes from `gtd_builder_create` through `Box::into_raw`,
        // and no earlier `gtd_builder_finish` or `gtd_builder_destroy` call has freed it (caller
        // contract).
        let Some(builder) = (!builder.is_null()).then(|| unsafe { Box::from_raw(builder) }) else {
            error::set_last_error("null pointer argument (builder)");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };
        // SAFETY: a non-null `out` points to a writable `GtdNavFile *` (caller contract).
        let Some(out) = (unsafe { out.as_mut() }) else {
            error::set_last_error("null pointer argument (out)");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };

        match builder.into_recorder().finish() {
            Ok(nav_file) => {
                *out = Box::into_raw(Box::new(GtdNavFile::from_nav_file(nav_file)));
                GtdStatus::GTD_OK
            }
            Err(error @ BuildError::NoNavFixes(_)) => {
                error::set_last_error(error);
                GtdStatus::GTD_ERR_NO_NAV_FIXES
            }
            Err(BuildError::AnnotationsOutsideRange { count }) => {
                error::set_last_error(format!(
                    "{count} annotation(s) fall outside the nav fix time range"
                ));
                GtdStatus::GTD_ERR_ANNOTATIONS_OOB
            }
            Err(BuildError::EventMarkersOutsideRange { count }) => {
                error::set_last_error(format!(
                    "{count} event marker(s) fall outside the nav fix time range"
                ));
                GtdStatus::GTD_ERR_EVENT_MARKERS_OOB
            }
            Err(BuildError::DuplicateChannelName { name }) => {
                error::set_last_error(format!("two channels share the name {name:?}"));
                GtdStatus::GTD_ERR_INVALID_CHANNEL
            }
            Err(error @ BuildError::GhostFixTimeOutOfRange { .. }) => {
                error::set_last_error(error);
                GtdStatus::GTD_ERR_INVALID_ARGUMENT
            }
            // Unreachable through the C API: `gtd_builder_add_event_marker` validates the variant
            // path before the recorder takes the event marker.
            Err(BuildError::InvalidEventMarkerVariantPath { source }) => {
                let status = error::status_for_event_marker_error(&source);
                error::set_last_error(source);
                status
            }
        }
    })
}
