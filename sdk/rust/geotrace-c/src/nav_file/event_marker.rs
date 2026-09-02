//! The `eventmarker` group of `geotrace.h` and the accessors of `navfile_read` that fill it.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::GtdTimestamp;
use crate::error::{self, GtdStatus};
use crate::timestamp;

/// Event marker data returned by `gtd_nav_file_get_event_marker`.
#[repr(C)]
pub struct GtdEventMarkerInfo {
    pub variant_path: [c_char; 257],
    pub sys_time: GtdTimestamp,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub has_annotation: u8,
    pub annotation: [c_char; 1024],
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_event_marker_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    unsafe { (*f).file.event_markers().len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_event_marker(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut GtdEventMarkerInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(marker) = f.file.event_markers().get(idx) else {
            error::set_last_error(format!("event marker index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        // SAFETY: GtdEventMarkerInfo is repr(C). Zeroing it is valid initial state
        *out = unsafe { std::mem::zeroed() };

        super::fill_c_str(&mut out.variant_path, &marker.variant_path);
        out.sys_time = timestamp::ts_from_datetime(marker.sys_time);
        out.lat_deg = marker.lat.as_degrees();
        out.lon_deg = marker.lon.as_degrees();

        if let Some(ann) = &marker.annotation {
            out.has_annotation = 1;
            super::fill_c_str(&mut out.annotation, ann);
        }

        GtdStatus::Ok
    })
}
