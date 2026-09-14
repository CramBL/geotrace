use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use egui_phosphor::regular::GEAR as ICON_GEAR;
use gt_pending_writes::WriteAccess;
use gt_store::{
    HistoryDatabase as _, JamStore, ReadOnlyHistoryDatabase as _, Recordings, RecordingsHandle,
};
use gt_test_utils::{ControlLabel, HarnessInteraction as _, TestHarness};
use rstest::rstest;

use crate::app::App;
use crate::app::archive_recovery::UnavailableArchives;
use crate::app::history_open::{
    AUTO_PRUNE_RECORDINGS_MOST_LINES, AUTO_PRUNE_TITLE, CLEAR_LOCK_BUTTON_LABEL,
};
use crate::app::instance_wait::TakenOverInstance;
use crate::app::settings_ui::SettingsPage;
use crate::app::storage::{DatabasesPending, OPENING_DATABASES};
use crate::app::storage_controls::AUTO_STORE_LABEL;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// The window is painted and takes input from the first frame, with the
/// databases still opening behind it.
#[test]
fn the_window_takes_input_while_the_databases_open_and_adopts_them_when_they_land() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.step();

    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::Opening)
    );
    harness.get_by_label_contains(OPENING_DATABASES);
    harness.get_by_label_contains(ICON_GEAR).click();
    harness.step();
    assert!(
        harness.state().settings_open,
        "the window took a click while the databases were opening"
    );

    ui_tests::land_the_databases(&mut harness, &databases, &store);

    assert_eq!(harness.state().storage_open.databases_pending(), None);
    assert_eq!(
        harness.state().history.path(),
        Some(store.recordings_path().as_path()),
        "the app stores through the database the open landed"
    );
    assert!(
        harness.query_by_label_contains(OPENING_DATABASES).is_none(),
        "the overlay went once the databases landed"
    );
}

/// A file named on the command line waits for the databases: loading it before
/// they land would leave it unstored, with nothing to store it later.
#[test]
fn a_file_named_before_the_databases_land_is_loaded_and_stored_once_they_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let gtd_path = dir.path().join("queued.gtd");
    std::fs::write(&gtd_path, ui_tests::minimal_gtd_bytes()).expect("write the recording");

    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[gtd_path]);
    harness.run_steps(3);

    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the load waits for the databases"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    ui_tests::land_the_databases(&mut harness, &databases, &store);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    let stored = harness
        .step_until_some(|_| {
            let recordings = Recordings::open_or_create(&store.recordings_path()).ok()?;
            let listed = recordings.list_recordings().ok()?;
            (!listed.is_empty()).then_some(listed)
        })
        .expect("the recording was never stored");
    assert_eq!(stored.len(), 1);
}

/// A read-only session stores nothing it opens: the recording is loaded into
/// the window, and the recording history beside it is left as it was.
#[test]
fn a_recording_loaded_in_a_read_only_session_is_not_stored() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let gtd_path = dir.path().join("read-only.gtd");
    std::fs::write(&gtd_path, ui_tests::minimal_gtd_bytes()).expect("write the recording");
    let (mut harness, databases) =
        ui_tests::app_with_the_databases_still_opening_for(&[gtd_path], WriteAccess::ReadOnly);
    harness.run_steps(3);

    ui_tests::land_the_databases(&mut harness, &databases, &store);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    let recordings =
        Recordings::open_or_create(&store.recordings_path()).expect("open the recording history");
    assert_eq!(
        recordings.list_recordings().expect("list").len(),
        0,
        "the read-only session stored the recording it loaded"
    );
    assert_eq!(
        harness.state().pending_writes.snapshot().recently_finished,
        Vec::<String>::new(),
        "the read-only session registered a write"
    );
}

/// A drop that arrives before the databases waits for them the same way a
/// command-line path does.
#[test]
fn a_file_dropped_before_the_databases_land_loads_once_they_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);

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
        "the drop waits for the databases"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    ui_tests::land_the_databases(&mut harness, &databases, &store);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
}

/// Pasted log text waits for the databases like any other load: a load that
/// started before them would trip the invariant adoption asserts.
#[test]
fn log_text_pasted_before_the_databases_land_loads_once_they_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);

    harness.input_mut().events.push(egui::Event::Paste(
        "2026-01-01 14:02:11 navsyncd: queue empty\n".to_owned(),
    ));
    harness.run_steps(3);
    assert!(
        harness.state().logs.is_empty(),
        "the paste loaded before the databases landed"
    );

    ui_tests::land_the_databases(&mut harness, &databases, &store);

    test_util::harness::step_until_a_log_is_loaded(&mut harness);
}

/// A storage open that ends without reporting leaves the run storing nothing,
/// and says so.
#[test]
fn a_storage_open_that_never_reports_still_runs_the_loads_that_waited() {
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);

    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::bytes(
            ui_tests::minimal_gtd_bytes().as_slice(),
            "dropped.gtd",
        )));
    harness.run_steps(3);
    drop(databases);

    test_util::harness::step_until_a_recording_is_loaded(&mut harness);
    assert_eq!(harness.state().storage_open.databases_pending(), None);
}

/// The startup auto-prune acts on the archives, so it runs when they land -
/// at construction there is nothing to delete from.
#[test]
fn the_environment_auto_prune_runs_when_the_archives_land() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let archive = store
        .open_or_create_archive::<JamStore>()
        .expect("the interference archive");
    test_util::day_archive::archive_an_empty_interference_day(&archive, old);

    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    ui_tests::enable_environment_auto_prune(&mut harness, 12);

    ui_tests::land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| !harness.state().environment_prune_running()),
        "the delete did not finish"
    );
    assert_eq!(archived_days(archive.read()), []);
}

/// A storage that lands after the close began installs nothing: the worker
/// shutdown already ended is not replaced by a live one, which would keep the
/// database open past the close.
#[test]
fn a_storage_landing_after_the_close_began_installs_no_worker() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.step();

    harness.state_mut().begin_shutdown();
    ui_tests::land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.state().history.path().is_none(),
        "the close left the app with no worker"
    );
    assert!(
        harness.state().loader.db_path.is_none(),
        "nothing is stored while the app closes"
    );
    assert!(
        harness.step_until(|harness| harness.state().shutdown.close_allowed()),
        "the app did not close"
    );
}

fn archived_days(store: &gt_store::ReadOnlyJamStore) -> Vec<chrono::NaiveDate> {
    store
        .days()
        .expect("read the archive index")
        .into_iter()
        .map(|stored| stored.day)
        .collect()
}

/// A day older than any age the control offers stays archived while
/// auto-pruning is off, which is how a fresh install runs.
#[test]
fn environment_auto_pruning_is_off_until_it_is_ticked() {
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, store) = ui_tests::app_with_interference_days(&[old]);

    assert!(harness.state().environment_auto_prune_request().is_none());

    harness.state_mut().auto_prune_environment_days();
    harness.run_steps(3);
    assert_eq!(archived_days(store.read()), [old]);
}

/// With nothing loaded the archives lose every day past the configured age
/// and keep the ones inside it.
#[test]
fn environment_auto_pruning_deletes_the_days_past_the_configured_age() {
    let today = chrono::Utc::now().date_naive();
    let recent = today - chrono::TimeDelta::days(2);
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, store) = ui_tests::app_with_interference_days(&[old, recent]);
    ui_tests::enable_environment_auto_prune(&mut harness, 12);

    harness.state_mut().auto_prune_environment_days();
    assert!(
        harness.step_until(|harness| !harness.state().environment_prune_running()),
        "the delete did not finish"
    );

    assert_eq!(archived_days(store.read()), [recent]);
}

/// A day a loaded recording needs survives however old it is: the schedulers
/// would fetch it again as soon as it went.
#[test]
fn environment_auto_pruning_keeps_the_days_the_loaded_recording_needs() {
    let recorded = ui_tests::base_time().date_naive();
    let before_the_recording = recorded - chrono::TimeDelta::days(1);
    let (mut harness, _dir, store) =
        ui_tests::app_with_interference_days(&[before_the_recording, recorded]);
    ui_tests::enable_environment_auto_prune(&mut harness, 1);

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(ui_tests::minimal_gtd_bytes().as_slice(), "ride.gtd"),
    );
    assert!(
        harness.step_until(|harness| !harness.state().environment_prune_running()),
        "the delete did not finish"
    );

    assert_eq!(
        archived_days(store.read()),
        [recorded],
        "the recording's own day is older than the configured age and stays"
    );
}

/// No delete starts once shutdown has begun: the archives keep their days and
/// the process has no rewrite to wait for.
#[test]
fn environment_pruning_does_not_start_during_shutdown() {
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, store) = ui_tests::app_with_interference_days(&[old]);
    ui_tests::enable_environment_auto_prune(&mut harness, 12);
    harness.state().pending_writes.begin_shutdown();

    harness.state_mut().auto_prune_environment_days();
    harness.run_steps(3);

    assert!(!harness.state().environment_prune_running());
    assert_eq!(archived_days(store.read()), [old]);
}

#[derive(Clone, Copy)]
struct WindowTitle<'a>(&'a str);

const SETTINGS_WINDOW: WindowTitle<'static> = WindowTitle("Settings");
const HISTORY_WINDOW: WindowTitle<'static> = WindowTitle("History");

/// Runs frames until the window titled `title` shows the control labelled
/// `label`, settles the pointer on it and clicks it, then runs the frames the
/// click's effect needs to reach the app state.
///
/// The wait searches the window's own subtree because both windows draw the
/// same storage controls, and
/// [`gt_test_utils::HarnessInteraction::step_until`] tests its predicate on
/// the accessibility tree of the previous frame. A search of the whole tree
/// also matches the control in the window that the test closed one frame
/// earlier, and ends the wait before the window named here has drawn.
fn click_the_control_once_the_window_shows_it(
    harness: &mut Harness<'_, App>,
    WindowTitle(title): WindowTitle<'_>,
    ControlLabel(label): ControlLabel<'_>,
) {
    assert!(
        harness.step_until(|h| {
            h.query_by_role_and_label(egui::accesskit::Role::Window, title)
                .and_then(|window| window.query_by_label(label))
                .is_some()
        }),
        "the {title} window shows the control labelled {label:?}"
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .get_by_label(label)
        .hover();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .get_by_label(label)
        .click();
    harness.run_steps(3);
}

/// The storage controls appear in the History window and on the settings
/// window's Application page, both driving the one setting: what one window
/// writes, the other reads.
#[test]
fn storage_controls_drive_one_setting_from_both_windows() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .reopen_history_database(&dir.path().join("recordings.h5"), &ctx);

    // The Application page turns auto-pruning on.
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    click_the_control_once_the_window_shows_it(
        &mut harness,
        SETTINGS_WINDOW,
        ControlLabel("Auto-prune when over"),
    );
    assert!(
        harness.state().storage_settings.auto_prune_enabled,
        "the Application page's auto-prune switch writes the setting"
    );

    // Clicking the History window's confirmation toggle proves it reads the
    // Application page's write and writes the same setting back: the toggle
    // only takes a click while auto-pruning is on.
    harness.state_mut().settings_open = false;
    harness.state_mut().history_window.open = true;
    click_the_control_once_the_window_shows_it(
        &mut harness,
        HISTORY_WINDOW,
        ControlLabel("Confirm before pruning"),
    );
    assert!(
        !harness.state().storage_settings.auto_prune_confirm,
        "the History window's confirmation toggle writes the setting"
    );

    // Auto-storing off in the History window empties the loader's database
    // path, the same live effect the Application page has.
    click_the_control_once_the_window_shows_it(
        &mut harness,
        HISTORY_WINDOW,
        ControlLabel(AUTO_STORE_LABEL),
    );
    assert!(
        !harness.state().storage_settings.enabled,
        "the History window's auto-store checkbox writes the setting"
    );
    assert_eq!(harness.state().loader.db_path, None);

    // The Application page reads the History window's write: its auto-store
    // checkbox turns storing back on, and the loader's path returns.
    harness.state_mut().history_window.open = false;
    harness.state_mut().settings_open = true;
    click_the_control_once_the_window_shows_it(
        &mut harness,
        SETTINGS_WINDOW,
        ControlLabel(AUTO_STORE_LABEL),
    );
    assert!(
        harness.state().storage_settings.enabled,
        "the Application page's auto-store checkbox writes the setting"
    );
    assert!(harness.state().loader.db_path.is_some());
}

#[test]
fn snapshot_history_locked_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(crate::app::storage::HistoryFailure::Locked(
        PathBuf::from("geotrace.h5"),
    ));
    harness.run();
    harness.snapshot_with_color_tolerance("history_locked_dialog");
}

#[test]
fn snapshot_history_corrupt_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(
        crate::app::storage::HistoryFailure::Unreadable(PathBuf::from("geotrace.h5")),
    );
    harness.run();
    harness.snapshot_with_color_tolerance("history_corrupt_dialog");
}

/// Startup hands the app the databases a completed open produced. The worker
/// it carries replaces the one the app was holding, and the loader takes the
/// path that worker stores under.
#[test]
fn adopting_an_open_storage_installs_its_history_worker() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let recordings_path = store.recordings_path();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    assert!(
        harness.state().history.path().is_none(),
        "the harness starts with storage disabled"
    );

    let opened = crate::app::storage::OpenStorage {
        history: crate::app::history_db::HistoryWorker::spawn(
            RecordingsHandle::Owner(store.open_recordings().expect("recordings")),
            harness.ctx.clone(),
            gt_pending_writes::PendingWrites::default(),
        ),
        history_failure: None,
        archive: None,
        geomagnetic_indices: None,
        tec_maps: None,
        solar_flares: None,
        unavailable_archives: UnavailableArchives::default(),
    };
    harness.state_mut().adopt_open_storage(opened);

    assert_eq!(
        harness.state().history.path(),
        Some(recordings_path.as_path()),
        "the adopted worker is the one the app now stores through"
    );
    assert_eq!(
        harness.state().loader.db_path.as_deref(),
        Some(recordings_path.as_path()),
        "the loader stores into the adopted database"
    );
}

/// Adopting a storage-open failure has to raise its prompt itself: the open
/// reports the failure, not the app.
#[test]
fn a_history_failure_in_the_adopted_storage_raises_its_prompt() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();

    harness
        .state_mut()
        .adopt_open_storage(crate::app::storage::OpenStorage {
            history: crate::app::history_db::HistoryWorker::disabled(),
            history_failure: Some(crate::app::storage::HistoryFailure::Busy(PathBuf::from(
                "recordings.h5",
            ))),
            archive: None,
            geomagnetic_indices: None,
            tec_maps: None,
            solar_flares: None,
            unavailable_archives: UnavailableArchives::default(),
        });
    harness.step();

    assert!(
        harness
            .query_by_label_contains("Another process has the recording history database open")
            .is_some(),
        "the busy prompt is up"
    );
}

/// "Try again" on a database that still will not open puts the prompt back.
/// Uses an unreadable file, since holding a real lock needs a second process.
#[test]
fn a_failed_retry_restores_the_prompt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("recordings.h5");
    std::fs::write(&path, b"not a database").expect("write");

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().history_failure =
        Some(crate::app::storage::HistoryFailure::Busy(path.clone()));

    let ctx = harness.ctx.clone();
    harness.state_mut().reopen_history_database(&path, &ctx);

    assert_eq!(
        harness.state().history_failure,
        Some(crate::app::storage::HistoryFailure::Unreadable(path)),
        "the retry reclassifies instead of clearing the prompt"
    );
    assert!(harness.state().history.path().is_none());
}

/// A shutdown that has begun rejects all three recovery paths: each writes to
/// the recordings database. `recreate_history_database` renames the file before
/// it reopens it, and a quit in between would leave the directory without one.
#[rstest::rstest]
#[case::reopen(|app: &mut App, path: &Path, ctx: &egui::Context| {
    app.reopen_history_database(path, ctx);
})]
#[case::recover(|app: &mut App, path: &Path, ctx: &egui::Context| {
    app.recover_history_database(path, ctx);
})]
#[case::recreate(|app: &mut App, path: &Path, ctx: &egui::Context| {
    app.recreate_history_database(path, true, ctx);
})]
fn a_history_database_recovery_is_rejected_once_shutdown_has_begun(
    #[case] recover: fn(&mut App, &Path, &egui::Context),
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let path = store.recordings_path();
    drop(
        store
            .open_recordings()
            .expect("create the recordings database"),
    );
    let files_in_the_data_directory = || {
        let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(dir.path())
            .expect("read the data directory")
            .filter_map(|entry| Some(entry.ok()?.file_name()))
            .collect();
        names.sort();
        names
    };
    let before = files_in_the_data_directory();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state().pending_writes.begin_shutdown();
    let ctx = harness.ctx.clone();

    recover(harness.state_mut(), &path, &ctx);

    assert!(
        harness.state().history.path().is_none(),
        "the rejected recovery opened the recordings database"
    );
    assert_eq!(
        files_in_the_data_directory(),
        before,
        "the rejected recovery renamed, removed or created a file"
    );
}

/// An app that took write access from another instance, with `failure` set as
/// the recordings database's open would have set it.
fn app_after_a_take_over_with<'a>(
    failure: crate::app::storage::HistoryFailure,
) -> Harness<'a, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().instance_taken_over_from = Some(TakenOverInstance {
        process_id: Some(4210),
    });
    harness.state_mut().history_failure = Some(failure);
    harness.run_steps(2);
    harness
}

/// The busy prompt states the GeoTrace the user took write access from: they
/// chose to keep it running.
#[test]
fn the_busy_prompt_after_a_take_over_names_the_instance_that_still_has_the_database() {
    let harness = app_after_a_take_over_with(crate::app::storage::HistoryFailure::Busy(
        PathBuf::from("recordings.h5"),
    ));

    harness.get_by_label_contains(
        "Another GeoTrace (process 4210) still has the recording history database open",
    );
    harness.get_by_label_contains("not stored until it exits");
    assert!(
        harness
            .query_by_label_contains("Close it and try again")
            .is_none(),
        "the prompt asks for the GeoTrace the user chose to keep running to be closed"
    );
    harness.get_by_label("Try again");
}

/// The lock clear is grayed after a take-over: clearing it while the other
/// GeoTrace writes can corrupt the database.
#[test]
fn the_locked_prompt_after_a_take_over_grays_the_lock_clear() {
    let mut harness = app_after_a_take_over_with(crate::app::storage::HistoryFailure::Locked(
        PathBuf::from("recordings.h5"),
    ));

    let clear = harness.get_by_label_contains(CLEAR_LOCK_BUTTON_LABEL);
    assert!(
        clear.accesskit_node().is_disabled(),
        "the clear is live while another GeoTrace has the database open"
    );
    let center = clear.rect().center();
    harness.hover_at_and_settle(center, 5);
    harness.get_by_label_contains(
        "Another GeoTrace (process 4210) still has the recording history database open",
    );
}

/// A database held by another instance is a wait, not a repair, so this
/// prompt offers neither the lock clear nor the recreate.
#[test]
fn snapshot_history_busy_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(crate::app::storage::HistoryFailure::Busy(
        PathBuf::from("geotrace.h5"),
    ));
    harness.run();
    harness.snapshot_with_color_tolerance("history_busy_dialog");
}

/// The button that dismisses a history database prompt.
const CANCEL_LABEL: &str = "Cancel";

/// A re-segment prompt for the recording named `filename`, stored with a split
/// rule and a placement rule that both differ from the current ones.
fn resegment_prompt_named(filename: &str) -> crate::app::ResegmentPrompt {
    crate::app::ResegmentPrompt {
        db_ref: gt_store::DatabaseRef {
            identity: format!("auto:{filename}"),
            group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
        },
        filename: filename.to_owned(),
        bytes: std::sync::Arc::from(Vec::<u8>::new()),
        stored: gt_store::StoredSegmentation {
            track_split_gap_us: 60_000_000,
            track_split_rule: gt_store::StoredTrackSplitRule::ForwardGapOnly,
            fix_placement_rule: gt_store::StoredFixPlacementRule::MissingHeading,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 4.0,
        },
        stored_tracks: Vec::new(),
        marker_settings_changed: false,
        placement: crate::app::loader::LoadedRecordingPlacement::AddAnEntry,
    }
}

fn app_showing_the_resegment_prompt(filename: &str) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_resegment = Some(resegment_prompt_named(filename));
    harness.run();
    harness
}

/// The marks the user set by hand on the recording of a re-segment prompt.
#[derive(Clone, Copy)]
struct MarksOnThePromptedRecording {
    shelved_tracks: usize,
    hidden_tracks: usize,
}

/// Nav points per track of the stored track table the prompt is built with.
const STORED_TRACK_NAV_POINTS: u64 = 100;

/// A stored track table of `shelved_tracks` shelved tracks and one live one.
fn stored_track_table_with_shelved_tracks(shelved_tracks: usize) -> Vec<gt_store::TrackRange> {
    let mut table = Vec::new();
    let mut start = 0;
    for state in std::iter::repeat_n(gt_store::TrackState::Shelved, shelved_tracks)
        .chain(std::iter::once(gt_store::TrackState::Live))
    {
        table.push(gt_store::TrackRange {
            start,
            end: start + STORED_TRACK_NAV_POINTS,
            state,
        });
        start += STORED_TRACK_NAV_POINTS;
    }
    table
}

fn app_showing_the_resegment_prompt_with(
    marks: MarksOnThePromptedRecording,
) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    let mut prompt = resegment_prompt_named("ride.gtd");
    prompt.stored_tracks = stored_track_table_with_shelved_tracks(marks.shelved_tracks);
    harness
        .inner
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_hidden_tracks_of_recording(prompt.db_ref.clone(), (1..=marks.hidden_tracks).collect());
    harness.inner.state_mut().pending_resegment = Some(prompt);
    harness.run();
    harness
}

/// What every warning about the marks a recalculation drops opens with.
const RECALCULATION_WARNING_OPENING: &str = "Recalculating ";

#[rstest]
#[case::shelved_tracks(
    MarksOnThePromptedRecording {
        shelved_tracks: 2,
        hidden_tracks: 0,
    },
    Some("Recalculating puts the 2 shelved tracks of this recording back in the working set.")
)]
#[case::hidden_tracks(
    MarksOnThePromptedRecording {
        shelved_tracks: 0,
        hidden_tracks: 1,
    },
    Some("Recalculating shows the 1 hidden track of this recording.")
)]
#[case::shelved_and_hidden_tracks(
    MarksOnThePromptedRecording {
        shelved_tracks: 1,
        hidden_tracks: 3,
    },
    Some(
        "Recalculating puts the 1 shelved track of this recording back in the working set and \
         shows its 3 hidden tracks."
    )
)]
#[case::neither(
    MarksOnThePromptedRecording {
        shelved_tracks: 0,
        hidden_tracks: 0,
    },
    None
)]
fn the_resegment_prompt_states_the_marks_a_recalculation_drops(
    #[case] marks: MarksOnThePromptedRecording,
    #[case] expected_warning: Option<&str>,
) {
    let harness = app_showing_the_resegment_prompt_with(marks);
    match expected_warning {
        Some(warning) => {
            harness.inner.get_by_label(warning);
        }
        None => assert!(
            harness
                .inner
                .query_by_label_contains(RECALCULATION_WARNING_OPENING)
                .is_none(),
            "the re-segment prompt warns about marks on a recording that has none"
        ),
    }
}

#[test]
fn snapshot_history_resegment_dialog_with_shelved_and_hidden_tracks() {
    let mut harness = app_showing_the_resegment_prompt_with(MarksOnThePromptedRecording {
        shelved_tracks: 1,
        hidden_tracks: 3,
    });
    harness
        .snapshot_with_color_tolerance("history_resegment_dialog_with_shelved_and_hidden_tracks");
}

/// A recording name of words, long enough for the prompt's intro to run past
/// the room it caps at.
fn recording_name_past_the_capped_room() -> String {
    format!("{}.gtd", ["ride"; 60].join(" "))
}

#[test]
fn snapshot_history_resegment_dialog() {
    let mut harness = app_showing_the_resegment_prompt("ride.gtd");
    harness.snapshot_with_color_tolerance("history_resegment_dialog");
}

#[test]
fn snapshot_history_resegment_dialog_past_the_capped_room() {
    let mut harness = app_showing_the_resegment_prompt(&recording_name_past_the_capped_room());
    harness.snapshot_with_color_tolerance("history_resegment_dialog_past_the_capped_room");
}

/// Opening a second recording from history replaces the prompt with one
/// stating that recording.
#[test]
fn the_resegment_prompt_keeps_its_buttons_in_place_while_a_longer_name_arrives() {
    let mut harness = app_showing_the_resegment_prompt("ride.gtd");
    let before = harness.inner.get_by_label(CANCEL_LABEL).rect();

    harness.inner.state_mut().pending_resegment = Some(resegment_prompt_named(
        &recording_name_past_the_capped_room(),
    ));
    harness.inner.run_steps(4);

    assert_eq!(
        harness.inner.get_by_label(CANCEL_LABEL).rect(),
        before,
        "the Cancel button of the re-segment prompt moved: a press where the user aimed misses it"
    );
}

/// The recordings a prune under the storage limit deletes, each named by its
/// identity and the group it is stored under.
fn auto_prune_candidates(count: usize) -> Vec<gt_store::DatabaseRef> {
    (0..count)
        .map(|index| gt_store::DatabaseRef {
            identity: format!("auto:ride-{index}.gtd"),
            group_name: format!("2025-05-2{index}T10:00:00Z_a1b2"),
        })
        .collect()
}

fn app_showing_the_auto_prune_confirmation(count: usize) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_auto_prune = Some(auto_prune_candidates(count));
    harness.run();
    harness
}

/// Candidates enough to fill the room the list caps at
/// [`AUTO_PRUNE_RECORDINGS_MOST_LINES`].
const AUTO_PRUNE_CANDIDATES_PAST_THE_CAPPED_ROOM: usize = 12;

const AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM: usize = 40;

#[test]
fn snapshot_auto_prune_dialog() {
    let mut harness = app_showing_the_auto_prune_confirmation(3);
    harness.snapshot_with_color_tolerance("auto_prune_dialog");
}

#[test]
fn snapshot_auto_prune_dialog_past_the_capped_room() {
    let mut harness =
        app_showing_the_auto_prune_confirmation(AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM);
    harness.snapshot_with_color_tolerance("auto_prune_dialog_past_the_capped_room");
}

#[test]
fn the_auto_prune_confirmation_opens_at_one_height_for_every_list_past_the_capped_room() {
    let past = app_showing_the_auto_prune_confirmation(AUTO_PRUNE_CANDIDATES_PAST_THE_CAPPED_ROOM);
    let far_past =
        app_showing_the_auto_prune_confirmation(AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM);

    assert_eq!(
        far_past
            .inner
            .window_rect(AUTO_PRUNE_TITLE)
            .expect("the auto-prune confirmation is shown")
            .size(),
        past.inner
            .window_rect(AUTO_PRUNE_TITLE)
            .expect("the auto-prune confirmation is shown")
            .size(),
        "{AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM} candidates made the auto-prune \
         confirmation taller than {AUTO_PRUNE_CANDIDATES_PAST_THE_CAPPED_ROOM} did: a list past \
         the room it caps at {AUTO_PRUNE_RECORDINGS_MOST_LINES} lines has to scroll inside that \
         room"
    );
}

/// A recording stored while the confirmation is open runs the auto-prune check
/// again, and the candidates it comes back with replace the list.
#[test]
fn the_auto_prune_confirmation_keeps_its_buttons_in_place_while_more_candidates_arrive() {
    let mut harness = app_showing_the_auto_prune_confirmation(3);
    let before = harness.inner.get_by_label(CANCEL_LABEL).rect();

    harness.inner.state_mut().pending_auto_prune = Some(auto_prune_candidates(
        AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM,
    ));
    harness.inner.run_steps(4);

    assert_eq!(
        harness.inner.get_by_label(CANCEL_LABEL).rect(),
        before,
        "the Cancel button of the auto-prune confirmation moved: a press where the user aimed \
         misses it"
    );
}
