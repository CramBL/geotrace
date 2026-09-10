use std::panic;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration as StdDuration;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use egui_phosphor::regular::CHECK as ICON_CHECK;
use gt_instance_lock::{
    DataDirectoryLock, DataDirectoryOwnership, InstanceState, InstanceStatus, InstanceStatusRead,
    MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES,
};
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::RecordingsHandle;
use gt_test_utils::{HarnessInteraction as _, TestHarness};

use crate::app::App;
use crate::app::test_util;
use crate::app::ui_tests;
use crate::termination_signal::TERMINATION_SIGNAL_FLAG;

/// Pressing the close button with nothing pending: the app takes the close
/// over, writes the settings, and closes the window itself.
#[test]
fn closing_the_window_takes_the_close_over_and_writes_the_settings() {
    let (mut harness, config_path) = TestHarness::builder().eframe(test_util::harness::build_app);
    harness.inner.step();
    assert!(
        !config_path.exists(),
        "nothing has written the settings yet"
    );

    ui_tests::request_window_close(&mut harness.inner);
    harness.inner.step();

    assert!(
        ui_tests::root_viewport_commands(&harness.inner)
            .contains(&egui::ViewportCommand::CancelClose),
        "the close was cancelled instead of tearing the app down"
    );
    assert!(config_path.exists(), "shutdown wrote the settings");
    assert!(
        harness.inner.step_until(ui_tests::closed_the_window),
        "the window never closed"
    );
}

/// A read-only session persists nothing, on the way out as much as during
/// the run: neither flush creates the settings file.
#[test]
fn closing_a_read_only_session_writes_no_settings() {
    let (mut harness, config_path) = TestHarness::builder().eframe(|cc, path, fading| {
        test_util::harness::build_app_with_write_access(cc, path, fading, WriteAccess::ReadOnly)
    });
    harness.inner.step();

    harness.inner.state_mut().flush_settings();
    assert!(
        !config_path.exists(),
        "a settings change wrote the settings file"
    );

    ui_tests::request_window_close(&mut harness.inner);
    harness.inner.step();

    assert!(
        !config_path.exists(),
        "the flush the shutdown performs wrote the settings file"
    );
    assert!(
        harness.inner.step_until(ui_tests::closed_the_window),
        "the window never closed"
    );
}

#[test]
fn a_settings_flush_during_the_run_registers_a_pending_write() {
    let (mut harness, config_path) = TestHarness::builder().eframe(test_util::harness::build_app);
    harness.inner.step();

    harness.inner.state_mut().flush_settings();

    assert!(config_path.exists(), "the flush wrote no settings file");
    assert_eq!(
        harness
            .inner
            .state()
            .pending_writes
            .snapshot()
            .recently_finished,
        vec!["Saving settings"]
    );
}

/// The debounced flush writes nothing once the shutdown has begun:
/// `App::begin_shutdown` already wrote the settings through
/// `PendingWrites::try_begin_shutdown_write`.
#[test]
fn a_settings_flush_after_the_shutdown_flush_writes_nothing() {
    let (mut harness, config_path) = TestHarness::builder().eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state().pending_writes.begin_shutdown();

    harness.inner.state_mut().flush_settings();

    assert!(
        !config_path.exists(),
        "the debounced flush wrote after the shutdown flush"
    );
}

/// A `config.toml` that cannot be replaced leaves no `config.toml.tmp` for
/// the next run to find.
#[test]
fn a_settings_flush_that_cannot_replace_the_settings_file_removes_its_temporary() {
    let (mut harness, config_path) = TestHarness::builder().eframe(test_util::harness::build_app);
    harness.inner.step();
    std::fs::create_dir(&config_path).expect("occupy the settings path with a directory");

    harness.inner.state_mut().flush_settings();

    assert!(
        !config_path.with_extension("toml.tmp").exists(),
        "the failed flush left its temporary behind"
    );
}

/// The write held running while the shutdown window is on screen.
const TEC_COMPACTION: gt_pending_writes::WriteKind =
    gt_pending_writes::WriteKind::ArchiveCompaction {
        archive: "ionospheric TEC",
    };

/// An app with a write running and no close request yet.
fn app_with_a_running_write<'a>() -> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let compaction = harness
        .state()
        .pending_writes
        .try_begin("Compacting the TEC archive", TEC_COMPACTION)
        .expect("the registry is running");
    (harness, compaction)
}

/// An app that took a close request while a write is still running, on the
/// frame shutdown began: the grace has not elapsed yet.
fn app_closing_over_a_running_write<'a>() -> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard)
{
    let (mut harness, compaction) = app_with_a_running_write();

    ui_tests::request_window_close(&mut harness);
    harness.step();
    (harness, compaction)
}

fn step_until_the_shutdown_window_is_up(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness.query_by_label("Shutting down").is_some()),
        "the shutdown window never came up"
    );
}

fn shrank_the_window(harness: &Harness<'_, App>) -> bool {
    ui_tests::root_viewport_commands(harness)
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::InnerSize(_)))
}

/// A write that was running when the close button was pressed holds the window
/// open. The normal UI keeps painting until the shutdown window replaces it,
/// and the window closes once the write finishes.
#[test]
fn the_normal_ui_paints_until_the_shutdown_window_replaces_it() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    assert!(
        harness.query_by_label("File").is_some(),
        "the normal UI stopped painting during the grace"
    );
    assert!(harness.query_by_label("Shutting down").is_none());
    assert!(
        !ui_tests::closed_the_window(&harness),
        "the window closed over a running write"
    );

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        harness.query_by_label("File").is_none(),
        "the shutdown window paints alongside the normal UI"
    );

    drop(compaction);
    assert!(
        harness.step_until(ui_tests::closed_the_window),
        "the window never closed after the write finished"
    );
}

/// A termination signal takes the same path as the close button: shutdown
/// begins, the shutdown window comes up over the running write, and the
/// window closes once that write finishes. There is no close event to cancel.
///
/// No other test sees the process-global flag this raises: every test runs
/// in its own process under `cargo nextest`.
#[test]
fn a_termination_signal_begins_the_same_shutdown_without_a_close_to_cancel() {
    let (mut harness, compaction) = app_with_a_running_write();

    TERMINATION_SIGNAL_FLAG.raise();
    harness.step();

    assert!(
        !ui_tests::root_viewport_commands(&harness).contains(&egui::ViewportCommand::CancelClose),
        "a close that was never requested was cancelled"
    );
    step_until_the_shutdown_window_is_up(&mut harness);
    assert!(
        !ui_tests::closed_the_window(&harness),
        "the window closed over a running write"
    );

    drop(compaction);
    assert!(
        harness.step_until(ui_tests::closed_the_window),
        "the window never closed after the write finished"
    );
}

/// A signal arriving after the close button started shutdown is still only
/// the first one: the close button already promised the writes would finish.
/// A force quit here would end this test's process.
#[test]
fn a_signal_after_the_close_button_leaves_the_writes_running() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    TERMINATION_SIGNAL_FLAG.raise();
    harness.step();

    step_until_the_shutdown_window_is_up(&mut harness);
    assert!(
        !ui_tests::closed_the_window(&harness),
        "the window closed over a running write"
    );

    drop(compaction);
    assert!(
        harness.step_until(ui_tests::closed_the_window),
        "the window never closed after the write finished"
    );
}

/// The shutdown window states the write it is waiting for, how far it has got
/// and which step it is on.
#[test]
fn the_shutdown_window_shows_a_running_write_with_its_progress_and_stage() {
    let (mut harness, compaction) = app_closing_over_a_running_write();
    compaction.set_progress(0.25);
    compaction.set_stage("Rewriting maps");

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some(),
        "the shutdown window never named the write it is waiting for"
    );
    assert!(harness.query_by_label("Rewriting maps").is_some());
    assert_eq!(
        harness
            .get_by_role(egui::accesskit::Role::ProgressIndicator)
            .accesskit_node()
            .numeric_value(),
        Some(25.0)
    );
    drop(compaction);
}

/// The writes shutdown already got through are listed as done.
#[test]
fn the_shutdown_window_marks_the_writes_that_finished() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        harness
            .query_by_label(&format!("{ICON_CHECK} Saving settings"))
            .is_some(),
        "the settings flush that shutdown ran is not listed as done"
    );
    drop(compaction);
}

/// "Run in background" closes the window without waiting: the write keeps
/// running, and the wait after `run_native` returns is what finishes it.
#[test]
fn running_in_the_background_closes_the_window_while_the_write_runs() {
    let (mut harness, compaction) = app_closing_over_a_running_write();
    step_until_the_shutdown_window_is_up(&mut harness);

    // The window has just shrunk: the button only takes a click at the place
    // the harness reports once the new size is laid out.
    harness.run_steps(2);

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run in background")
        .click();

    assert!(
        harness.step_until(ui_tests::closed_the_window),
        "the window stayed up after running in the background"
    );
    assert!(
        !harness.state().pending_writes.is_idle(),
        "the window closed only once the write had finished"
    );
    drop(compaction);
}

/// A shutdown window whose force-quit confirmation the user has just opened,
/// with the write it is waiting for still running.
fn app_with_the_force_quit_confirmation_open<'a>()
-> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard) {
    let (mut harness, compaction) = app_closing_over_a_running_write();
    step_until_the_shutdown_window_is_up(&mut harness);
    // The window has just shrunk: the button only takes a click at the place
    // the harness reports once the new size is laid out.
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label(&TEC_COMPACTION.interruption_cost())
            .is_none(),
        "the confirmation was up before anyone asked to quit"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Force quit…")
        .click();
    harness.run_steps(2);
    (harness, compaction)
}

/// Cancelling the confirmation goes back to the shutdown window, which is
/// still up and still waiting for the write.
#[test]
fn cancelling_the_force_quit_confirmation_returns_to_the_shutdown_window() {
    let (mut harness, compaction) = app_with_the_force_quit_confirmation_open();

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
        .click();
    harness.run_steps(2);

    assert!(
        harness
            .query_by_label(&TEC_COMPACTION.interruption_cost())
            .is_none(),
        "the confirmation stayed up after it was cancelled"
    );
    assert!(harness.query_by_label("Shutting down").is_some());
    assert!(
        harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some(),
        "the shutdown window stopped listing the write it is waiting for"
    );
    assert!(
        !ui_tests::closed_the_window(&harness),
        "cancelling closed the window"
    );
    assert!(!harness.state().pending_writes.is_idle());
    drop(compaction);
}

/// With the pointer over Cancel when the last write finishes, the confirmation
/// stays up reporting the finished writes. The window waits for it, and the
/// press aimed at Cancel lands on the Close button that replaced it.
#[test]
fn the_force_quit_confirmation_takes_the_press_aimed_at_it_when_the_last_write_finishes() {
    let (mut harness, compaction) = app_with_the_force_quit_confirmation_open();
    let aimed_at = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
        .rect()
        .center();
    harness.hover_at(aimed_at);
    harness.run_steps(2);

    drop(compaction);
    // The button's label counts the seconds down, and the hover text of that
    // same button opens with "Close" too.
    assert!(
        harness.step_until(|harness| harness.query_by_label_contains("Close (").is_some()),
        "the confirmation never reported the finished writes"
    );
    assert!(
        harness
            .query_by_label_contains("The work finished")
            .is_some(),
        "the confirmation never said the work finished"
    );
    assert!(
        !ui_tests::closed_the_window(&harness),
        "the window closed while the confirmation was up"
    );

    harness.press_where_the_pointer_rests(aimed_at);

    assert!(
        harness.query_by_label_contains("Close (").is_none(),
        "the press aimed at Cancel missed the Close button that took its place"
    );
    assert!(
        harness.state().shutdown.close_allowed(),
        "the window never closed after the confirmation did"
    );
}

/// The window shrinks to the shutdown window's size as it comes up, and is
/// left at whatever size the user drags it to from then on.
#[test]
fn the_shutdown_window_shrinks_the_window_once() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        shrank_the_window(&harness),
        "the window never shrank to the shutdown window's size"
    );
    for _ in 0..3 {
        harness.step();
        assert!(!shrank_the_window(&harness), "the window shrank again");
    }
    drop(compaction);
}

/// The instance that owns `data_directory`, with a write running that nothing
/// finishes on its own. A close request there raises the shutdown window over
/// that write, and the status this instance keeps is what a second one reads.
fn app_holding_the_data_directory_over_a_running_write<'a>(
    data_directory: &Path,
) -> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard) {
    let instance_lock = DataDirectoryLock::acquire(Some(data_directory));
    assert_eq!(
        instance_lock.ownership(),
        DataDirectoryOwnership::MarkedByThisInstance,
        "the holder is meant to own the data directory it reports on"
    );
    let pending_writes = PendingWrites::default();
    let compaction = pending_writes
        .try_begin("Compacting the TEC archive", TEC_COMPACTION)
        .expect("the registry is running");
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            test_util::harness::transient_app_with_the_instance_lock(
                cc,
                &[],
                instance_lock,
                pending_writes,
            )
        });
    harness.step();
    (harness, compaction)
}

/// The window being up is no reason to switch to it once its shutdown has
/// begun: the wait lists the writes that instance is finishing instead.
#[test]
fn an_instance_shutting_down_with_its_window_up_is_named_with_what_it_is_writing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (mut holder, compaction) =
        app_holding_the_data_directory_over_a_running_write(directory.path());
    let mut waiting = ui_tests::app_waiting_for_the_data_directory(&[], directory.path());
    waiting.step();
    waiting.get_by_label_contains("Its window is open");

    ui_tests::request_window_close(&mut holder);
    step_until_the_shutdown_window_is_up(&mut holder);

    assert!(
        waiting.step_until(|waiting| waiting
            .query_by_label_contains("Compacting the TEC archive")
            .is_some()),
        "the wait never named the write the shutting-down instance is finishing"
    );
    waiting.get_by_label_contains("It is shutting down");
    drop(compaction);
}

/// The shutdown window keeps the status file current while it is up: a write
/// that finishes there drops off what a second instance reads.
#[test]
fn the_shutdown_window_reports_the_writes_left_as_they_finish() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (mut holder, compaction) =
        app_holding_the_data_directory_over_a_running_write(directory.path());

    ui_tests::request_window_close(&mut holder);
    step_until_the_shutdown_window_is_up(&mut holder);

    let read = InstanceStatusRead::read_from(directory.path());
    let status = read.status().expect("the status file");
    assert_eq!(status.state, InstanceState::ShuttingDown);
    assert!(
        reports_the_compaction(status),
        "the shutdown window never reported the write it is waiting for"
    );

    drop(compaction);
    thread::sleep(MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES);

    assert!(
        holder.step_until(|_| InstanceStatusRead::read_from(directory.path())
            .status()
            .is_some_and(|status| !reports_the_compaction(status))),
        "the shutdown window went on reporting a write that had finished"
    );
}

/// Whether the compaction the shutdown is waiting for is among the writes
/// `status` names.
fn reports_the_compaction(status: &InstanceStatus) -> bool {
    status
        .pending_writes
        .iter()
        .any(|write| write.label == "Compacting the TEC archive")
}

/// A close frame runs in well under this even on a loaded CI machine, while
/// joining the held-open history worker on it would never return.
const CLOSE_FRAME_BUDGET: StdDuration = StdDuration::from_secs(5);

/// The history worker ends on a thread of its own: the close frame returns
/// while the worker is still on its loop, and the window closes once the
/// worker's thread ends.
///
/// The test holds that worker's request channel open, so a close frame that
/// joined the worker itself would never return. The app therefore runs on a
/// thread of its own and reports the close frame back over a channel: a
/// receive that times out fails the test within the budget.
#[test]
fn closing_the_window_hands_the_history_worker_to_its_own_thread() {
    let (close_frame_returned, close_frame_report) = mpsc::channel();
    let closing_app = thread::Builder::new()
        .name("shutdown-close-frame".to_owned())
        .spawn(move || {
            let dir = tempfile::tempdir().expect("temp dir");
            let mut harness = Harness::builder()
                .with_wait_for_pending_images(false)
                .build_eframe(test_util::harness::transient_app);
            harness.step();
            let (worker, held_open) = test_util::recordings::spawn_worker_held_open(
                RecordingsHandle::Owner(ui_tests::open_temporary_history_database(
                    &dir.path().join("geotrace.h5"),
                )),
                harness.ctx.clone(),
                harness.state().pending_writes.clone(),
            );
            harness.state_mut().history = worker;

            ui_tests::request_window_close(&mut harness);
            harness.step();
            close_frame_returned.send(()).ok();

            assert!(
                !harness.state().history.available(),
                "the app kept a worker whose drop would join on the GUI thread"
            );
            harness.run_steps(3);
            assert!(
                !ui_tests::closed_the_window(&harness),
                "the window closed while the worker's write was still registered"
            );

            held_open.release();

            assert!(
                harness.step_until(ui_tests::closed_the_window),
                "the window never closed after the worker's thread ended"
            );
        })
        .expect("spawn the thread the app runs on");

    assert!(
        close_frame_report.recv_timeout(CLOSE_FRAME_BUDGET).is_ok(),
        "the close frame waited for the history worker on the GUI thread"
    );
    if let Err(panic) = closing_app.join() {
        panic::resume_unwind(panic);
    }
}

/// A closing app stops requesting snaps: the auto sweep a load armed is
/// dropped once the close began.
#[test]
fn the_auto_snap_sweep_is_paused_once_the_close_began() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    // A scheduler that queues the way an online run does, over a transport
    // that reaches nothing.
    harness.state_mut().snap = crate::app::snap::SnapScheduler::new(
        harness.ctx.clone(),
        gt_fetch::TransportSource::Offline,
        false,
    );
    harness.state_mut().offline = false;
    harness.state_mut().snap_settings.acknowledge_consent();
    harness.state_mut().snap_settings.auto_snap = Some(true);
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    // Armed the way a load or a snap dialog arms it, on the frame the close
    // request arrives.
    harness.state_mut().snap_auto_sweep = true;

    ui_tests::request_window_close(&mut harness);
    harness.run_steps(3);

    assert!(harness.state().snap_settings.auto_snap_active());
    assert!(
        harness.state().snap.activity_for(track).is_none(),
        "a closing app enqueues no snap run"
    );
}

/// A second delete never starts underneath a running one: both would rewrite
/// the same columns.
#[test]
fn environment_auto_pruning_waits_for_a_running_delete() {
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, _store) = ui_tests::app_with_interference_days(&[old]);
    ui_tests::enable_environment_auto_prune(&mut harness, 12);
    let request = harness
        .state()
        .environment_auto_prune_request()
        .expect("the archive holds a day past the age");

    let ctx = harness.ctx.clone();
    harness.state_mut().start_environment_prune(&ctx, request);

    assert!(
        harness.state().environment_auto_prune_request().is_none(),
        "a delete is already running"
    );
}
