//! Event marker data and the accessors that fill it.

use std::ffi::c_char;

use geotrace_sdk::EventMarkerPoint;

use super::{GtdNavFile, StructFieldName};
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
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_event_marker_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    unsafe { (*file).file.event_markers().len() }
}

/// Fill @p out with data for the event marker at @p index.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than `gtd_nav_file_event_marker_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last event marker.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_event_marker(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdEventMarkerInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(marker) = handle.file.event_markers().get(index) else {
            error::set_last_error(format!("event marker index {index} is out of range"));
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };

        match GtdEventMarkerInfo::new(marker) {
            Ok(info) => {
                *out = info;
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

impl GtdEventMarkerInfo {
    fn new(marker: &EventMarkerPoint) -> Result<Self, GtdStatus> {
        let mut info = Self {
            variant_path: [0; 257],
            sys_time: timestamp::ts_from_datetime(marker.sys_time),
            lat_deg: marker.lat.as_degrees(),
            lon_deg: marker.lon.as_degrees(),
            has_annotation: u8::from(marker.annotation.is_some()),
            annotation: [0; 1024],
        };
        super::fill_struct_field(
            &mut info.variant_path,
            &marker.variant_path,
            StructFieldName("GtdEventMarkerInfo::variant_path"),
        )?;
        super::fill_struct_field(
            &mut info.annotation,
            marker.annotation.as_deref().unwrap_or(""),
            StructFieldName("GtdEventMarkerInfo::annotation"),
        )?;
        Ok(info)
    }
}
