use std::path::PathBuf;

use gt_store::{AttachedLog, LogAttachmentError};

use super::*;
use crate::app::load_failures::LoadRetryAttempt;
use crate::app::log_viewer::association_dialog::LogAssociationRequest;

fn file_failure(path: PathBuf) -> LoadFailure {
    LoadFailure::new(
        LoadSource::Filename("display-name.log".into()),
        LoadFailureCause::ReadLogFile(io::ErrorKind::PermissionDenied.into()),
    )
    .with_original_read(Some(LoadReadRequest::ReadLogFile(path)))
}

fn controlled_retry(
    app: &mut App,
    attempt: LoadRetryAttempt,
) -> impl FnOnce(Result<LooseLogResult, LoadFailure>) + use<> {
    let batch = app.association_batches.begin_submission(
        app.shared
            .borrow()
            .loaded_files
            .view()
            .entries()
            .map(|entry| entry.id()),
    );
    let complete = app
        .loader
        .controlled_loose_log_load_for_test("display-name.log", batch.log());
    let job = app.loader.loading_jobs.last().unwrap().id;
    app.loader.mark_retry(job, attempt);
    complete
}

#[test]
fn retry_failures_and_background_recovery_preserve_the_active_reading_position() {
    let mut harness = harness(false);
    let text: String = (0..80)
        .map(|index| format!("2026-05-29 19:00:00 navsyncd: reading entry {index}\n"))
        .collect();
    let parsed = gt_logfile::parse_log(text.into(), ui_tests::base_time()).unwrap();
    harness
        .state_mut()
        .load_parsed_log(Some("reading.log".into()), parsed, None);
    harness.run_steps(3);
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test("recording.gtd", RecordingOperationOrigin::Independent);
    recording(Ok(recording_outcome("recording.gtd")));
    harness.run_steps(2);
    harness.state_mut().initial_position_source_policy = InitialPositionSourcePolicy::Ask;
    let active = harness.state().log_viewer.selected_log();
    let anchor = "navsyncd: reading entry 8";
    let position = harness
        .get(By::new().role(Role::Label).value(anchor))
        .rect()
        .center();
    harness.scroll_wheel_at(position, -READING_SCROLL_PX, 4);
    let reading_y = harness
        .get(By::new().role(Role::Label).value(anchor))
        .rect()
        .top();
    let failure = harness
        .state_mut()
        .load_failures
        .push(file_failure("original.log".into()));
    let unrelated = harness
        .state_mut()
        .load_failures
        .push(file_failure("unrelated.log".into()));
    let first = harness
        .state_mut()
        .load_failures
        .begin_retry(failure)
        .unwrap()
        .attempt;
    let failed = controlled_retry(harness.state_mut(), first);
    harness.run_steps(2);
    assert_eq!(harness.state().log_viewer.selected_log(), active);
    failed(Err(file_failure("original.log".into())));
    harness.run_steps(2);
    assert_eq!(harness.state().log_viewer.selected_log(), active);
    let second = harness
        .state_mut()
        .load_failures
        .begin_retry(failure)
        .unwrap()
        .attempt;
    assert_ne!(first, second);
    let recovered = controlled_retry(harness.state_mut(), second);
    let stale = controlled_retry(harness.state_mut(), first);
    stale(Ok(log_outcome("stale.log")));
    harness.step();
    assert_eq!(harness.state().logs.len(), 1);
    assert!(harness.state().load_failures.retry_is_pending(second));
    recovered(Ok(log_outcome("recovered.log")));
    harness.run_steps(3);
    assert_eq!(harness.state().logs.len(), 2);
    assert!(harness.state().association_dialog.is_none());
    assert_eq!(harness.state().log_viewer.selected_log(), active);
    assert!(
        (harness
            .get(By::new().role(Role::Label).value(anchor))
            .rect()
            .top()
            - reading_y)
            .abs()
            < READING_TOLERANCE_PX
    );
    assert_eq!(harness.state().load_failures.iter().count(), 1);
    let dismissed = harness
        .state_mut()
        .load_failures
        .begin_retry(unrelated)
        .unwrap()
        .attempt;
    let discarded = controlled_retry(harness.state_mut(), dismissed);
    harness.state_mut().load_failures.dismiss(unrelated);
    discarded(Ok(log_outcome("dismissed.log")));
    harness.step();
    assert_eq!(harness.state().logs.len(), 2);
    assert_eq!(harness.state().load_failures.iter().count(), 0);
    let recovered = harness
        .state()
        .logs
        .iter_with_ids()
        .find(|(_, log)| log.name() == "recovered.log")
        .unwrap()
        .0;
    harness.state_mut().log_viewer.open_on_log(recovered);
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .association_dialog
            .as_ref()
            .map(|dialog| dialog.log()),
        Some(recovered)
    );
}

#[test]
fn concurrent_normal_and_retry_completions_update_only_their_own_failures() {
    let mut harness = harness(false);
    let first = harness
        .state_mut()
        .load_failures
        .push(file_failure("first.log".into()));
    let second = harness
        .state_mut()
        .load_failures
        .push(file_failure("second.log".into()));
    let a = harness
        .state_mut()
        .load_failures
        .begin_retry(first)
        .unwrap()
        .attempt;
    let b = harness
        .state_mut()
        .load_failures
        .begin_retry(second)
        .unwrap()
        .attempt;
    let pending_a = controlled_retry(harness.state_mut(), a);
    let pending_b = controlled_retry(harness.state_mut(), b);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let normal = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("normal.log", batch.log());
    normal(Ok(log_outcome("normal.log")));
    harness.step();
    let selected = harness.state().log_viewer.selected_log();
    assert_eq!(harness.state().load_failures.iter().count(), 2);
    pending_b(Ok(log_outcome("second.log")));
    pending_a(Err(file_failure("first.log".into())));
    harness.run_steps(2);
    assert_eq!(harness.state().load_failures.iter().count(), 1);
    assert_eq!(harness.state().log_viewer.selected_log(), selected);
    assert!(
        harness
            .state_mut()
            .load_failures
            .begin_retry(first)
            .is_some()
    );
    assert!(
        harness
            .state_mut()
            .load_failures
            .begin_retry(second)
            .is_none()
    );
}

#[test]
fn retry_dispatch_reads_the_original_path_and_rejects_a_duplicate_pending_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actual.log");
    std::fs::write(&path, "2026-05-29 19:00:00 navsyncd: actual source\n").unwrap();
    let mut harness = harness(false);
    let failure = harness.state_mut().load_failures.push(file_failure(path));
    harness.state_mut().retry_load_failure(failure);
    harness.state_mut().retry_load_failure(failure);
    assert_eq!(harness.state().loader.loading_jobs.len(), 1);
    assert!(harness.step_until(|harness| harness.state().logs.len() == 1));
    assert_eq!(harness.state().first_log().unwrap().name(), "actual.log");
    assert_eq!(harness.state().load_failures.iter().count(), 0);
    let invalid = harness.state_mut().load_failures.push(LoadFailure::new(
        LoadSource::PastedText,
        LoadFailureCause::ParseLog(LogParseError::Empty),
    ));
    harness.state_mut().retry_load_failure(invalid);
    assert!(harness.state().loader.loading_jobs.is_empty());
}

#[derive(Clone, Copy)]
enum AttachmentRetryOutcome {
    Dismiss,
    DismissParsing,
    FailParse,
    FailRead,
    Recover,
}

#[rstest]
fn attachment_retry_preserves_its_attempt_across_read_and_parse_channels(
    #[values(
        AttachmentRetryOutcome::Dismiss,
        AttachmentRetryOutcome::DismissParsing,
        AttachmentRetryOutcome::FailParse,
        AttachmentRetryOutcome::FailRead,
        AttachmentRetryOutcome::Recover
    )]
    outcome: AttachmentRetryOutcome,
) {
    let mut harness = harness(false);
    let attachment = LogAttachmentRef {
        recording: DatabaseRef {
            identity: "device".into(),
            group_name: "recording".into(),
        },
        id: LogAttachmentId::new_random(),
    };
    let failure = LoadFailure::new(
        LoadSource::Attachment {
            attachment: attachment.clone(),
            name: "display.log".into(),
        },
        LoadFailureCause::ReadLogAttachment(Box::new(LogAttachmentError::Io {
            path: "original.zst".into(),
            source: io::ErrorKind::PermissionDenied.into(),
        })),
    )
    .with_original_read(Some(LoadReadRequest::ReadAttachment(attachment.clone())));
    let id = harness.state_mut().load_failures.push(failure);
    let attempt = harness
        .state_mut()
        .load_failures
        .begin_retry(id)
        .unwrap()
        .attempt;
    if matches!(outcome, AttachmentRetryOutcome::Dismiss) {
        harness.state_mut().load_failures.dismiss(id);
    }
    let result = if matches!(outcome, AttachmentRetryOutcome::FailRead) {
        Err(LogAttachmentError::Io {
            path: "original.zst".into(),
            source: io::ErrorKind::Interrupted.into(),
        })
    } else {
        Ok(AttachedLog {
            name: "actual.log".into(),
            text: if matches!(outcome, AttachmentRetryOutcome::FailParse) {
                "invalid".into()
            } else {
                "2026-05-29 19:00:00 navsyncd: attached source\n".into()
            },
            settings: StoredLogSettings {
                association_window: None,
                filters: Default::default(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
            parse_year_reference: ui_tests::base_time(),
        })
    };
    harness
        .state_mut()
        .handle_history_response(history_db::Response::AttachedLogLoaded {
            attachment: attachment.clone(),
            name: "display.log".into(),
            retry: Some(attempt),
            log: result,
        });
    if matches!(outcome, AttachmentRetryOutcome::DismissParsing) {
        assert!(harness.state().load_failures.retry_is_pending(attempt));
        harness.state_mut().load_failures.dismiss(id);
        assert!(harness.step_until(|harness| harness.state().loader.loading_jobs.is_empty()));
    }
    assert!(harness.step_until(|harness| !harness.state().load_failures.retry_is_pending(attempt)));
    match outcome {
        AttachmentRetryOutcome::Dismiss | AttachmentRetryOutcome::DismissParsing => {
            assert!(harness.state().loader.loading_jobs.is_empty());
            assert_eq!(harness.state().logs.len(), 0);
        }
        AttachmentRetryOutcome::FailParse => {
            let retained = harness.state().load_failures.iter().next().unwrap();
            assert!(
                matches!(&retained.source, LoadSource::Attachment { attachment: original, .. } if original == &attachment)
            );
            assert!(matches!(
                retained.recovery(),
                LoadRecovery::EnterText("invalid")
            ));
        }
        AttachmentRetryOutcome::FailRead => {
            let next = harness
                .state_mut()
                .load_failures
                .begin_retry(id)
                .unwrap()
                .attempt;
            harness
                .state_mut()
                .handle_history_response(history_db::Response::AttachedLogLoaded {
                    attachment: attachment.clone(),
                    name: "stale.log".into(),
                    retry: Some(attempt),
                    log: Err(LogAttachmentError::UnknownAttachment { id: attachment.id }),
                });
            assert!(harness.state().load_failures.retry_is_pending(next));
        }
        AttachmentRetryOutcome::Recover => {
            assert_eq!(
                harness.state().first_log().unwrap().attachment(),
                Some(&attachment)
            );
            assert_eq!(harness.state().load_failures.iter().count(), 0);
        }
    }
}

#[rstest]
#[case::cancel("Cancel", false)]
#[case::dismiss("Cancel", true)]
#[case::load("Use stored tracks", false)]
fn a_recording_retry_settings_prompt_completes_or_cancels_its_attempt(
    #[case] choice: &str,
    #[case] dismiss: bool,
) {
    let mut harness = harness(false);
    let recording = DatabaseRef {
        identity: "device".into(),
        group_name: "recording".into(),
    };
    let failure = LoadFailure::new(
        LoadSource::StoredRecording(recording.clone()),
        LoadFailureCause::OpenStoredRecording(Box::new(DbError::Io(
            io::ErrorKind::PermissionDenied.into(),
        ))),
    )
    .with_original_read(Some(LoadReadRequest::OpenStoredRecording {
        recording: recording.clone(),
        placement: LoadedRecordingPlacement::AddAnEntry,
    }));
    let id = harness.state_mut().load_failures.push(failure);
    assert!(!harness.state().history.available());
    harness.state_mut().retry_load_failure(id);
    let first = harness
        .state_mut()
        .load_failures
        .begin_retry(id)
        .unwrap()
        .attempt;
    harness
        .state_mut()
        .handle_history_response(history_db::Response::Opened {
            origin: RecordingOperationOrigin::Retry(first),
            db_ref: recording.clone(),
            placement: LoadedRecordingPlacement::AddAnEntry,
            result: Err(DbError::Io(io::ErrorKind::Interrupted.into())),
        });
    assert!(!harness.state().load_failures.retry_is_pending(first));
    let attempt = harness
        .state_mut()
        .load_failures
        .begin_retry(id)
        .unwrap()
        .attempt;
    harness
        .state_mut()
        .handle_history_response(history_db::Response::Opened {
            origin: RecordingOperationOrigin::Retry(first),
            db_ref: recording.clone(),
            placement: LoadedRecordingPlacement::AddAnEntry,
            result: Err(DbError::SchemaTooNew {
                found: 99,
                supported: 1,
            }),
        });
    assert!(harness.state().load_failures.retry_is_pending(attempt));
    let mut segmentation =
        loader::stored_segmentation_from_config(&harness.state().processing_config);
    segmentation.track_split_gap_us += 1;
    harness
        .state_mut()
        .handle_history_response(history_db::Response::Opened {
            origin: RecordingOperationOrigin::Retry(attempt),
            db_ref: recording,
            placement: LoadedRecordingPlacement::AddAnEntry,
            result: Ok(OpenedRecording {
                stored: StoredRecording {
                    bytes: ui_tests::minimal_gtd_bytes(),
                    tracks: ui_tests::two_live_track_ranges().to_vec(),
                    segmentation: Some(segmentation),
                    debug_tag: None,
                },
                ui_state: Ok(RecordingUiState::default()),
            }),
        });
    harness.run_steps(2);
    assert!(harness.state().pending_resegment.is_some());
    assert!(harness.state().load_failures.retry_is_pending(attempt));
    if dismiss {
        harness.state_mut().load_failures.dismiss(id);
    } else {
        harness.get_by_label(choice).click();
    }
    assert!(harness.step_until(|harness| !harness.state().load_failures.retry_is_pending(attempt)));
    harness.step();
    assert!(harness.state().pending_resegment.is_none());
    if choice == "Use stored tracks" {
        assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
    } else if !dismiss {
        assert!(harness.state_mut().load_failures.begin_retry(id).is_some());
    }
}

#[rstest]
fn load_problems_consume_escape_after_explicit_activation(#[values(false, true)] activate: bool) {
    let mut harness = harness(false);
    harness.state_mut().load_parsed_log(
        Some("reading.log".into()),
        parsed_log("reading context"),
        None,
    );
    harness.run_steps(3);
    harness.ctx.memory_mut(|memory| memory.stop_text_input());
    let selected = harness.state().log_viewer.selected_log().unwrap();
    let reading_y = harness
        .get(By::new().role(Role::Label).value("reading context"))
        .rect()
        .top();
    harness
        .state_mut()
        .load_failures
        .push(file_failure("first.log".into()));
    harness
        .state_mut()
        .load_failures
        .push(file_failure("second.log".into()));
    harness.run_steps(3);
    if activate {
        harness.get_by_label("Details").click();
        harness.run_steps(2);
    } else {
        assert_eq!(harness.ctx.memory(|memory| memory.focused()), None);
    }
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().load_failures.iter().count(),
        if activate { 1 } else { 2 }
    );
    assert!(harness.state().log_viewer.open);
    if !activate {
        return;
    }
    assert_eq!(harness.state().log_viewer.selected_log(), Some(selected));
    assert_eq!(
        harness
            .get(By::new().role(Role::Label).value("reading context"))
            .rect()
            .top(),
        reading_y
    );
    harness
        .state_mut()
        .log_viewer_requests
        .open_association_dialog = Some(LogAssociationRequest::SelectSource(selected));
    harness.state_mut().apply_log_viewer_requests();
    harness.run_steps(3);
    assert!(harness.state().association_dialog.is_some());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.state().association_dialog.is_none());
    assert_eq!(harness.state().load_failures.iter().count(), 1);
    assert!(harness.state().log_viewer.open);
}

const READING_SCROLL_PX: f32 = 40.0;
const READING_TOLERANCE_PX: f32 = 1.0;
