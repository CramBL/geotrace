//! The store's half of a log attachment: the compressed log beside the
//! recording history, and what a load checks it against.

use std::path::{Path, PathBuf};

use gt_store::{
    DatabaseRef, HistoryDatabase as _, LogAttachmentError, LogAttachmentId, LogAttachments as _,
    LogToAttach, ReadOnlyHistoryDatabase as _, ReadOnlyLogAttachments as _, RecordingMeta,
    Recordings, Store, StoredLogFilter, StoredLogFilterMode, StoredSegmentation, TrackRange,
};

/// A journald-shaped log, long enough that its stored copy is visibly
/// compressed.
const LOG_TEXT: &str = concat!(
    "2026-01-01 14:02:11 navsyncd: gnss fix acquired\n",
    "2026-01-01 14:02:12 hal-powerd: battery low\n",
    "2026-01-01 14:02:13 navsyncd: gnss fix lost\n",
    "2026-01-01 14:02:14 hal-powerd: battery critical\n",
);

const OTHER_LOG_TEXT: &str = "2026-01-01 14:03:01 nav-devkit-mk2: booted\n";

/// A stack with one chip of each mode, with and without a palette slot.
fn log_filters() -> Vec<StoredLogFilter> {
    vec![
        StoredLogFilter {
            text: "gnss".to_owned(),
            regex: false,
            enabled: true,
            mode: StoredLogFilterMode::Layer { color_slot: 3 },
        },
        StoredLogFilter {
            text: "hal-powerd|navsyncd".to_owned(),
            regex: true,
            enabled: false,
            mode: StoredLogFilterMode::Refine,
        },
    ]
}

/// A store with one recording in its history, ready to attach logs to.
struct RecordedStore {
    _directory: tempfile::TempDir,
    store: Store,
    recordings: Recordings,
    recording: DatabaseRef,
}

#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
impl RecordedStore {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(directory.path());
        let mut recordings = store.open_recordings().expect("recordings");
        let bytes = gtd_bytes();
        let meta = gt_store::extract_meta(&bytes).expect("meta");
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            hidden: false,
        }];
        let recording = recordings
            .insert("nav-devkit-mk2", &meta, &tracks, segmentation(), &bytes)
            .expect("insert");
        Self {
            _directory: directory,
            store,
            recordings,
            recording,
        }
    }

    fn attach(&mut self, name: &str, text: &str) -> LogAttachmentId {
        self.recordings
            .attach_log(
                &self.recording,
                &LogToAttach {
                    name,
                    text,
                    filters: log_filters(),
                },
            )
            .expect("attach")
            .id
    }

    fn log_path(&self, id: LogAttachmentId) -> PathBuf {
        id.file_path(&self.store.logs_path())
    }

    /// Logs in the store directory, counted from disk rather than from the
    /// attributes naming them.
    fn stored_log_count(&self) -> usize {
        match std::fs::read_dir(self.store.logs_path()) {
            Ok(entries) => entries.count(),
            Err(_) => 0,
        }
    }
}

fn segmentation() -> StoredSegmentation {
    StoredSegmentation {
        track_split_gap_us: 300_000_000,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    }
}

/// A minimal GTD file: one `nav_points/time` dataset, which is all the
/// history database needs to store a recording.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
fn gtd_bytes() -> Vec<u8> {
    let timestamps: Vec<i64> = (0..20).map(|i| 1_000_000_000 + i).collect();
    let mut builder = hdf5_pure::FileBuilder::new();
    let mut nav_points = builder.create_group("nav_points");
    let time = nav_points.create_dataset("time");
    time.with_shape(&[timestamps.len() as u64]);
    time.with_i64_data(&timestamps);
    builder.add_group(nav_points.finish());
    builder.finish().expect("serialize the gtd file")
}

#[test_log::test]
fn an_attached_log_comes_back_with_its_name_text_and_filters() {
    let mut recorded = RecordedStore::new();

    let id = recorded.attach("navsyncd.log", LOG_TEXT);

    let attached = recorded
        .recordings
        .load_attached_log(&recorded.recording, id)
        .expect("load");
    assert_eq!(attached.name, "navsyncd.log");
    assert_eq!(attached.text, LOG_TEXT);
    assert_eq!(attached.filters, log_filters());
}

/// The attach returns the entry the recording now lists.
#[test_log::test]
fn attaching_returns_the_attachment_the_recording_now_lists() {
    let mut recorded = RecordedStore::new();

    let attached = recorded
        .recordings
        .attach_log(
            &recorded.recording,
            &LogToAttach {
                name: "navsyncd.log",
                text: LOG_TEXT,
                filters: log_filters(),
            },
        )
        .expect("attach");

    assert_eq!(
        recorded
            .recordings
            .log_attachments(&recorded.recording)
            .expect("list"),
        vec![attached]
    );
}

#[test_log::test]
fn an_attached_log_is_stored_compressed_under_the_stores_logs_directory() {
    let mut recorded = RecordedStore::new();

    let id = recorded.attach("navsyncd.log", LOG_TEXT);

    let path = recorded.log_path(id);
    assert_eq!(path.parent(), Some(recorded.store.logs_path().as_path()));
    assert_eq!(path.extension(), Some(Path::new("zst").as_os_str()));
    let stored = std::fs::metadata(&path).expect("stored log");
    assert!(
        stored.len() < LOG_TEXT.len() as u64,
        "the stored log must be compressed"
    );
}

#[test_log::test]
fn detaching_a_log_removes_it_and_leaves_the_other_attachments() {
    let mut recorded = RecordedStore::new();
    let detached = recorded.attach("navsyncd.log", LOG_TEXT);
    let kept = recorded.attach("hal-powerd.log", OTHER_LOG_TEXT);

    recorded
        .recordings
        .detach_log(&recorded.recording, detached)
        .expect("detach");

    assert!(!recorded.log_path(detached).exists());
    assert_eq!(recorded.stored_log_count(), 1);
    assert!(matches!(
        recorded
            .recordings
            .load_attached_log(&recorded.recording, detached),
        Err(LogAttachmentError::UnknownAttachment { .. })
    ));
    recorded
        .recordings
        .load_attached_log(&recorded.recording, kept)
        .expect("the other attachment is untouched");
}

/// Detaching works when the log it names is already gone.
#[test_log::test]
fn detaching_a_log_that_is_already_gone_removes_the_attachment() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("navsyncd.log", LOG_TEXT);
    std::fs::remove_file(recorded.log_path(id)).expect("remove the log by hand");

    recorded
        .recordings
        .detach_log(&recorded.recording, id)
        .expect("detach");

    assert!(
        recorded
            .recordings
            .log_attachments(&recorded.recording)
            .expect("list")
            .is_empty()
    );
}

/// A log the store no longer holds is reported as missing, and the recording
/// stays readable.
#[test_log::test]
fn loading_an_attached_log_that_is_gone_reports_it_missing() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("navsyncd.log", LOG_TEXT);
    std::fs::remove_file(recorded.log_path(id)).expect("remove the log by hand");

    assert!(matches!(
        recorded
            .recordings
            .load_attached_log(&recorded.recording, id),
        Err(LogAttachmentError::MissingLog { .. })
    ));
    assert_eq!(
        recorded
            .recordings
            .log_attachments(&recorded.recording)
            .expect("list")
            .len(),
        1,
        "the attachment is still listed, for the viewer to report or remove"
    );
}

/// A stored log that is no longer the one attached is rejected.
#[test_log::test]
fn an_attached_log_replaced_on_disk_reports_a_content_hash_mismatch() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("navsyncd.log", LOG_TEXT);
    let other = recorded.attach("hal-powerd.log", OTHER_LOG_TEXT);
    std::fs::copy(recorded.log_path(other), recorded.log_path(id))
        .expect("put another log in its place");

    assert!(matches!(
        recorded
            .recordings
            .load_attached_log(&recorded.recording, id),
        Err(LogAttachmentError::ContentHashMismatch { .. })
    ));
}

/// Changing filters rewrites the attribute. The stored log is
/// content-addressed and never rewritten.
#[test_log::test]
fn changing_the_filters_of_an_attachment_leaves_its_log_alone() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("navsyncd.log", LOG_TEXT);
    let stored_log = std::fs::read(recorded.log_path(id)).expect("read the stored log");

    recorded
        .recordings
        .set_attached_log_filters(&recorded.recording, id, Vec::new())
        .expect("rewrite the filters");

    let attached = recorded
        .recordings
        .load_attached_log(&recorded.recording, id)
        .expect("load");
    assert_eq!(attached.filters, Vec::new());
    assert_eq!(attached.name, "navsyncd.log");
    assert_eq!(attached.text, LOG_TEXT);
    assert_eq!(
        std::fs::read(recorded.log_path(id)).expect("read the stored log"),
        stored_log
    );
    assert_eq!(recorded.stored_log_count(), 1);
}

#[test_log::test]
fn changing_the_filters_of_an_attachment_a_recording_never_had_fails() {
    let mut recorded = RecordedStore::new();

    assert!(matches!(
        recorded.recordings.set_attached_log_filters(
            &recorded.recording,
            LogAttachmentId::new_random(),
            log_filters(),
        ),
        Err(LogAttachmentError::UnknownAttachment { .. })
    ));
}

/// Attaching to a recording that was deleted from under the session fails
/// and leaves no log behind.
#[test_log::test]
fn attaching_to_a_deleted_recording_fails_and_stores_no_log() {
    let mut recorded = RecordedStore::new();
    recorded
        .recordings
        .delete_batch(std::slice::from_ref(&recorded.recording))
        .expect("delete");

    let attached = recorded.recordings.attach_log(
        &recorded.recording,
        &LogToAttach {
            name: "navsyncd.log",
            text: LOG_TEXT,
            filters: log_filters(),
        },
    );

    assert!(matches!(attached, Err(LogAttachmentError::Database(_))));
    assert_eq!(recorded.stored_log_count(), 0);
}

/// Deleting a recording deletes the logs attached to it, compressed files
/// included.
#[test_log::test]
fn deleting_a_recording_deletes_the_logs_attached_to_it() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("navsyncd.log", LOG_TEXT);

    recorded
        .recordings
        .delete_batch(std::slice::from_ref(&recorded.recording))
        .expect("delete");

    assert!(!recorded.log_path(id).exists());
    assert_eq!(recorded.stored_log_count(), 0);
}

/// The same log attached twice is stored twice, each copy named by its own
/// attachment. The duplicate query is what the dialog warns from.
#[test_log::test]
fn the_same_log_attached_twice_is_found_by_its_content_hash() {
    let mut recorded = RecordedStore::new();
    let first = recorded.attach("navsyncd.log", LOG_TEXT);

    let hash = recorded
        .recordings
        .log_attachments(&recorded.recording)
        .expect("list")
        .first()
        .map(|entry| entry.attachment.content_hash)
        .expect("the attachment that was just written");
    let duplicate = recorded
        .recordings
        .log_attachment_with_content(&recorded.recording, hash)
        .expect("query");
    assert_eq!(duplicate.map(|entry| entry.id), Some(first));

    let second = recorded.attach("navsyncd.log", LOG_TEXT);
    assert_ne!(second, first);
    assert_eq!(recorded.stored_log_count(), 2);
}

/// `RecordingMeta` is what the history database dedupes recordings by.
/// Attaching a log leaves it untouched.
#[test_log::test]
fn attaching_a_log_leaves_the_recording_itself_unchanged() {
    let mut recorded = RecordedStore::new();
    let before: RecordingMeta = recorded
        .recordings
        .load(&recorded.recording)
        .map(|stored| gt_store::extract_meta(&stored.bytes).expect("meta"))
        .expect("load");

    recorded.attach("navsyncd.log", LOG_TEXT);

    let after = recorded
        .recordings
        .load(&recorded.recording)
        .map(|stored| gt_store::extract_meta(&stored.bytes).expect("meta"))
        .expect("load");
    assert_eq!(after, before);
}
