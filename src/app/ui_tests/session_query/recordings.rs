use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use gt_session_bridge::{
    ErrorCode, ListRecordings, ManageRecordings, RecordingReference, Reply, Request,
};
use gt_store::{HistoryAccess, ReadOnlyHistoryDatabase as _, Store};
use uuid::Uuid;

use crate::app::test_util::{harness, recordings, session};
use crate::app::{App, ui_tests};

#[test]
fn session_files_use_the_desktop_loader_and_keep_recording_history() {
    let dir = tempfile::tempdir().unwrap();
    let recording = dir.path().join("ride.GTD");
    let log = dir.path().join("service.LoG");
    fs::write(&recording, recordings::sample_bytes()).unwrap();
    fs::write(&log, "2026-02-01 14:00:01 host service[1]: ready\n").unwrap();
    let store = Store::open_in(dir.path());
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);

    let open = self::open_paths(vec![recording.clone(), log, recording.clone()]);
    assert_eq!(
        session::execute(&mut harness, open),
        Reply::FilesQueued { queued_files: 2 }
    );
    assert!(harness.state().loader.loading_jobs.is_empty());
    databases
        .send(harness::storage_opened_in(
            &store,
            &harness.ctx,
            &harness.state().pending_writes,
        ))
        .unwrap();
    self::wait_without_painting(&mut harness, |app| {
        app.shared.borrow().loaded_files.len() == 1 && app.logs.len() == 1
    });

    let reference = self::loaded_recordings(&mut harness)[0];
    assert!(harness.state().log_viewer.open);
    assert_eq!(
        session::execute(&mut harness, self::unload(vec![reference, reference])),
        Reply::RecordingsUnloaded {
            unloaded_recordings: 1
        }
    );
    assert!(self::loaded_recordings(&mut harness).is_empty());
    {
        let _access = HistoryAccess::lock();
        let db = store.open_recordings_read_only().unwrap().unwrap();
        assert_eq!(db.list_recordings().unwrap().len(), 1);
    }
    assert!(recording.is_file());

    assert_eq!(
        session::execute(&mut harness, self::open_paths(vec![recording])),
        Reply::FilesQueued { queued_files: 1 }
    );
    self::wait_without_painting(&mut harness, |app| {
        app.pending_recordings_already_in_history.is_some()
    });
    assert!(self::loaded_recordings(&mut harness).is_empty());
}

#[test]
fn invalid_file_batches_admit_no_loads_and_shutdown_rejects_new_commands() {
    let mut harness = ui_tests::app_with_a_recording();
    let dir = tempfile::tempdir().unwrap();
    let valid = dir.path().join("ride.gtd");
    let original_notices = harness.state().toasts.len();
    for (invalid, error) in [
        (
            PathBuf::from("relative.log"),
            ErrorCode::InvalidLocalFilePath,
        ),
        #[cfg(windows)]
        (
            PathBuf::from(r"\\host\share\receiver.log"),
            ErrorCode::InvalidLocalFilePath,
        ),
        #[cfg(windows)]
        (
            PathBuf::from(r"\\.\receiver.log"),
            ErrorCode::InvalidLocalFilePath,
        ),
        (
            dir.path().join("invalid\0.log"),
            ErrorCode::InvalidLocalFilePath,
        ),
        (
            dir.path().join("notes.txt"),
            ErrorCode::UnsupportedFileExtension,
        ),
    ] {
        let request = self::open_paths(vec![valid.clone(), invalid]);
        assert_eq!(session::execute(&mut harness, request), Reply::from(error));
        assert!(harness.state().loader.loading_jobs.is_empty());
        assert_eq!(harness.state().toasts.len(), original_notices);
    }
    let reference = self::loaded_recordings(&mut harness)[0];
    harness.state_mut().begin_shutdown();
    for request in [self::open_paths(vec![valid]), self::unload(vec![reference])] {
        assert_eq!(
            session::execute(&mut harness, request),
            Reply::from(ErrorCode::ShuttingDown)
        );
    }
    assert_eq!(self::loaded_recordings(&mut harness), [reference]);
}

#[test]
fn a_missing_file_is_queued_and_then_reports_the_desktop_load_failure() {
    let mut harness = ui_tests::app_with_a_recording();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.log");
    assert_eq!(
        session::execute(&mut harness, self::open_paths(vec![missing])),
        Reply::FilesQueued { queued_files: 1 }
    );
    self::wait_without_painting(&mut harness, |app| !app.load_failures.is_empty());
    assert!(harness.state().logs.is_empty());
}

#[cfg(unix)]
#[test]
fn a_separator_spelling_does_not_skip_the_file_path() {
    let mut harness = ui_tests::app_with_a_recording();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("receiver.log");
    fs::write(&log, "2026-02-01 14:00:01 host service[1]: ready\n").unwrap();
    let mut with_separator = log.as_os_str().to_owned();
    with_separator.push("/");
    let request = self::open_paths(vec![PathBuf::from(with_separator), log]);
    assert_eq!(
        session::execute(&mut harness, request),
        Reply::FilesQueued { queued_files: 2 }
    );
    self::wait_without_painting(&mut harness, |app| {
        app.logs.len() == 1 && !app.load_failures.is_empty()
    });
}

#[test]
fn an_invalid_unload_batch_leaves_all_recordings_and_their_references_in_place() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let original = self::loaded_recordings(&mut harness);
    let first = original[0];
    for (invalid, error) in [
        (
            RecordingReference {
                number: u64::MAX,
                ..first
            },
            ErrorCode::RecordingNotFound,
        ),
        (
            RecordingReference {
                session: Uuid::from_u128(99),
                ..first
            },
            ErrorCode::StaleSession,
        ),
    ] {
        assert_eq!(
            session::execute(&mut harness, self::unload(vec![first, invalid])),
            Reply::from(error)
        );
        assert_eq!(self::loaded_recordings(&mut harness), original);
    }
    assert_eq!(
        session::execute(&mut harness, self::unload(vec![first])),
        Reply::RecordingsUnloaded {
            unloaded_recordings: 1
        }
    );
    assert_eq!(self::loaded_recordings(&mut harness), original[1..]);
}

fn open_paths(paths: Vec<PathBuf>) -> Request {
    Request::ManageRecordings(ManageRecordings::Open {
        paths: paths.try_into().unwrap(),
    })
}

fn unload(recordings: Vec<RecordingReference>) -> Request {
    Request::ManageRecordings(ManageRecordings::Unload {
        recordings: recordings.try_into().unwrap(),
    })
}

fn loaded_recordings(harness: &mut Harness<'_, App>) -> Vec<RecordingReference> {
    let reply = session::execute(
        harness,
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    );
    let Reply::Recordings { page } = reply else {
        panic!("expected loaded recordings, received {reply:?}");
    };
    page.recordings
        .into_iter()
        .map(|recording| recording.id)
        .collect()
}

fn wait_without_painting(harness: &mut Harness<'_, App>, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready(harness.state()) {
        assert!(Instant::now() < deadline, "recording operation timed out");
        harness::step_minimized_logic(harness);
        thread::sleep(Duration::from_millis(5));
    }
}
