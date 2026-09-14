use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;

use egui_kittest::Harness;
use gt_instance_lock::DataDirectoryLock;
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_test_utils::HarnessInteraction as _;

use crate::app::App;
use crate::app::archive_recovery::UnavailableArchives;
use crate::app::storage::OpenStorage;
use crate::app::test_util;
use crate::app::ui_tests;

/// [`ui_tests::app_with_the_databases_still_opening`] for a session that reads and
/// writes the settings file at `config_path`.
fn app_with_the_databases_still_opening_reading<'a>(
    config_path: PathBuf,
    write_access: WriteAccess,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    ui_tests::app_with_the_databases_still_opening_built_by(move |cc| {
        test_util::harness::transient_app_with_the_settings_file(
            cc,
            &[],
            Some(config_path.clone()),
            DataDirectoryLock::marking_nothing(),
            PendingWrites::new(write_access),
        )
    })
}

/// Lands the databases of a run that opened no recording history, as a run
/// whose database another process holds does.
fn land_the_databases_without_a_recording_history(
    harness: &mut Harness<'_, App>,
    databases: &mpsc::Sender<OpenStorage>,
) {
    let opened = OpenStorage {
        history: crate::app::history_db::HistoryWorker::disabled(),
        history_failure: None,
        archive: None,
        geomagnetic_indices: None,
        tec_maps: None,
        solar_flares: None,
        unavailable_archives: UnavailableArchives::default(),
    };
    databases.send(opened).expect("the app holds the receiver");
    harness.step();
}

/// The app before its databases land, reading a settings file that lists the
/// second track of the stored recording `db_ref` as hidden.
struct AppReadingHiddenTracksFromTheSettingsFile {
    harness: Harness<'static, App>,
    dir: tempfile::TempDir,
    store: gt_store::Store,
    db_ref: gt_store::DatabaseRef,
    databases: mpsc::Sender<OpenStorage>,
}

/// One recording of two live tracks in the history database under `store`.
fn seed_a_two_track_recording(store: &gt_store::Store) -> gt_store::DatabaseRef {
    let settings = crate::app::loader::stored_segmentation_from_config(
        &gt_track_builder::SegmentationConfig::default(),
    );
    ui_tests::insert_recording_into(
        store,
        &ui_tests::two_track_gtd_bytes(),
        &ui_tests::two_live_track_ranges(),
        settings,
    )
}

fn app_reading_hidden_tracks_from_the_settings_file(
    write_access: WriteAccess,
) -> AppReadingHiddenTracksFromTheSettingsFile {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let db_ref = seed_a_two_track_recording(&store);

    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            "[[ui.hidden_tracks]]\nidentity = {:?}\ngroup_name = {:?}\ntrack_numbers = [2]\n",
            db_ref.identity, db_ref.group_name
        ),
    )
    .expect("write the settings file");

    let (mut harness, databases) =
        app_with_the_databases_still_opening_reading(config_path, write_access);
    harness.step();
    AppReadingHiddenTracksFromTheSettingsFile {
        harness,
        dir,
        store,
        db_ref,
        databases,
    }
}

/// The hidden tracks of `db_ref` as this session holds them, and [`None`]
/// where it has not read that recording's UI state.
fn hidden_track_numbers_of(
    harness: &Harness<'_, App>,
    db_ref: &gt_store::DatabaseRef,
) -> Option<BTreeSet<usize>> {
    let state = harness.state();
    let shared = state.shared.borrow();
    shared.tree.hidden_track_numbers(db_ref).cloned()
}

/// The hidden tracks a settings file written before the history database held
/// them are stored with their recording at startup, and the settings file this
/// version writes lists none.
#[test]
fn the_hidden_tracks_the_settings_file_lists_are_stored_with_their_recording() {
    let AppReadingHiddenTracksFromTheSettingsFile {
        mut harness,
        dir,
        store,
        db_ref,
        databases,
    } = app_reading_hidden_tracks_from_the_settings_file(WriteAccess::Owner);
    ui_tests::land_the_databases(&mut harness, &databases, &store);

    harness.state().history.open(db_ref.clone());

    assert!(
        harness.step_until(
            |harness| hidden_track_numbers_of(harness, &db_ref) == Some(BTreeSet::from([2]))
        ),
        "the recording opened without the hidden track the settings file lists"
    );
    harness.state().flush_settings();
    let written = std::fs::read_to_string(dir.path().join("config.toml"))
        .expect("read the settings file back");
    assert!(
        !written.contains("hidden_tracks"),
        "the settings file still lists hidden tracks: {written}"
    );
}

/// A session that opened no recording history keeps the hidden tracks the
/// settings file lists, and stores them with their recording once the user has
/// a database open again.
#[test]
fn the_hidden_tracks_the_settings_file_lists_wait_for_a_recording_history() {
    let AppReadingHiddenTracksFromTheSettingsFile {
        mut harness,
        dir: _dir,
        store,
        db_ref,
        databases,
    } = app_reading_hidden_tracks_from_the_settings_file(WriteAccess::Owner);
    land_the_databases_without_a_recording_history(&mut harness, &databases);

    harness
        .state_mut()
        .install_history_worker(test_util::recordings::worker_on(&store.recordings_path()));
    harness.state().history.open(db_ref.clone());

    assert!(
        harness.step_until(
            |harness| hidden_track_numbers_of(harness, &db_ref) == Some(BTreeSet::from([2]))
        ),
        "the session that opened no recording history dropped the hidden tracks"
    );
}

/// A read-only session stores none of them: the recording keeps the UI state
/// the database holds, and the settings file keeps its list for a session with
/// write access.
#[test]
fn a_read_only_session_stores_none_of_the_hidden_tracks_the_settings_file_lists() {
    let AppReadingHiddenTracksFromTheSettingsFile {
        mut harness,
        dir,
        store,
        db_ref,
        databases,
    } = app_reading_hidden_tracks_from_the_settings_file(WriteAccess::ReadOnly);
    ui_tests::land_the_databases(&mut harness, &databases, &store);

    harness.state().history.open(db_ref.clone());

    assert!(
        harness.step_until(
            |harness| hidden_track_numbers_of(harness, &db_ref) == Some(BTreeSet::new())
        ),
        "the read-only session stored the hidden tracks the settings file lists"
    );
    harness.state().flush_settings();
    let written = std::fs::read_to_string(dir.path().join("config.toml"))
        .expect("read the settings file back");
    assert!(
        written.contains("hidden_tracks"),
        "the read-only session rewrote the settings file: {written}"
    );
}

/// The version report covers the whole session: two recordings a newer version
/// of GeoTrace stored UI state for raise one message between them.
#[test]
fn ui_state_a_newer_version_stored_raises_one_message_for_two_recordings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.step();
    ui_tests::land_the_databases(&mut harness, &databases, &store);

    assert_eq!(
        harness.state().toasts.len(),
        0,
        "the session opened with a message about its UI state"
    );

    for group_name in ["2026-01-01T00:00:00Z_ride", "2026-01-02T00:00:00Z_ride"] {
        harness.state().history.ui_state_versions().report_too_new(
            &gt_store::DatabaseRef {
                identity: "dev".to_owned(),
                group_name: group_name.to_owned(),
            },
            2,
        );
    }
    harness.run_steps(2);

    assert_eq!(
        harness.state().toasts.len(),
        1,
        "each recording raised a message of its own"
    );

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame raised the message again"
    );
}
