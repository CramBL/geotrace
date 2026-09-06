//! Map marker data and the accessors that fill it.

use std::ffi::c_char;

use geotrace_sdk::AnnotationIcon;

use super::GtdNavFile;
use crate::GtdTimestamp;
use crate::error::{self, GtdStatus};
use crate::icon::GtdMarkerIcon;
use crate::timestamp;

/// Map marker data returned by `gtd_nav_file_get_marker()`.
///
/// All string fields are null-terminated.
#[repr(C)]
pub struct GtdMarkerInfo {
    /// Display label, when @ref has_label.
    pub label: [c_char; 256],
    /// Non-zero if @ref label is set.
    pub has_label: u8,
    /// Icon the marker is drawn with. An @ref icon_code outside the
    /// `GtdMarkerIcon` set gives `GTD_ICON_PIN`. The application draws such a
    /// marker with the pin icon.
    pub icon: GtdMarkerIcon,
    /// The icon code the file stores. A newer writer can store a code outside
    /// the `GtdMarkerIcon` set.
    pub icon_code: u8,
    /// Time the marker is placed at.
    pub time: GtdTimestamp,
    /// WGS-84 latitude, interpolated from the surrounding nav fixes.
    pub lat_deg: f64,
    /// WGS-84 longitude, interpolated from the surrounding nav fixes.
    pub lon_deg: f64,
}

/// Return the number of map markers in the file.
///
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_marker_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    unsafe { (*file).file.markers().len() }
}

/// Fill @p out with data for the map marker at @p index.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than `gtd_nav_file_marker_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last map marker.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_marker(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdMarkerInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(marker) = handle.file.markers().get(index) else {
            error::set_last_error(format!("marker index {index} is out of range"));
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };

        let label = marker.annotation.label();
        super::fill_c_str(&mut out.label, label.unwrap_or(""));
        out.has_label = u8::from(label.is_some());

        let icon = marker.annotation.icon();
        out.icon = match icon {
            AnnotationIcon::Icon(icon) => GtdMarkerIcon::from(icon),
            AnnotationIcon::Unrecognized(_) => GtdMarkerIcon::GTD_ICON_PIN,
        };
        out.icon_code = icon.wire_code();

        out.time = timestamp::ts_from_datetime(marker.annotation.time());
        out.lat_deg = marker.lat.as_degrees();
        out.lon_deg = marker.lon.as_degrees();

        GtdStatus::GTD_OK
    })
}
