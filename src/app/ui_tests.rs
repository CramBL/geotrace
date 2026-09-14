use egui::TextEdit;
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui_kittest::{Harness, Node, kittest::Queryable as _};
use geotrace_sdk::{Channel, DateTime, Duration, Unit, Utc};
use gt_instance_lock::{DataDirectoryLock, DataDirectoryOwnership};
use gt_log_view::LoadedLog;
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::{
    FlareStore, HistoryDatabase as _, IonexStore, JamStore, Recordings, RecordingsHandle,
    SolarStore,
};
use gt_test_utils::{By, HarnessInteraction as _, SyntheticGtdSpec, TestHarness};

use super::App;
use super::archive_recovery::UnavailableArchives;
use super::storage::OpenStorage;
use crate::app::log_viewer::filters;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;

mod about_dialog;
mod app_snapshots;
mod archive_recovery;
mod detached_panel;
mod instance_wait;
mod loading;
mod log_association;
mod log_viewer;
mod plot;
mod query_editor;
mod query_results;
mod recording_from_disk;
mod recording_names;
mod recording_ui_state;
mod settings_window;
mod shutdown;
mod snap;
mod storage;
#[cfg(feature = "self-update")]
mod update_prompt;
mod window_fit;

/// Fails listing every captured tile the map requested while drawing the frame
/// about to be snapshotted and did not get, so no fixture-backed snapshot is
/// recorded over blank ground. The record starts fresh and one more frame is
/// drawn, so the check covers only that frame.
fn assert_the_capture_covers_the_map(harness: &mut TestHarness<'_, App>, snapshot_name: &str) {
    harness
        .inner
        .state_mut()
        .map
        .forget_missing_captured_tiles();
    harness.inner.step();
    let missing = harness
        .inner
        .state()
        .map
        .missing_captured_tiles()
        .expect("the map draws the captured tiles");
    gt_test_utils::assert_map_tile_capture_is_complete(snapshot_name, missing);
}

/// The second node labelled `label`, in render order. The side panel renders
/// first and its Visible section lists every recording drawn on the map: the
/// first node is the panel's, the second the surface under test.
fn node_outside_the_side_panel<'h>(harness: &'h Harness<'_, App>, label: &'h str) -> Node<'h> {
    harness.nth_matching(By::new().label(label), 1)
}

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is valid")
}

fn minimal_gtd_bytes() -> Vec<u8> {
    gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: base_time(),
        point_count: 61,
        step_secs: 1,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 7,
    })
}

fn load_three_overlapping_files(harness: &mut Harness<App>) {
    let t0 = base_time();
    let overlapping_files = [
        (
            "overlap_a.gtd",
            gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
                start: t0,
                point_count: 240,
                step_secs: 1,
                start_lat_deg: 55.0000,
                start_lon_deg: 12.0000,
                lat_step_deg: 0.00005,
                lon_step_deg: 0.00008,
                heading_deg: 20.0,
                speed_kmh: 28.0,
                eph_m: 1.8,
                sats_seen: 14,
                sats_in_fix: 11,
            }),
        ),
        (
            "overlap_b.gtd",
            gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
                start: t0,
                point_count: 240,
                step_secs: 1,
                start_lat_deg: 55.0003,
                start_lon_deg: 12.0002,
                lat_step_deg: 0.00006,
                lon_step_deg: 0.00007,
                heading_deg: 32.0,
                speed_kmh: 31.0,
                eph_m: 2.1,
                sats_seen: 13,
                sats_in_fix: 10,
            }),
        ),
        (
            "overlap_c.gtd",
            gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
                start: t0,
                point_count: 240,
                step_secs: 1,
                start_lat_deg: 54.9998,
                start_lon_deg: 11.9997,
                lat_step_deg: 0.00004,
                lon_step_deg: 0.00009,
                heading_deg: 14.0,
                speed_kmh: 26.0,
                eph_m: 2.6,
                sats_seen: 12,
                sats_in_fix: 9,
            }),
        ),
    ];

    for (name, bytes) in overlapping_files {
        test_util::harness::drop_file_and_wait_for_load(
            harness,
            TestDroppedFile::bytes(bytes, name),
        );
    }
}

/// A demo-trip query with more matches than the matches table lists at once,
/// each of them holding rows for the points table below it.
const MANY_MATCH_QUERY: &str = "points | where accel < -0.2 m/s2";

/// The query window's title, which both its area and its accesskit node are
/// addressed by.
const QUERY_WINDOW_TITLE: &str = "Query";

/// The query window's button moving the matches list into a window of its own.
/// The side panel offers the same icon, so this looks only in the query window.
fn pop_out_button<'h>(harness: &'h Harness<'_, App>) -> egui_kittest::Node<'h> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, QUERY_WINDOW_TITLE)
        .get_by_label(ICON_ARROW_SQUARE_OUT)
}

fn press_escape<State>(harness: &mut Harness<'_, State>) {
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}

/// Builds a harness with three overlapping files loaded and the plot settled.
fn harness_with_three_files_loaded() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    load_three_overlapping_files(&mut harness);
    harness.run_steps(20);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 3);
    harness
}

/// Build an app with one loaded file and the query window open. Shared setup
/// for the interactive query-history tests.
fn app_with_a_recording() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(minimal_gtd_bytes(), "test.gtd"),
    );
    harness
}

/// [`app_with_a_recording`] with the query window open over it.
fn app_with_query_window_open() -> Harness<'static, App> {
    let mut harness = app_with_a_recording();
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    harness
}

/// Run a query through the Run button and wait for its result.
fn run_query(harness: &mut Harness<App>, text: &str) {
    harness.state_mut().query_window.set_text(text.to_owned());
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(harness);
    harness.run_steps(3);
}

/// A key-press event for `key` with no modifiers.
fn key_press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The fixture stretches whose `accel` x-component exceeds 1 g, shared by the
/// value generation and the expected-match assertion so they cannot drift.
const ACCEL_HIGH_RANGES: [std::ops::Range<usize>; 2] = [60..120, 180..200];

/// Synthetic `.gtd` bytes whose track carries an aligned 3-component `accel`
/// channel in g, one sample per nav fix. The [`ACCEL_HIGH_RANGES`] stretches
/// exceed 1 g on x, so an `@accel.x` filter has multi-sample matches to table
/// on the window and halo on the map.
fn accel_channel_gtd_bytes(speed_kmh: f64) -> Vec<u8> {
    let spec = SyntheticGtdSpec {
        start: base_time(),
        point_count: 240,
        step_secs: 1,
        start_lat_deg: 55.0,
        start_lon_deg: 12.0,
        lat_step_deg: 0.00005,
        lon_step_deg: 0.00008,
        heading_deg: 20.0,
        speed_kmh,
        eph_m: 1.8,
        sats_seen: 14,
        sats_in_fix: 11,
    };
    let mut times = Vec::with_capacity(spec.point_count);
    let mut values = Vec::with_capacity(spec.point_count * 3);
    for i in 0..spec.point_count {
        times.push(spec.start + Duration::seconds(i as i64));
        let x = if ACCEL_HIGH_RANGES.iter().any(|r| r.contains(&i)) {
            1.5
        } else {
            0.2
        };
        values.extend([x, 0.1, 0.98]);
    }
    let channel = Channel::builder()
        .name("accel")
        .unit(Unit::G)
        .description("IMU acceleration")
        .components(["x", "y", "z"])
        .times(times)
        .values(values)
        .build()
        .expect("fixture channel is valid");
    gt_test_utils::synthetic_gtd_bytes_with_channels(spec, vec![channel])
}

/// Focus the query editor and drop the caret at the end of `text`, so the
/// caret-driven autocomplete and hover paths run in a snapshot.
fn focus_query_editor_at_end(harness: &Harness<App>, text: &str) {
    let editor_id = egui::Id::new(super::query::EDITOR_ID_SALT);
    harness.ctx.memory_mut(|m| m.request_focus(editor_id));
    let mut state = TextEdit::load_state(&harness.ctx, editor_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(text.chars().count()),
        )));
    TextEdit::store_state(&harness.ctx, editor_id, state);
}

/// The app as it starts with its databases still opening, and the sender the
/// test lands them through. `paths` are the ones a command line named.
///
/// The open is taken over before the harness's first frame, so nothing is
/// adopted until the test says so.
fn app_with_the_databases_still_opening<'a>(
    paths: &[PathBuf],
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    app_with_the_databases_still_opening_for(paths, WriteAccess::Owner)
}

/// [`app_with_the_databases_still_opening`] for a session with `write_access`,
/// which controls whether anything it loads is stored.
fn app_with_the_databases_still_opening_for<'a>(
    paths: &[PathBuf],
    write_access: WriteAccess,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    let paths = paths.to_vec();
    app_with_the_databases_still_opening_built_by(move |cc| {
        test_util::harness::transient_app_with_the_instance_lock(
            cc,
            &paths,
            DataDirectoryLock::marking_nothing(),
            PendingWrites::new(write_access),
        )
    })
}

/// The app `build` returns, holding back the databases it opens: the returned
/// sender lands them, as [`land_the_databases`] does.
fn app_with_the_databases_still_opening_built_by<'a, F>(
    build: F,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>)
where
    F: FnOnce(&eframe::CreationContext<'_>) -> App + 'a,
{
    let (sender_tx, sender_rx) = mpsc::channel();
    let harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| {
            let mut app = build(cc);
            sender_tx.send(app.storage_open.take_over_for_test()).ok();
            app
        });
    let databases = sender_rx.recv().expect("the app was built");
    (harness, databases)
}

/// Every database under `store`, as a finished open hands them over.
fn storage_opened_in(
    store: &gt_store::Store,
    ctx: &egui::Context,
    pending_writes: &gt_pending_writes::PendingWrites,
) -> OpenStorage {
    OpenStorage {
        history: crate::app::history_db::HistoryWorker::spawn(
            RecordingsHandle::Owner(
                store
                    .open_recordings()
                    .expect("open the recordings database"),
            ),
            ctx.clone(),
            pending_writes.clone(),
        ),
        history_failure: None,
        archive: store.open_or_create_archive::<JamStore>().ok(),
        geomagnetic_indices: store.open_or_create_archive::<SolarStore>().ok(),
        tec_maps: store.open_or_create_archive::<IonexStore>().ok(),
        solar_flares: store.open_or_create_archive::<FlareStore>().ok(),
        unavailable_archives: UnavailableArchives::default(),
    }
}

/// Lands `store` behind the app, as the open thread does when it finishes.
fn land_the_databases(
    harness: &mut Harness<'_, App>,
    databases: &mpsc::Sender<OpenStorage>,
    store: &gt_store::Store,
) {
    let pending_writes = harness.state().pending_writes.clone();
    let opened = storage_opened_in(store, &harness.ctx, &pending_writes);
    databases.send(opened).expect("the app holds the receiver");
    harness.step();
}

/// The app started on `data_directory`, which the caller's own
/// [`DataDirectoryLock`] holds, with the files a command line named.
///
/// The app takes its own lock on that very directory, which is rejected for as
/// long as the caller keeps its lock - the same rejection a second GeoTrace
/// gets from the first.
fn lock_on_a_directory_another_instance_holds(data_directory: &Path) -> DataDirectoryLock {
    let instance_lock = DataDirectoryLock::acquire(Some(data_directory));
    assert_eq!(
        instance_lock.ownership(),
        DataDirectoryOwnership::HeldByAnotherInstance,
        "the app is meant to start out waiting"
    );
    instance_lock
}

fn app_waiting_for_the_data_directory<'a>(
    paths: &[PathBuf],
    data_directory: &Path,
) -> Harness<'a, App> {
    let instance_lock = lock_on_a_directory_another_instance_holds(data_directory);
    let paths = paths.to_vec();
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            test_util::harness::transient_app_with_the_instance_lock(
                cc,
                &paths,
                instance_lock,
                PendingWrites::default(),
            )
        })
}

/// A recording that segments into two tracks under the default split gap: ten
/// fixes a second apart, an hour of nothing, then ten more.
fn two_track_gtd_bytes() -> Vec<u8> {
    use geotrace_sdk::{Angle, Duration as SdkDuration, NavFileBuilder, NavFix, NavFixTime};

    let mut recorder = NavFileBuilder::new().open();
    for fix in 0..20i64 {
        let seconds = if fix < 10 { fix } else { fix + 3_600 };
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(
                    base_time() + SdkDuration::seconds(seconds),
                ))
                .lat(Angle::degrees(51.5 + 0.0002 * fix as f64))
                .lon(Angle::degrees(-0.1))
                .heading(Angle::degrees(270.0))
                .build(),
        );
    }
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write bytes");
    bytes
}

/// The stored track table of [`two_track_gtd_bytes`], both tracks live.
fn two_live_track_ranges() -> [gt_store::TrackRange; 2] {
    use gt_store::{TrackRange, TrackState};

    [
        TrackRange {
            start: 0,
            end: 10,
            state: TrackState::Live,
        },
        TrackRange {
            start: 10,
            end: 20,
            state: TrackState::Live,
        },
    ]
}

/// Store `bytes` in the history database under `store`, cut into `tracks` and
/// segmented under `settings`, and return the reference it is stored under.
fn insert_recording_into(
    store: &gt_store::Store,
    bytes: &[u8],
    tracks: &[gt_store::TrackRange],
    settings: gt_store::StoredSegmentation,
) -> gt_store::DatabaseRef {
    let mut db =
        Recordings::open_or_create(&store.recordings_path()).expect("open the history database");
    let meta = gt_store::extract_meta(bytes).expect("the recording has metadata");
    db.insert("dev", &meta, tracks, settings, bytes)
        .expect("store the recording")
}

/// Installs an interference scheduler whose archive holds `days`, and hands
/// the archive back so a test can read what a delete left in it.
fn install_interference_archive(
    harness: &mut Harness<'_, App>,
    days: &[chrono::NaiveDate],
) -> (tempfile::TempDir, gt_store::InterferenceArchive) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<JamStore>()
        .expect("archive");
    for day in days {
        test_util::day_archive::archive_an_empty_interference_day(&store, *day);
    }
    install_interference_scheduler(harness, &store);
    (dir, store)
}

/// Point the app at `store` for interference, with nothing to fetch from.
fn install_interference_scheduler(
    harness: &mut Harness<'_, App>,
    store: &gt_store::InterferenceArchive,
) {
    let ctx = harness.ctx.clone();
    harness.state_mut().jamming = crate::app::jamming::JammingScheduler::new(
        ctx,
        Some(store.clone()),
        gt_jam::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
}

fn app_with_interference_days<'a>(
    days: &[chrono::NaiveDate],
) -> (
    Harness<'a, App>,
    tempfile::TempDir,
    gt_store::InterferenceArchive,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let (dir, store) = install_interference_archive(&mut harness, days);
    (harness, dir, store)
}

fn enable_environment_auto_prune(harness: &mut Harness<'_, App>, max_age_months: u32) {
    let settings = &mut harness.state_mut().environment_storage_settings;
    settings.auto_prune_enabled = true;
    settings.auto_prune_max_age_months = max_age_months;
}

/// The close request a window manager sends when the close button is pressed.
fn request_window_close(harness: &mut Harness<'_, App>) {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
}

fn root_viewport_commands(harness: &Harness<'_, App>) -> Vec<egui::ViewportCommand> {
    harness
        .output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| viewport.commands.clone())
        .unwrap_or_default()
}

fn closed_the_window(harness: &Harness<'_, App>) -> bool {
    root_viewport_commands(harness).contains(&egui::ViewportCommand::Close)
}

/// Push a file built with `travel_mode` into the app's loaded files, returning
/// the ref of its single track.
fn push_file_with_travel_mode(
    harness: &mut Harness<'_, App>,
    name: &str,
    travel_mode: Option<gt_types::TravelMode>,
) -> gt_types::TrackRef {
    push_file_with(
        harness,
        name,
        travel_mode,
        gt_loaded_files::FileHistory::None,
    )
}

fn push_file_with(
    harness: &mut Harness<'_, App>,
    name: &str,
    travel_mode: Option<gt_types::TravelMode>,
    history: gt_loaded_files::FileHistory,
) -> gt_types::TrackRef {
    let points = gt_test_utils::nav_test_data();
    let fi = push_points_as(harness, name, &points, travel_mode, history);
    gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(0))
}

/// Build a recording from `points` and push it into the app's loaded
/// files, returning its file index.
fn push_points_as(
    harness: &mut Harness<'_, App>,
    name: &str,
    points: &[gt_types::NavPoint],
    travel_mode: Option<gt_types::TravelMode>,
    history: gt_loaded_files::FileHistory,
) -> gt_types::FileIdx {
    let file = gt_track_builder::build_loaded_file(
        name.to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(std::path::PathBuf::from(name)),
        gt_track_builder::FileMeta {
            travel_mode,
            ..gt_track_builder::FileMeta::default()
        },
        vec![],
    );
    let state = harness.state_mut();
    let mut shared = state.shared.borrow_mut();
    shared.loaded_files.push(file, history);
    let fi = gt_types::FileIdx::new(shared.loaded_files.files().len() - 1);
    shared.sync_tree_from_loaded_files();
    fi
}

/// A recording running from the moment the generated log starts, so the log
/// finds it as an association candidate when it loads after it.
fn recording_alongside_the_log(name: &str, start_lat_deg: f64) -> TestDroppedFile {
    TestDroppedFile::bytes(recording_bytes_alongside_the_log(start_lat_deg), name)
}

/// The GTD bytes [`recording_alongside_the_log`] drops as a file, for a test
/// that stores the same recording in a database of its own instead.
fn recording_bytes_alongside_the_log(start_lat_deg: f64) -> Vec<u8> {
    gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: gt_test_utils::synthetic_log_start(),
        point_count: 600,
        step_secs: 1,
        start_lat_deg,
        start_lon_deg: 12.0,
        lat_step_deg: 0.00005,
        lon_step_deg: 0.00008,
        heading_deg: 20.0,
        speed_kmh: 28.0,
        eph_m: 1.8,
        sats_seen: 14,
        sats_in_fix: 11,
    })
}

fn drop_log_and_wait_for_load(harness: &mut Harness<App>, text: &str, name: &str) {
    test_util::harness::drop_file_and_wait_for_load(
        harness,
        TestDroppedFile::bytes(text.as_bytes(), name),
    );
}

impl App {
    fn shown_log(&self) -> Option<&LoadedLog> {
        self.log_viewer
            .selected_log()
            .and_then(|id| self.logs.get_by_id(id))
    }

    /// The log that loaded first, which is the only one in every fixture that
    /// loads one.
    fn first_log(&self) -> Option<&LoadedLog> {
        self.logs.iter().next()
    }

    /// The log that loaded last, for the fixtures loading a second one.
    fn last_log(&self) -> Option<&LoadedLog> {
        self.logs.iter().last()
    }

    /// How many lines the loaded logs put on the map, resolved against the
    /// loaded recordings the way the frame does.
    fn log_map_match_count(&mut self) -> usize {
        let shared = self.shared.borrow();
        let names = gt_loaded_files::RecordingNames::resolve(
            shared.loaded_files.view(),
            &shared.recording_name_template,
        );
        self.logs
            .map_matches(shared.loaded_files.view(), &names)
            .match_count()
    }
}

fn type_into_log_filter_of(harness: &mut Harness<'_, App>, text: &str) {
    focus_the_live_log_filter(harness);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    run_until_the_log_filter_scans_land(harness);
}

fn focus_the_live_log_filter(harness: &mut Harness<'_, App>) {
    harness.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(filters::LIVE_FILTER_FIELD_ID));
    });
    harness.run_steps(2);
}

/// Runs until every scan the shown log's filters started has landed. The scans
/// run on worker threads, and the filter row draws
/// [`filters::PENDING_NOTE`] until they do.
fn run_until_the_log_filter_scans_land(harness: &mut Harness<'_, App>) {
    // The frame that reads the keystroke or the click is the one that starts
    // the scan: the wait below is only meaningful after it.
    harness.run_steps(1);
    let landed = harness.step_until(|harness| {
        harness
            .state()
            .shown_log()
            .is_some_and(|log| !log.filters().is_query_pending())
    });
    assert!(landed, "the filter scans landed");
    harness.run_steps(2);
}

fn add_log_filter_in(harness: &mut Harness<'_, App>, text: &str) {
    type_into_log_filter_of(harness, text);
    harness.get_by_label(filters::ADD_FILTER_LABEL).click();
    run_until_the_log_filter_scans_land(harness);
}

/// A recordings database of this run's own, so a test can store a log with a
/// recording without touching the user's history.
fn open_temporary_history_database(path: &std::path::Path) -> gt_store::Recordings {
    use gt_store::HistoryDatabase as _;
    gt_store::Recordings::open_or_create(path).expect("the temporary database opens")
}
