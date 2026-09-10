use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use egui_phosphor::regular::GEAR as ICON_GEAR;
use gt_pending_writes::WriteAccess;
use gt_store::{HistoryDatabase as _, JamStore, ReadOnlyHistoryDatabase as _, Recordings};
use gt_test_utils::HarnessInteraction as _;

use crate::app::storage::{DatabasesPending, OPENING_DATABASES};
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
