//! The dialog driven on its own: what it lists for a log, what it reports back,
//! and when it lets the log be attached.

use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone as _, Utc};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_loaded_files::{FileHistory, LoadedFileId, LoadedFiles, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs, LogAttachmentRef, PositionSourceSelection};
use gt_pending_writes::WriteAccess;

use crate::app::history_db::ExistingLogAttachment;
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::settings::InitialPositionSourcePolicy;
use gt_store::{
    DatabaseRef, LogAttachmentId, RecordingMeta, StoredLogSettings, StoredYearResolution,
};
use gt_test_utils::window_fit::{
    CRAMPED_VIEWPORT, NARROW_VIEWPORT, OVERSIZED_ROW_COUNT, SHORT_VIEWPORT,
};
use gt_test_utils::{
    AuditedWindow, By, ControlLabel, HarnessInteraction as _, TestHarness, WindowFitAssertions as _,
};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::{AssociationWindow, FileSource, Latitude, Longitude};
use gt_ui_types::LoadedLogId;
use rstest::rstest;

use super::{
    AUTOMATIC_POSITION_SOURCE_LABEL, CANCEL_LABEL, CONFIRM_LABEL, LogAssociationChoice,
    LogAssociationDialog, LogAssociationRequest, SAVE_CONFIRM_LABEL, SAVE_TITLE, TITLE,
};

fn log_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 29, 18, 48, 25)
        .single()
        .unwrap_or_default()
}

pub(in crate::app) struct DialogState {
    dialog: LogAssociationDialog,
    log: LoadedLog,
    recordings: LoadedFiles,
    pub(in crate::app) choice: Option<LogAssociationChoice>,
    write_access: WriteAccess,
    initial_position_source_policy: InitialPositionSourcePolicy,
}

impl DialogState {
    pub(in crate::app) fn open_save_dialog(&mut self) {
        self.dialog = LogAssociationDialog::for_request(
            LogAssociationRequest::Save(self.dialog.log()),
            self.dialog.selected,
        );
    }
}

/// A recording of `seconds` fixes starting `offset` after the log does.
pub(in crate::app) fn recording(
    filename: &str,
    offset: Duration,
    seconds: usize,
) -> gt_types::LoadedFile {
    let points = gt_test_utils::fixtures::nav_points_walking_from(
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
pub(in crate::app) fn stored_in_history(identity: &str) -> FileHistory {
    FileHistory::recording(
        identity.to_owned(),
        RecordingMeta {
            time_range: None,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
            debug_tag: None,
        },
        Some(DatabaseRef {
            identity: identity.to_owned(),
            group_name: "2026-05-29T18-48-25".to_owned(),
        }),
    )
}

fn harness_over(
    recordings: Vec<(gt_types::LoadedFile, FileHistory)>,
) -> Harness<'static, DialogState> {
    harness_over_sized(recordings, DIALOG_SIZE)
}

fn harness_for_saving(
    recordings: Vec<(gt_types::LoadedFile, FileHistory)>,
) -> Harness<'static, DialogState> {
    let mut state = dialog_state(LOG, recordings);
    state.dialog = LogAssociationDialog::for_request(LogAssociationRequest::Save(SHOWN_LOG), None);
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_size(DIALOG_SIZE)
        .build_ui_state(dialog_ui, state);
    harness.run_steps(3);
    harness
}

pub(in crate::app) fn harness_over_sized(
    recordings: Vec<(gt_types::LoadedFile, FileHistory)>,
    viewport: egui::Vec2,
) -> Harness<'static, DialogState> {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_size(viewport)
        .build_ui_state(dialog_ui, dialog_state(LOG, recordings));
    harness.run_steps(3);
    harness
}

fn dialog_state(
    log_text: &str,
    recordings: Vec<(gt_types::LoadedFile, FileHistory)>,
) -> DialogState {
    let mut loaded_recordings = LoadedFiles::new();
    for (file, history) in recordings {
        loaded_recordings.push(file, history);
    }
    let parsed = gt_logfile::parse_log(log_text.into(), log_start())
        .unwrap_or_else(|error| panic!("the fixture log parses: {error}"));
    DialogState {
        dialog: LogAssociationDialog::for_request(
            LogAssociationRequest::SelectSource(SHOWN_LOG),
            None,
        ),
        log: LoadedLog::new(
            Some("navsyncd.log".to_owned()),
            parsed,
            AssociationWindow::try_from_seconds(60).unwrap(),
        ),
        recordings: loaded_recordings,
        choice: None,
        write_access: WriteAccess::Owner,
        initial_position_source_policy: InitialPositionSourcePolicy::Ask,
    }
}

fn dialog_ui(ui: &mut egui::Ui, state: &mut DialogState) {
    let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
    let choice = state.dialog.show(
        ui.ctx(),
        &state.log,
        state.recordings.view(),
        &names,
        state.write_access,
        &mut state.initial_position_source_policy,
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

/// Sends the query the app sends for the chosen recording, and hands the
/// dialog the result that this recording already holds the log under `name`.
pub(in crate::app) fn deliver_the_stored_attachment(
    harness: &mut Harness<DialogState>,
    name: &str,
) {
    let Some(queried) = duplicate_query_to_send(harness) else {
        panic!("the chosen recording is queried about duplicates");
    };
    harness.state_mut().dialog.set_duplicate_attachment(
        &queried,
        Some(ExistingLogAttachment {
            id: LogAttachmentId::new_random(),
            name: name.to_owned(),
            settings: StoredLogSettings {
                association_window: None,
                filters: Default::default(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
        }),
    );
}

fn select(harness: &mut Harness<DialogState>, name: &str) {
    harness.get_by_label(name).click();
    harness.run_steps(2);
}

fn confirm(harness: &mut Harness<DialogState>) {
    harness.get_by_label(CONFIRM_LABEL).click();
    harness.run_steps(2);
}

#[test]
fn source_selection_explicitly_applies_to_the_session() {
    let mut harness = harness_over(Vec::new());
    harness.get_by_label("Position source for this session");
    harness.get_by_label(super::NO_SOURCE_LABEL);
    confirm(&mut harness);
    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::SourceSelected(
            PositionSourceSelection::NoSource
        ))
    );
}

#[rstest]
#[case::dark(true, "unresolved_save_dialog_dark")]
#[case::light(false, "unresolved_save_dialog_light")]
fn unresolved_save_destinations_have_focusable_unavailable_overlap(
    #[case] dark_mode: bool,
    #[case] snapshot_name: &str,
) {
    let mut state = dialog_state(
        gt_test_utils::UNRESOLVED_YEARS_LOG,
        vec![(
            recording("stored.gtd", Duration::zero(), 10),
            stored_in_history("nav-devkit-mk2"),
        )],
    );
    state.open_save_dialog();
    let mut harness = TestHarness::builder()
        .size(DIALOG_SIZE)
        .theme(dark_mode)
        .ui_state(dialog_ui, state);
    harness.inner.run_steps(3);
    assert!(
        harness
            .inner
            .query_by_label(super::NO_OVERLAP_LABEL)
            .is_none()
    );
    let status = harness
        .inner
        .get_by_label_contains(super::UNAVAILABLE_OVERLAP_LABEL);
    assert_eq!(
        status.accesskit_node().description().as_deref(),
        Some(super::UNRESOLVED_OVERLAP_HOVER)
    );
    status.focus();
    harness.inner.run_steps(2);
    harness.inner.get_by_label(super::UNRESOLVED_OVERLAP_HOVER);
    select(&mut harness.inner, "stored.gtd");
    let save = harness.inner.get_by_label(SAVE_CONFIRM_LABEL);
    assert!(!save.accesskit_node().is_disabled());
    assert_eq!(
        save.accesskit_node().description().as_deref(),
        Some("Save navsyncd.log with stored.gtd")
    );
    harness.snapshot(snapshot_name);
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
    harness.run_steps(gt_test_utils::TOOLTIP_LAYOUT_FRAMES);

    harness.get_by_label_contains("This recording has no overlapping track duration");
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
        Some(LogAssociationChoice::SourceSelected(
            PositionSourceSelection::Recording(recording_id(&harness, 0).unwrap())
        ))
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

#[test]
fn saving_requires_a_stored_destination() {
    let mut harness = harness_for_saving(vec![
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
            .get_by_label(SAVE_CONFIRM_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_label("dropped.gtd")
            .accesskit_node()
            .is_disabled()
    );
    assert!(harness.query_by_label(super::NO_SOURCE_LABEL).is_none());
    assert!(
        harness
            .query_by_label(AUTOMATIC_POSITION_SOURCE_LABEL)
            .is_none()
    );
    select(&mut harness, "stored.gtd");
    harness.get_by_label(SAVE_CONFIRM_LABEL).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::SaveRequested(
            recording_id(&harness, 1).unwrap()
        ))
    );
}

#[test]
fn saving_is_disabled_in_a_read_only_session() {
    let mut harness = harness_for_saving(vec![(
        recording("stored.gtd", Duration::zero(), 10),
        stored_in_history("nav-devkit-mk2"),
    )]);
    harness.state_mut().write_access = WriteAccess::ReadOnly;
    harness.run_steps(2);
    let save = harness.get_by_label(SAVE_CONFIRM_LABEL);
    assert!(save.accesskit_node().is_disabled());
    let position = save.rect().center();
    harness.hover_at_and_settle(position, 3);
    harness.run_steps(gt_test_utils::TOOLTIP_LAYOUT_FRAMES);
    harness.get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);
}

#[test]
fn saved_log_save_action_has_the_capability_restriction_hover() {
    let mut state = dialog_state(
        LOG,
        vec![(
            recording("stored.gtd", Duration::zero(), 10),
            stored_in_history("nav-devkit-mk2"),
        )],
    );
    state.dialog = LogAssociationDialog::for_request(LogAssociationRequest::Save(SHOWN_LOG), None);
    let settings = state.log.settings_snapshot();
    let mut logs = LoadedLogs::default();
    let id = logs
        .restore_attachment(
            state.log,
            LogAttachmentRef {
                recording: stored_db_ref(),
                id: LogAttachmentId::new_random(),
            },
            settings,
            &state.recordings.view(),
        )
        .id();
    state.log = logs.remove_by_id(id).unwrap();
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_size(DIALOG_SIZE)
        .build_ui_state(dialog_ui, state);
    harness.run_steps(3);
    let save = harness.get_by_label(SAVE_CONFIRM_LABEL);
    assert!(save.accesskit_node().is_disabled());
    let position = save.rect().center();
    harness.hover_at_and_settle(position, 3);
    harness.run_steps(gt_test_utils::TOOLTIP_LAYOUT_FRAMES);
    harness.get_by_label_contains(super::OPERATION_UNAVAILABLE_HOVER);
}

#[test]
fn a_recording_that_already_holds_this_log_offers_that_attachment_for_reuse() {
    let mut harness = harness_for_saving(vec![(
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
            settings: StoredLogSettings {
                association_window: None,
                filters: Default::default(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
        }),
    );
    harness.run_steps(2);

    harness.get_by_label_contains("Saving reuses this attachment");
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
        "the result stands for the recording it was queried about"
    );
}

/// A click queued on the frame the line about a stored attachment appears
/// reports the decision: that line lands inside the room the dialog reserved
/// and the confirm button stays where it was.
#[test]
fn confirming_as_the_stored_attachment_line_appears_reports_the_decision() {
    let mut harness = harness_for_saving(vec![(
        recording("stored.gtd", Duration::zero(), 10),
        stored_in_history("nav-devkit-mk2"),
    )]);
    select(&mut harness, "stored.gtd");
    deliver_the_stored_attachment(&mut harness, "navsyncd.log");
    harness.step();

    let confirm = harness
        .get(By::new().label(SAVE_CONFIRM_LABEL))
        .rect()
        .center();
    harness.click_at(confirm);
    harness.run_steps(2);

    assert_eq!(
        harness.state().choice,
        Some(LogAssociationChoice::SaveRequested(
            recording_id(&harness, 0).unwrap()
        ))
    );
}

/// The dialog queries the database about each recording once, not once a
/// frame.
#[test]
fn the_duplicate_query_is_sent_once_per_chosen_recording() {
    let mut harness = harness_for_saving(vec![(
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

#[rstest::rstest]
#[case::save_dark(
    DIALOG_SIZE,
    true,
    LogAssociationRequest::Save(SHOWN_LOG),
    "association_dialog_answer_pending"
)]
#[case::save_light(
    DIALOG_SIZE,
    false,
    LogAssociationRequest::Save(SHOWN_LOG),
    "save_log_dialog_light"
)]
#[case::source_dark(
    DIALOG_SIZE,
    true,
    LogAssociationRequest::SelectSource(SHOWN_LOG),
    "position_source_dialog_dark"
)]
#[case::source_light(
    DIALOG_SIZE,
    false,
    LogAssociationRequest::SelectSource(SHOWN_LOG),
    "position_source_dialog_light"
)]
#[case::narrow_width(
    egui::vec2(440.0, 420.0),
    true,
    LogAssociationRequest::Save(SHOWN_LOG),
    "association_dialog_answer_pending_narrow"
)]
fn snapshot_log_save_and_position_source_dialogs(
    #[case] viewport: egui::Vec2,
    #[case] dark_mode: bool,
    #[case] request: LogAssociationRequest,
    #[case] snapshot_name: &str,
) {
    let state = dialog_state(
        LOG,
        vec![
            (
                recording("alongside.gtd", Duration::zero(), 10),
                stored_in_history("nav-devkit-mk2"),
            ),
            (
                recording("late.gtd", Duration::seconds(5), 10),
                stored_in_history("nav-devkit-mk4"),
            ),
        ],
    );
    let mut harness = TestHarness::builder()
        .size(viewport)
        .theme(dark_mode)
        .ui_state(dialog_ui, state);
    let saving = matches!(request, LogAssociationRequest::Save(_));
    harness.inner.state_mut().dialog = LogAssociationDialog::for_request(request, None);
    harness.inner.run_steps(3);
    harness.inner.get_by_label("late.gtd").click();
    harness.inner.run_steps(2);
    let state = harness.inner.state_mut();
    let recordings = state.recordings.view();
    state.dialog.duplicate_query_to_send(recordings);
    harness.inner.run_steps(2);

    let (title, confirm_label) = if saving {
        (SAVE_TITLE, SAVE_CONFIRM_LABEL)
    } else {
        (TITLE, CONFIRM_LABEL)
    };
    let window = harness
        .inner
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(Some(title))))
        .expect("association dialog");
    assert!(window.width() <= 460.0);
    let confirm = harness.inner.get_by_label(confirm_label).rect();
    let cancel = harness.inner.get_by_label(CANCEL_LABEL).rect();
    assert!((confirm.center().y - cancel.center().y).abs() < 1.0);
    assert!(cancel.max.x < confirm.min.x);
    for control in [cancel, confirm] {
        assert!(window.contains_rect(control));
    }
    harness
        .inner
        .assert_window_fits_the_viewport(AuditedWindow::titled(title));
    harness.snapshot(snapshot_name);
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
    harness.assert_control_is_reachable(
        AuditedWindow::titled(TITLE),
        ControlLabel(AUTOMATIC_POSITION_SOURCE_LABEL),
    );
}

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
