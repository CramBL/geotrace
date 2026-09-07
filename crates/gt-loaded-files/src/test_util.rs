#![cfg(test)]
//! Fixtures shared between the test modules of gt-loaded-files.

use gt_history_types::RecordingMeta;
use gt_types::{FileMetadata, LoadedFile};

/// A recording the history database holds nothing measured about.
pub fn empty_recording_meta() -> RecordingMeta {
    RecordingMeta {
        time_range: None,
        nav_point_count: 0,
        sat_report_count: 0,
        marker_count: 0,
        event_marker_count: 0,
        gtd_size_bytes: 0,
    }
}

pub fn empty_file() -> LoadedFile {
    gt_test_utils::loaded_file_with_tracks(Vec::new())
}

/// [`empty_file`] under a filename and a title, for the tests over the label
/// a recording is listed under.
pub fn named_file(filename: &str, title: Option<&str>) -> LoadedFile {
    named_file_with_tracks(filename, title, 0)
}

/// [`named_file`] with `track_count` tracks of no fixes, which is what decides
/// whether a track label carries a track number.
pub fn named_file_with_tracks(
    filename: &str,
    title: Option<&str>,
    track_count: usize,
) -> LoadedFile {
    let tracks = (0..track_count)
        .map(|_| gt_test_utils::loaded_track_with_points(Vec::new()))
        .collect();
    let file = gt_test_utils::loaded_file_with_tracks(tracks);
    LoadedFile {
        metadata: FileMetadata {
            filename: filename.to_owned(),
            title: title.map(ToOwned::to_owned),
            ..file.metadata
        },
        ..file
    }
}
