//! Logs and recordings the crate's tests associate against each other.

use chrono::{DateTime, Duration, TimeZone as _, Utc};
use gt_history_types::{DatabaseRef, RecordingMeta};
use gt_loaded_files::{FileHistory, LoadedFileId, LoadedFiles};
use gt_logfile::ParsedLog;
use gt_test_utils::{empty_file_metadata, loaded_track_with_points, nav_points_walking_from};
use gt_types::{FileMetadata, FileSource, Latitude, LoadedFile, Longitude, NavPoint, TimeRange};
use rustc_hash::FxHashMap;

use crate::{LoadedLog, RecordingKey};

/// The window the tests associate with, matching the app's default.
pub(crate) fn association_window() -> Duration {
    Duration::seconds(60)
}

/// The moment the fixture log's first entry is written at, and the fixture
/// recordings' first fix.
pub(crate) fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 14, 2, 11)
        .single()
        .expect("valid timestamp")
}

/// A log of `count` entries, one per second from [`start`], in the ISO format
/// the parser detects from the head of the log.
pub(crate) fn parsed_log(count: usize) -> ParsedLog {
    parsed_log_of_service("navsyncd", count)
}

/// [`parsed_log`] with `service` writing the entries, which is what makes two
/// fixture logs differ in content.
pub(crate) fn parsed_log_of_service(service: &str, count: usize) -> ParsedLog {
    let text: String = (0..count)
        .map(|second| {
            let time = start() + Duration::seconds(second as i64);
            format!(
                "{} {service}: entry {second}\n",
                time.format("%Y-%m-%d %H:%M:%S")
            )
        })
        .collect();
    parsed_log_of_text(&text)
}

/// The log `text` parses into, in the format its head declares.
pub(crate) fn parsed_log_of_text(text: &str) -> ParsedLog {
    gt_logfile::parse_log(text.into(), start()).expect("the fixture log parses")
}

pub(crate) fn log_of(count: usize) -> LoadedLog {
    LoadedLog::new(
        Some("navsyncd.log".to_owned()),
        parsed_log(count),
        association_window(),
    )
}

/// A log of `count` entries written by `service`, which
/// [`LoadedLogs::push`](crate::LoadedLogs::push) tells apart from [`log_of`].
pub(crate) fn log_of_service(service: &str, count: usize) -> LoadedLog {
    LoadedLog::new(
        Some(format!("{service}.log")),
        parsed_log_of_service(service, count),
        association_window(),
    )
}

/// A recording of `count` fixes from [`start`], its first fix at the given
/// latitude so tests can tell two recordings apart by where an entry landed.
pub(crate) fn recording_at(first_lat_deg: f64, count: usize) -> LoadedFile {
    recording_of(nav_points_walking_from(
        start(),
        count,
        1,
        Latitude::new(first_lat_deg),
        Longitude::new(12.0),
    ))
}

/// A recording of `count` fixes, one per second, starting `offset` after the
/// fixture log does.
pub(crate) fn recording_from(offset: Duration, count: usize) -> LoadedFile {
    recording_of(nav_points_walking_from(
        start() + offset,
        count,
        1,
        Latitude::new(55.0),
        Longitude::new(12.0),
    ))
}

fn recording_of(points: Vec<NavPoint>) -> LoadedFile {
    let time_range = TimeRange::new(
        points.first().map_or_else(start, |p| p.tpv.time().utc()),
        points.last().map_or_else(start, |p| p.tpv.time().utc()),
    );
    LoadedFile {
        metadata: FileMetadata {
            time_range,
            ..empty_file_metadata()
        },
        tracks: vec![loaded_track_with_points(points)],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdPath(std::path::PathBuf::new()),
        load_warnings: Vec::new(),
    }
}

pub(crate) fn loaded(files: Vec<LoadedFile>) -> LoadedFiles {
    let mut loaded = LoadedFiles::new();
    for file in files {
        loaded.push(file, FileHistory::None);
    }
    loaded
}

pub(crate) fn id_of(files: &LoadedFiles, index: usize) -> LoadedFileId {
    files
        .view()
        .get(index)
        .map(|entry| entry.id())
        .expect("the fixture file is loaded")
}

/// The key the fixture recording at `index` is anchored under.
pub(crate) fn key_of(files: &LoadedFiles, index: usize) -> RecordingKey {
    files
        .view()
        .get(index)
        .map(RecordingKey::of_loaded_recording)
        .expect("the fixture file is loaded")
}

/// Anchors `log` to the fixture recording at `index`, as choosing that
/// recording in the viewer does.
pub(crate) fn anchor_to(log: &mut LoadedLog, files: &LoadedFiles, index: usize) {
    log.anchor_to(key_of(files, index), &files.view());
}

/// The recording in the history database the crate's tests anchor a stored log
/// to.
pub(crate) fn stored_recording_ref() -> DatabaseRef {
    DatabaseRef {
        identity: "nav-devkit-mk2".to_owned(),
        group_name: "2026-01-01T14-02-11".to_owned(),
    }
}

/// The session sidecar of a recording the history database holds as `db_ref`.
pub(crate) fn stored_in_history(db_ref: &DatabaseRef) -> FileHistory {
    FileHistory::recording(
        db_ref.identity.clone(),
        RecordingMeta {
            start_us: 0,
            end_us: 0,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        Some(db_ref.clone()),
    )
}
