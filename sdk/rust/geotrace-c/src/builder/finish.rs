//! The `builder` group of `geotrace.h`: consuming a builder into a `GtdNavFile`.

use geotrace_sdk::BuildError;

use super::GtdFileBuilder;
use crate::GtdNavFile;
use crate::error::{self, GtdStatus};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_finish(
    b: *mut GtdFileBuilder,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    error::run_catching_panics(|| {
        if b.is_null() {
            error::set_last_error("null pointer argument (b)");
            return GtdStatus::ErrNullArgument;
        }
        if out.is_null() {
            error::set_last_error("null pointer argument (out)");
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: b is non-null, was created by `gtd_builder_create` via Box::into_raw
        let b_box = unsafe { Box::from_raw(b) };
        // SAFETY: out is non-null (checked above)
        let out_ref = unsafe { &mut *out };
        *out_ref = std::ptr::null_mut();

        let recorder = b_box.into_recorder();

        match recorder.finish() {
            Ok(nav_file) => {
                let handle = Box::new(GtdNavFile::from_nav_file(nav_file));
                *out_ref = Box::into_raw(handle);
                GtdStatus::Ok
            }
            Err(BuildError::NoNavFixes) => {
                error::set_last_error("no nav fixes were added; at least one is required");
                GtdStatus::ErrNoNavFixes
            }
            Err(BuildError::AnnotationsOutsideRange { count }) => {
                error::set_last_error(format!(
                    "{count} annotation(s) fall outside the nav fix time range"
                ));
                GtdStatus::ErrAnnotationsOob
            }
            Err(BuildError::DuplicateChannelName { name }) => {
                error::set_last_error(format!("two channels share the name {name:?}"));
                GtdStatus::ErrInvalidChannel
            }
        }
    })
}
