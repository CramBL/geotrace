use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use egui_kittest::{Harness, kittest::Queryable as _};
use egui_phosphor::regular::GEAR as ICON_GEAR;
use gt_instance_lock::{
    DataDirectoryLock, DataDirectoryOwnership, InstanceState, InstanceStatus, InstanceStatusRead,
    TakeOverRecord,
};
use gt_pending_writes::{PendingWrites, WriteAccess, WriteKind};
use gt_store::{HistoryDatabase as _, ReadOnlyHistoryDatabase as _, Recordings};
use gt_test_utils::snapshot_harness;
use gt_test_utils::{HarnessInteraction as _, TestHarness};

use crate::app::App;
use crate::app::instance_wait::{
    DATA_DIRECTORY_HELD_TITLE, DATA_DIRECTORY_RETRY_INTERVAL, LOCK_FILE_UNUSABLE_TITLE,
    START_READ_ONLY_BUTTON_LABEL, TAKE_OVER_BUTTON_LABEL, TAKE_OVER_CONFIRMATION_TITLE,
    TAKE_OVER_WARNING, TakenOverInstance,
};
use crate::app::read_only_session::READ_ONLY_MARKER_LABEL;
use crate::app::storage::{DatabasesPending, OPENING_DATABASES, StorageOpen};
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// The same wait, with the registry every write of the run goes through left
/// to the case.
fn app_waiting_for_the_data_directory_registering_writes_in<'a>(
    data_directory: &Path,
    pending_writes: PendingWrites,
) -> Harness<'a, App> {
    let instance_lock = ui_tests::lock_on_a_directory_another_instance_holds(data_directory);
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            test_util::harness::transient_app_with_the_instance_lock(
                cc,
                &[],
                instance_lock,
                pending_writes,
            )
        })
}

/// A second GeoTrace on a data directory the first is using leaves its own
/// databases closed: recovery here would run against archives the first is
/// part-way through rewriting. Its window is up and takes input all the same.
#[test]
fn a_data_directory_another_instance_holds_is_waited_for_and_nothing_is_opened() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    harness.get_by_label_contains("Its window is open");
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "the databases were opened under the instance holding the directory"
    );
    assert!(
        harness.query_by_label_contains(OPENING_DATABASES).is_none(),
        "nothing is opening, so nothing says it is"
    );

    harness.get_by_label_contains(ICON_GEAR).click();
    harness.step();

    assert!(
        harness.state().settings_open,
        "the window took a click while it waited for the data directory"
    );
}

/// What the wait finds where the instance holding the data directory keeps
/// its status file.
#[derive(Debug, Clone, Copy)]
enum StatusFileOnDisk {
    Removed,
    /// A directory in its place, which `fs::read` fails on with an error
    /// other than `NotFound`.
    ADirectory,
    NotJson,
}

/// The lock is what says the directory is in use: the dialog says so however
/// the status file reads, and states which of the three reads it got.
#[rstest::rstest]
#[case::absent(StatusFileOnDisk::Removed, "It has not reported what it is doing yet")]
#[case::unreadable(StatusFileOnDisk::ADirectory, "Its status file cannot be read")]
#[case::malformed(StatusFileOnDisk::NotJson, "Its status file is damaged")]
fn a_held_data_directory_without_a_readable_status_still_says_it_is_held(
    #[case] status_file: StatusFileOnDisk,
    #[case] expected_label: &str,
) {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let status_path = directory.path().join(gt_instance_lock::STATUS_FILE_NAME);
    std::fs::remove_file(&status_path).expect("remove the status file");
    match status_file {
        StatusFileOnDisk::Removed => {}
        StatusFileOnDisk::ADirectory => {
            std::fs::create_dir(&status_path)
                .expect("create a directory where the status file goes");
        }
        StatusFileOnDisk::NotJson => {
            std::fs::write(&status_path, b"{not json").expect("write the status file");
        }
    }
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    harness.get_by_label_contains(expected_label);
}

const STALE_STATUS_WRITTEN_SECONDS_AGO: u64 = 60;

/// Puts a shutting-down status in `directory`, with `written_at` left to the
/// caller, over the one `DataDirectoryLock::acquire` wrote.
fn write_a_shutting_down_status(directory: &Path, written_at: Option<u64>) {
    std::fs::write(
        directory.join(gt_instance_lock::STATUS_FILE_NAME),
        serde_json::to_vec(&InstanceStatus {
            process_id: process::id(),
            state: InstanceState::ShuttingDown,
            pending_writes: vec![gt_instance_lock::PendingWriteReport {
                label: "Compacting the TEC archive".to_owned(),
                progress: None,
                stage: None,
            }],
            written_at,
        })
        .expect("serialize the status"),
    )
    .expect("write the status file");
}

/// A shutdown that stopped reporting - a `write_status` that failed, or an
/// instance stuck before its next `report_shutdown_progress` - still lists
/// the writes it had running, and the dialog states how old that is.
#[test]
fn a_status_the_holding_instance_stopped_refreshing_is_marked_as_out_of_date() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let written_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past the Unix epoch")
        .as_secs()
        - STALE_STATUS_WRITTEN_SECONDS_AGO;
    write_a_shutting_down_status(directory.path(), Some(written_at));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains("It is shutting down");
    harness.get_by_label_contains("Compacting the TEC archive");
    harness.get_by_label_contains("its last report is a minute old");
}

/// A status file from a GeoTrace before the `written_at` field existed: what
/// it reports is shown, and the dialog states that its age cannot be
/// measured.
#[test]
fn a_status_without_a_written_at_is_marked_as_being_of_unknown_age() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    write_a_shutting_down_status(directory.path(), None);
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains("It is shutting down");
    harness.get_by_label_contains("The age of this report is unknown");
}

/// The instance holding the directory writes an `InstanceState::Running`
/// status once, when it takes the mark, so the dialog states nothing about
/// the age of one however long that instance stays open.
#[test]
fn a_running_instance_is_never_reported_as_out_of_date() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let written_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past the Unix epoch")
        .as_secs()
        - STALE_STATUS_WRITTEN_SECONDS_AGO;
    std::fs::write(
        directory.path().join(gt_instance_lock::STATUS_FILE_NAME),
        serde_json::to_vec(&InstanceStatus {
            process_id: process::id(),
            state: InstanceState::Running,
            pending_writes: Vec::new(),
            written_at: Some(written_at),
        })
        .expect("serialize the status"),
    )
    .expect("write the status file");
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains("Its window is open");
    assert!(
        harness
            .query_by_label_contains("its last report is")
            .is_none(),
        "the dialog reported the age of a status the instance rewrites only once it shuts down"
    );
}

/// The wait ends by itself: the app takes the directory the instance holding
/// it let go of, and opens what it held back.
#[test]
fn the_wait_ends_when_the_instance_holding_the_data_directory_lets_go() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the app never opened its databases"
    );
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the dialog outlived the wait"
    );
    assert_eq!(
        InstanceStatusRead::read_from(directory.path())
            .status()
            .map(|status| status.state),
        Some(InstanceState::Running),
        "the app marks the data directory as its own once it takes it"
    );
}

/// A lock file that stops opening says nothing about who has the directory,
/// and whatever stopped it may pass: the wait goes on through the retries
/// that lock file is given, and ends by taking the lock once it opens again.
#[test]
fn a_lock_file_that_briefly_cannot_be_opened_leaves_the_wait_running() {
    let parent = tempfile::tempdir().expect("temp dir");
    let data_directory = parent.path().join("data");
    let holder = DataDirectoryLock::acquire(Some(&data_directory));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], &data_directory);
    harness.step();
    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);

    drop(holder);
    std::fs::remove_dir_all(&data_directory).expect("remove the data directory");
    std::fs::write(&data_directory, b"not a directory").expect("put a file in its place");
    thread::sleep(DATA_DIRECTORY_RETRY_INTERVAL);
    harness.run_steps(3);

    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "the databases opened on a directory this instance never locked"
    );
    harness.get_by_label_contains(LOCK_FILE_UNUSABLE_TITLE);

    std::fs::remove_file(&data_directory).expect("clear the way");

    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the wait never ended once the lock file could be opened"
    );
}

/// Waiting is not a trap: the window closes on request, and an app on its way
/// out leaves the directory unmarked and the databases closed.
#[test]
fn a_window_closed_while_it_waits_for_the_data_directory_opens_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();

    ui_tests::request_window_close(&mut harness);
    assert!(
        harness.step_until(|harness| ui_tests::root_viewport_commands(harness)
            .contains(&egui::ViewportCommand::Close)),
        "the window never closed"
    );
    drop(holder);
    thread::sleep(DATA_DIRECTORY_RETRY_INTERVAL);
    harness.run_steps(3);

    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "a closing app retried the directory and opened the databases on it"
    );
}

/// A file named on the command line of a second GeoTrace waits for the data
/// directory, and is loaded and stored once this instance owns it.
#[test]
fn a_file_named_while_the_data_directory_is_held_loads_once_it_frees() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let gtd_path = directory.path().join("queued.gtd");
    std::fs::write(&gtd_path, ui_tests::minimal_gtd_bytes()).expect("write the recording");
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[gtd_path], directory.path());
    harness.run_steps(3);

    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the load ran while another instance held the data directory"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    drop(holder);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
}

/// A drop lands in the same queue, which the dialog being up does not stop.
#[test]
fn a_file_dropped_while_the_data_directory_is_held_loads_once_it_frees() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::bytes(
            ui_tests::minimal_gtd_bytes().as_slice(),
            "dropped.gtd",
        )));
    harness.run_steps(3);
    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the drop loaded while another instance held the data directory"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    drop(holder);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
}

/// Pasted log text waits for the data directory like any other load: paste is
/// its own surface, and has escaped this queue before.
#[test]
fn log_text_pasted_while_the_data_directory_is_held_loads_once_it_frees() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.input_mut().events.push(egui::Event::Paste(
        "2026-01-01 14:02:11 navsyncd: queue empty\n".to_owned(),
    ));
    harness.run_steps(3);
    assert!(
        harness.state().logs.is_empty(),
        "the paste loaded while another instance held the data directory"
    );

    drop(holder);

    test_util::harness::step_until_a_log_is_loaded(&mut harness);
}

/// Reports `holder` as shutting down with one archive compaction left, which
/// is what its status file then names.
fn report_the_holder_as_compacting_an_archive(holder: &DataDirectoryLock) {
    let pending_writes = PendingWrites::default();
    let _compaction = pending_writes
        .try_begin(
            "Compacting the TEC archive",
            WriteKind::ArchiveCompaction {
                archive: "ionospheric TEC",
            },
        )
        .expect("the registry is running");
    holder.mark_shutting_down(&pending_writes);
}

/// Takes write access as the user does: the button in the wait dialog, then
/// the confirmation it leads to.
fn take_over_write_access(harness: &mut Harness<'_, App>) {
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);
    harness.get_by_label("Take over").click();
    harness.run_steps(3);
}

/// The wait is not a dead end: the button opens a confirmation stating what
/// the instance holding the directory is doing, which it reads afresh for as
/// long as the confirmation is up.
#[test]
fn the_take_over_confirmation_names_what_the_other_instance_is_doing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();

    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.get_by_label_contains(TAKE_OVER_CONFIRMATION_TITLE);
    harness.get_by_label_contains("window is open");
    harness.get_by_label_contains(TAKE_OVER_WARNING);
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the confirmation and the wait dialog are stacked on the same anchor"
    );

    report_the_holder_as_compacting_an_archive(&holder);

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some()),
        "the confirmation names a state the other instance has left"
    );
    harness.get_by_label_contains("still finishing these writes");
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "the databases opened before the user made a choice in the confirmation"
    );
}

/// The frame time the wait dialog's harness pins its clock at. The dialog
/// paints the same however many frames a case runs: a raw input keeps its
/// time until something sets a new one. egui draws the dialog's spinner from
/// that clock, and its arc is at its longest here.
const PINNED_FRAME_TIME: f64 = std::f64::consts::FRAC_PI_2;

/// The wait over a data directory held by a GeoTrace that reported a shutdown
/// [`STALE_STATUS_WRITTEN_SECONDS_AGO`] ago and has not reported since. That
/// state fills both dialogs: a statement, the write the other instance is
/// finishing, and the note marking the report as out of date.
fn app_waiting_on_a_stale_shutdown_report(directory: &Path) -> TestHarness<'static, App> {
    let written_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past the Unix epoch")
        .as_secs()
        - STALE_STATUS_WRITTEN_SECONDS_AGO;
    write_a_shutting_down_status(directory, Some(written_at));
    let instance_lock = ui_tests::lock_on_a_directory_another_instance_holds(directory);
    let harness = Harness::builder()
        .with_size(egui::vec2(640.0, 420.0))
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            test_util::harness::transient_app_with_the_instance_lock(
                cc,
                &[],
                instance_lock,
                PendingWrites::default(),
            )
        });
    let mut harness = TestHarness::from_harness(harness);
    harness.inner.input_mut().time = Some(PINNED_FRAME_TIME);
    harness.inner.run_steps(4);
    harness
}

#[test]
fn snapshot_data_directory_wait_dialog() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_on_a_stale_shutdown_report(directory.path());

    harness.snapshot_loose("data_directory_wait_dialog");
}

/// The pinned frame clock is what makes the baseline above a fixed image: the
/// spinner's arc would otherwise turn with every frame the case runs.
#[test]
fn the_wait_dialog_paints_the_same_however_many_frames_it_runs() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_on_a_stale_shutdown_report(directory.path());
    let dialog = harness
        .inner
        .window_rect(DATA_DIRECTORY_HELD_TITLE)
        .expect("the wait dialog is shown");
    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let opened = harness.inner.render().expect("the harness renders a frame");

    harness.inner.run_steps(5);

    let later = harness.inner.render().expect("the harness renders a frame");
    assert!(
        !snapshot_harness::pixels_differ(&opened, &later, dialog, pixels_per_point),
        "the wait dialog at {dialog:?} paints differently after five more frames"
    );
}

/// The wait re-reads the status file while its dialog is up, and the instance
/// holding the data directory reports the writes it is finishing once it
/// begins shutting down.
#[test]
fn the_wait_dialog_keeps_its_buttons_in_place_while_the_holder_reports_a_shutdown() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    let before = harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .rect();

    report_the_holder_as_compacting_an_archive(&holder);

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some()),
        "the wait dialog never named the write the holder is finishing"
    );
    assert_eq!(
        harness
            .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
            .rect(),
        before,
        "the read-only button of the wait dialog moved: a press where the user aimed misses it"
    );
}

/// The confirmation reads the same status file the wait does, for as long as
/// it is up.
#[test]
fn the_take_over_confirmation_keeps_its_buttons_in_place_while_the_holder_reports_a_shutdown() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);
    let before = harness.get_by_label("Cancel").rect();

    report_the_holder_as_compacting_an_archive(&holder);

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some()),
        "the confirmation never named the write the holder is finishing"
    );
    assert_eq!(
        harness.get_by_label("Cancel").rect(),
        before,
        "the Cancel button of the take-over confirmation moved: a press where the user aimed \
         misses it"
    );
}

#[test]
fn snapshot_take_over_confirmation() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_on_a_stale_shutdown_report(directory.path());

    harness
        .inner
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    // The harness paints a pointer wherever it last was, over the Cancel
    // button of the confirmation the click opens.
    harness.inner.remove_cursor();
    harness.inner.run_steps(4);

    harness.snapshot_loose("take_over_confirmation");
}

/// Cancelling leaves everything as it was: the wait dialog is back and
/// nothing has been opened.
#[test]
fn cancelling_the_take_over_returns_to_the_wait() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.get_by_label("Cancel").click();
    harness.run_steps(3);

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    assert!(
        harness
            .query_by_label_contains(TAKE_OVER_CONFIRMATION_TITLE)
            .is_none(),
        "the confirmation outlived the cancel"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "a cancelled take-over opened the databases"
    );
}

/// Escape makes the confirmation's Cancel choice, as it does for every other
/// destructive confirmation.
#[test]
fn escape_cancels_the_take_over_confirmation() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "escape opened the databases"
    );
}

/// Taking over opens the databases with the other instance still holding the
/// lock, and the loads that waited run against them.
#[test]
fn taking_over_opens_the_databases_and_runs_the_loads_that_waited() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let gtd_path = directory.path().join("queued.gtd");
    std::fs::write(&gtd_path, ui_tests::minimal_gtd_bytes()).expect("write the recording");
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[gtd_path], directory.path());
    harness.run_steps(3);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    take_over_write_access(&mut harness);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        None,
        "the take-over left the databases unopened"
    );
    assert_eq!(
        harness.state().instance_lock.ownership(),
        DataDirectoryOwnership::HeldByAnotherInstance,
        "the take-over took the lock instead of proceeding without it"
    );
    assert_eq!(
        harness.state().instance_taken_over_from,
        Some(TakenOverInstance {
            process_id: Some(process::id())
        }),
        "the take-over left no record of the instance it took write access from"
    );
    assert_eq!(
        harness.state().environment_deletes_blocked_by(),
        None,
        "the delete controls stayed grayed with the reason the wait gave"
    );
}

/// The take-over writes a record into the data directory: which process took
/// write access, which process it took it from, and when.
#[test]
fn taking_over_records_the_take_over_in_the_data_directory() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();

    take_over_write_access(&mut harness);

    let record = TakeOverRecord::read_from(directory.path()).expect("the take-over record");
    assert_eq!(record.taken_by_process_id, process::id());
    assert_eq!(
        record.taken_from_process_id,
        Some(process::id()),
        "the record holds no process id from the status file the wait read"
    );
    assert!(
        record.written_at.is_some(),
        "the record is stamped with no time"
    );
    assert!(
        harness.state().pending_writes.is_idle(),
        "the write the record was made under is still registered"
    );
}

/// A read-only session never reaches the wait's take-over: the registry
/// rejecting the write is the guard this exercises.
#[test]
fn a_take_over_in_a_read_only_session_records_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let pending_writes = PendingWrites::default();
    pending_writes.become_read_only_for_the_rest_of_the_run();
    let mut harness =
        app_waiting_for_the_data_directory_registering_writes_in(directory.path(), pending_writes);
    harness.step();

    take_over_write_access(&mut harness);

    assert_eq!(TakeOverRecord::read_from(directory.path()), None);
}

/// Taking over reads the archives before it opens any of them: the other
/// instance may be part-way through a delete right now, and the user chooses
/// what to do with what that leaves.
#[test]
fn taking_over_reads_the_archives_before_it_opens_them() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.get_by_label("Take over").click();

    assert!(
        harness.step_until(|harness| matches!(
            harness.state().storage_open,
            StorageOpen::InspectingArchives { .. }
        )),
        "the take-over opened the archives without reading them for an interrupted delete"
    );
}

/// Taking the lock late is a promotion and nothing more: the instance that
/// took over becomes the marked owner without reopening anything.
#[test]
fn the_lock_freed_after_a_take_over_makes_this_instance_the_marked_owner() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    take_over_write_access(&mut harness);
    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the take-over never opened the databases"
    );

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().instance_lock.ownership()
            == DataDirectoryOwnership::MarkedByThisInstance),
        "this instance never took the data directory the other one let go of"
    );
    assert_eq!(
        InstanceStatusRead::read_from(directory.path())
            .status()
            .map(|status| status.process_id),
        Some(process::id()),
        "the status file describes another instance than the one holding the directory"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        None,
        "the promotion opened the databases a second time"
    );
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the promotion put the app back in the wait"
    );
    assert!(
        harness.state().background_mark_retry.is_none(),
        "the retry goes on after this instance became the marked owner"
    );
}

/// A second GeoTrace waiting for the data directory the caller holds, saving
/// its settings at the returned path.
fn app_waiting_for_the_data_directory_that_saves_settings<'a>(
    data_directory: &Path,
) -> (TestHarness<'a, App>, PathBuf) {
    let instance_lock = ui_tests::lock_on_a_directory_another_instance_holds(data_directory);
    TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(move |cc, config_path, fading| {
            test_util::harness::build_app_with_the_instance_lock(
                cc,
                config_path,
                fading,
                PendingWrites::default(),
                instance_lock,
                gt_map::TileAccess::Synthetic,
            )
        })
}

/// Both instances write the same `config.toml` after a take-over. The
/// settings the other instance saves stand for as long as it runs: this one
/// holds its debounced flush back until the other lets go of the mark.
#[test]
fn a_settings_flush_during_the_run_waits_for_the_mark_after_a_take_over() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let (mut harness, config_path) =
        app_waiting_for_the_data_directory_that_saves_settings(directory.path());
    harness.inner.step();
    take_over_write_access(&mut harness.inner);

    harness.inner.state_mut().flush_settings();
    assert!(
        !config_path.exists(),
        "the flush overwrote the settings of the GeoTrace still holding the mark"
    );

    drop(holder);
    assert!(
        harness
            .inner
            .step_until(|harness| harness.state().background_mark_retry.is_none()),
        "this instance never took the mark the other one let go of"
    );
    harness.inner.state_mut().flush_settings();

    assert!(
        config_path.exists(),
        "the flush stayed held back after this instance took the mark"
    );
}

/// The flush `App::begin_shutdown` performs is not held back, because the
/// GeoTrace this session took write access from may never exit.
#[test]
fn closing_after_a_take_over_writes_the_settings_while_the_other_instance_holds_the_mark() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let (mut harness, config_path) =
        app_waiting_for_the_data_directory_that_saves_settings(directory.path());
    harness.inner.step();
    take_over_write_access(&mut harness.inner);

    ui_tests::request_window_close(&mut harness.inner);
    harness.inner.step();

    assert!(
        harness.inner.state().background_mark_retry.is_some(),
        "the other GeoTrace let go of the mark before the close"
    );
    assert!(config_path.exists(), "the shutdown wrote no settings");
}

/// Starts the session read-only as the user does: the wait dialog's button,
/// which leads to no confirmation.
fn start_the_session_read_only(harness: &mut Harness<'_, App>) {
    harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .click();
    harness.run_steps(3);
}

/// The wait offers a second way out: reading the recordings and archives
/// beside the instance that owns the data directory, which leaves that
/// instance's mark where it is.
#[test]
fn starting_read_only_leaves_the_wait_and_opens_the_databases_without_the_lock() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);

    start_the_session_read_only(&mut harness);

    assert_eq!(
        harness.state().pending_writes.write_access(),
        WriteAccess::ReadOnly,
        "the session went on writing after the user chose to read"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        None,
        "the read-only choice left the databases unopened"
    );
    assert_eq!(
        harness.state().instance_lock.ownership(),
        DataDirectoryOwnership::NoDataDirectory,
        "the read-only session kept a claim on the data directory"
    );
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the wait dialog outlived the read-only choice"
    );
}

/// No promotion, ever: the instance that owns the data directory letting go
/// leaves the read-only session as it is, and the directory free for whoever
/// starts next.
#[test]
fn a_read_only_session_does_not_become_the_owner_when_the_other_instance_exits() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    start_the_session_read_only(&mut harness);

    drop(holder);
    thread::sleep(DATA_DIRECTORY_RETRY_INTERVAL);
    harness.run_steps(3);

    assert_eq!(
        DataDirectoryLock::acquire(Some(directory.path())).ownership(),
        DataDirectoryOwnership::MarkedByThisInstance,
        "the read-only session holds the lock the next instance needs"
    );
    assert_eq!(
        harness.state().instance_lock.ownership(),
        DataDirectoryOwnership::NoDataDirectory,
        "the read-only session took the data directory the other instance let go of"
    );
    assert_eq!(
        harness.state().pending_writes.write_access(),
        WriteAccess::ReadOnly,
        "the read-only session started writing once the directory was free"
    );
}

/// The marker states what the session is, as the debug-build warning does,
/// and a session that owns the data directory shows none.
#[test]
fn only_a_read_only_session_shows_the_read_only_marker() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    assert!(
        harness
            .query_by_label_contains(READ_ONLY_MARKER_LABEL)
            .is_none(),
        "a session that has yet to choose is marked as read-only"
    );

    start_the_session_read_only(&mut harness);

    let marker = harness.get_by_label_contains(READ_ONLY_MARKER_LABEL).rect();
    harness.hover_at_and_settle(marker.center(), 3);
    harness.get_by_label_contains(&format!(
        "Another GeoTrace (process {}) owns the data directory",
        process::id()
    ));
    drop(holder);
}

/// The file a command line named waits through the choice: it loads once the
/// databases are open, and the read-only session stores none of it.
#[test]
fn a_file_queued_while_waiting_loads_read_only_and_is_not_stored() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let store = gt_store::Store::open_in(directory.path());
    let gtd_path = directory.path().join("queued.gtd");
    std::fs::write(&gtd_path, ui_tests::minimal_gtd_bytes()).expect("write the recording");
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[gtd_path], directory.path());
    harness.run_steps(3);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .click();
    // The load runs against a real recording history from here on: the test
    // itself supplies the databases in place of the app opening its own, and
    // this is the one frame the choice takes before that lands.
    harness.step();
    let databases = harness.state_mut().storage_open.take_over_for_test();
    ui_tests::land_the_databases(&mut harness, &databases, &store);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    let recordings =
        Recordings::open_or_create(&store.recordings_path()).expect("open the recording history");
    assert_eq!(
        recordings.list_recordings().expect("list").len(),
        0,
        "the read-only session stored the recording that waited for it"
    );
}

/// Paste is its own load surface and has escaped this queue before, so the
/// read-only exit from the wait carries it too.
#[test]
fn log_text_pasted_while_waiting_loads_in_the_read_only_session_it_starts() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let store = gt_store::Store::open_in(directory.path());
    let mut harness = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());

    harness.input_mut().events.push(egui::Event::Paste(
        "2026-01-01 14:02:11 navsyncd: queue empty\n".to_owned(),
    ));
    harness.run_steps(3);
    assert!(
        harness.state().logs.is_empty(),
        "the paste loaded while another instance held the data directory"
    );

    harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .click();
    harness.step();
    let databases = harness.state_mut().storage_open.take_over_for_test();
    ui_tests::land_the_databases(&mut harness, &databases, &store);

    test_util::harness::step_until_a_log_is_loaded(&mut harness);
}
