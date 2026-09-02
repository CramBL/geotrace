//! Event marker data and the accessors that fill it.

use std::ffi::c_char;

use super::GtdNavFile;
use crate::GtdTimestamp;
use crate::error::{self, GtdStatus};
use crate::timestamp;

/// Event marker data returned by `gtd_nav_file_get_event_marker()`.
///
/// All string fields are null-terminated.
#[repr(C)]
pub struct GtdEventMarkerInfo {
    /// Hierarchical event type path, e.g. `"system/startup"`.
    pub variant_path: [c_char; 257],
    /// System time when the event occurred.
    pub sys_time: GtdTimestamp,
    /// WGS-84 latitude of the event.
    pub lat_deg: f64,
    /// WGS-84 longitude of the event.
    pub lon_deg: f64,
    /// Non-zero if @ref annotation is set.
    pub has_annotation: u8,
    /// Human-readable annotation text, when @ref has_annotation.
    pub annotation: [c_char; 1024],
}

/// Return the number of event markers in the file.
///
/// @param f File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_event_marker_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    unsafe { (*f).file.event_markers().len() }
}

/// Fill @p out with data for the event marker at @p idx.
///
/// @param f   File handle.
/// @param idx Zero-based index. Must be less than `gtd_nav_file_event_marker_count(f)`.
/// @param out Caller-allocated struct to fill.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if @p idx is out of range.
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
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
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

        GtdStatus::GTD_OK
    })
}
