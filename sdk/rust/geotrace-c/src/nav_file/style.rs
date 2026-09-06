//! Event marker style data and the accessors that fill it.

use std::ffi::c_char;

use geotrace_sdk::{EventMarkerColor, EventMarkerIconChoice};

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::icon::GtdMarkerIcon;

/// Event marker style data returned by `gtd_nav_file_get_event_marker_style()`.
///
/// All string fields are null-terminated.
#[repr(C)]
pub struct GtdEventMarkerStyleInfo {
    /// Hierarchical event type path the style applies to, e.g. `"system/startup"`.
    pub variant_path: [c_char; 257],
    /// Icon shape for the variant. `GTD_ICON_AUTO` where the style leaves the
    /// icon to the application, and where @ref icon_name is outside the
    /// `GtdMarkerIcon` set.
    pub icon: GtdMarkerIcon,
    /// The icon name the file stores. Empty where the style leaves the icon
    /// to the application, and a name outside the `GtdMarkerIcon` set where a
    /// newer writer stored one.
    pub icon_name: [c_char; 32],
    /// Non-zero if @ref color_hex is set.
    pub has_color: u8,
    /// Fill color, when @ref has_color: `#RRGGBB` unless a newer writer stored
    /// another notation. Without a color the application derives one from
    /// @ref variant_path.
    pub color_hex: [c_char; 8],
}

/// Return the number of event marker styles in the file.
///
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_event_marker_style_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null
    unsafe { (*file).file.event_marker_styles().len() }
}

/// Fill @p out with data for the event marker style at @p index.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than
///              `gtd_nav_file_event_marker_style_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last event marker style.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_event_marker_style(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdEventMarkerStyleInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(style) = handle.file.event_marker_styles().get(index) else {
            error::set_last_error(format!("event marker style index {index} is out of range"));
            return GtdStatus::GTD_ERR_OUT_OF_RANGE;
        };

        super::fill_c_str(&mut out.variant_path, &style.variant_path);

        out.icon = match style.icon {
            EventMarkerIconChoice::Icon(icon) => GtdMarkerIcon::from(icon),
            EventMarkerIconChoice::Auto | EventMarkerIconChoice::Unrecognized(_) => {
                GtdMarkerIcon::GTD_ICON_AUTO
            }
        };
        super::fill_c_str(&mut out.icon_name, style.icon.wire_name());

        let color_hex = match &style.color {
            EventMarkerColor::Auto => None,
            EventMarkerColor::Hex(hex) | EventMarkerColor::Unrecognized(hex) => Some(hex.as_str()),
        };
        super::fill_c_str(&mut out.color_hex, color_hex.unwrap_or(""));
        out.has_color = u8::from(color_hex.is_some());

        GtdStatus::GTD_OK
    })
}
