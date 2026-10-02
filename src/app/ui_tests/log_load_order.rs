use chrono::Duration;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable as _};
use gt_loaded_files::FileHistory;
use gt_log_view::{LoadedLog, LogAttachmentRef, RecordingKey};
use gt_logfile::ParsedLog;
use gt_plot::AnalysisConfig;
use gt_store::{DatabaseRef, LogAttachmentId, StoredLogFilter, StoredLogFilterMode};
use gt_test_utils::{By, HarnessInteraction as _};
use rstest::rstest;

use crate::app::App;
use crate::app::history_db;
use crate::app::loader::{LoadOutcome, LoadedRecordingPlacement};
use crate::app::log_viewer::association_dialog;
use crate::app::recording_from_disk::{
    LOAD_FROM_DISK_LABEL, RecordingAlreadyInHistory, RecordingContent, RecordingFromDisk,
    ScreenedRecordings,
};
use crate::app::{loader, test_util, ui_tests};

fn harness(ask: bool) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().ask_log_association_target = ask;
    harness
}

fn parsed_log(message: &str) -> ParsedLog {
    let time = ui_tests::base_time() + Duration::seconds(5);
    let text = format!("{} {message}\n", time.format(gt_fmt::UTC_SECOND_FORMAT));
    gt_logfile::parse_log(text.into(), time).expect("parse log timestamp")
}

fn log_outcome(name: &str) -> LoadOutcome {
    LoadOutcome::Log {
        filename: Some(name.to_owned()),
        parsed: parsed_log(name),
        restored: None,
    }
}

fn recording_outcome(name: &str) -> LoadOutcome {
    let file = gt_loader::load_bytes(&ui_tests::minimal_gtd_bytes(), name.to_owned())
        .expect("parse recording");
    let series = gt_plot::prepare_file_series(&file, AnalysisConfig::default());
    LoadOutcome::GtdFile {
        file,
        series,
        history: FileHistory::None,
        applied_current_marker_settings: false,
        placement: LoadedRecordingPlacement::AddAnEntry,
    }
}

fn assert_initial_association(harness: &mut Harness<App>, ask: bool, overlapping: usize) {
    harness.run_steps(3);
    assert!(harness.state().pending_initial_log_associations.is_empty());
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
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
    let recording = harness
        .state_mut()
        .loader
        .controlled_load_for_test("recording.gtd");
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
    let mut completions: Vec<_> = ["log.txt", "first.gtd", "second.gtd"]
        .into_iter()
        .map(|name| Some(harness.state_mut().loader.controlled_load_for_test(name)))
        .collect();
    for (step, index) in order.into_iter().enumerate() {
        let outcome = match index {
            0 => log_outcome("log.txt"),
            1 => recording_outcome("first.gtd"),
            _ => recording_outcome("second.gtd"),
        };
        completions
            .get_mut(index)
            .expect("completion index")
            .take()
            .expect("complete once")(Ok(outcome));
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
    assert_initial_association(&mut harness, ask, 2);
}

#[rstest]
fn a_failed_recording_load_releases_pending_log_association(#[values(true, false)] ask: bool) {
    let mut harness = harness(ask);
    let good = harness
        .state_mut()
        .loader
        .controlled_load_for_test("good.gtd");
    let failed = harness
        .state_mut()
        .loader
        .controlled_load_for_test("failed.gtd");
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
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
    harness.state_mut().recordings_awaiting_a_history_lookup = 1;
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
    log(Ok(log_outcome("log.txt")));
    harness.step();
    assert!(harness.state().association_dialog.is_none());
    assert_eq!(harness.state().pending_initial_log_associations.len(), 1);
    harness
        .state_mut()
        .handle_history_response(history_db::Response::RecordingsFromDiskScreened(
            ScreenedRecordings {
                already_in_history: Vec::new(),
                new_to_history: vec![RecordingFromDisk {
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
    harness.state_mut().history = test_util::recordings::worker_on(&path);
    let requested = if succeeds {
        stored
    } else {
        DatabaseRef {
            identity: "missing".to_owned(),
            group_name: "missing".to_owned(),
        }
    };
    harness.state().history.open(requested);
    harness
        .state_mut()
        .load_parsed_log(Some("log.txt".to_owned()), parsed_log("log"), None);
    harness.state_mut().resolve_initial_log_associations();
    assert_eq!(harness.state().pending_initial_log_associations.len(), 1);
    assert!(
        harness.step_until(|h| !h.state().history.has_pending_recording_opens()
            && h.state().loader.loading_jobs.is_empty())
    );
    assert!(harness.state().pending_initial_log_associations.is_empty());
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
    let recording = harness
        .state_mut()
        .loader
        .controlled_load_for_test("recording.gtd");
    let z = harness.state_mut().loader.controlled_load_for_test("z.txt");
    let a = harness.state_mut().loader.controlled_load_for_test("a.txt");
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
            .and_then(LoadedLog::associated_recording),
        None
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
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
    log(Ok(log_outcome("log.txt")));
    harness.run_steps(2);
    let recording = harness
        .state_mut()
        .loader
        .controlled_load_for_test("recording.gtd");
    recording(Ok(recording_outcome("recording.gtd")));
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording),
        None
    );
    assert!(harness.state().pending_initial_log_associations.is_empty());
}

#[rstest]
fn an_explicit_footer_choice_preserves_the_source_when_pending_loads_finish(
    #[values(true, false)] choose_recording: bool,
) {
    let mut harness = harness(false);
    let first = harness
        .state_mut()
        .loader
        .controlled_load_for_test("first.gtd");
    first(Ok(recording_outcome("first.gtd")));
    harness.step();
    let second = harness
        .state_mut()
        .loader
        .controlled_load_for_test("second.gtd");
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
    log(Ok(log_outcome("log.txt")));
    harness.run_steps(2);
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
    assert!(harness.state().pending_initial_log_associations.is_empty());
    let selected = harness
        .state()
        .first_log()
        .and_then(LoadedLog::associated_recording);
    assert_eq!(selected.is_some(), choose_recording);
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
        .controlled_load_for_test("unrelated.gtd");
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("saved.log");
    let recording_key = DatabaseRef {
        identity: "saved".to_owned(),
        group_name: "recording".to_owned(),
    };
    let attachment = LogAttachmentRef {
        recording: recording_key.clone(),
        id: LogAttachmentId::new_random(),
    };
    log(Ok(LoadOutcome::Log {
        filename: Some("saved.log".to_owned()),
        parsed: parsed_log("saved"),
        restored: Some(loader::AttachedLogRestore {
            attachment: attachment.clone(),
            filters: Vec::new(),
            requested_by: loader::AttachedLogRequester::RecordingLoad,
            year_reference: ui_tests::base_time(),
        }),
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
    assert!(harness.state().pending_initial_log_associations.is_empty());
    assert!(harness.state().association_dialog.is_none());
}

#[test]
fn cancelling_the_history_recording_prompt_releases_pending_logs() {
    let mut harness = harness(true);
    harness.state_mut().recordings_awaiting_a_history_lookup = 1;
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
    log(Ok(log_outcome("log.txt")));
    harness.step();
    harness
        .state_mut()
        .handle_history_response(history_db::Response::RecordingsFromDiskScreened(
            ScreenedRecordings {
                already_in_history: vec![RecordingAlreadyInHistory {
                    from_disk: RecordingFromDisk {
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
    assert_eq!(harness.state().pending_initial_log_associations.len(), 1);
    assert!(harness.state().association_dialog.is_none());
    harness.get_by_label("Cancel").click();
    harness.run_steps(2);
    assert!(harness.state().pending_initial_log_associations.is_empty());
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
    harness.state_mut().recordings_awaiting_a_history_lookup = 2;
    let log = harness
        .state_mut()
        .loader
        .controlled_load_for_test("log.txt");
    log(Ok(log_outcome("log.txt")));
    harness.step();
    for name in ["first.gtd", "second.gtd"] {
        harness.state_mut().handle_history_response(
            history_db::Response::RecordingsFromDiskScreened(ScreenedRecordings {
                already_in_history: vec![RecordingAlreadyInHistory {
                    from_disk: RecordingFromDisk {
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
        assert_eq!(harness.state().pending_initial_log_associations.len(), 1);
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
            assert_eq!(state.pending_initial_log_associations.len(), 1);
            assert_eq!(
                state.first_log().and_then(LoadedLog::associated_recording),
                None
            );
            false
        } else {
            assert!(state.loader.loading_jobs.is_empty());
            state.pending_initial_log_associations.is_empty()
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
    let saved_filters = vec![StoredLogFilter {
        text: "shared".to_owned(),
        regex: false,
        enabled: false,
        mode: StoredLogFilterMode::Refine,
    }];
    let mut results = Vec::new();
    for loose_first in [true, false] {
        let mut harness = harness(ask);
        let recording = harness
            .state_mut()
            .loader
            .controlled_load_for_test("recording.gtd");
        recording(Ok(recording_outcome("recording.gtd")));
        harness.step();
        let loose = harness
            .state_mut()
            .loader
            .controlled_load_for_test("loose.log");
        let saved = harness
            .state_mut()
            .loader
            .controlled_load_for_test("saved.log");
        let loose_outcome = LoadOutcome::Log {
            filename: Some("loose.log".to_owned()),
            parsed: parsed_log("shared"),
            restored: None,
        };
        let saved_outcome = LoadOutcome::Log {
            filename: Some("saved.log".to_owned()),
            parsed: parsed_log("shared"),
            restored: Some(loader::AttachedLogRestore {
                attachment: attachment.clone(),
                filters: saved_filters.clone(),
                requested_by: loader::AttachedLogRequester::RecordingLoad,
                year_reference: ui_tests::base_time(),
            }),
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
                    log.filters().to_stored_filters(),
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
