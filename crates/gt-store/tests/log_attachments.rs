//! The store's half of a log attachment: the compressed log beside the
//! recording history, and what a load checks it against.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::slice;

use chrono::{DateTime, Duration};
use gt_history_types::{
    self as history_types, LogAttachmentReadError, StoredLogFilterCondition, fixtures,
};
use gt_store::{
    DatabaseRef, DbError, HistoryDatabase as _, LogAttachmentEntry, LogAttachmentError,
    LogAttachmentId, LogAttachments as _, LogToAttach, ReadOnlyHistoryDatabase as _,
    ReadOnlyLogAttachments as _, RecordingMeta, Recordings, Store, StoredLogFilterEffects,
    StoredLogFilterGroup, StoredLogFilterOperator, StoredLogFilterStack, StoredLogFilterStackParts,
    StoredLogSettings, StoredYearResolution, TrackRange, TrackState,
};
use gt_types::AssociationWindow;
use hdf5_pure::AttrValue;
use rstest::rstest;
use serde_json::Value;

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
            state: TrackState::Live,
        }];
        let recording = recordings
            .insert(
                "nav-devkit-mk2",
                &meta,
                &tracks,
                fixtures::default_segmentation(),
                &bytes,
            )
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
                    settings: StoredLogSettings {
                        association_window: None,
                        filters: fixtures::log_filters(),
                        year_reference: None,
                        year_resolution: StoredYearResolution::Infer,
                    },
                },
            )
            .expect("attach")
            .id
    }

    fn log_path(&self, id: LogAttachmentId) -> PathBuf {
        id.file_path(&self.store.logs_path())
    }

    /// Logs in the store directory, counted from disk.
    fn stored_log_count(&self) -> usize {
        match std::fs::read_dir(self.store.logs_path()) {
            Ok(entries) => entries.count(),
            Err(_) => 0,
        }
    }
}

struct MixedAttachmentMetadata {
    recorded: RecordedStore,
    readable: LogAttachmentEntry,
    failures: Vec<(LogAttachmentId, AttrValue, Option<&'static str>)>,
    group_path: String,
}

#[expect(
    clippy::expect_used,
    reason = "test setup requires successful I/O and metadata encoding"
)]
impl MixedAttachmentMetadata {
    fn new() -> Self {
        let RecordedStore {
            _directory,
            store,
            mut recordings,
            recording,
        } = RecordedStore::new();
        let readable = recordings
            .attach_log(
                &recording,
                &LogToAttach {
                    name: "readable.log",
                    text: LOG_TEXT,
                    settings: StoredLogSettings {
                        association_window: None,
                        filters: fixtures::log_filters(),
                        year_reference: None,
                        year_resolution: StoredYearResolution::Infer,
                    },
                },
            )
            .expect("attach readable log");
        let base = serde_json::to_value(&readable.attachment).expect("metadata");
        let base = base.as_object().expect("metadata object").clone();
        let mut future = base.clone();
        future.insert("name".to_owned(), serde_json::json!("future.log"));
        future.insert("format_version".to_owned(), serde_json::json!(8));
        let mut unknown = base.clone();
        unknown.insert("name".to_owned(), serde_json::json!("unknown.log"));
        unknown.insert(
            "year_resolution".to_owned(),
            serde_json::json!({"kind": "future_policy"}),
        );
        let mut invalid_window = base.clone();
        invalid_window.insert("name".to_owned(), serde_json::json!("invalid-window.log"));
        invalid_window.insert("association_window_ns".to_owned(), Value::Null);
        let mut malformed = base;
        malformed.insert("name".to_owned(), serde_json::json!("malformed.log"));
        malformed.insert("year_resolution".to_owned(), Value::Null);
        let failures: Vec<_> = [
            (
                AttrValue::String(Value::Object(invalid_window).to_string()),
                Some("invalid-window.log"),
            ),
            (
                AttrValue::String(Value::Object(future).to_string()),
                Some("future.log"),
            ),
            (
                AttrValue::String(Value::Object(unknown).to_string()),
                Some("unknown.log"),
            ),
            (
                AttrValue::String(Value::Object(malformed).to_string()),
                Some("malformed.log"),
            ),
            (AttrValue::String("{".to_owned()), None),
            (AttrValue::U64(42), None),
        ]
        .into_iter()
        .map(|(value, name)| (LogAttachmentId::new_random(), value, name))
        .collect();
        drop(recordings);
        let group_path = format!(
            "by_identity/{}/{}",
            history_types::identity_group_name(&recording.identity),
            recording.group_name
        );
        {
            let file = hdf5_pure::File::open_rw(store.recordings_path()).expect("edit metadata");
            let group = file.root().group(&group_path).expect("recording group");
            for (id, value, _) in &failures {
                group
                    .set_attr(&id.attr_key(), value.clone())
                    .expect("write unreadable metadata");
            }
            file.commit().expect("commit metadata");
        }
        let recordings = store.open_recordings().expect("open mixed metadata");
        Self {
            recorded: RecordedStore {
                _directory,
                store,
                recordings,
                recording,
            },
            readable,
            failures,
            group_path,
        }
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

#[rstest]
#[case::legacy(None)]
#[case::explicit(Some(AssociationWindow::ZERO))]
fn read_only_loading_preserves_raw_settings_and_resolves_a_missing_reference(
    #[case] association_window: Option<AssociationWindow>,
) {
    let mut recorded = RecordedStore::new();

    let settings = StoredLogSettings {
        association_window,
        filters: fixtures::log_filters(),
        year_reference: None,
        year_resolution: StoredYearResolution::Infer,
    };
    let entry = recorded
        .recordings
        .attach_log(
            &recorded.recording,
            &LogToAttach {
                name: "navsyncd.log",
                text: LOG_TEXT,
                settings: settings.clone(),
            },
        )
        .expect("attach");

    let database_before = fs::read(recorded.store.recordings_path()).expect("database before load");
    let read_only = recorded
        .store
        .open_recordings_read_only()
        .expect("open read-only database")
        .expect("database");
    let attached = read_only
        .load_attached_log(&recorded.recording, entry.id)
        .expect("load");
    assert_eq!(attached.name, "navsyncd.log");
    assert_eq!(attached.text, LOG_TEXT);
    assert_eq!(attached.settings, settings);
    assert_eq!(
        attached.parse_year_reference.timestamp_micros(),
        1_000_000_019
    );
    assert_eq!(
        fs::read(recorded.store.recordings_path()).expect("database after load"),
        database_before
    );
}

#[test_log::test]
fn a_saved_year_reference_and_year_resolution_survive_filter_updates_and_reopening() {
    let mut recorded = RecordedStore::new();
    let year_reference = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
        .expect("reference date")
        .to_utc();
    let attached = recorded
        .recordings
        .attach_log(
            &recorded.recording,
            &LogToAttach {
                name: "syslog.log",
                text: "Dec 31 23:59:59 before\nJan  1 00:00:01 after\n",
                settings: StoredLogSettings {
                    association_window: Some(
                        AssociationWindow::try_from(Duration::nanoseconds(987_654_321))
                            .expect("window"),
                    ),
                    filters: fixtures::log_filters(),
                    year_reference: Some(year_reference),
                    year_resolution: StoredYearResolution::IgnoreSummaryBounds,
                },
            },
        )
        .expect("attach with a reference");
    recorded
        .recordings
        .set_attached_log_year_resolution(
            &recorded.recording,
            attached.id,
            StoredYearResolution::PlaceLastEntryInYear { year: 2024 },
        )
        .expect("update the year resolution");
    recorded
        .recordings
        .set_attached_log_filters(&recorded.recording, attached.id, Default::default())
        .expect("update filters");
    let reopened = recorded
        .store
        .open_recordings_read_only()
        .expect("reopen the database")
        .expect("the database exists");
    let restored = reopened
        .load_attached_log(&recorded.recording, attached.id)
        .expect("restore the log");
    assert_eq!(restored.parse_year_reference, year_reference);
    assert_eq!(
        restored.settings.year_resolution,
        StoredYearResolution::PlaceLastEntryInYear { year: 2024 }
    );
    assert_eq!(restored.settings.filters, Default::default());
    assert_eq!(
        restored.settings.association_window.map(Duration::from),
        Some(Duration::nanoseconds(987_654_321))
    );
}

#[test]
fn complete_settings_can_restore_the_recording_end_reference() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("syslog.log", LOG_TEXT);
    let mut settings = StoredLogSettings {
        association_window: None,
        filters: StoredLogFilterStack::default(),
        year_reference: Some(DateTime::UNIX_EPOCH),
        year_resolution: StoredYearResolution::IgnoreSummaryBounds,
    };
    recorded
        .recordings
        .set_attached_log_settings(&recorded.recording, id, settings.clone())
        .expect("set explicit reference");
    settings.year_reference = None;
    recorded
        .recordings
        .set_attached_log_settings(&recorded.recording, id, settings.clone())
        .expect("restore recording reference");

    let reopened = recorded
        .store
        .open_recordings_read_only()
        .expect("reopen")
        .expect("database");
    assert_eq!(
        reopened
            .log_attachment(&recorded.recording, id)
            .expect("metadata")
            .expect("attachment")
            .attachment
            .settings,
        settings
    );
    assert_eq!(
        reopened
            .load_attached_log(&recorded.recording, id)
            .expect("load")
            .parse_year_reference
            .timestamp_micros(),
        1_000_000_019
    );
}

#[cfg(unix)]
#[test]
fn a_settings_write_failure_preserves_the_complete_metadata_and_log() {
    let mut recorded = RecordedStore::new();
    let id = recorded.attach("syslog.log", LOG_TEXT);
    let db_path = recorded.store.recordings_path();
    let before = fs::read(&db_path).expect("database bytes");
    let compressed = fs::read(recorded.log_path(id)).expect("compressed log");
    let permissions = fs::metadata(&db_path)
        .expect("database metadata")
        .permissions();
    fs::set_permissions(&db_path, fs::Permissions::from_mode(0o444))
        .expect("make database read-only");
    if fs::OpenOptions::new().write(true).open(&db_path).is_ok() {
        fs::set_permissions(&db_path, permissions).expect("restore database permissions");
        eprintln!("Skipped: the process can write to a file with mode 0o444");
        return;
    }

    let result = recorded.recordings.set_attached_log_settings(
        &recorded.recording,
        id,
        StoredLogSettings {
            association_window: None,
            filters: StoredLogFilterStack::default(),
            year_reference: Some(DateTime::UNIX_EPOCH),
            year_resolution: StoredYearResolution::IgnoreSummaryBounds,
        },
    );
    fs::set_permissions(&db_path, permissions).expect("restore database permissions");
    assert!(matches!(result, Err(LogAttachmentError::Database(_))));
    assert_eq!(fs::read(&db_path).expect("database after failure"), before);
    assert_eq!(
        fs::read(recorded.log_path(id)).expect("compressed log"),
        compressed
    );
    let reopened = recorded
        .store
        .open_recordings_read_only()
        .expect("reopen")
        .expect("database");
    let restored = reopened
        .load_attached_log(&recorded.recording, id)
        .expect("load after failure");
    assert_eq!(restored.settings.filters, fixtures::log_filters());
    assert_eq!(
        restored.settings.year_resolution,
        StoredYearResolution::Infer
    );
    assert_eq!(
        restored.parse_year_reference.timestamp_micros(),
        1_000_000_019
    );
}

enum SettingsUpdate {
    AssociationWindow,
    Complete,
    Filters,
    YearResolution,
}

#[rstest]
#[case::filters(SettingsUpdate::Filters)]
#[case::year_resolution(SettingsUpdate::YearResolution)]
#[case::complete(SettingsUpdate::Complete)]
#[case::association_window(SettingsUpdate::AssociationWindow)]
fn legacy_settings_upgrade_during_store_updates(
    #[case] update: SettingsUpdate,
    #[values(5, 6)] version: u32,
) {
    let RecordedStore {
        _directory,
        store,
        mut recordings,
        recording,
    } = RecordedStore::new();
    let reference = DateTime::parse_from_rfc3339("2026-10-01T00:00:00.123456789Z")
        .expect("reference date")
        .to_utc();
    let entry = recordings
        .attach_log(
            &recording,
            &LogToAttach {
                name: "legacy.log",
                text: LOG_TEXT,
                settings: StoredLogSettings {
                    association_window: None,
                    filters: fixtures::log_filters(),
                    year_reference: Some(reference),
                    year_resolution: StoredYearResolution::PlaceLastEntryInYear { year: 2024 },
                },
            },
        )
        .expect("attach");
    let log_path = entry.id.file_path(&store.logs_path());
    let compressed = std::fs::read(&log_path).expect("compressed log");
    let legacy = serde_json::json!({
        "format_version": version,
        "name": "legacy.log",
        "content_hash": entry.attachment.content_hash,
        "filters": fixtures::log_filters(),
        "year_reference": "2026-10-01T00:00:00.123456789Z",
        "year_resolution": { "kind": "place_last_entry_in_year", "year": 2024 },
    });
    drop(recordings);
    let group_path = format!(
        "by_identity/{}/{}",
        history_types::identity_group_name(&recording.identity),
        recording.group_name
    );
    {
        let file = hdf5_pure::File::open_rw(store.recordings_path()).expect("edit legacy metadata");
        file.root()
            .group(&group_path)
            .expect("recording group")
            .set_attr(&entry.id.attr_key(), AttrValue::String(legacy.to_string()))
            .expect("write legacy version");
        file.commit().expect("commit legacy metadata");
    }
    let mut recordings = store.open_recordings().expect("open legacy database");
    assert_eq!(
        recordings
            .log_attachments(&recording)
            .expect("legacy listing")
            .into_iter()
            .map(|listed| listed
                .into_attachment(&recording)
                .expect("readable metadata"))
            .collect::<Vec<_>>(),
        slice::from_ref(&entry)
    );
    let mut expected = entry.attachment;
    match update {
        SettingsUpdate::AssociationWindow => {
            expected.settings.association_window = Some(
                AssociationWindow::try_from(Duration::nanoseconds(123_456_789)).expect("window"),
            );
            recordings
                .set_attached_log_settings(&recording, entry.id, expected.settings.clone())
                .expect("update window");
        }
        SettingsUpdate::Filters => {
            expected.settings.filters = StoredLogFilterStack::default();
            recordings
                .set_attached_log_filters(&recording, entry.id, expected.settings.filters.clone())
                .expect("update filters");
        }
        SettingsUpdate::YearResolution => {
            expected.settings.year_resolution = StoredYearResolution::IgnoreSummaryBounds;
            recordings
                .set_attached_log_year_resolution(
                    &recording,
                    entry.id,
                    expected.settings.year_resolution,
                )
                .expect("update policy");
        }
        SettingsUpdate::Complete => {
            expected.settings = StoredLogSettings {
                association_window: Some(
                    AssociationWindow::try_from(Duration::nanoseconds(123_456_789))
                        .expect("window"),
                ),
                filters: StoredLogFilterStack::default(),
                year_reference: DateTime::from_timestamp(1, 987_654_321),
                year_resolution: StoredYearResolution::IgnoreSummaryBounds,
            };
            recordings
                .set_attached_log_settings(&recording, entry.id, expected.settings.clone())
                .expect("update complete settings");
        }
    }
    drop(recordings);
    let reopened = store
        .open_recordings_read_only()
        .expect("reopen")
        .expect("database");
    let listed = reopened
        .log_attachments(&recording)
        .expect("list upgraded metadata");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed.first().map(|listed| listed.id), Some(entry.id));
    assert_eq!(
        listed
            .first()
            .map(|entry| entry.attachment.as_ref().expect("metadata")),
        Some(&expected)
    );
    let restored = reopened
        .load_attached_log(&recording, entry.id)
        .expect("load upgraded log");
    assert_eq!(
        restored.parse_year_reference,
        expected.settings.year_reference.expect("reference")
    );
    assert_eq!(restored.settings, expected.settings);
    assert_eq!(restored.text, LOG_TEXT);
    assert_eq!(
        std::fs::read(&log_path).expect("compressed log"),
        compressed
    );
    let file = hdf5_pure::File::open(store.recordings_path()).expect("read persisted version");
    let attrs = file
        .root()
        .group(&group_path)
        .expect("recording group")
        .attrs()
        .expect("attributes");
    let json = attrs
        .get(&entry.id.attr_key())
        .and_then(AttrValue::as_str)
        .expect("attachment JSON");
    let wire: Value = serde_json::from_str(json).expect("JSON");
    assert_eq!(wire.get("format_version"), Some(&serde_json::json!(7)));
}

#[test]
fn unreadable_metadata_is_listed_with_sources_and_readable_siblings() {
    let MixedAttachmentMetadata {
        recorded:
            RecordedStore {
                _directory,
                recordings,
                recording,
                ..
            },
        readable,
        failures,
        ..
    } = MixedAttachmentMetadata::new();
    let listed = recordings
        .log_attachments(&recording)
        .expect("list attachments");
    assert_eq!(listed.len(), failures.len() + 1);
    assert_eq!(
        recordings
            .list_recordings()
            .expect("list recordings")
            .first()
            .expect("recording")
            .log_attachments
            .len(),
        listed.len()
    );
    recordings.load(&recording).expect("readable recording");
    assert_eq!(
        recordings
            .load_attached_log(&recording, readable.id)
            .expect("readable sibling")
            .text,
        LOG_TEXT
    );
    for (id, value, name) in &failures {
        let entry = listed
            .iter()
            .find(|entry| entry.id == *id)
            .expect("failed entry is listed");
        let source = entry.attachment.as_ref().expect_err("unreadable metadata");
        match (value, source.as_ref()) {
            (AttrValue::String(_), LogAttachmentReadError::Decode(error)) => {
                assert_eq!(error.name.as_deref(), *name)
            }
            (
                AttrValue::U64(_),
                LogAttachmentReadError::NonTextAttribute | LogAttachmentReadError::ReadAttribute(_),
            ) => {}
            _ => panic!("unexpected metadata error: {source:#}"),
        }
        if let Some(name) = name {
            assert_eq!(entry.display_name(), *name);
        } else {
            assert!(entry.display_name().contains(&id.to_string()));
        }
    }
}

#[test]
fn unreadable_metadata_rejects_updates_and_survives_other_attachment_updates() {
    let MixedAttachmentMetadata {
        recorded:
            RecordedStore {
                _directory,
                store,
                mut recordings,
                recording,
            },
        readable,
        failures,
        group_path,
    } = MixedAttachmentMetadata::new();
    assert_eq!(
        recordings
            .log_attachment_with_content(&recording, readable.attachment.content_hash)
            .expect("readable match")
            .expect("matching log")
            .id,
        readable.id
    );
    assert!(matches!(
        recordings.log_attachment_with_content(
            &recording,
            history_types::LogContentHash::of_log_bytes(b"another log")
        ),
        Err(DbError::UnreadableLogAttachment { .. })
    ));
    for (id, _, _) in &failures {
        let results = [
            recordings.load_attached_log(&recording, *id).map(|_| ()),
            recordings.set_attached_log_filters(&recording, *id, StoredLogFilterStack::default()),
            recordings.set_attached_log_year_resolution(
                &recording,
                *id,
                StoredYearResolution::Infer,
            ),
            recordings.set_attached_log_settings(
                &recording,
                *id,
                readable.attachment.settings.clone(),
            ),
        ];
        for result in results {
            let error = result.expect_err("metadata must remain unreadable");
            assert!(
                matches!(&error, LogAttachmentError::Database(DbError::UnreadableLogAttachment { recording: found, id: found_id, .. }) if found == &recording && *found_id == *id)
            );
        }
    }
    recordings
        .set_attached_log_settings(
            &recording,
            readable.id,
            StoredLogSettings {
                association_window: Some(AssociationWindow::ZERO),
                filters: StoredLogFilterStack::default(),
                year_reference: Some(DateTime::UNIX_EPOCH),
                year_resolution: StoredYearResolution::IgnoreSummaryBounds,
            },
        )
        .expect("update readable sibling");
    drop(recordings);
    {
        let file = hdf5_pure::File::open(store.recordings_path()).expect("read persisted metadata");
        let attrs = file
            .root()
            .group(&group_path)
            .expect("recording group")
            .attrs()
            .expect("attributes");
        for (id, value, _) in &failures {
            assert_eq!(attrs.get(&id.attr_key()), Some(value));
        }
    }
    let mut reopened = store.open_recordings().expect("reopen metadata");
    let removed = failures.first().expect("failed attachment").0;
    reopened
        .detach_log(&recording, removed)
        .expect("detach unreadable metadata");
    let kept = reopened
        .log_attachments(&recording)
        .expect("list remaining attachments");
    assert_eq!(kept.len(), failures.len());
    assert!(kept.iter().all(|entry| entry.id != removed));
    assert_eq!(
        reopened
            .load_attached_log(&recording, readable.id)
            .expect("readable sibling after removal")
            .text,
        LOG_TEXT
    );
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
                settings: StoredLogSettings {
                    association_window: None,
                    filters: fixtures::log_filters(),
                    year_reference: None,
                    year_resolution: StoredYearResolution::Infer,
                },
            },
        )
        .expect("attach");

    assert_eq!(
        recorded
            .recordings
            .log_attachments(&recorded.recording)
            .expect("list")
            .into_iter()
            .map(|listed| listed
                .into_attachment(&recorded.recording)
                .expect("metadata"))
            .collect::<Vec<_>>(),
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

/// Detaching works when the log it refers to is already gone.
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

    let fixture_filters = fixtures::log_filters();
    let mut groups = fixture_filters.groups().to_vec();
    groups.first_mut().expect("default group").operator = StoredLogFilterOperator::Any;
    groups.push(StoredLogFilterGroup {
        id: 9,
        operator: StoredLogFilterOperator::All,
    });
    groups.push(StoredLogFilterGroup {
        id: 42,
        operator: StoredLogFilterOperator::Any,
    });
    let mut chips = fixture_filters.chips().to_vec();
    let highlighted = chips.first_mut().expect("highlighted filter");
    highlighted.group_id = 9;
    highlighted.effects = StoredLogFilterEffects::Both {
        table_enabled: true,
        map_enabled: false,
        color_slot: 3,
    };
    highlighted.condition = StoredLogFilterCondition::Service {
        text: "navsyncd".into(),
    };
    chips.last_mut().expect("table filter").condition = StoredLogFilterCondition::Hostname {
        text: "receiver".into(),
    };
    let filters = StoredLogFilterStack::try_from_parts(StoredLogFilterStackParts {
        groups,
        selected_group_id: 42,
        chips,
    })
    .expect("valid group memberships");
    recorded
        .recordings
        .set_attached_log_filters(&recorded.recording, id, filters.clone())
        .expect("rewrite the filters");

    let reopened = recorded
        .store
        .open_recordings_read_only()
        .expect("reopen")
        .expect("stored database");
    let attached = reopened
        .load_attached_log(&recorded.recording, id)
        .expect("load");
    assert_eq!(attached.settings.filters, filters);
    assert_eq!(attached.name, "navsyncd.log");
    assert_eq!(attached.text, LOG_TEXT);
    assert_eq!(
        std::fs::read(recorded.log_path(id)).expect("read the stored log"),
        stored_log
    );
    assert_eq!(recorded.stored_log_count(), 1);
}

#[test_log::test]
fn updating_settings_for_an_unknown_attachment_returns_unknown_attachment() {
    let mut recorded = RecordedStore::new();

    assert!(matches!(
        recorded.recordings.set_attached_log_settings(
            &recorded.recording,
            LogAttachmentId::new_random(),
            StoredLogSettings {
                association_window: None,
                filters: fixtures::log_filters(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
        ),
        Err(LogAttachmentError::UnknownAttachment { .. })
    ));
}

/// Attaching to a recording that was deleted from under the session fails
/// and leaves the logs directory as it was.
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
            settings: StoredLogSettings {
                association_window: None,
                filters: fixtures::log_filters(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
        },
    );

    assert!(matches!(attached, Err(LogAttachmentError::Database(_))));
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
        .map(|entry| entry.attachment.as_ref().expect("metadata").content_hash)
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

/// A journald-shaped log, long enough that its stored copy is visibly
/// compressed.
const LOG_TEXT: &str = concat!(
    "2026-01-01 14:02:11 navsyncd: gnss fix acquired\n",
    "2026-01-01 14:02:12 hal-powerd: battery low\n",
    "2026-01-01 14:02:13 navsyncd: gnss fix lost\n",
    "2026-01-01 14:02:14 hal-powerd: battery critical\n",
);

const OTHER_LOG_TEXT: &str = "2026-01-01 14:03:01 nav-devkit-mk2: booted\n";
