//! The dialog every loaded log raises while a recording is open, and what
//! confirming it does.

use chrono::Duration;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;
use gt_loaded_files::FileHistory;
use gt_log_view::{LoadedLog, LogAttachmentRef, RecordingKey};
use gt_store::{
    DatabaseRef, HistoryDatabase as _, LogAttachmentEntry, LogAttachments as _, LogToAttach,
    ReadOnlyHistoryDatabase as _, Recordings, RecordingsHandle, StoredLogFilter,
    StoredLogFilterMode, TrackRange, TrackState,
};
use gt_test_utils::{
    By, HarnessInteraction as _, SyntheticGtdSpec, SyntheticLogSpec, SyntheticLogTimestamps,
};
use gt_track_builder::SegmentationConfig;
use gt_types::{FileIdx, Latitude, Longitude};

use crate::app::App;
use crate::app::history_db::{HistoryWorker, StoredLogAttachment};
use crate::app::loader::{self, AttachedLogRequester, AttachedLogRestore};
use crate::app::log_viewer::{self, association_dialog};
use crate::app::modals::{DELETE_PERMANENTLY_BUTTON_LABEL, SHELVE_BUTTON_LABEL};
use crate::app::settings_ui;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// An app whose history worker owns a database of its own, so a log can be
/// stored with a recording and read back.
fn app_over_a_history_database(db_path: &std::path::Path) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().history = HistoryWorker::spawn(
        RecordingsHandle::Owner(ui_tests::open_temporary_history_database(db_path)),
        egui::Context::default(),
        gt_pending_writes::PendingWrites::default(),
    );
    harness.state_mut().sync_db_path();
    harness
}

fn drop_the_log(harness: &mut Harness<App>) {
    drop_a_log(harness, FIXTURE_LOG_SEED);
}

fn drop_a_log(harness: &mut Harness<App>, seed: u64) {
    ui_tests::drop_log_and_wait_for_load(harness, &fixture_log_text(seed), FIXTURE_LOG_NAME);
    harness.run_steps(3);
}

fn fixture_log_text(seed: u64) -> String {
    gt_test_utils::synthetic_journald_log(SyntheticLogSpec {
        approx_bytes: 8 * 1024,
        seed,
        timestamps: SyntheticLogTimestamps::Iso8601Space,
    })
}

fn dialog_is_open(harness: &Harness<App>) -> bool {
    harness.state().association_dialog.is_some()
}

fn confirm(harness: &mut Harness<App>) {
    harness
        .get(By::new().label(association_dialog::CONFIRM_LABEL))
        .click();
    harness.run_steps(3);
}

fn cancel(harness: &mut Harness<App>) {
    harness.get_by_label("Cancel").click();
    harness.run_steps(3);
}

fn shown_log_target(harness: &Harness<App>) -> Option<gt_loaded_files::LoadedFileId> {
    harness
        .state()
        .first_log()
        .and_then(gt_log_view::LoadedLog::associated_recording)
}

/// Every attachment the recording carries, as the database holds it.
fn stored_attachments(
    db_path: &std::path::Path,
    db_ref: &gt_store::DatabaseRef,
) -> Vec<LogAttachmentEntry> {
    Recordings::open_or_create(db_path)
        .ok()
        .and_then(|db| db.log_attachments(db_ref).ok())
        .unwrap_or_default()
}

/// An app over a database of its own, holding the fixture recording and
/// the fixture log with its association dialog open. Returns the recording
/// the database stored.
fn harness_over_a_recording_and_its_log(
    db_path: &std::path::Path,
) -> (Harness<'static, App>, gt_store::DatabaseRef) {
    let mut harness = app_over_a_history_database(db_path);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    let db_ref = stored_recording(&harness);
    drop_the_log(&mut harness);
    (harness, db_ref)
}

/// Ticks the dialog's attach box and confirms it, leaving the log stored
/// with the recording it was associated against.
fn attach_the_log(
    harness: &mut Harness<App>,
    db_path: &std::path::Path,
    db_ref: &gt_store::DatabaseRef,
) {
    harness
        .get_by_label(association_dialog::ATTACH_LABEL)
        .click();
    harness.run_steps(2);
    confirm(harness);
    assert!(
        harness.step_until(|_| !stored_attachments(db_path, db_ref).is_empty()),
        "the worker stored the log with the recording"
    );
    assert!(
        harness.step_until(|harness| harness
            .state()
            .first_log()
            .is_some_and(|log| log.attachment().is_some())),
        "the viewer noted the attachment the worker stored"
    );
}

/// The recording the app stored when the fixture recording was dropped.
fn stored_recording(harness: &Harness<App>) -> gt_store::DatabaseRef {
    let state = harness.state();
    let shared = state.shared.borrow();
    let entry = shared
        .loaded_files
        .view()
        .get(0)
        .expect("the recording is loaded");
    entry
        .history()
        .db_ref()
        .cloned()
        .expect("the dropped recording was stored in history")
}

/// The one recording the log overlaps is preselected, and confirming takes
/// the log's positions from it.
#[test]
fn confirming_the_dialog_associates_the_log_with_the_preselected_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_the_log(&mut harness);

    assert!(dialog_is_open(&harness), "a loaded log raises the dialog");
    harness.get_by_label(association_dialog::TITLE);
    assert_eq!(
        shown_log_target(&harness),
        None,
        "the log takes no position until the choice is made"
    );

    confirm(&mut harness);

    assert!(!dialog_is_open(&harness));
    assert!(
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_entry_count() > 0),
        "the preselected recording is what the log associates against"
    );
}

/// Several overlapping recordings leave the choice to the user: confirming
/// without making one leaves the log untargeted.
#[test]
fn several_overlapping_recordings_leave_the_dialog_without_a_preselection() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk_a.gtd", 55.0),
    );
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk_b.gtd", 60.0),
    );
    drop_the_log(&mut harness);
    assert!(dialog_is_open(&harness));

    confirm(&mut harness);

    assert_eq!(shown_log_target(&harness), None);
    assert_eq!(
        harness
            .state()
            .first_log()
            .map(gt_log_view::LoadedLog::associated_entry_count),
        Some(0)
    );
}

/// Cancelling loads the log as text: no target, and the viewer's footer
/// left as the way to pick one.
#[test]
fn cancelling_the_dialog_loads_the_log_untargeted() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_the_log(&mut harness);

    cancel(&mut harness);

    assert!(!dialog_is_open(&harness));
    assert_eq!(shown_log_target(&harness), None);
    assert_eq!(
        harness.state().logs.len(),
        1,
        "the log is loaded either way"
    );
    assert!(harness.state().log_viewer.open);
}

/// Escape belongs to the dialog while it is open: the viewer it stands over
/// stays open.
#[test]
fn escape_cancels_the_dialog_and_leaves_the_viewer_open() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_the_log(&mut harness);
    assert!(dialog_is_open(&harness));

    ui_tests::press_escape(&mut harness);
    harness.run_steps(3);

    assert!(!dialog_is_open(&harness));
    assert!(harness.state().log_viewer.open, "the viewer stays open");
    assert_eq!(shown_log_target(&harness), None);

    ui_tests::press_escape(&mut harness);
    harness.run_steps(3);

    assert!(
        !harness.state().log_viewer.open,
        "with the dialog gone, Escape closes the viewer"
    );
}

/// A stored log that is not the log the attribute names it as: the same
/// warning path as one that went missing.
#[test]
fn an_attachment_whose_stored_log_changed_is_reported_in_the_viewer() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);

    // The attribute now holds the content hash of a different log.
    let stored = stored_attachments(&db_path, &db_ref);
    let entry = stored.first().expect("the attachment was stored");
    let mut db = ui_tests::open_temporary_history_database(&db_path);
    db.write_log_attachment_attribute(
        &db_ref,
        entry.id,
        &gt_store::LogAttachment::new(
            entry.attachment.name.clone(),
            gt_store::LogContentHash::of_log_bytes(b"a different log"),
            Vec::new(),
        ),
    )
    .expect("the attribute is writable");
    drop(db);

    test_util::harness::drop_a_stored_recording_and_load_it_from_disk(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("is not the log it was stored as")
            .is_some()),
        "the viewer says the stored log is not the one that was attached"
    );
    assert_eq!(
        harness.state().logs.len(),
        1,
        "nothing was restored from the attachment"
    );
}

/// Clicks the tickbox of the settings row headed `label`.
///
/// A settings row is its label beside its control, and the tickbox carries no
/// label of its own: it is the one drawn across the label's row.
fn click_settings_row_tickbox(harness: &mut Harness<App>, label: &str) {
    let row = harness.get_by_label_contains(label).rect();
    let tickbox = harness
        .query_all(By::new().role(egui::accesskit::Role::CheckBox))
        .find(|node| row.y_range().contains(node.rect().center().y));
    match tickbox {
        Some(tickbox) => tickbox.click(),
        None => panic!("the settings row {label:?} draws a tickbox"),
    }
    harness.run_steps(2);
}

#[test]
fn enabling_automatic_position_source_in_the_dialog_and_disabling_in_settings_restores_dialogs() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_the_log(&mut harness);

    harness
        .get_by_label(association_dialog::AUTOMATIC_POSITION_SOURCE_LABEL)
        .click();
    harness.run_steps(2);
    assert!(!harness.state().ask_log_association_target);
    assert!(
        !harness
            .state()
            .collect_settings_for_flush()
            .processing
            .ask_log_association_target
    );
    cancel(&mut harness);
    assert!(!harness.state().ask_log_association_target);

    drop_a_log(&mut harness, FIXTURE_LOG_SEED + 1);

    assert!(!dialog_is_open(&harness));
    assert!(
        harness
            .state()
            .last_log()
            .is_some_and(|log| log.associated_entry_count() > 0),
        "the only overlapping recording is selected automatically"
    );

    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = settings_ui::SettingsPage::Processing;
    harness.run_steps(2);
    let row = harness
        .get_by_label_contains(settings_ui::processing::AUTOMATIC_LOG_POSITION_SOURCE_LABEL)
        .rect();
    let tickbox = harness
        .query_all(By::new().role(Role::CheckBox))
        .find(|node| row.y_range().contains(node.rect().center().y))
        .expect("automatic position source checkbox");
    assert_eq!(tickbox.accesskit_node().toggled(), Some(Toggled::True));
    click_settings_row_tickbox(
        &mut harness,
        settings_ui::processing::AUTOMATIC_LOG_POSITION_SOURCE_LABEL,
    );
    harness.run_steps(2);
    harness.state_mut().settings_open = false;
    harness.run_steps(2);
    assert!(harness.state().ask_log_association_target);

    drop_a_log(&mut harness, FIXTURE_LOG_SEED + 2);

    assert!(dialog_is_open(&harness));
    assert_eq!(
        harness
            .get_by_label(association_dialog::AUTOMATIC_POSITION_SOURCE_LABEL)
            .accesskit_node()
            .toggled(),
        Some(Toggled::False)
    );
}

#[rstest::rstest]
#[case::zero_overlaps(0)]
#[case::one_overlap(1)]
#[case::multiple_overlaps(2)]
fn enabling_automatic_position_source_in_settings_and_disabling_in_the_dialog(
    #[case] overlaps: usize,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    if overlaps == 0 {
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            TestDroppedFile::bytes(ui_tests::minimal_gtd_bytes(), "earlier.gtd"),
        );
    }
    for index in 0..overlaps {
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            ui_tests::recording_alongside_the_log(&format!("walk{index}.gtd"), 55.0 + index as f64),
        );
    }
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = settings_ui::SettingsPage::Processing;
    harness.run_steps(2);
    click_settings_row_tickbox(
        &mut harness,
        settings_ui::processing::AUTOMATIC_LOG_POSITION_SOURCE_LABEL,
    );
    let persisted = harness.state().collect_settings_for_flush();
    assert!(!persisted.processing.ask_log_association_target);
    harness.state_mut().ask_log_association_target = true;
    harness.state_mut().apply_startup_settings(&persisted);
    assert!(!harness.state().ask_log_association_target);
    harness.state_mut().settings_open = false;
    harness.run_steps(2);

    drop_the_log(&mut harness);
    assert!(!dialog_is_open(&harness));
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::associated_recording)
            .is_some(),
        overlaps == 1
    );
    let log_id = harness.state().logs.first_id().expect("loaded log");
    harness
        .state_mut()
        .log_viewer_requests
        .open_association_dialog = Some(log_id);
    harness.state_mut().apply_log_viewer_requests();
    harness.run_steps(3);
    let checkbox = harness.get_by_label(association_dialog::AUTOMATIC_POSITION_SOURCE_LABEL);
    assert_eq!(checkbox.accesskit_node().toggled(), Some(Toggled::True));
    checkbox.click();
    harness.run_steps(2);
    assert!(harness.state().ask_log_association_target);
    assert!(
        harness
            .state()
            .collect_settings_for_flush()
            .processing
            .ask_log_association_target
    );
    confirm(&mut harness);

    drop_a_log(&mut harness, FIXTURE_LOG_SEED + 1);
    assert!(dialog_is_open(&harness));
}

/// The whole attachment path over a real database: the dialog stores the
/// log with the recording, chip edits follow it, and opening that recording
/// again brings the log back with its filter stack.
#[test]
fn an_attached_log_comes_back_with_its_filters_when_the_recording_opens_again() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);

    attach_the_log(&mut harness, &db_path, &db_ref);

    let attachments = stored_attachments(&db_path, &db_ref);
    assert_eq!(
        attachments
            .iter()
            .map(|entry| entry.attachment.name.as_str())
            .collect::<Vec<_>>(),
        ["navsyncd.log"]
    );

    // A chip added after the attachment was stored is written to it.
    ui_tests::add_log_filter_in(&mut harness, "kernel");
    assert!(
        harness.step_until(|_| {
            !stored_attachments(&db_path, &db_ref)
                .first()
                .is_none_or(|entry| entry.attachment.filters.is_empty())
        }),
        "the chip reached the stored attachment"
    );

    // With the session copy unloaded, the attachment is what brings the
    // log back.
    unload_the_log(&mut harness);
    assert_eq!(harness.state().logs.len(), 0);

    // Opening the recording again restores the log it carries.
    test_util::harness::drop_a_stored_recording_and_load_it_from_disk(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    test_util::harness::step_until_a_log_is_loaded(&mut harness);
    let restored = harness
        .state()
        .first_log()
        .map(|log| {
            log.filters()
                .chips()
                .iter()
                .map(|chip| (chip.pattern().text.clone(), chip.mode()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(
        restored,
        [("kernel".to_owned(), gt_log_view::FilterChipMode::Layer)],
        "the restored log carries the stack it was stored with"
    );
    assert!(
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_recording().is_some()),
        "a restored log is associated with the recording that carried it"
    );
}

/// A log the recording holds is listed under it as soon as it is unloaded,
/// and its load button reads it back with its anchor, its attachment and
/// the filters it was stored with.
#[test]
fn an_unloaded_attachment_is_listed_under_its_recording_and_loads_back() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);
    ui_tests::add_log_filter_in(&mut harness, "kernel");
    assert!(
        harness.step_until(|_| stored_attachments(&db_path, &db_ref)
            .first()
            .is_some_and(|entry| entry.attachment.filters.len() == 1)),
        "the chip reached the stored attachment"
    );
    let stored = stored_attachments(&db_path, &db_ref)
        .first()
        .map(|entry| entry.id)
        .expect("the attachment was stored");
    assert!(
        harness
            .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
            .is_none(),
        "a log that is loaded is listed as the loaded log it is"
    );

    unload_the_log(&mut harness);
    assert_eq!(harness.state().logs.len(), 0);

    harness
        .get_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
        .click();
    test_util::harness::step_until_a_log_is_loaded(&mut harness);

    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::attachment)
            .map(|attachment| attachment.id),
        Some(stored),
        "the log that came back is the one the recording holds"
    );
    assert!(
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_recording().is_some()),
        "the log takes its positions from the recording that holds it"
    );
    assert_eq!(
        harness
            .state()
            .first_log()
            .map(|log| log
                .filters()
                .chips()
                .iter()
                .map(|chip| chip.pattern().text.clone())
                .collect::<Vec<_>>())
            .unwrap_or_default(),
        ["kernel".to_owned()],
        "the log came back under the stack it was stored with"
    );
    assert!(
        harness
            .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
            .is_none(),
        "the attachment is the loaded log's now"
    );
}

/// A recording leaving the session takes the logs it holds off the list
/// with it.
#[test]
fn the_unloaded_attachments_of_a_removed_recording_leave_the_list() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);
    unload_the_log(&mut harness);
    assert!(
        harness
            .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
            .is_some()
    );

    open_the_shelve_confirmation(&mut harness, false);
    harness.get_by_label(SHELVE_BUTTON_LABEL).click();
    harness.run_steps(3);

    assert!(
        harness
            .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
            .is_none(),
        "the recording that held the log is gone from the session"
    );
    assert!(
        harness
            .state()
            .log_attachments
            .of_recording(&db_ref)
            .is_empty()
    );
}

/// Opens the shelve confirmation on the recording the session loaded
/// first, with the permanent-delete box ticked when `permanently`.
fn open_the_shelve_confirmation(harness: &mut Harness<App>, permanently: bool) {
    harness.state_mut().shared.borrow_mut().tree.shelve_confirm =
        Some(gt_side_panel::ShelveConfirmState {
            items: vec![gt_side_panel::NodeKey::File(FileIdx::new(0))],
            delete_permanently: permanently,
        });
    harness.run_steps(3);
}

/// A recording leaving the session takes its attached logs with it, and the
/// dialog states that before the user confirms.
#[test]
fn shelving_every_track_of_a_recording_unloads_the_logs_it_holds() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);

    open_the_shelve_confirmation(&mut harness, false);
    harness.get_by_label_contains("Unloads 1 attached log");

    harness.get_by_label(SHELVE_BUTTON_LABEL).click();
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
}

/// Deleting the recording permanently takes its attached logs out of the
/// session with it, and the dialog counts them before the user confirms.
#[test]
fn permanently_deleting_a_recording_states_the_attached_logs_it_deletes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);

    open_the_shelve_confirmation(&mut harness, true);
    harness.get_by_label_contains("Deletes 1 attached log with them");

    harness
        .get_by_label(DELETE_PERMANENTLY_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    assert!(
        harness.step_until(|_| stored_attachments(&db_path, &db_ref).is_empty()),
        "the deleted recording took its attachment with it"
    );
}

/// The recording is opened again while its log is still loaded: the
/// session keeps the one log, which keeps the attachment it holds.
#[test]
fn a_recording_opened_again_restores_no_attachment_that_is_already_loaded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);
    let loaded = harness.state().logs.first_id();

    restore_the_stored_attachment(&mut harness, &db_path, &db_ref);

    assert_eq!(harness.state().logs.len(), 1, "no second copy was loaded");
    assert_eq!(
        harness.state().logs.first_id(),
        loaded,
        "the loaded log kept its identity"
    );
}

#[test]
fn a_restored_attachment_keeps_a_separate_context_from_a_loose_log_with_the_same_source() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);
    let stored = stored_attachments(&db_path, &db_ref)
        .first()
        .map(|entry| entry.id)
        .expect("the attachment was stored");

    // The same text, loaded by itself and anchored to the recording that
    // holds the attachment in the database.
    unload_the_log(&mut harness);
    drop_the_log(&mut harness);
    confirm(&mut harness);
    let loaded = harness.state().logs.first_id();
    assert_eq!(
        harness.state().first_log().and_then(LoadedLog::attachment),
        None,
        "a log the user loaded is stored nowhere"
    );

    restore_the_stored_attachment(&mut harness, &db_path, &db_ref);

    assert_eq!(harness.state().logs.len(), 2);
    assert_eq!(harness.state().logs.first_id(), loaded);
    assert_eq!(
        harness.state().first_log().and_then(LoadedLog::attachment),
        None
    );
    let attachment = LogAttachmentRef {
        recording: db_ref,
        id: stored,
    };
    let saved = harness
        .state()
        .logs
        .id_of_attachment(&attachment)
        .expect("saved context");
    assert_ne!(Some(saved), loaded);
}

/// The toolbar's log button counts a log that comes back with a recording,
/// and the viewer stays closed on the log it was showing.
#[test]
fn the_toolbar_counts_a_log_restored_with_a_recording_until_the_viewer_opens() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);
    unload_the_log(&mut harness);
    drop_a_log(&mut harness, FIXTURE_LOG_SEED + 1);
    cancel(&mut harness);
    let shown = harness.state().log_viewer.selected_log();
    harness.state_mut().log_viewer.open = false;
    harness.run_steps(2);

    restore_the_stored_attachment(&mut harness, &db_path, &db_ref);

    assert_eq!(harness.state().logs.len(), 2, "the attachment came back");
    assert!(!harness.state().log_viewer.open, "the viewer stays closed");
    assert_eq!(
        harness.state().log_viewer.selected_log(),
        shown,
        "the viewer still shows the log the user was reading"
    );
    assert_eq!(harness.state().log_viewer.restored_logs.count(), 1);
    assert!(harness.state().log_viewer.restored_logs.is_pulsing());
    harness.get_by_label(format!("{ICON_ARTICLE} 1").as_str());

    // The harness clock ticks a quarter second per frame, past the two
    // seconds the pulse runs for.
    harness.run_steps(12);

    assert!(
        !harness.state().log_viewer.restored_logs.is_pulsing(),
        "the pulse ends by itself"
    );
    assert_eq!(
        harness.state().log_viewer.restored_logs.count(),
        1,
        "the count stands until the viewer is opened"
    );

    harness
        .get_by_label(format!("{ICON_ARTICLE} 1").as_str())
        .click();
    harness.run_steps(3);

    assert!(harness.state().log_viewer.open);
    assert_eq!(harness.state().log_viewer.restored_logs.count(), 0);
    harness.get_by_label(ICON_ARTICLE);
}

/// Attaching a log the recording already holds reuses the stored
/// attachment: the loaded log takes it, and the app writes its filter
/// stack there.
#[test]
fn attaching_a_log_the_recording_already_holds_reuses_the_stored_attachment() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);
    ui_tests::add_log_filter_in(&mut harness, "kernel");
    assert!(
        harness.step_until(|_| stored_attachments(&db_path, &db_ref)
            .first()
            .is_some_and(|entry| entry.attachment.filters.len() == 1)),
        "the chip reached the stored attachment"
    );
    let stored = stored_attachments(&db_path, &db_ref)
        .first()
        .map(|entry| entry.id)
        .expect("the attachment was stored");

    // The same text loaded again, filtered by none of the chips the stored
    // attachment holds.
    unload_the_log(&mut harness);
    drop_the_log(&mut harness);
    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("Saving uses that log again")
            .is_some()),
        "the dialog states what the recording already holds"
    );
    harness
        .get(By::new().label(association_dialog::ATTACH_LABEL))
        .click();
    harness.state_mut().toasts.dismiss_all_toasts();
    confirm(&mut harness);

    assert!(
        harness.step_until(|_| stored_attachments(&db_path, &db_ref)
            .first()
            .is_some_and(|entry| entry.attachment.filters.is_empty())),
        "the stack of the loaded log reached the attachment it took"
    );
    assert_eq!(
        stored_attachments(&db_path, &db_ref)
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [stored],
        "the recording holds the one attachment it held before"
    );
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::attachment)
            .map(|attachment| attachment.id),
        Some(stored)
    );
    assert_eq!(harness.state().toasts.len(), 1, "the reuse raises a toast");
}

/// A database holding the fixture recording and the fixture log stored
/// with it, as a session that attached the log left them.
fn seed_a_recording_and_the_log_stored_with_it(
    db_path: &std::path::Path,
) -> (gt_store::DatabaseRef, gt_store::LogAttachmentId) {
    let bytes = ui_tests::recording_bytes_alongside_the_log(55.0);
    let meta = gt_store::extract_meta(&bytes).expect("the fixture recording carries metadata");
    let tracks = [TrackRange {
        start: 0,
        end: meta.nav_point_count,
        state: TrackState::Live,
    }];
    let mut db = ui_tests::open_temporary_history_database(db_path);
    let db_ref = db
        .insert(
            "walk.gtd",
            &meta,
            &tracks,
            crate::app::loader::stored_segmentation_from_config(&SegmentationConfig::default()),
            &bytes,
        )
        .expect("the fixture recording is stored");
    let stored = db
        .attach_log(
            &db_ref,
            &LogToAttach {
                name: FIXTURE_LOG_NAME,
                text: &fixture_log_text(FIXTURE_LOG_SEED),
                filters: vec![StoredLogFilter {
                    text: "kernel".to_owned(),
                    regex: false,
                    enabled: true,
                    mode: StoredLogFilterMode::Layer { color_slot: 0 },
                }],
                year_reference: None,
            },
        )
        .expect("the fixture log is stored with the recording");
    (db_ref, stored.id)
}

/// Opens one of a recording's stored logs, as the history window's
/// "Open log" does.
fn open_the_stored_log(
    harness: &Harness<App>,
    db_ref: &gt_store::DatabaseRef,
    id: gt_store::LogAttachmentId,
) {
    harness.state().history.load_attached_log(
        LogAttachmentRef {
            recording: db_ref.clone(),
            id,
        },
        FIXTURE_LOG_NAME.to_owned(),
    );
}

/// The chips the shown log is filtered by, in stack order.
fn shown_log_chips(harness: &Harness<App>) -> Vec<String> {
    harness
        .state()
        .first_log()
        .map(|log| {
            log.filters()
                .chips()
                .iter()
                .map(|chip| chip.pattern().text.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// A log opened out of history while its recording is not loaded: it comes
/// back attached, anchored to that recording, under the stack it was
/// stored with, and no line of it has a position.
#[test]
fn opening_a_stored_log_alone_loads_it_anchored_and_without_positions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
    let mut harness = app_over_a_history_database(&db_path);

    open_the_stored_log(&harness, &db_ref, stored);

    test_util::harness::step_until_a_log_is_loaded(&mut harness);
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(LoadedLog::attachment)
            .map(|attachment| attachment.id),
        Some(stored)
    );
    assert_eq!(
        harness.state().first_log().and_then(LoadedLog::anchor_key),
        Some(gt_log_view::RecordingKey::Stored(db_ref)),
        "the log is anchored to the recording that holds it"
    );
    assert_eq!(shown_log_chips(&harness), ["kernel".to_owned()]);
    assert_eq!(
        harness
            .state()
            .first_log()
            .map(|log| (log.associated_recording(), log.associated_entry_count())),
        Some((None, 0)),
        "no line has a position while the anchored recording is not loaded"
    );
    assert!(harness.state().log_viewer.open);
    assert_eq!(
        harness.state().log_viewer.selected_log(),
        harness.state().logs.first_id(),
        "the viewer opens on the log the user asked for"
    );
}

#[test]
fn opening_a_stored_log_that_is_already_loaded_loads_no_second_copy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
    let mut harness = app_over_a_history_database(&db_path);
    open_the_stored_log(&harness, &db_ref, stored);
    test_util::harness::step_until_a_log_is_loaded(&mut harness);
    let loaded = harness.state().logs.first_id();
    harness.state_mut().toasts.dismiss_all_toasts();

    open_the_stored_log(&harness, &db_ref, stored);

    assert!(
        harness.step_until(|harness| harness.state().toasts.len() == 1),
        "the second copy raises a toast"
    );
    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(harness.state().logs.first_id(), loaded);
    assert_eq!(harness.state().log_viewer.selected_log(), loaded);
}

/// The footer's "Load recording" opens the anchored recording from
/// history, and the log takes its positions from it as soon as it loads.
#[test]
fn loading_the_recording_of_a_stored_log_gives_its_lines_positions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
    let mut harness = app_over_a_history_database(&db_path);
    open_the_stored_log(&harness, &db_ref, stored);
    test_util::harness::step_until_a_log_is_loaded(&mut harness);
    harness.run_steps(3);
    harness.get_by_label("Positions from walk.gtd (not loaded)");

    harness
        .get_by_label(log_viewer::LOAD_RECORDING_LABEL)
        .click();

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    assert!(
        harness.step_until(|harness| harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_entry_count() > 0)),
        "the lines take their positions from the recording that loaded"
    );
    assert!(
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_recording().is_some())
    );
    assert!(
        harness
            .query_by_label(log_viewer::LOAD_RECORDING_LABEL)
            .is_none(),
        "the footer states the loaded recording, with nothing left to open"
    );
}

/// Hands the app the attachment a recording load reads back, parsed as the
/// loader's worker parses it.
fn restore_the_stored_attachment(
    harness: &mut Harness<App>,
    db_path: &std::path::Path,
    db_ref: &gt_store::DatabaseRef,
) {
    let stored = stored_attachments(db_path, db_ref);
    let entry = stored.first().expect("the attachment was stored");
    let text = fixture_log_text(FIXTURE_LOG_SEED);
    let parsed = gt_logfile::parse_log(text.as_str().into(), gt_test_utils::synthetic_log_start())
        .expect("the fixture log parses");
    let restore = AttachedLogRestore {
        attachment: LogAttachmentRef {
            recording: db_ref.clone(),
            id: entry.id,
        },
        filters: entry.attachment.filters.clone(),
        requested_by: crate::app::loader::AttachedLogRequester::RecordingLoad,
        year_reference: gt_test_utils::synthetic_log_start(),
    };
    harness
        .state_mut()
        .load_parsed_log(Some(entry.attachment.name.clone()), parsed, Some(restore));
    harness.run_steps(2);
}

/// Unloads the shown log through the viewer's own button, which the
/// selector row draws before any filter chip's.
fn unload_the_log(harness: &mut Harness<App>) {
    harness
        .nth_matching(By::new().label(egui_phosphor::regular::X), 0)
        .click();
    harness.run_steps(3);
}

/// The stored log is gone from the store, and the recording still opens.
#[test]
fn an_attachment_whose_log_file_is_gone_is_reported_in_the_viewer() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);

    for entry in std::fs::read_dir(dir.path().join(gt_store::LOGS_DIRECTORY))
        .expect("the logs directory exists")
        .flatten()
    {
        std::fs::remove_file(entry.path()).expect("the stored log is removable");
    }
    test_util::harness::drop_a_stored_recording_and_load_it_from_disk(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("log missing from recording history")
            .is_some()),
        "the viewer says the attachment did not come back"
    );
    assert_eq!(
        harness.state().shared.borrow().loaded_files.len(),
        2,
        "the recording loads either way"
    );
}

/// Removing the attachment takes it out of the database and leaves the log
/// loaded.
#[test]
fn removing_an_attachment_leaves_the_log_loaded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);

    harness.get_by_label(log_viewer::DETACH_LABEL).click();
    harness.run_steps(3);

    assert!(
        harness.step_until(|_| stored_attachments(&db_path, &db_ref).is_empty()),
        "the database no longer holds the log"
    );
    assert!(
        harness.step_until(|harness| harness
            .state()
            .first_log()
            .is_some_and(|log| log.attachment().is_none())),
        "the viewer noted the attachment the worker removed"
    );
    assert_eq!(harness.state().logs.len(), 1, "the session copy stays");
}

#[rstest::rstest]
#[case(true)]
#[case(false)]
fn the_footer_changes_a_saved_logs_recording_only_after_attachment_removal(
    #[case] attached_recording_loaded: bool,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = if attached_recording_loaded {
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        (harness, db_ref)
    } else {
        let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
        let mut harness = app_over_a_history_database(&db_path);
        open_the_stored_log(&harness, &db_ref, stored);
        test_util::harness::step_until_a_log_is_loaded(&mut harness);
        (harness, db_ref)
    };
    let attachment = harness
        .state()
        .first_log()
        .expect("the log is loaded")
        .attachment()
        .cloned();
    let target = shown_log_target(&harness);
    let points = gt_test_utils::fixtures::nav_points_walking_from(
        gt_test_utils::synthetic_log_start() + Duration::seconds(1),
        600,
        1,
        Latitude::new(60.0),
        Longitude::new(12.0),
    );
    ui_tests::push_points_as(&mut harness, "walk_b.gtd", &points, None, FileHistory::None);
    harness.run_steps(3);

    let current_source = if attached_recording_loaded {
        "walk.gtd"
    } else {
        gt_ui_theme::EM_DASH
    };
    harness
        .get(By::new().role(Role::ComboBox).value(current_source))
        .click();
    harness.run_steps(2);
    let other = harness.bottommost_matching(By::new().label("walk_b.gtd"));
    assert!(other.accesskit_node().is_disabled());
    let position = other.rect().center();
    harness.hover_at_and_settle(position, 3);
    harness.get_by_label_contains("Remove the saved log before changing its position source");
    harness
        .bottommost_matching(By::new().label("walk_b.gtd"))
        .click();
    harness.run_steps(2);

    let log = harness.state().first_log().expect("the log stays loaded");
    assert_eq!(
        log.anchor_key().as_ref(),
        Some(&RecordingKey::Stored(db_ref.clone()))
    );
    assert_eq!(log.attachment(), attachment.as_ref());
    assert_eq!(shown_log_target(&harness), target);

    harness.get_by_label("Positions from").click();
    harness.run_steps(2);
    harness.get_by_label(log_viewer::DETACH_LABEL).click();
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.attachment().is_none())
    }));
    assert!(stored_attachments(&db_path, &db_ref).is_empty());

    harness
        .get(By::new().role(Role::ComboBox).value(current_source))
        .click();
    harness.run_steps(2);
    let other = harness.bottommost_matching(By::new().label("walk_b.gtd"));
    assert!(!other.accesskit_node().is_disabled());
    other.click();
    harness.run_steps(2);

    let state = harness.state();
    let log = state.first_log().expect("the log stays loaded");
    let shared = state.shared.borrow();
    let other = shared
        .loaded_files
        .view()
        .entries()
        .last()
        .expect("recording B is loaded");
    assert_eq!(
        log.anchor_key().as_ref(),
        Some(&RecordingKey::of_loaded_recording(other))
    );
    assert_eq!(log.associated_recording(), Some(other.id()));
    assert_eq!(log.attachment(), None);
}

/// The recording's database entry is gone by the time the attach runs: the
/// failure is reported and the loaded log is untouched.
#[test]
fn attaching_to_a_recording_deleted_mid_session_reports_the_failure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut harness = app_over_a_history_database(&db_path);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    let db_ref = stored_recording(&harness);
    drop_the_log(&mut harness);

    let mut db = Recordings::open_or_create(&db_path).expect("the database opens");
    db.delete_batch(std::slice::from_ref(&db_ref))
        .expect("the recording is deletable");
    drop(db);

    harness
        .get_by_label(association_dialog::ATTACH_LABEL)
        .click();
    harness.run_steps(2);
    confirm(&mut harness);

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("Could not save navsyncd.log with the recording")
            .is_some()),
        "the viewer reports what the database rejected"
    );
    assert_eq!(harness.state().logs.len(), 1);
    assert!(
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.attachment().is_none()),
        "nothing about the loaded log changed"
    );
}

/// The stored stack is the chips, in the modes and colours they were in.
#[test]
fn the_stored_stack_holds_every_chips_mode_and_colour() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
    attach_the_log(&mut harness, &db_path, &db_ref);

    ui_tests::add_log_filter_in(&mut harness, "kernel");
    ui_tests::add_log_filter_in(&mut harness, "rotated");
    assert!(
        harness.step_until(|_| stored_attachments(&db_path, &db_ref)
            .first()
            .is_some_and(|entry| entry.attachment.filters.len() == 2)),
        "both chips reached the stored attachment"
    );

    let stored = stored_attachments(&db_path, &db_ref);
    let filters = stored
        .first()
        .map(|entry| entry.attachment.filters.clone())
        .unwrap_or_default();
    assert_eq!(
        filters
            .iter()
            .map(|filter| (filter.text.as_str(), filter.enabled, filter.mode))
            .collect::<Vec<_>>(),
        [
            ("kernel", true, StoredLogFilterMode::Layer { color_slot: 0 }),
            (
                "rotated",
                true,
                StoredLogFilterMode::Layer { color_slot: 1 }
            ),
        ]
    );
}

#[rstest::rstest]
#[case(AttachedLogRequester::RecordingLoad)]
#[case(AttachedLogRequester::UserOpenedTheAttachment)]
fn identical_attachments_in_two_recordings_load_with_independent_sources_and_filters(
    #[case] requested_by: AttachedLogRequester,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (first_recording, first_attachment) = seed_a_recording_and_the_log_stored_with_it(&db_path);
    let bytes = gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: gt_test_utils::synthetic_log_start() + Duration::days(1),
        point_count: 600,
        step_secs: 1,
        start_lat_deg: 60.0,
        start_lon_deg: 12.0,
        lat_step_deg: 0.00005,
        lon_step_deg: 0.00008,
        heading_deg: 20.0,
        speed_kmh: 28.0,
        eph_m: 1.8,
        sats_seen: 14,
        sats_in_fix: 11,
    });
    let meta = gt_store::extract_meta(&bytes).expect("fixture metadata");
    let filters = vec![StoredLogFilter {
        text: "systemd".to_owned(),
        regex: false,
        enabled: false,
        mode: StoredLogFilterMode::Refine,
    }];
    let mut db = ui_tests::open_temporary_history_database(&db_path);
    let second_recording = db
        .insert(
            "drive.gtd",
            &meta,
            &[TrackRange {
                start: 0,
                end: meta.nav_point_count,
                state: TrackState::Live,
            }],
            loader::stored_segmentation_from_config(&SegmentationConfig::default()),
            &bytes,
        )
        .expect("second recording is stored");
    assert_ne!(first_recording, second_recording);
    let second_attachment = db
        .attach_log(
            &second_recording,
            &LogToAttach {
                name: FIXTURE_LOG_NAME,
                text: &fixture_log_text(FIXTURE_LOG_SEED),
                filters: filters.clone(),
                year_reference: None,
            },
        )
        .expect("second attachment is stored")
        .id;
    drop(db);
    let mut harness = app_over_a_history_database(&db_path);
    open_the_stored_log(&harness, &first_recording, first_attachment);
    test_util::harness::step_until_a_log_is_loaded(&mut harness);
    let first_id = harness
        .state()
        .logs
        .first_id()
        .expect("first log is loaded");
    let first_ref = LogAttachmentRef {
        recording: first_recording.clone(),
        id: first_attachment,
    };
    let second_ref = LogAttachmentRef {
        recording: second_recording.clone(),
        id: second_attachment,
    };
    let first_filters = harness
        .state()
        .logs
        .get_by_id(first_id)
        .expect("first log")
        .filters()
        .to_stored_filters();

    match requested_by {
        AttachedLogRequester::RecordingLoad => {
            restore_the_stored_attachment(&mut harness, &db_path, &second_recording)
        }
        AttachedLogRequester::UserOpenedTheAttachment => {
            open_the_stored_log(&harness, &second_recording, second_attachment);
            assert!(harness.step_until(|harness| harness.state().logs.len() == 2));
        }
    }

    assert_eq!(harness.state().logs.len(), 2);
    let second_id = harness
        .state()
        .logs
        .id_of_attachment(&second_ref)
        .expect("second attachment is loaded");
    assert_ne!(first_id, second_id);
    for (id, attachment, stored_filters) in [
        (first_id, &first_ref, &first_filters),
        (second_id, &second_ref, &filters),
    ] {
        let log = harness.state().logs.get_by_id(id).expect("saved log");
        assert_eq!(log.attachment(), Some(attachment));
        assert_eq!(
            log.anchor_key().as_ref(),
            Some(&RecordingKey::Stored(attachment.recording.clone()))
        );
        assert_eq!(log.filters().to_stored_filters(), *stored_filters);
        assert!(harness.state().logs.any_loaded_log_holds(attachment));
    }
    assert_eq!(
        harness.state().log_viewer.selected_log(),
        Some(match requested_by {
            AttachedLogRequester::RecordingLoad => first_id,
            AttachedLogRequester::UserOpenedTheAttachment => second_id,
        })
    );
    harness.state_mut().toasts.dismiss_all_toasts();
    open_the_stored_log(&harness, &second_recording, second_attachment);
    assert!(harness.step_until(|harness| harness.state().toasts.len() == 1));
    assert_eq!(harness.state().logs.len(), 2);
    assert_eq!(harness.state().log_viewer.selected_log(), Some(second_id));

    let text = fixture_log_text(FIXTURE_LOG_SEED);
    let parsed = gt_logfile::parse_log(text.as_str().into(), gt_test_utils::synthetic_log_start())
        .expect("fixture log");
    harness
        .state_mut()
        .load_parsed_log(Some("loose.log".to_owned()), parsed, None);
    assert_eq!(harness.state().logs.len(), 3);
    let loose_id = harness
        .state()
        .log_viewer
        .selected_log()
        .expect("loose context");
    assert_ne!(loose_id, first_id);
    assert_ne!(loose_id, second_id);

    harness.state_mut().logs.forget_attachment(&second_ref);
    harness.state_mut().toasts.dismiss_all_toasts();
    open_the_stored_log(&harness, &second_recording, second_attachment);
    assert!(harness.step_until(|harness| harness.state().logs.any_loaded_log_holds(&second_ref)));
    assert_eq!(harness.state().logs.len(), 4);
    let restored_id = harness
        .state()
        .logs
        .id_of_attachment(&second_ref)
        .expect("restored context");
    assert_ne!(restored_id, second_id);
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(second_id)
            .expect("detached context")
            .filters()
            .to_stored_filters(),
        filters
    );
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(restored_id)
            .expect("restored context")
            .filters()
            .to_stored_filters(),
        filters
    );
    assert_eq!(harness.state().log_viewer.selected_log(), Some(restored_id));
}

#[rstest::rstest]
#[case(None)]
#[case(Some(RecordingKey::Stored(DatabaseRef { identity: "other".to_owned(), group_name: "other".to_owned() })))]
fn a_saved_log_loads_separately_from_incompatible_loose_content(
    #[case] anchor: Option<RecordingKey>,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let (recording, attachment_id) = seed_a_recording_and_the_log_stored_with_it(&db_path);
    let mut harness = app_over_a_history_database(&db_path);
    let text = fixture_log_text(FIXTURE_LOG_SEED);
    let parsed = gt_logfile::parse_log(text.as_str().into(), gt_test_utils::synthetic_log_start())
        .expect("fixture log");
    harness
        .state_mut()
        .load_parsed_log(Some("loose.log".to_owned()), parsed, None);
    let loose_id = harness
        .state()
        .logs
        .first_id()
        .expect("loose log is loaded");
    if let Some(key) = &anchor {
        let shared = harness.state().shared.clone();
        harness
            .state_mut()
            .logs
            .get_mut_by_id(loose_id)
            .expect("loose log")
            .anchor_to(key.clone(), &shared.borrow().loaded_files.view());
    }
    open_the_stored_log(&harness, &recording, attachment_id);
    assert!(harness.step_until(|harness| harness.state().logs.len() == 2));
    let loose = harness.state().logs.get_by_id(loose_id).expect("loose log");
    assert_eq!(loose.anchor_key().as_ref(), anchor.as_ref());
    assert_eq!(loose.attachment(), None);
    let attachment = LogAttachmentRef {
        recording,
        id: attachment_id,
    };
    let saved_id = harness
        .state()
        .logs
        .id_of_attachment(&attachment)
        .expect("saved log is loaded");
    assert_ne!(loose_id, saved_id);
    assert_eq!(harness.state().log_viewer.selected_log(), Some(saved_id));
    let entry = stored_attachments(&db_path, &attachment.recording)
        .into_iter()
        .next()
        .expect("saved attachment");
    let saved_filters = entry.attachment.filters.clone();
    let stored = StoredLogAttachment {
        recording: attachment.recording.clone(),
        entry,
    };
    harness.state_mut().log_viewer.open_on_log(loose_id);

    harness
        .state_mut()
        .apply_log_attach_outcome(loose_id, "loose.log", Ok(stored));

    assert_eq!(harness.state().logs.len(), 2);
    assert_eq!(harness.state().log_viewer.selected_log(), Some(saved_id));
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(loose_id)
            .expect("loose log")
            .attachment(),
        None
    );
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(saved_id)
            .expect("saved log")
            .filters()
            .to_stored_filters(),
        saved_filters
    );
    harness.state_mut().log_viewer.open_on_log(loose_id);

    harness.state().history.open(attachment.recording.clone());
    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    harness
        .state_mut()
        .log_viewer_requests
        .open_association_dialog = Some(loose_id);
    harness.state_mut().apply_log_viewer_requests();
    harness.run_steps(3);
    assert!(harness.step_until(|harness| {
        harness
            .query_by_label_contains("Saving uses that log again")
            .is_some()
    }));
    harness
        .get_by_label(association_dialog::ATTACH_LABEL)
        .click();
    confirm(&mut harness);

    assert_eq!(harness.state().logs.len(), 2);
    assert_eq!(harness.state().log_viewer.selected_log(), Some(saved_id));
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(loose_id)
            .expect("loose log")
            .attachment(),
        None
    );
    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(saved_id)
            .expect("saved log")
            .filters()
            .to_stored_filters(),
        saved_filters
    );
    assert_eq!(
        stored_attachments(&db_path, &attachment.recording)
            .first()
            .expect("saved attachment")
            .attachment
            .filters,
        saved_filters
    );
}

/// The seed [`drop_the_log`] writes its lines from.
const FIXTURE_LOG_SEED: u64 = 7;

/// The name the fixture log is loaded and stored under.
const FIXTURE_LOG_NAME: &str = "navsyncd.log";
