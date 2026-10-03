use chrono::Duration;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable as _};
use gt_loaded_files::FileHistory;
use gt_log_view::{LoadedLog, LogAttachmentRef, PositionSourceState, RecordingKey};
use gt_logfile::ParsedLog;
use gt_plot::AnalysisConfig;
use gt_store::{
    DatabaseRef, LogAttachmentId, RecordingUiState, Store, StoredLogFilter, StoredLogFilterMode,
    StoredLogFilterStack, StoredRecording,
};
use gt_test_utils::{By, HarnessInteraction as _};
use rstest::rstest;

use crate::app::association_batches::RecordingOperationOrigin;
use crate::app::history_db::{self, OpenedRecording};
use crate::app::loader::{
    LoadedRecordingPlacement, LoadedRecordingResult, LooseLogResult, SavedLogResult,
};
use crate::app::log_viewer::association_dialog;
use crate::app::recording_from_disk::{
    LOAD_FROM_DISK_LABEL, RecordingAlreadyInHistory, RecordingContent, RecordingFromDisk,
    ScreenedRecordings,
};
use crate::app::storage::QueuedLoad;
use crate::app::{App, ParsedLogOrigin, loader, test_util, ui_tests};
use crate::settings::InitialPositionSourcePolicy;

fn harness(ask: bool) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().initial_position_source_policy = if ask {
        InitialPositionSourcePolicy::Ask
    } else {
        InitialPositionSourcePolicy::AutomaticallyUseUnambiguous
    };
    harness
}

fn parsed_log(message: &str) -> ParsedLog {
    let time = ui_tests::base_time() + Duration::seconds(5);
    let text = format!("{} {message}\n", time.format(gt_fmt::UTC_SECOND_FORMAT));
    gt_logfile::parse_log(text.into(), time).expect("parse log timestamp")
}

fn log_outcome(name: &str) -> LooseLogResult {
    LooseLogResult {
        filename: Some(name.to_owned()),
        parsed: parsed_log(name),
    }
}

fn recording_outcome(name: &str) -> LoadedRecordingResult {
    let file = gt_loader::load_bytes(&ui_tests::minimal_gtd_bytes(), name.to_owned())
        .expect("parse recording");
    let series = gt_plot::prepare_file_series(&file, AnalysisConfig::default());
    LoadedRecordingResult {
        file,
        series,
        history: FileHistory::None,
        applied_current_marker_settings: false,
        placement: LoadedRecordingPlacement::AddAnEntry,
    }
}

fn assert_initial_association(harness: &mut Harness<App>, ask: bool, overlapping: usize) {
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .len(),
        usize::from(ask)
    );
    assert_eq!(harness.state().association_dialog.is_some(), ask);
    if ask {
        assert_eq!(
            harness
                .state()
                .first_log()
                .and_then(LoadedLog::associated_recording),
            None
        );
        harness
            .get_by_label(association_dialog::CONFIRM_LABEL)
            .click();
        harness.run_steps(2);
    }
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
    let associated = overlapping == 1;
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording)
            .is_some(),
        associated,
    );
    assert_eq!(
        harness
            .state()
            .first_log()
            .expect("loaded log")
            .associated_entry_count(),
        usize::from(associated)
    );
}

#[rstest]
#[case(true, true)]
#[case(true, false)]
#[case(false, true)]
#[case(false, false)]
fn initial_association_is_independent_of_log_and_recording_completion_order(
    #[case] ask: bool,
    #[case] log_first: bool,
) {
    let mut harness = harness(ask);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "recording.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    drop(batch);
    if log_first {
        log(Ok(log_outcome("log.txt")));
        harness.step();
        assert!(harness.state().association_dialog.is_none());
        assert_eq!(
            harness
                .state()
                .first_log()
                .and_then(LoadedLog::associated_recording),
            None
        );
        recording(Ok(recording_outcome("recording.gtd")));
    } else {
        recording(Ok(recording_outcome("recording.gtd")));
        harness.step();
        log(Ok(log_outcome("log.txt")));
    }
    harness.run_steps(2);
    assert_initial_association(&mut harness, ask, 1);
}

#[rstest]
fn initial_association_waits_for_every_overlapping_recording(
    #[values(true, false)] ask: bool,
    #[values([0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0])] order: [usize; 3],
    #[values(true, false)] separate_frames: bool,
) {
    let mut harness = harness(ask);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    let first = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "first.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let second = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "second.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    drop(batch);
    let mut completions: Vec<Option<Box<dyn FnOnce()>>> = vec![
        Some(Box::new(move || log(Ok(log_outcome("log.txt"))))),
        Some(Box::new(move || first(Ok(recording_outcome("first.gtd"))))),
        Some(Box::new(move || {
            second(Ok(recording_outcome("second.gtd")))
        })),
    ];
    for (step, index) in order.into_iter().enumerate() {
        completions
            .get_mut(index)
            .expect("completion index")
            .take()
            .expect("complete once")();
        if separate_frames {
            harness.step();
            if step < 2 {
                assert!(harness.state().association_dialog.is_none());
                assert_eq!(
                    harness
                        .state()
                        .first_log()
                        .and_then(LoadedLog::associated_recording),
                    None
                );
            }
        }
    }
    harness.run_steps(2);
    if ask {
        let dialog = harness.get(
            By::new()
                .role(Role::Window)
                .label(association_dialog::TITLE),
        );
        let first_row = dialog.get(By::new().role(Role::Button).label("first.gtd"));
        let second_row = dialog.get(By::new().role(Role::Button).label("second.gtd"));
        assert!(first_row.rect().top() < second_row.rect().top());
    }
    assert_initial_association(&mut harness, ask, 2);
}

#[rstest]
fn initial_chooser_ties_follow_loaded_order_then_arrival_order(
    #[values(true, false)] second_completes_first: bool,
) {
    let mut harness = harness(true);
    for name in ["existing-first.gtd", "existing-second.gtd"] {
        let complete = harness
            .state_mut()
            .loader
            .controlled_recording_load_for_test(name, RecordingOperationOrigin::Independent);
        complete(Ok(recording_outcome(name)));
        harness.step();
    }
    let initial_candidates: Vec<_> = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .view()
        .entries()
        .map(|entry| entry.id())
        .collect();
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(initial_candidates.into_iter());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    let first = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "arriving-first.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let second = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "arriving-second.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    if second_completes_first {
        second(Ok(recording_outcome("arriving-second.gtd")));
        harness.step();
        first(Ok(recording_outcome("arriving-first.gtd")));
    } else {
        first(Ok(recording_outcome("arriving-first.gtd")));
        harness.step();
        second(Ok(recording_outcome("arriving-second.gtd")));
    }
    harness.run_steps(3);
    let dialog = harness.get(
        By::new()
            .role(Role::Window)
            .label(association_dialog::TITLE),
    );
    let row_tops = [
        "existing-first.gtd",
        "existing-second.gtd",
        "arriving-first.gtd",
        "arriving-second.gtd",
    ]
    .map(|name| {
        dialog
            .get(By::new().role(Role::Button).label(name))
            .rect()
            .top()
    });
    assert!(row_tops.windows(2).all(|pair| pair.first() < pair.get(1)));
}

#[rstest]
fn a_failed_recording_load_releases_pending_log_association(#[values(true, false)] ask: bool) {
    let mut harness = harness(ask);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let good = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "good.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let failed = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "failed.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    good(Ok(recording_outcome("good.gtd")));
    log(Ok(log_outcome("log.txt")));
    harness.step();
    assert!(harness.state().association_dialog.is_none());
    failed(Err("Truncated recording".to_owned()));
    harness.run_steps(2);
    assert_initial_association(&mut harness, ask, 1);
}

#[rstest]
fn initial_association_waits_for_history_screening_and_the_recording_load(
    #[values(true, false)] ask: bool,
) {
    let mut harness = harness(ask);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let screening = batch.recording();
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    harness.step();
    assert!(harness.state().association_dialog.is_none());
    assert_eq!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .len(),
        1
    );
    harness
        .state_mut()
        .handle_history_response(history_db::Response::RecordingsFromDiskScreened(
            ScreenedRecordings {
                already_in_history: Vec::new(),
                new_to_history: vec![RecordingFromDisk {
                    arrival: screening,
                    filename: "recording.gtd".to_owned(),
                    content: RecordingContent::Bytes(ui_tests::minimal_gtd_bytes().into()),
                    mode: loader::GtdLoadMode::Regular,
                }],
            },
        ));
    assert!(harness.step_until(|h| h.state().loader.loading_jobs.is_empty()));
    assert_initial_association(&mut harness, ask, 1);
}

#[rstest]
fn initial_association_waits_for_history_open_results(#[values(true, false)] succeeds: bool) {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("history.h5");
    let stored = test_util::recordings::store_recording(
        &path,
        &ui_tests::minimal_gtd_bytes(),
        &ui_tests::two_live_track_ranges(),
    );
    let mut harness = harness(false);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    harness
        .state_mut()
        .install_history_worker(test_util::recordings::worker_on(&path));
    let requested = if succeeds {
        stored
    } else {
        DatabaseRef {
            identity: "missing".to_owned(),
            group_name: "missing".to_owned(),
        }
    };
    harness.state().history.open_with_origin(
        requested,
        RecordingOperationOrigin::Arrival(batch.recording()),
    );
    let arrival = batch.log();
    drop(batch);
    harness.state_mut().integrate_parsed_log(
        Some("log.txt".to_owned()),
        parsed_log("log"),
        ParsedLogOrigin::Loose(&arrival),
    );
    harness.state_mut().resolve_initial_log_associations();
    assert_eq!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .len(),
        1
    );
    assert!(
        harness.step_until(|h| !h.state().association_batches.has_recording_work()
            && h.state().loader.loading_jobs.is_empty())
    );
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording)
            .is_some(),
        succeeds
    );
}

#[test]
fn pending_logs_receive_separate_dialogs_in_filename_order() {
    let mut harness = harness(true);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "recording.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let z = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("z.txt", batch.log());
    let a = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("a.txt", batch.log());
    drop(batch);
    z(Ok(log_outcome("z.txt")));
    a(Ok(log_outcome("a.txt")));
    harness.step();
    recording(Ok(recording_outcome("recording.gtd")));
    harness.run_steps(2);
    let first = harness
        .state()
        .association_dialog
        .as_ref()
        .expect("first dialog")
        .log();
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(first)
            .expect("first log")
            .name(),
        "a.txt"
    );
    harness.get_by_label("Cancel").click();
    harness.run_steps(3);
    let second = harness
        .state()
        .association_dialog
        .as_ref()
        .expect("second dialog")
        .log();
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(second)
            .expect("second log")
            .name(),
        "z.txt"
    );
    harness
        .get_by_label(association_dialog::CONFIRM_LABEL)
        .click();
    harness.run_steps(2);
    assert!(harness.state().association_dialog.is_none());
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(first)
            .expect("first log")
            .position_source_state(),
        PositionSourceState::None
    );
    assert!(
        harness
            .state()
            .logs
            .get_by_id(second)
            .and_then(LoadedLog::associated_recording)
            .is_some()
    );
}

#[test]
fn a_later_unrelated_recording_load_preserves_a_resolved_log_without_a_source() {
    let mut harness = harness(false);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    harness.run_steps(2);
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test("recording.gtd", RecordingOperationOrigin::Independent);
    recording(Ok(recording_outcome("recording.gtd")));
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording),
        None
    );
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
}

#[rstest]
fn an_explicit_footer_choice_preserves_the_source_when_pending_loads_finish(
    #[values(true, false)] choose_recording: bool,
) {
    let mut harness = harness(false);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let first = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test("first.gtd", RecordingOperationOrigin::Independent);
    first(Ok(recording_outcome("first.gtd")));
    harness.step();
    let second = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "second.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .first_log()
            .expect("pending log")
            .position_source_state(),
        PositionSourceState::PendingInitialSelection
    );
    harness
        .get(By::new().role(Role::ComboBox).value(gt_ui_theme::EM_DASH))
        .click();
    harness.run_steps(2);
    let label = if choose_recording {
        "first.gtd"
    } else {
        gt_ui_theme::EM_DASH
    };
    harness.bottommost_matching(By::new().label(label)).click();
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
    let selected = harness
        .state()
        .first_log()
        .and_then(LoadedLog::associated_recording);
    assert_eq!(selected.is_some(), choose_recording);
    let source = harness
        .state()
        .first_log()
        .expect("resolved log")
        .position_source_state();
    assert_eq!(
        matches!(source, PositionSourceState::Recording(_)),
        choose_recording
    );
    if !choose_recording {
        assert_eq!(source, PositionSourceState::None);
    }
    second(Ok(recording_outcome("second.gtd")));
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording),
        selected
    );
    assert!(harness.state().association_dialog.is_none());
}

#[test]
fn restored_attachments_preserve_their_recording_during_other_loads() {
    let mut harness = harness(true);
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test("unrelated.gtd", RecordingOperationOrigin::Independent);
    let log = harness
        .state_mut()
        .loader
        .controlled_saved_log_load_for_test("saved.log");
    let recording_key = DatabaseRef {
        identity: "saved".to_owned(),
        group_name: "recording".to_owned(),
    };
    let attachment = LogAttachmentRef {
        recording: recording_key.clone(),
        id: LogAttachmentId::new_random(),
    };
    log(Ok(SavedLogResult {
        filename: Some("saved.log".to_owned()),
        parsed: parsed_log("saved"),
        restored: loader::AttachedLogRestore {
            attachment: attachment.clone(),
            filters: Default::default(),
            requested_by: loader::AttachedLogRequester::RecordingLoad,
            year_reference: ui_tests::base_time(),
        },
    }));
    harness.step();
    recording(Ok(recording_outcome("unrelated.gtd")));
    harness.run_steps(2);
    let loaded = harness.state().first_log().expect("saved log");
    assert_eq!(loaded.attachment(), Some(&attachment));
    assert_eq!(
        loaded.anchor_key().as_ref(),
        Some(&RecordingKey::Stored(recording_key))
    );
    assert_eq!(loaded.associated_recording(), None);
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
    assert!(harness.state().association_dialog.is_none());
}

#[test]
fn cancelling_the_history_recording_prompt_releases_pending_logs() {
    let mut harness = harness(true);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let screening = batch.recording();
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    harness.step();
    harness
        .state_mut()
        .handle_history_response(history_db::Response::RecordingsFromDiskScreened(
            ScreenedRecordings {
                already_in_history: vec![RecordingAlreadyInHistory {
                    from_disk: RecordingFromDisk {
                        arrival: screening,
                        filename: "recording.gtd".to_owned(),
                        content: RecordingContent::Bytes(ui_tests::minimal_gtd_bytes().into()),
                        mode: loader::GtdLoadMode::Regular,
                    },
                    db_ref: DatabaseRef {
                        identity: "stored".to_owned(),
                        group_name: "recording".to_owned(),
                    },
                    stored_tracks: Vec::new(),
                }],
                new_to_history: Vec::new(),
            },
        ));
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .len(),
        1
    );
    assert!(harness.state().association_dialog.is_none());
    harness.get_by_label("Cancel").click();
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording),
        None
    );
}

#[test]
fn successive_history_screening_responses_preserve_every_recording_before_association() {
    let mut harness = harness(false);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let screenings = [batch.recording(), batch.recording()];
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    harness.step();
    for (name, screening) in ["first.gtd", "second.gtd"].into_iter().zip(screenings) {
        harness.state_mut().handle_history_response(
            history_db::Response::RecordingsFromDiskScreened(ScreenedRecordings {
                already_in_history: vec![RecordingAlreadyInHistory {
                    from_disk: RecordingFromDisk {
                        arrival: screening,
                        filename: name.to_owned(),
                        content: RecordingContent::Bytes(ui_tests::minimal_gtd_bytes().into()),
                        mode: loader::GtdLoadMode::Regular,
                    },
                    db_ref: DatabaseRef {
                        identity: "stored".to_owned(),
                        group_name: name.to_owned(),
                    },
                    stored_tracks: ui_tests::two_live_track_ranges().to_vec(),
                }],
                new_to_history: Vec::new(),
            }),
        );
        harness.run_steps(2);
        assert_eq!(
            harness
                .state()
                .logs
                .pending_initial_position_sources()
                .len(),
            1
        );
        assert!(harness.state().association_dialog.is_none());
    }
    let names: Vec<_> = harness
        .state()
        .pending_recordings_already_in_history
        .as_ref()
        .expect("history prompt")
        .recordings
        .iter()
        .map(|recording| recording.from_disk.filename.as_str())
        .collect();
    assert_eq!(names, ["first.gtd", "second.gtd"]);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert!(harness.step_until(|h| {
        let state = h.state();
        let loaded_count = state.shared.borrow().loaded_files.len();
        if loaded_count < 2 {
            assert_eq!(state.logs.pending_initial_position_sources().len(), 1);
            assert_eq!(
                state.first_log().and_then(LoadedLog::associated_recording),
                None
            );
            false
        } else {
            assert!(state.loader.loading_jobs.is_empty());
            state.logs.pending_initial_position_sources().is_empty()
        }
    }));
    assert_initial_association(&mut harness, false, 2);
}

#[rstest]
fn loose_and_saved_contexts_are_independent_of_completion_order(#[values(true, false)] ask: bool) {
    let attachment = LogAttachmentRef {
        recording: DatabaseRef {
            identity: "saved".to_owned(),
            group_name: "recording".to_owned(),
        },
        id: LogAttachmentId::new_random(),
    };
    let saved_filters: StoredLogFilterStack = vec![StoredLogFilter {
        text: "shared".to_owned(),
        regex: false,
        enabled: false,
        mode: StoredLogFilterMode::Refine,
    }]
    .into();
    let mut results = Vec::new();
    for loose_first in [true, false] {
        let mut harness = harness(ask);
        let recording = harness
            .state_mut()
            .loader
            .controlled_recording_load_for_test(
                "recording.gtd",
                RecordingOperationOrigin::Independent,
            );
        recording(Ok(recording_outcome("recording.gtd")));
        harness.step();
        let candidates: Vec<_> = harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .view()
            .entries()
            .map(|entry| entry.id())
            .collect();
        let batch = harness
            .state_mut()
            .association_batches
            .begin_submission(candidates.into_iter());
        let loose = harness
            .state_mut()
            .loader
            .controlled_loose_log_load_for_test("loose.log", batch.log());
        drop(batch);
        let saved = harness
            .state_mut()
            .loader
            .controlled_saved_log_load_for_test("saved.log");
        let loose_outcome = LooseLogResult {
            filename: Some("loose.log".to_owned()),
            parsed: parsed_log("shared"),
        };
        let saved_outcome = SavedLogResult {
            filename: Some("saved.log".to_owned()),
            parsed: parsed_log("shared"),
            restored: loader::AttachedLogRestore {
                attachment: attachment.clone(),
                filters: saved_filters.clone(),
                requested_by: loader::AttachedLogRequester::RecordingLoad,
                year_reference: ui_tests::base_time(),
            },
        };
        if loose_first {
            loose(Ok(loose_outcome));
            harness.step();
            saved(Ok(saved_outcome));
        } else {
            saved(Ok(saved_outcome));
            harness.step();
            loose(Ok(loose_outcome));
        }
        harness.run_steps(3);
        assert_eq!(harness.state().association_dialog.is_some(), ask);
        if ask {
            harness
                .get_by_label(association_dialog::CONFIRM_LABEL)
                .click();
            harness.run_steps(2);
        }
        let mut contexts: Vec<_> = harness
            .state()
            .logs
            .iter()
            .map(|log| {
                (
                    log.name().to_owned(),
                    log.content_hash(),
                    log.context_origin(),
                    log.attachment().cloned(),
                    log.anchor_key(),
                    log.filters().to_stored_stack(),
                    log.association_window(),
                    log.associated_entry_count(),
                    log.entry_placement(0),
                    log.is_visible(),
                )
            })
            .collect();
        contexts.sort_by(|left, right| left.0.cmp(&right.0));
        results.push(contexts);
    }
    assert_eq!(results.first(), results.last());
    assert_eq!(results.first().expect("context set").len(), 2);
}

#[rstest]
fn a_slow_log_load_does_not_delay_another_logs_initial_selection(#[values(true, false)] ask: bool) {
    let mut harness = harness(ask);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "recording.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    let fast = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("fast.log", batch.log());
    let slow = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("slow.log", batch.log());
    drop(batch);
    fast(Ok(log_outcome("fast.log")));
    recording(Ok(recording_outcome("recording.gtd")));
    harness.run_steps(2);
    assert_eq!(harness.state().loader.loading_jobs.len(), 1);
    assert_initial_association(&mut harness, ask, 1);
    slow(Ok(log_outcome("slow.log")));
    harness.run_steps(2);
    if ask {
        assert_eq!(
            harness.state().association_dialog.as_ref().map(|dialog| {
                harness
                    .state()
                    .logs
                    .get_by_id(dialog.log())
                    .expect("second log")
                    .name()
            }),
            Some("slow.log")
        );
        harness
            .get_by_label(association_dialog::CONFIRM_LABEL)
            .click();
        harness.run_steps(2);
    }
    assert_eq!(harness.state().logs.len(), 2);
    assert!(
        harness
            .state()
            .logs
            .iter()
            .all(|log| log.associated_recording().is_some())
    );
}

#[rstest]
fn later_batches_preserve_initial_candidate_membership(
    #[values(true, false)] ask: bool,
    #[values(true, false)] later_completes_first: bool,
) {
    let mut harness = harness(ask);
    let first_batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let arrival = first_batch.recording();
    let first = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "first.gtd",
            RecordingOperationOrigin::Arrival(arrival),
        );
    let arrival = first_batch.log();
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", arrival);
    drop(first_batch);
    log(Ok(log_outcome("log.txt")));
    harness.step();
    let later_batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let arrival = later_batch.recording();
    let later = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "later.gtd",
            RecordingOperationOrigin::Arrival(arrival),
        );
    drop(later_batch);
    if later_completes_first {
        later(Ok(recording_outcome("later.gtd")));
        harness.step();
        assert_eq!(
            harness
                .state()
                .first_log()
                .expect("pending log")
                .position_source_state(),
            PositionSourceState::PendingInitialSelection
        );
        first(Ok(recording_outcome("first.gtd")));
    } else {
        first(Ok(recording_outcome("first.gtd")));
        harness.run_steps(2);
        assert_initial_association(&mut harness, ask, 1);
        later(Ok(recording_outcome("later.gtd")));
    }
    harness.run_steps(2);
    if later_completes_first {
        if ask {
            assert!(
                harness
                    .get_by_role_and_label(Role::Window, association_dialog::TITLE)
                    .query_by_label("later.gtd")
                    .is_none()
            );
        }
        assert_initial_association(&mut harness, ask, 1);
    }
    let source = harness
        .state()
        .first_log()
        .and_then(LoadedLog::associated_recording)
        .expect("source");
    let shared = harness.state().shared.borrow();
    assert_eq!(
        shared
            .loaded_files
            .view()
            .entry_for_id(source)
            .expect("source recording")
            .file()
            .metadata
            .filename,
        "first.gtd"
    );
}

#[rstest]
#[case::cancel("Cancel", 0, true)]
#[case::recalculate("Recalculate with current settings", 1, true)]
#[case::stored_tracks("Use stored tracks", 1, true)]
#[case::failed_recalculate("Recalculate with current settings", 0, false)]
fn resegment_decisions_complete_every_recording_operation(
    #[case] first_choice: &str,
    #[case] loaded: usize,
    #[case] readable: bool,
) {
    let mut harness = harness(false);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let operations = [batch.recording(), batch.recording()];
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    harness.step();
    let mut segmentation =
        loader::stored_segmentation_from_config(&harness.state().processing_config);
    segmentation.track_split_gap_us += 1;
    for (name, arrival) in ["first.gtd", "second.gtd"].into_iter().zip(operations) {
        harness
            .state_mut()
            .handle_history_response(history_db::Response::Opened {
                origin: RecordingOperationOrigin::Arrival(arrival),
                db_ref: DatabaseRef {
                    identity: name.to_owned(),
                    group_name: name.to_owned(),
                },
                placement: LoadedRecordingPlacement::AddAnEntry,
                result: Ok(OpenedRecording {
                    stored: StoredRecording {
                        bytes: if readable {
                            ui_tests::minimal_gtd_bytes()
                        } else {
                            Vec::new()
                        },
                        tracks: ui_tests::two_live_track_ranges().to_vec(),
                        segmentation: Some(segmentation),
                        debug_tag: None,
                    },
                    ui_state: Ok(RecordingUiState::default()),
                }),
            });
    }
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .pending_resegment
            .as_ref()
            .expect("first prompt")
            .filename,
        "first.gtd"
    );
    assert_eq!(harness.state().queued_resegments.len(), 1);
    harness.get_by_label(first_choice).click();
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .pending_resegment
            .as_ref()
            .expect("second prompt")
            .filename,
        "second.gtd"
    );
    assert_eq!(
        harness
            .state()
            .first_log()
            .expect("pending log")
            .position_source_state(),
        PositionSourceState::PendingInitialSelection
    );
    harness.get_by_label("Cancel").click();
    assert!(harness.step_until(|h| h.state().logs.pending_initial_position_sources().is_empty()));
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), loaded);
    assert_initial_association(&mut harness, false, loaded);
}

#[rstest]
#[case::cancel("Cancel", 0)]
#[case::disk(LOAD_FROM_DISK_LABEL, 1)]
#[case::history("Open the stored version", 1)]
fn arrival_batches_complete_history_screening_and_prompt_decisions(
    #[case] choice: &str,
    #[case] loaded: usize,
) {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("history.h5");
    let bytes = ui_tests::minimal_gtd_bytes();
    test_util::recordings::store_recording(&path, &bytes, &ui_tests::two_live_track_ranges());
    let mut harness = harness(false);
    harness
        .state_mut()
        .install_history_worker(test_util::recordings::worker_on(&path));
    harness.state_mut().load_arriving_files(vec![
        QueuedLoad::Bytes {
            bytes: bytes.into(),
            name: "recording.gtd".to_owned(),
        },
        QueuedLoad::PastedText(parsed_log("log").text().to_string()),
    ]);
    assert!(harness.step_until(
        |h| h.state().pending_recordings_already_in_history.is_some() && h.state().logs.len() == 1
    ));
    assert_eq!(
        harness
            .state()
            .first_log()
            .expect("pending log")
            .position_source_state(),
        PositionSourceState::PendingInitialSelection
    );
    harness.run_steps(3);
    harness.get_by_label(choice).click();
    harness.run_steps(3);
    assert!(harness.step_until(|h| h.state().logs.pending_initial_position_sources().is_empty()));
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), loaded);
    assert_initial_association(&mut harness, false, loaded);
}

#[test]
fn deferred_arrival_sets_keep_independent_candidates_after_storage_opens() {
    let dir = tempfile::tempdir().expect("temp directory");
    let store = Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.state_mut().initial_position_source_policy =
        InitialPositionSourcePolicy::AutomaticallyUseUnambiguous;
    harness
        .state_mut()
        .load_arriving_files(vec![QueuedLoad::PastedText(
            parsed_log("log").text().to_string(),
        )]);
    harness
        .state_mut()
        .load_arriving_files(vec![QueuedLoad::Bytes {
            bytes: ui_tests::minimal_gtd_bytes().into(),
            name: "recording.gtd".to_owned(),
        }]);
    assert!(harness.state().loader.loading_jobs.is_empty());
    ui_tests::land_the_databases(&mut harness, &databases, &store);
    assert!(harness.step_until(|h| h.state().logs.len() == 1
        && h.state().shared.borrow().loaded_files.len() == 1
        && h.state().logs.pending_initial_position_sources().is_empty()));
    assert_eq!(
        harness
            .state()
            .first_log()
            .expect("resolved log")
            .position_source_state(),
        PositionSourceState::None
    );
}

#[rstest]
fn independent_history_and_later_arrivals_preserve_initial_selection_scope(
    #[values(true, false)] ask: bool,
    #[values([0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0])] order: [usize; 3],
) {
    let mut harness = harness(ask);
    let arrival = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", arrival.log());
    let first = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "arrival-a.gtd",
            RecordingOperationOrigin::Arrival(arrival.recording()),
        );
    drop(arrival);
    let later = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let second = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "arrival-b.gtd",
            RecordingOperationOrigin::Arrival(later.recording()),
        );
    drop(later);
    let independent = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test("history-c.gtd", RecordingOperationOrigin::Independent);
    log(Ok(log_outcome("log.txt")));
    harness.run_steps(2);
    let mut completions: Vec<Option<Box<dyn FnOnce()>>> = vec![
        Some(Box::new(move || {
            first(Ok(recording_outcome("arrival-a.gtd")))
        })),
        Some(Box::new(move || {
            second(Ok(recording_outcome("arrival-b.gtd")))
        })),
        Some(Box::new(move || {
            independent(Ok(recording_outcome("history-c.gtd")))
        })),
    ];
    let mut first_completed = false;
    for index in order {
        completions
            .get_mut(index)
            .expect("completion index")
            .take()
            .expect("complete once")();
        harness.run_steps(2);
        first_completed |= index == 0;
        if !first_completed {
            assert_eq!(
                harness
                    .state()
                    .first_log()
                    .expect("pending log")
                    .position_source_state(),
                PositionSourceState::PendingInitialSelection
            );
            assert!(harness.state().association_dialog.is_none());
        } else if index == 0 {
            if ask {
                let chooser =
                    harness.get_by_role_and_label(Role::Window, association_dialog::TITLE);
                assert!(chooser.query_by_label("arrival-a.gtd").is_some());
                assert!(chooser.query_by_label("arrival-b.gtd").is_none());
                assert!(chooser.query_by_label("history-c.gtd").is_none());
            }
            assert_initial_association(&mut harness, ask, 1);
        }
    }
    let source = harness
        .state()
        .first_log()
        .and_then(LoadedLog::associated_recording)
        .expect("position source");
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .view()
            .entry_for_id(source)
            .expect("source recording")
            .file()
            .metadata
            .filename,
        "arrival-a.gtd"
    );
}

#[rstest]
fn a_saved_attachment_load_preserves_an_explicit_loose_arrivals_readiness(
    #[values(true, false)] ask: bool,
    #[values(true, false)] saved_first: bool,
) {
    let mut harness = harness(ask);
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("loose.log", batch.log());
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "recording.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    drop(batch);
    let saved = harness
        .state_mut()
        .loader
        .controlled_saved_log_load_for_test("saved.log");
    let saved_result = SavedLogResult {
        filename: Some("saved.log".to_owned()),
        parsed: parsed_log("saved"),
        restored: loader::AttachedLogRestore {
            attachment: LogAttachmentRef {
                recording: DatabaseRef {
                    identity: "stored".to_owned(),
                    group_name: "recording".to_owned(),
                },
                id: LogAttachmentId::new_random(),
            },
            filters: Default::default(),
            requested_by: loader::AttachedLogRequester::RecordingLoad,
            year_reference: ui_tests::base_time(),
        },
    };
    log(Ok(log_outcome("loose.log")));
    harness.step();
    if saved_first {
        saved(Ok(saved_result));
        harness.run_steps(2);
        assert_eq!(
            harness
                .state()
                .logs
                .pending_initial_position_sources()
                .len(),
            1
        );
        recording(Ok(recording_outcome("recording.gtd")));
        assert_initial_association(&mut harness, ask, 1);
    } else {
        recording(Ok(recording_outcome("recording.gtd")));
        assert_initial_association(&mut harness, ask, 1);
        assert_eq!(harness.state().loader.loading_jobs.len(), 1);
        saved(Ok(saved_result));
    }
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .logs
            .pending_initial_position_sources()
            .is_empty()
    );
    assert_eq!(harness.state().logs.len(), 2);
}

#[test]
fn a_duplicate_loose_arrival_completes_without_registering_the_existing_context() {
    let mut harness = harness(false);
    harness.state_mut().load_parsed_log(
        Some("original.log".to_owned()),
        parsed_log("duplicate"),
        None,
    );
    harness.run_steps(2);
    let id = harness
        .state()
        .logs
        .iter_with_ids()
        .next()
        .expect("existing log")
        .0;
    let source = harness
        .state()
        .logs
        .get_by_id(id)
        .expect("existing log")
        .position_source_state();
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("duplicate.log", batch.log());
    let recording = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "failed.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    drop(batch);
    log(Ok(LooseLogResult {
        filename: Some("duplicate.log".to_owned()),
        parsed: parsed_log("duplicate"),
    }));
    harness.run_steps(2);
    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(id)
            .expect("existing log")
            .position_source_state(),
        source
    );
    recording(Err("Failed recording".to_owned()));
    harness.run_steps(2);
    assert!(harness.state().association_batches.is_empty_for_test());
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(id)
            .expect("existing log")
            .position_source_state(),
        source
    );
}

#[rstest]
fn independent_history_open_apis_preserve_an_arrivals_candidate_scope(
    #[values(true, false)] replace_loaded_entry: bool,
) {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("history.h5");
    let stored = test_util::recordings::store_recording(
        &path,
        &ui_tests::minimal_gtd_bytes(),
        &ui_tests::two_live_track_ranges(),
    );
    let mut harness = harness(false);
    harness
        .state_mut()
        .install_history_worker(test_util::recordings::worker_on(&path));
    let batch = harness
        .state_mut()
        .association_batches
        .begin_submission(std::iter::empty());
    let log = harness
        .state_mut()
        .loader
        .controlled_loose_log_load_for_test("log.txt", batch.log());
    let arrival = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test(
            "arrival.gtd",
            RecordingOperationOrigin::Arrival(batch.recording()),
        );
    drop(batch);
    log(Ok(log_outcome("log.txt")));
    if replace_loaded_entry {
        harness.state().history.open_over_the_loaded_entry(stored);
    } else {
        harness.state().history.open(stored);
    }
    assert!(harness.step_until(|h| h.state().shared.borrow().loaded_files.len() == 1));
    assert_eq!(
        harness
            .state()
            .first_log()
            .expect("pending log")
            .position_source_state(),
        PositionSourceState::PendingInitialSelection
    );
    arrival(Ok(recording_outcome("arrival.gtd")));
    assert_initial_association(&mut harness, false, 1);
    let source = harness
        .state()
        .first_log()
        .and_then(LoadedLog::associated_recording)
        .expect("source");
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .view()
            .entry_for_id(source)
            .expect("source recording")
            .file()
            .metadata
            .filename,
        "arrival.gtd"
    );
}
