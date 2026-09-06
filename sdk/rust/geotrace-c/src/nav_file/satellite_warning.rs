//! Satellite data warnings and the accessors that fill them.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::error::{self, GtdStatus};

/// One satellite data quality issue, returned by `gtd_nav_file_get_satellite_warning()`.
///
/// All string fields are null-terminated.
#[repr(C)]
pub struct GtdSatelliteWarningInfo {
    /// How many satellites over all of the file's reports show the issue, or
    /// how many reports where the issue is a property of a whole report.
    pub count: u32,
    /// What the issue is, e.g. `"satellite(s) with PRN 0"`.
    pub issue: [c_char; 128],
    /// Why the value is a problem, and what a recorder should write instead.
    pub description: [c_char; 512],
}

/// Return the number of satellite data warnings for the file.
///
/// These are the checks `gtd_builder_finish()` runs over the satellite reports
/// it is given, one warning per issue found, and they are computed when the
/// file handle is created.
///
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_satellite_warning_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    unsafe { (*file).satellite_warnings.len() }
}

/// Fill @p out with the satellite data warning at @p index.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than
///              `gtd_nav_file_satellite_warning_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last warning.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite_warning(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdSatelliteWarningInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(warning) = handle.satellite_warnings.get(index) else {
            error::set_last_error(format!("satellite warning index {index} is out of range"));
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };

        out.count = warning.count;
        super::fill_c_str(&mut out.issue, warning.issue);
        super::fill_c_str(&mut out.description, warning.description);

        GtdStatus::GTD_OK
    })
}
