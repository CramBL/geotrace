//! The dialog driven on its own: what it lists for a log, what it reports back,
//! and when it lets the log be attached.

use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone as _, Utc};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_loaded_files::{FileHistory, LoadedFileId, LoadedFiles, RecordingNames};
use gt_log_view::LoadedLog;
use gt_pending_writes::WriteAccess;

use crate::app::history_db::ExistingLogAttachment;
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use gt_store::{DatabaseRef, LogAttachmentId, RecordingMeta};
use gt_test_utils::window_fit::{
    CRAMPED_VIEWPORT, NARROW_VIEWPORT, OVERSIZED_ROW_COUNT, SHORT_VIEWPORT,
};
use gt_test_utils::{
    AuditedWindow, By, ControlLabel, HarnessInteraction as _, WindowFitAssertions as _,
};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::{FileSource, Latitude, Longitude};
use gt_ui_types::LoadedLogId;

use super::{
    ATTACH_LABEL, CANCEL_LABEL, CONFIRM_LABEL, DONT_SHOW_AGAIN_LABEL, LogAssociationChoice,
    LogAssociationDialog, TITLE,
};

/// Three entries spanning nine seconds from [`log_start`].
const LOG: &str = "\
2026-05-29 18:48:25 navsyncd: starting
2026-05-29 18:48:27 navsyncd: fix acquired
2026-05-29 18:48:34 navsyncd: fix lost
";

/// The span the fixture log covers, which the listed overlaps are shares of.
const LOG_SPAN_SECS: i64 = 9;

const DIALOG_SIZE: egui::Vec2 = egui::vec2(560.0, 420.0);

/// The log the dialog is shown for, the only one in these tests.
const SHOWN_LOG: LoadedLogId = LoadedLogId::new(0);

fn log_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 29, 18, 48, 25)
        .single()
        .unwrap_or_default()
}

struct DialogState {
    dialog: LogAssociationDialog,
    log: LoadedLog,
    recordings: LoadedFiles,
    choice: Option<LogAssociationChoice>,
    /// What the session may write, which is what grays the attach tickbox.
    write_access: WriteAccess,
}

/// A recording of `seconds` fixes starting `offset` after the log does.
fn recording(filename: &str, offset: Duration, seconds: usize) -> gt_types::LoadedFile {
    let points = gt_test_utils::nav_points_walking_from(
        log_start() + offset,
        seconds,
        1,
        Latitude::new(55.0),
        Longitude::new(12.0),
    );
    gt_track_builder::build_loaded_file(
        filename.to_owned(),
        &points,
        &[],
        Vec::new(),
        Vec::new(),
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from(filename)),
        FileMeta::default(),
        Vec::new(),
    )
}

/// How a recording the history database holds is filed in the session.
fn stored_in_history(identity: &str) -> FileHistory {
    FileHistory::recording(
        identity.to_owned(),
        RecordingMeta {
            start_us: 0,
            end_us: 0,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        Some(DatabaseRef {
            identity: identity.to_owned(),
            group_name: "2026-05-29T18-48-25".to_owned(),
        }),
    )
}

/// The dialog open on the fixture log over `recordings`, with nothing
/// preselected.
fn harness_over(
    recordings: Vec<(gt_types::LoadedFile, FileHistory)>,
) -> Harness<'static, DialogState> {
    harness_over_sized(recordings, DIALOG_SIZE)
}

fn harness_over_sized(
    recordings: Vec<(gt_types::LoadedFile, FileHistory)>,
    viewport: egui::Vec2,
) -> Harness<'static, DialogState> {
    let mut loaded_recordings = LoadedFiles::new();
    for (file, history) in recordings {
        loaded_recordings.push(file, history);
    }
    let parsed = gt_logfile::parse_log(LOG.into(), log_start())
        .unwrap_or_else(|error| panic!("the fixture log parses: {error}"));
    let state = DialogState {
        dialog: LogAssociationDialog::new(SHOWN_LOG, None),
        log: LoadedLog::new(
            Some("navsyncd.log".to_owned()),
            parsed,
            Duration::seconds(60),
        ),
        recordings: loaded_recordings,
        choice: None,
        write_access: WriteAccess::Owner,
    };
    let mut harness = Harness::builder()
        .with_size(viewport)
        .build_ui_state(dialog_ui, state);
    harness.run_steps(3);
    harness
}

fn dialog_ui(ui: &mut egui::Ui, state: &mut DialogState) {
    let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
    let choice = state.dialog.show(
        ui.ctx(),
        &state.log,
        state.recordings.view(),
        &names,
        state.write_access,
    );
    if choice.is_some() {
        state.choice = choice;
    }
}

/// The recording loaded at `index`, as the dialog names it in a choice.
fn recording_id(harness: &Harness<DialogState>, index: usize) -> Option<LoadedFileId> {
    harness
        .state()
        .recordings
        .view()
        .get(index)
        .map(|entry| entry.id())
}

/// The recording the dialog would query the history database about, as the app
/// does after every frame the dialog draws.
fn duplicate_query_to_send(harness: &mut Harness<DialogState>) -> Option<DatabaseRef> {
    let state = harness.state_mut();
    let recordings = state.recordings.view();
    state.dialog.duplicate_query_to_send(recordings)
}

fn select(harness: &mut Harness<DialogState>, name: &str) {
    harness.get_by_label(name).click();
    harness.run_steps(2);
}

fn confirm(harness: &mut Harness<DialogState>) {
    harness.get_by_label(CONFIRM_LABEL).click();
    harness.run_steps(2);
}

/// The ranking the dialog lists in, each row stating how much of the log the
/// recording ran alongside, down to the ones that missed it entirely.
#[test]
fn every_loaded_recording_is_listed_with_the_share_of_the_log_it_covers() {
    let harness = harness_over(vec![
        (
            recording("late.gtd", Duration::seconds(5), 10),
            FileHistory::None,
        ),
        (
            recording("alongside.gtd", Duration::zero(), 10),
            FileHistory::None,
        ),
        (
            recording("elsewhen.gtd", Duration::seconds(600), 10),
            FileHistory::None,
        ),
    ]);

    harness.get_by_label("alongside.gtd");
    harness.get_by_label("overlaps 9s · 100% of log");
    harness.get_by_label(format!("overlaps 4s · {}% of log", 4 * 100 / LOG_SPAN_SECS).as_str());
    harness.get_by_label("elsewhen.gtd");
    harness.get_by_label(super::NO_OVERLAP_LABEL);
}

/// A recording that missed the log stays selectable, and says on hover what
/// choosing it would mean.
#[test]
fn a_recording_that_missed_the_log_says_why_it_is_grayed() {
    let mut harness = harness_over(vec![(
        recording("elsewhen.gtd", Duration::seconds(600), 10),
        FileHistory::None,
    )]);

    harness.get_by_label(super::NO_OVERLAP_LABEL);
    harness.hover_and_settle(By::new().label("elsewhen.gtd"), 3);

    harness.get_by_label_contains("every line would stay unassociated");
}

#[test]
fn confirming_reports_the_chosen_recording_as_the_target() {
    let mut harness = harness_over(vec![(
        recording("alongside.gtd", Duration::zero(), 10),
        FileHistory::None,
    )]);

    select(&mut harness, "alongside.gtd");
    confirm(&mut harness);

    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::Confirmed {
            target: recording_id(&harness, 0),
            attach: false,
        })
    );
}

/// Cancelling leaves the log as it loaded: untargeted, and stored nowhere.
#[test]
fn cancelling_reports_that_nothing_was_decided() {
    let mut harness = harness_over(vec![(
        recording("alongside.gtd", Duration::zero(), 10),
        FileHistory::None,
    )]);
    select(&mut harness, "alongside.gtd");

    harness.get_by_label("Cancel").click();
    harness.run_steps(2);

    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::Cancelled)
    );
}

/// Only a recording the history database holds can take an attachment: the
/// tickbox stays drawn, grayed with hover text saying so.
#[test]
fn attaching_is_offered_only_for_a_recording_the_history_database_holds() {
    let mut harness = harness_over(vec![
        (
            recording("dropped.gtd", Duration::zero(), 10),
            FileHistory::None,
        ),
        (
            recording("stored.gtd", Duration::zero(), 10),
            stored_in_history("nav-devkit-mk2"),
        ),
    ]);
    assert!(
        harness
            .get_by_label(ATTACH_LABEL)
            .accesskit_node()
            .is_disabled(),
        "nothing to attach to yet: no recording is selected"
    );

    select(&mut harness, "dropped.gtd");
    assert!(
        harness
            .get_by_label(ATTACH_LABEL)
            .accesskit_node()
            .is_disabled()
    );

    select(&mut harness, "stored.gtd");
    harness.get_by_label(ATTACH_LABEL).click();
    harness.run_steps(2);
    confirm(&mut harness);

    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::Confirmed {
            target: recording_id(&harness, 1),
            attach: true,
        })
    );
}

/// A read-only session attaches nothing either: the tickbox for a recording
/// the history database holds is grayed, saying what the session leaves
/// alone.
#[test]
fn attaching_is_not_offered_in_a_read_only_session() {
    let mut harness = harness_over(vec![(
        recording("stored.gtd", Duration::zero(), 10),
        stored_in_history("nav-devkit-mk2"),
    )]);
    harness.state_mut().write_access = WriteAccess::ReadOnly;
    select(&mut harness, "stored.gtd");

    let attach = harness.get_by_label(ATTACH_LABEL);
    assert!(attach.accesskit_node().is_disabled());
    let attach_center = attach.rect().center();

    harness.hover_at_and_settle(attach_center, 3);

    harness.get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);
}

/// Selecting a recording outside history clears the attach tickbox, so
/// confirming never reports `attach` for a recording that can hold no
/// attachment.
#[test]
fn switching_to_a_recording_outside_history_clears_the_attach_tickbox() {
    let mut harness = harness_over(vec![
        (
            recording("dropped.gtd", Duration::zero(), 10),
            FileHistory::None,
        ),
        (
            recording("stored.gtd", Duration::zero(), 10),
            stored_in_history("nav-devkit-mk2"),
        ),
    ]);
    select(&mut harness, "stored.gtd");
    harness.get_by_label(ATTACH_LABEL).click();
    harness.run_steps(2);

    select(&mut harness, "dropped.gtd");
    confirm(&mut harness);

    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::Confirmed {
            target: recording_id(&harness, 0),
            attach: false,
        })
    );
}

/// Attaching a log the recording already holds offers the stored attachment
/// for reuse: the dialog states that, and hands the attachment to the app.
#[test]
fn a_recording_that_already_holds_this_log_offers_that_attachment_for_reuse() {
    let mut harness = harness_over(vec![(
        recording("stored.gtd", Duration::zero(), 10),
        stored_in_history("nav-devkit-mk2"),
    )]);
    select(&mut harness, "stored.gtd");
    let queried = duplicate_query_to_send(&mut harness);

    assert!(
        queried.is_some(),
        "a recording in the database is queried once it is selected"
    );
    let existing = LogAttachmentId::new_random();
    harness.state_mut().dialog.set_duplicate_attachment(
        &stored_db_ref(),
        Some(ExistingLogAttachment {
            id: existing,
            name: "navsyncd.log".to_owned(),
        }),
    );
    harness.run_steps(2);

    harness.get_by_label_contains("Attaching reuses that attachment");
    assert_eq!(
        harness
            .state()
            .dialog
            .duplicate_attachment_of(&stored_db_ref())
            .map(|attachment| attachment.id),
        Some(existing)
    );
    assert_eq!(
        harness
            .state()
            .dialog
            .duplicate_attachment_of(&another_db_ref())
            .map(|attachment| attachment.id),
        None,
        "the answer stands for the recording it was queried about"
    );
}

/// The line about the attachment the recording already holds grows the
/// dialog, which re-anchors around its centre on the next frame. A confirm
/// click queued on that frame still reports the decision.
#[test]
fn confirming_as_the_stored_attachment_line_appears_reports_the_decision() {
    let mut harness = harness_over(vec![(
        recording("stored.gtd", Duration::zero(), 10),
        stored_in_history("nav-devkit-mk2"),
    )]);
    select(&mut harness, "stored.gtd");
    duplicate_query_to_send(&mut harness);
    harness.state_mut().dialog.set_duplicate_attachment(
        &stored_db_ref(),
        Some(ExistingLogAttachment {
            id: LogAttachmentId::new_random(),
            name: "navsyncd.log".to_owned(),
        }),
    );
    harness.step();

    harness.click_after_the_layout_settles(By::new().label(CONFIRM_LABEL));
    harness.run_steps(2);

    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::Confirmed {
            target: recording_id(&harness, 0),
            attach: false,
        })
    );
}

/// The dialog queries the database about each recording once, not once a
/// frame.
#[test]
fn the_duplicate_query_is_sent_once_per_chosen_recording() {
    let mut harness = harness_over(vec![(
        recording("stored.gtd", Duration::zero(), 10),
        stored_in_history("nav-devkit-mk2"),
    )]);
    select(&mut harness, "stored.gtd");

    assert_eq!(duplicate_query_to_send(&mut harness), Some(stored_db_ref()));
    assert_eq!(
        duplicate_query_to_send(&mut harness),
        None,
        "the result of the query in flight is what the dialog is waiting for"
    );
}

#[test]
fn ticking_dont_show_this_again_is_reported_with_the_decision() {
    let mut harness = harness_over(vec![(
        recording("alongside.gtd", Duration::zero(), 10),
        FileHistory::None,
    )]);
    assert!(!harness.state().dialog.dont_show_again());

    harness.get(By::new().label(DONT_SHOW_AGAIN_LABEL)).click();
    harness.run_steps(2);

    assert!(harness.state().dialog.dont_show_again());
}

fn stored_db_ref() -> DatabaseRef {
    DatabaseRef {
        identity: "nav-devkit-mk2".to_owned(),
        group_name: "2026-05-29T18-48-25".to_owned(),
    }
}

/// A recording of the database other than the one the dialog is selected on.
fn another_db_ref() -> DatabaseRef {
    DatabaseRef {
        identity: "nav-devkit-mk2".to_owned(),
        group_name: "2026-05-29T19-13-40".to_owned(),
    }
}

/// The dialog keeps its confirm and cancel buttons reachable at any viewport,
/// however many recordings the log could take its positions from and however
/// long their names are.
#[rstest::rstest]
fn the_dialog_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let long = gt_test_utils::oversized_text('r');
    let recordings = (0..OVERSIZED_ROW_COUNT)
        .map(|index| {
            (
                recording(&format!("{long}{index}.gtd"), Duration::seconds(0), 9),
                stored_in_history(&format!("{long}{index}")),
            )
        })
        .collect();
    let mut harness = harness_over_sized(recordings, viewport);
    harness.run_steps(8);

    harness.assert_window_fits_the_viewport(AuditedWindow::titled(TITLE));
    harness.assert_control_is_reachable(AuditedWindow::titled(TITLE), ControlLabel(CANCEL_LABEL));
}
