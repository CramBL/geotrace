use std::fs;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use chrono::DateTime;
use gt_loaded_files::{FileHistory, LoadedFileId, LoadedFiles};
use gt_log_view::{
    AttachmentSettingsStatus, InitialLogSaveOutcome, InitialLogSaveStatus, LoadedLog, LoadedLogs,
    LogAttachmentRef, LogSaveCompletion, LogSaveRequest,
};
use gt_logfile::YearResolution;
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::{
    self as store, DatabaseRef, HistoryDatabase as _, LogAttachmentId, LogAttachments as _,
    LogToAttach, ReadOnlyHistoryDatabase as _, ReadOnlyLogAttachments as _, ReadOnlyRecordings,
    RecordingMeta, Recordings, RecordingsHandle, StoredLogSettings, StoredYearResolution,
    UiStateVersionReporter,
};
use gt_types::AssociationWindow;
use gt_ui_types::LoadedLogId;
use hdf5_pure::{AttrValue, File};
use rstest::rstest;

use crate::app::history_db::{self, HistoryWorker, Response, WriteRequest};
use crate::app::test_util::recordings;

use super::{SettingsSubmissionFailure, SettingsWriteFailure};

#[test]
fn queued_saves_reuse_one_stored_attachment() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.h5");
    recordings::seed_two_track_recording(&path);
    let worker = recordings::worker_on(&path);
    let recording = recordings::only_recording(&worker).db_ref;
    let mut fixture = SaveFixture::new(recording.clone());
    let request = fixture.prepare();
    let initial = request.settings().clone();
    assert!(worker.save_log(request.clone()).is_none());
    assert!(worker.save_log(request).is_none());
    let Response::LogSaved(first) = worker.resp_rx.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("log save completion");
    };
    let Response::LogSaved(repeated) = worker.resp_rx.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("log save completion");
    };
    let saved = first.result().unwrap().entry.clone();
    assert_eq!(repeated.result().unwrap().entry, saved);
    assert_eq!(saved.attachment.settings, initial);
    assert_eq!(
        fixture.acknowledge(first),
        Some(InitialLogSaveOutcome::Saved(fixture.id))
    );
    assert_eq!(fixture.acknowledge(repeated), None);
    worker.shutdown();
    let db = Recordings::open_or_create(&path).unwrap();
    let attachments = db.log_attachments(&recording).unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments.first().unwrap().id, saved.id);
}

#[test]
fn duplicate_save_acknowledges_actual_metadata_before_persisting_current_complete_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.h5");
    recordings::seed_two_track_recording(&path);
    let mut db = Recordings::open_or_create(&path).unwrap();
    let recording = db
        .list_recordings()
        .unwrap()
        .first()
        .unwrap()
        .db_ref
        .clone();
    let mut fixture = SaveFixture::new(recording.clone());
    let existing = db
        .attach_log(
            &recording,
            &LogToAttach {
                name: "existing.log",
                text: gt_test_utils::UNRESOLVED_YEARS_LOG,
                settings: StoredLogSettings {
                    association_window: None,
                    filters: Default::default(),
                    year_reference: Some(DateTime::UNIX_EPOCH),
                    year_resolution: StoredYearResolution::Infer,
                },
            },
        )
        .unwrap();
    drop(db);
    let worker = recordings::worker_on(&path);
    assert!(
        fixture
            .logs
            .get_mut_by_id(fixture.id)
            .unwrap()
            .resolve_years_as(
                YearResolution::PlaceLastEntryInYear(2026),
                &fixture.files.view(),
            )
    );
    let request = fixture.prepare();
    let (stack, _) = fixture.logs.filter_stack_mut_by_id(fixture.id).unwrap();
    stack.set_live_filter_text("clock");
    stack.add_live_filter_as_chip().unwrap();
    let desired = fixture
        .logs
        .get_by_id(fixture.id)
        .unwrap()
        .settings_snapshot();
    assert_eq!(
        (desired.year_reference, desired.year_resolution),
        (
            Some(gt_test_utils::synthetic_log_start()),
            StoredYearResolution::PlaceLastEntryInYear { year: 2026 }
        ),
    );
    assert!(worker.save_log(request).is_none());
    let Response::LogSaved(completion) = recordings::next_response(&worker) else {
        panic!("reused attachment completion");
    };
    assert_eq!(completion.result().unwrap().entry, existing);
    assert_eq!(
        fixture.acknowledge(completion),
        Some(InitialLogSaveOutcome::Saved(fixture.id))
    );
    assert_eq!(
        fixture
            .logs
            .get_by_id(fixture.id)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
    let write = fixture
        .logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(write.settings(), &desired);
    assert!(worker.store_attached_log_settings(write).is_none());
    let Response::AttachedLogSettingsStored(completion) = recordings::next_response(&worker) else {
        panic!("settings completion");
    };
    assert!(fixture.logs.acknowledge_settings(completion));
    worker.shutdown();
    let db = Recordings::open_or_create(&path).unwrap();
    let restored = db.load_attached_log(&recording, existing.id).unwrap();
    assert_eq!(restored.settings, desired);
    assert_eq!(db.log_attachments(&recording).unwrap().len(), 1);
}

#[rstest]
#[case::same_window(AssociationWindow::try_from_seconds(17).unwrap(), true)]
#[case::different_window(AssociationWindow::ZERO, false)]
fn legacy_reuse_compares_resolved_settings_and_persists_explicit_defaults(
    #[case] loose_window: AssociationWindow,
    #[case] reusable: bool,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.h5");
    recordings::seed_two_track_recording(&path);
    let mut db = Recordings::open_or_create(&path).unwrap();
    let recording = db
        .list_recordings()
        .unwrap()
        .first()
        .unwrap()
        .db_ref
        .clone();
    let legacy = StoredLogSettings {
        association_window: None,
        filters: Default::default(),
        year_reference: None,
        year_resolution: StoredYearResolution::Infer,
    };
    let entry = db
        .attach_log(
            &recording,
            &LogToAttach {
                name: "legacy.log",
                text: gt_test_utils::UNRESOLVED_YEARS_LOG,
                settings: legacy.clone(),
            },
        )
        .unwrap();
    let attached = db.load_attached_log(&recording, entry.id).unwrap();
    drop(db);
    let parsed = gt_logfile::parse_log_with_year_resolution(
        attached.text.into(),
        attached.parse_year_reference,
        YearResolution::Infer,
    )
    .unwrap();
    let default_window = AssociationWindow::try_from_seconds(17).unwrap();
    let mut fixture = SaveFixture::new(recording.clone());
    fixture.logs.remove_by_id(fixture.id);
    let saved = fixture
        .logs
        .restore_attachment(
            LoadedLog::new(
                Some("legacy.log".to_owned()),
                parsed.clone(),
                default_window,
            ),
            LogAttachmentRef {
                recording: recording.clone(),
                id: entry.id,
            },
            legacy.clone(),
            &fixture.files.view(),
        )
        .id();
    assert!(
        fixture
            .logs
            .prepare_settings_writes(WriteAccess::Owner)
            .is_empty()
    );
    fixture.id = fixture
        .logs
        .push(LoadedLog::new(
            Some("loose.log".to_owned()),
            parsed,
            loose_window,
        ))
        .id();
    let worker = recordings::worker_on(&path);
    assert!(worker.save_log(fixture.prepare()).is_none());
    let Response::LogSaved(completion) = recordings::next_response(&worker) else {
        panic!("reuse completion");
    };
    assert_eq!(
        fixture.acknowledge(completion),
        Some(if reusable {
            InitialLogSaveOutcome::AlreadyLoaded(saved)
        } else {
            InitialLogSaveOutcome::Failed
        })
    );
    assert!(
        fixture
            .logs
            .get_by_id(saved)
            .unwrap()
            .association_window_is_defaulted()
    );
    let mut writes = fixture.logs.prepare_settings_writes(WriteAccess::Owner);
    let expected = if reusable {
        let expected = StoredLogSettings {
            association_window: Some(default_window),
            year_reference: Some(attached.parse_year_reference),
            ..legacy
        };
        let write = writes.pop().unwrap();
        assert_eq!(write.settings(), &expected);
        assert!(worker.store_attached_log_settings(write).is_none());
        let Response::AttachedLogSettingsStored(completion) = recordings::next_response(&worker)
        else {
            panic!("settings completion");
        };
        assert!(fixture.logs.acknowledge_settings(completion));
        assert!(
            !fixture
                .logs
                .get_by_id(saved)
                .unwrap()
                .association_window_is_defaulted()
        );
        expected
    } else {
        assert!(writes.is_empty());
        assert!(
            fixture
                .logs
                .get_by_id(fixture.id)
                .unwrap()
                .save_error()
                .is_some()
        );
        legacy
    };
    worker.shutdown();
    let db = ReadOnlyRecordings::open_existing_read_only(&path).unwrap();
    let restored = db.load_attached_log(&recording, entry.id).unwrap();
    assert_eq!(restored.settings, expected);
    assert_eq!(restored.text, gt_test_utils::UNRESOLVED_YEARS_LOG);
    assert_eq!(
        db.log_attachments(&recording)
            .unwrap()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [entry.id]
    );
}

#[rstest]
#[case::read_only_handle(SettingsWriteFailure::ReadOnlyHandle)]
#[case::registry_downgrade(SettingsWriteFailure::RegistryDowngrade)]
#[case::missing_recording(SettingsWriteFailure::MissingRecording)]
fn save_operations_complete_after_live_worker_rejection_or_storage_failure(
    #[case] failure: SettingsWriteFailure,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.h5");
    recordings::seed_two_track_recording(&path);
    let worker = recordings::worker_on(&path);
    let mut recording = recordings::only_recording(&worker).db_ref;
    worker.shutdown();
    if matches!(failure, SettingsWriteFailure::MissingRecording) {
        recording.group_name = "missing".to_owned();
    }
    let mut db = if matches!(failure, SettingsWriteFailure::ReadOnlyHandle) {
        RecordingsHandle::ReadOnly(ReadOnlyRecordings::open_existing_read_only(&path).unwrap())
    } else {
        RecordingsHandle::Owner(Recordings::open_or_create(&path).unwrap())
    };
    let registry = PendingWrites::default();
    let mut fixture = SaveFixture::new(recording);
    let request = fixture.prepare();
    if matches!(failure, SettingsWriteFailure::RegistryDowngrade) {
        registry.become_read_only_for_the_rest_of_the_run();
    }
    let before = fs::read(&path).unwrap();
    let Response::LogSaved(completion) = history_db::run_write_request(
        &mut db,
        WriteRequest::SaveLog(request),
        &registry,
        &UiStateVersionReporter::default(),
    ) else {
        panic!("log save failure");
    };
    assert_eq!(
        fixture.acknowledge(completion),
        Some(InitialLogSaveOutcome::Failed)
    );
    assert!(matches!(
        fixture
            .logs
            .get_by_id(fixture.id)
            .unwrap()
            .initial_save_status(),
        InitialLogSaveStatus::Failed(_)
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(registry.is_idle());
}

#[test]
fn unreadable_attachment_metadata_prevents_duplicate_creation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.h5");
    recordings::seed_two_track_recording(&path);
    let worker = recordings::worker_on(&path);
    let recording = recordings::only_recording(&worker).db_ref;
    worker.shutdown();
    let file = File::open_rw(&path).unwrap();
    let group = file
        .root()
        .group(&format!(
            "by_identity/{}/{}",
            store::identity_group_name(&recording.identity),
            recording.group_name,
        ))
        .unwrap();
    group
        .set_attr(
            &LogAttachmentId::new_random().attr_key(),
            AttrValue::String("{".to_owned()),
        )
        .unwrap();
    file.commit().unwrap();
    drop(group);
    drop(file);
    let worker = recordings::worker_on(&path);
    let mut fixture = SaveFixture::new(recording.clone());
    assert!(worker.save_log(fixture.prepare()).is_none());
    let Response::LogSaved(completion) = recordings::next_response(&worker) else {
        panic!("metadata failure");
    };
    assert_eq!(
        fixture.acknowledge(completion),
        Some(InitialLogSaveOutcome::Failed)
    );
    worker.shutdown();
    let db = Recordings::open_or_create(&path).unwrap();
    assert_eq!(db.log_attachments(&recording).unwrap().len(), 1);
    assert!(!directory.path().join(store::LOGS_DIRECTORY).exists());
}

#[rstest]
#[case::unavailable(
    SettingsSubmissionFailure::Unavailable,
    "Recording history is unavailable"
)]
#[case::disconnected(
    SettingsSubmissionFailure::Disconnected,
    "The recording history worker stopped"
)]
#[case::read_only(SettingsSubmissionFailure::ReadOnly, "this session is read-only")]
fn save_submission_failures_release_the_pending_operation(
    #[case] failure: SettingsSubmissionFailure,
    #[case] error: &str,
) {
    let mut fixture = SaveFixture::new(super::db_ref());
    let request = fixture.prepare();
    let mut worker = HistoryWorker::disabled();
    let mut readable_receiver = None;
    if !matches!(failure, SettingsSubmissionFailure::Unavailable) {
        let (sender, receiver) = mpsc::channel();
        worker.req_tx = Some(sender);
        worker.writable_registry = Some(PendingWrites::default());
        if matches!(failure, SettingsSubmissionFailure::Disconnected) {
            drop(receiver);
        } else {
            readable_receiver = Some(receiver);
            worker
                .writable_registry
                .as_ref()
                .unwrap()
                .become_read_only_for_the_rest_of_the_run();
        }
    }
    let completion = worker.save_log(request).unwrap();
    assert_eq!(
        fixture.acknowledge(completion),
        Some(InitialLogSaveOutcome::Failed)
    );
    assert_eq!(
        fixture
            .logs
            .get_by_id(fixture.id)
            .unwrap()
            .initial_save_status(),
        InitialLogSaveStatus::Failed(error)
    );
    if let Some(receiver) = readable_receiver {
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }
    let (stack, _) = fixture.logs.filter_stack_mut_by_id(fixture.id).unwrap();
    stack.set_live_filter_text("clock");
    stack.add_live_filter_as_chip().unwrap();
    let retry = fixture
        .logs
        .retry_initial_save(fixture.id, &fixture.files.view(), WriteAccess::Owner)
        .unwrap();
    assert_eq!(
        retry.settings(),
        &fixture
            .logs
            .get_by_id(fixture.id)
            .unwrap()
            .settings_snapshot()
    );
}

struct SaveFixture {
    files: LoadedFiles,
    logs: LoadedLogs,
    id: LoadedLogId,
    target: LoadedFileId,
}

impl SaveFixture {
    fn new(recording: DatabaseRef) -> Self {
        let mut files = LoadedFiles::new();
        files.push(
            gt_test_utils::loaded_file_with_tracks(Vec::new()),
            FileHistory::recording(
                recording.identity.clone(),
                RecordingMeta::default(),
                Some(recording),
            ),
        );
        let target = files.view().entries().next().unwrap().id();
        let parsed = gt_logfile::parse_log(
            gt_test_utils::UNRESOLVED_YEARS_LOG.into(),
            gt_test_utils::synthetic_log_start(),
        )
        .unwrap();
        let mut logs = LoadedLogs::default();
        let id = logs
            .push(LoadedLog::new(
                Some("pending.log".to_owned()),
                parsed,
                AssociationWindow::try_from_seconds(60).unwrap(),
            ))
            .id();
        Self {
            files,
            logs,
            id,
            target,
        }
    }

    fn prepare(&mut self) -> LogSaveRequest {
        self.logs
            .prepare_save(self.id, self.target, &self.files.view(), WriteAccess::Owner)
            .unwrap()
    }

    fn acknowledge(&mut self, completion: LogSaveCompletion) -> Option<InitialLogSaveOutcome> {
        self.logs
            .acknowledge_initial_save(completion, &self.files.view())
    }
}
