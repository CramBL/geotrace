use chrono::Duration;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_loaded_files::LoadedFiles;
use gt_log_view::{AttachmentSettingsStatus, LoadedLog, LoadedLogs, LogAttachmentRef};
use gt_pending_writes::WriteAccess;
use gt_store::{DatabaseRef, LogAttachmentId};
use gt_test_utils::{By, TestHarness};
use gt_ui_types::LoadedLogId;
use rstest::rstest;

use crate::app::log_viewer::LogViewerRequests;
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

use super::{ICON_CHECK, ICON_WARNING};

struct StatusHarness {
    logs: LoadedLogs,
    requests: LogViewerRequests,
    access: WriteAccess,
    failed_log: LoadedLogId,
}

impl StatusHarness {
    fn new() -> Self {
        let mut logs = LoadedLogs::default();
        let recordings = LoadedFiles::new();
        let mut failed_log = None;
        for name in [
            "failed.log",
            "saving.log",
            "unsaved.log",
            "saved.log",
            "session.log",
        ] {
            let parsed = gt_logfile::parse_log(
                gt_test_utils::UNRESOLVED_YEARS_LOG.into(),
                gt_test_utils::synthetic_log_start(),
            )
            .unwrap();
            let log = LoadedLog::new(Some(name.to_owned()), parsed, Duration::seconds(60));
            let settings = log.settings_snapshot();
            let id = logs.push(log).id();
            if name != "session.log" {
                logs.save_attachment(
                    id,
                    LogAttachmentRef {
                        recording: DatabaseRef {
                            identity: "receiver".to_owned(),
                            group_name: "recording".to_owned(),
                        },
                        id: LogAttachmentId::new_random(),
                    },
                    settings,
                    &recordings.view(),
                );
            }
            if matches!(name, "failed.log" | "saving.log" | "unsaved.log") {
                let stack = logs.filter_stack_mut_by_id(id).unwrap().0;
                stack.set_live_filter_text("clock");
                stack.add_live_filter_as_chip().unwrap();
            }
            if matches!(name, "failed.log" | "saving.log") {
                let write = logs
                    .prepare_settings_writes(WriteAccess::Owner)
                    .pop()
                    .unwrap();
                if name == "failed.log" {
                    logs.acknowledge_settings(
                        write.complete(Err("Disk full: history.h5".to_owned())),
                    );
                    failed_log = Some(id);
                }
            }
        }
        Self {
            logs,
            requests: LogViewerRequests::default(),
            access: WriteAccess::Owner,
            failed_log: failed_log.unwrap(),
        }
    }
}

#[rstest]
#[case::dark(true)]
#[case::light(false)]
fn snapshot_log_settings_save_inspector(#[case] dark_mode: bool) {
    let mut harness = TestHarness::builder()
        .size(egui::vec2(640.0, 440.0))
        .theme(dark_mode)
        .ui_state(
            |ui, state: &mut StatusHarness| {
                super::toolbar_ui(ui, &state.logs, state.access, &mut state.requests);
            },
            StatusHarness::new(),
        );
    harness
        .inner
        .get_by_label(&format!("Log saves {ICON_WARNING} 1 … 1"))
        .click();
    harness.inner.run_steps(2);
    for label in [
        "Settings unsaved",
        "Saving settings…",
        "Session-only settings",
    ] {
        assert!(harness.inner.query_by_label(label).is_some(), "{label}");
    }
    assert!(
        harness
            .inner
            .query_by_label(&format!("{ICON_CHECK} Settings saved"))
            .is_some()
    );
    harness.inner.get_by_label("Error details").click();
    harness.inner.run_steps(2);
    assert!(
        harness
            .inner
            .query_by_label("Disk full: history.h5")
            .is_some()
    );
    harness.snapshot(if dark_mode {
        "log_settings_save_inspector_dark"
    } else {
        "log_settings_save_inspector_light"
    });
    harness.inner.get_by_label("Retry").click();
    harness.inner.step();
    assert_eq!(
        harness.state().requests.retry_settings_save,
        Some(harness.state().failed_log)
    );
}

#[test]
fn a_read_only_inspector_retains_errors_and_disables_retry() {
    let mut state = StatusHarness::new();
    state.access = WriteAccess::ReadOnly;
    let mut harness = Harness::builder().build_ui_state(
        |ui, state: &mut StatusHarness| {
            super::toolbar_ui(ui, &state.logs, state.access, &mut state.requests);
        },
        state,
    );
    harness
        .get_by_label(&format!("Log saves {ICON_WARNING} 1"))
        .click();
    harness.run_steps(2);
    let retry = harness.get_by_label("Retry");
    assert!(retry.accesskit_node().is_disabled());
    assert_eq!(
        retry.accesskit_node().description().as_deref(),
        Some(READ_ONLY_RECORDING_HISTORY_HOVER)
    );
    harness.get_by_label("Error details").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Disk full: history.h5").is_some());
    assert!(harness.state().requests.retry_settings_save.is_none());
}

#[test]
fn retry_distinguishes_equal_log_names_in_different_recording_groups() {
    let mut state = StatusHarness::new();
    let remaining: Vec<_> = state
        .logs
        .iter_with_ids()
        .filter(|(id, _)| *id != state.failed_log)
        .map(|(id, _)| id)
        .collect();
    for id in remaining {
        state.logs.remove_by_id(id);
    }
    let parsed = gt_logfile::parse_log(
        gt_test_utils::UNRESOLVED_YEARS_LOG.into(),
        gt_test_utils::synthetic_log_start(),
    )
    .unwrap();
    let log = LoadedLog::new(Some("failed.log".to_owned()), parsed, Duration::seconds(60));
    let settings = log.settings_snapshot();
    let recordings = LoadedFiles::new();
    let other = state
        .logs
        .restore_attachment(
            log,
            LogAttachmentRef {
                recording: DatabaseRef {
                    identity: "receiver".to_owned(),
                    group_name: "recording-two".to_owned(),
                },
                id: LogAttachmentId::new_random(),
            },
            settings,
            &recordings.view(),
        )
        .id();
    let stack = state.logs.filter_stack_mut_by_id(other).unwrap().0;
    stack.set_live_filter_text("clock");
    stack.add_live_filter_as_chip().unwrap();
    let write = state
        .logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    state
        .logs
        .acknowledge_settings(write.complete(Err("Permission denied".to_owned())));
    let mut harness = Harness::builder().build_ui_state(
        |ui, state: &mut StatusHarness| {
            super::toolbar_ui(ui, &state.logs, state.access, &mut state.requests);
        },
        state,
    );
    harness
        .get_by_label(&format!("Log saves {ICON_WARNING} 2"))
        .click();
    harness.run_steps(2);
    for context in ["receiver/recording", "receiver/recording-two"] {
        assert!(harness.query_by_label(context).is_some());
    }
    let retry = By::new().predicate(|node| {
        node.description().as_deref()
            == Some("Save the latest settings for failed.log (receiver/recording)")
    });
    harness.get(retry).click();
    harness.step();
    let state = harness.state_mut();
    assert_eq!(state.requests.retry_settings_save, Some(state.failed_log));
    assert!(state.logs.retry_settings_save(state.failed_log));
    let mut writes = state.logs.prepare_settings_writes(WriteAccess::Owner);
    assert_eq!(writes.len(), 1);
    let write = writes.pop().unwrap();
    state.logs.acknowledge_settings(write.complete(Ok(())));
    assert_eq!(
        state
            .logs
            .get_by_id(state.failed_log)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
    assert_eq!(
        state
            .logs
            .get_by_id(other)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Failed("Permission denied")
    );
}
