//! The opaque handle for a parsed or built nav file.

mod channel;
mod event_marker;
mod marker;
mod metadata;
mod nav_point;
mod read;
mod satellite_warning;
mod style;
mod write;

use std::ffi::{CString, c_char};
use std::fmt;

use geotrace_sdk::{NavFile, SatelliteWarning};

use crate::GtdTimestamp;
use crate::error::{self, GtdStatus};
use crate::timestamp;
use metadata::TerminatedMetaValue;

pub use channel::GtdChannelInfo;
pub use event_marker::GtdEventMarkerInfo;
pub use marker::GtdMarkerInfo;
pub use nav_point::GtdNavPointInfo;
pub use satellite_warning::GtdSatelliteWarningInfo;
pub use style::GtdEventMarkerStyleInfo;

/// Opaque handle for a parsed or freshly-built navigation file.
pub struct GtdNavFile {
    file: NavFile,
    title: Option<TerminatedMetaValue>,
    device: Option<TerminatedMetaValue>,
    notes: Option<TerminatedMetaValue>,
    identity: Option<TerminatedMetaValue>,
    travel_mode: Option<TerminatedMetaValue>,
    sdk_version: Option<CString>,
    sdk_git_commit: Option<CString>,
    sdk_commit_time: GtdTimestamp,
    satellite_warnings: Vec<SatelliteWarning>,
    channel_c_strings: Vec<Result<channel::ChannelCStrings, channel::ChannelStringWithNul>>,
}

impl GtdNavFile {
    pub(crate) fn from_nav_file(file: NavFile) -> Self {
        let to_cstring = |s: &str| CString::new(s).ok();
        Self {
            title: file.meta().title().map(TerminatedMetaValue::new),
            device: file.meta().device().map(TerminatedMetaValue::new),
            notes: file.meta().notes().map(TerminatedMetaValue::new),
            identity: file.meta().identity().map(TerminatedMetaValue::new),
            travel_mode: file
                .meta()
                .travel_mode()
                .map(geotrace_sdk::TravelMode::name)
                .map(TerminatedMetaValue::new),
            sdk_version: file.meta().sdk_version().and_then(to_cstring),
            sdk_git_commit: file.meta().sdk_git_commit().and_then(to_cstring),
            sdk_commit_time: file
                .meta()
                .sdk_commit_time()
                .map_or_else(|| timestamp::gtd_ts_none(), timestamp::ts_from_datetime),
            satellite_warnings: geotrace_sdk::collect_satellite_warnings(
                file.nav_points()
                    .iter()
                    .filter_map(|point| point.satellites.as_ref()),
            ),
            channel_c_strings: file
                .channels()
                .iter()
                .map(channel::ChannelCStrings::new)
                .collect(),
            file,
        }
    }
}

/// Copy `s` and a nul terminator into `dst` and zero-fill the rest of `dst`, or
/// leave `dst` unwritten when the two do not fit.
fn fill_c_str(dst: &mut [c_char], s: &str) -> Result<(), CStringPastBuffer> {
    let length_with_terminator = s.len().saturating_add(1);
    if length_with_terminator > dst.len() {
        return Err(CStringPastBuffer {
            length_with_terminator,
            capacity: dst.len(),
        });
    }
    dst.fill(0);
    for (slot, byte) in dst.iter_mut().zip(s.bytes()) {
        *slot = byte as c_char;
    }
    Ok(())
}

/// [`fill_c_str`] into the struct field `field_name`, returning
/// `GTD_ERR_FIELD_TOO_LONG` when `s` does not fit.
fn fill_struct_field(
    field: &mut [c_char],
    s: &str,
    StructFieldName(field_name): StructFieldName,
) -> Result<(), GtdStatus> {
    fill_c_str(field, s).map_err(|error| {
        error::set_last_error(format!("{field_name}: {error}"));
        GtdStatus::GTD_ERR_FIELD_TOO_LONG
    })
}

struct StructFieldName(&'static str);

struct CStringPastBuffer {
    length_with_terminator: usize,
    capacity: usize,
}

impl fmt::Display for CStringPastBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            length_with_terminator,
            capacity,
        } = self;
        write!(
            f,
            "{length_with_terminator} bytes with the nul terminator, past a buffer of {capacity} bytes"
        )
    }
}
