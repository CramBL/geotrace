//! The opaque handle for a parsed or built nav file.

mod channel;
mod event_marker;
mod metadata;
mod nav_point;
mod read;
mod write;

use std::ffi::{CString, c_char};

use geotrace_sdk::NavFile;

use crate::GtdTimestamp;
use crate::timestamp;

pub use channel::GtdChannelInfo;
pub use event_marker::GtdEventMarkerInfo;
pub use nav_point::GtdNavPointInfo;

/// Opaque handle for a parsed or freshly-built navigation file.
pub struct GtdNavFile {
    file: NavFile,
    title: Option<CString>,
    device: Option<CString>,
    notes: Option<CString>,
    identity: Option<CString>,
    travel_mode: Option<CString>,
    sdk_version: Option<CString>,
    sdk_git_commit: Option<CString>,
    sdk_commit_time: GtdTimestamp,
}

impl GtdNavFile {
    pub(crate) fn from_nav_file(file: NavFile) -> Self {
        let to_cstring = |s: &str| CString::new(s).ok();
        Self {
            title: file.meta().title.as_deref().and_then(to_cstring),
            device: file.meta().device.as_deref().and_then(to_cstring),
            notes: file.meta().notes.as_deref().and_then(to_cstring),
            identity: file.meta().identity.as_deref().and_then(to_cstring),
            travel_mode: file
                .meta()
                .travel_mode
                .as_ref()
                .map(geotrace_sdk::TravelMode::name)
                .and_then(to_cstring),
            sdk_version: file.meta().sdk_version().and_then(to_cstring),
            sdk_git_commit: file.meta().sdk_git_commit().and_then(to_cstring),
            sdk_commit_time: file
                .meta()
                .sdk_commit_time()
                .map_or_else(|| timestamp::gtd_ts_none(), timestamp::ts_from_datetime),
            file,
        }
    }
}

/// Copy `s` into a fixed C-string buffer, zero-filling and always leaving a
/// trailing NUL (truncating an over-long string).
fn fill_c_str(dst: &mut [c_char], s: &str) {
    dst.fill(0);
    let cap = dst.len().saturating_sub(1);
    for (slot, byte) in dst.iter_mut().zip(s.bytes().take(cap)) {
        *slot = byte as c_char;
    }
}
