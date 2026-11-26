//! App constructors for the egui test harness, and the waits that step one
//! until a background job has landed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{AccessKitNode, NodeT as _, Queryable as _};
use gt_instance_lock::DataDirectoryLock;
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::{FlareStore, IonexStore, JamStore, RecordingsHandle, SolarStore, Store};
use gt_test_utils::{By, HarnessInteraction as _};

use crate::app::archive_recovery::UnavailableArchives;
use crate::app::history_db::HistoryWorker;
use crate::app::query::results::MATCH_LIST_WINDOW_TITLE;
use crate::app::recording_from_disk::LOAD_FROM_DISK_LABEL;
use crate::app::storage::OpenStorage;
use crate::app::{App, StartupOptions, Storage};

mod tests {
    use egui_kittest::Harness;

    use crate::app::test_util;

    /// The harness reaches `App::new_with_config`, the same constructor `main`
    /// uses. A test run opens neither the user's recordings database nor their
    /// interference archive.
    #[test]
    fn the_test_harness_opens_no_user_databases() {
        let mut harness = Harness::builder()
            .with_context_setup(gt_ui_theme::install_app_style)
            .with_wait_for_pending_images(false)
            .build_eframe(test_util::harness::transient_app);
        harness.step();

        assert!(
            harness.state().history.path().is_none(),
            "no recordings database"
        );
        assert!(
            !harness.state().jamming.archive_available(),
            "no interference archive"
        );
        assert!(
            harness.state().loader.db_path.is_none(),
            "nothing for the loader to store into"
        );
    }
}

/// In-memory [`egui::DroppedFile`] for drag-drop tests. `bytes` drops carry a
/// relative path holding the display name, matching how web drops expose only
/// the file name, `path` drops behave like native drops from disk.
#[derive(Debug)]
pub struct TestDroppedFile {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl TestDroppedFile {
    pub fn bytes(bytes: impl Into<Vec<u8>>, name: &str) -> Self {
        Self {
            path: PathBuf::from(name),
            bytes: Some(bytes.into()),
        }
    }

    pub fn path(path: PathBuf) -> Self {
        Self { path, bytes: None }
    }
}

impl egui::DroppedFile for TestDroppedFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        match &self.bytes {
            Some(bytes) => Ok(bytes.clone()),
            None => std::fs::read(&self.path).map_err(|e| e.to_string()),
        }
    }
}

/// App constructor for snapshot harnesses, persisting settings at the harness's
/// temp config path. `fading` is supplied by the harness (off by default) so
/// snapshots don't capture mid-animation hover fades.
pub fn build_app(cc: &eframe::CreationContext<'_>, config_path: &Path, fading: bool) -> App {
    build_app_with_write_access(cc, config_path, fading, WriteAccess::Owner)
}

/// [`build_app`] with the map drawing the captured Mapbox satellite tiles, so
/// its snapshot shows the ground the recording was made on.
pub fn build_app_on_captured_tiles(
    cc: &eframe::CreationContext<'_>,
    config_path: &Path,
    fading: bool,
) -> App {
    build_app_with_the_instance_lock(
        cc,
        config_path,
        fading,
        PendingWrites::new(WriteAccess::Owner),
        DataDirectoryLock::marking_nothing(),
        gt_map::TileAccess::Captured(gt_test_utils::map_tile_capture_dir()),
    )
}

/// [`build_app`] for a session with `write_access`, which controls whether the
/// settings are persisted at all.
pub fn build_app_with_write_access(
    cc: &eframe::CreationContext<'_>,
    config_path: &Path,
    fading: bool,
    write_access: WriteAccess,
) -> App {
    build_app_with_the_instance_lock(
        cc,
        config_path,
        fading,
        PendingWrites::new(write_access),
        DataDirectoryLock::marking_nothing(),
        gt_map::TileAccess::Synthetic,
    )
}

/// [`build_app`] on the data directory `instance_lock` was taken on, which is
/// what decides whether the run opens anything, with `pending_writes`
/// deciding whether it writes to it at all.
pub fn build_app_with_the_instance_lock(
    cc: &eframe::CreationContext<'_>,
    config_path: &Path,
    fading: bool,
    pending_writes: PendingWrites,
    instance_lock: DataDirectoryLock,
    tile_access: gt_map::TileAccess,
) -> App {
    App::new_with_config(
        cc,
        &[],
        Some(config_path.to_path_buf()),
        StartupOptions {
            fading_enabled: fading,
            offline: true,
            tile_access,
            storage: Storage::Disabled,
            app_version: TEST_APP_VERSION,
            pending_writes,
            instance_lock,
        },
    )
}

/// App constructor for the functional (non-snapshot) tests that don't touch a
/// config file. Fading stays off so frame counts are deterministic.
pub fn transient_app(cc: &mut eframe::CreationContext<'_>) -> App {
    transient_app_with_the_instance_lock(
        cc,
        &[],
        DataDirectoryLock::marking_nothing(),
        PendingWrites::default(),
    )
}

pub fn app_with_the_databases_still_opening<'a>(
    paths: &[PathBuf],
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    app_with_the_databases_still_opening_for(paths, WriteAccess::Owner)
}

pub fn app_with_the_databases_still_opening_for<'a>(
    paths: &[PathBuf],
    write_access: WriteAccess,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    let paths = paths.to_vec();
    app_with_the_databases_still_opening_built_by(move |cc| {
        transient_app_with_the_instance_lock(
            cc,
            &paths,
            DataDirectoryLock::marking_nothing(),
            PendingWrites::new(write_access),
        )
    })
}

/// The harness replaces the app's storage channel before its first frame.
pub fn app_with_the_databases_still_opening_built_by<'a>(
    build: impl FnOnce(&eframe::CreationContext<'_>) -> App + 'a,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    let (sender_tx, sender_rx) = mpsc::channel();
    let harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
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

pub fn send_open_storage_and_step(
    harness: &mut Harness<'_, App>,
    databases: &mpsc::Sender<OpenStorage>,
    store: &Store,
) {
    let pending_writes = harness.state().pending_writes.clone();
    let opened = storage_opened_in(store, &harness.ctx, &pending_writes);
    databases.send(opened).expect("the app holds the receiver");
    harness.step();
}

pub fn storage_opened_in(
    store: &Store,
    ctx: &egui::Context,
    pending_writes: &PendingWrites,
) -> OpenStorage {
    OpenStorage {
        history: HistoryWorker::spawn(
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

pub fn step_minimized_logic(harness: &mut Harness<'_, App>) -> egui::LogicOutput {
    let ctx = harness.ctx.clone();
    let mut input = egui::RawInput {
        viewports: harness.input_mut().viewports.clone(),
        focused: false,
        ..Default::default()
    };
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(true);
    ctx.run_logic(&input, |ctx| {
        eframe::App::logic(harness.state_mut(), ctx, &mut eframe::Frame::_new_kittest());
    })
}

pub fn finish_shutdown_without_painting(harness: &mut Harness<'_, App>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let close_commands = loop {
        let output = self::step_minimized_logic(harness);
        if harness.state().shutdown.close_allowed() {
            break output.viewport_commands;
        }
        assert!(Instant::now() < deadline, "Background work did not finish");
        thread::sleep(Duration::from_millis(5));
    };
    assert!(
        close_commands
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|commands| { commands.contains(&egui::ViewportCommand::Close) })
    );
    assert!(harness.state().pending_writes.is_idle());
}

/// [`transient_app`] started with the files a command line named, on the data
/// directory `instance_lock` was taken on, which is what decides whether the
/// run opens anything, with `pending_writes` deciding whether it writes to it
/// at all.
pub fn transient_app_with_the_instance_lock(
    cc: &eframe::CreationContext<'_>,
    paths: &[PathBuf],
    instance_lock: DataDirectoryLock,
    pending_writes: PendingWrites,
) -> App {
    transient_app_with_the_settings_file(cc, paths, None, instance_lock, pending_writes)
}

/// [`transient_app_with_the_instance_lock`] reading and writing the settings
/// file at `config_path`, and none where that is [`None`].
pub fn transient_app_with_the_settings_file(
    cc: &eframe::CreationContext<'_>,
    paths: &[PathBuf],
    config_path: Option<PathBuf>,
    instance_lock: DataDirectoryLock,
    pending_writes: PendingWrites,
) -> App {
    App::new_with_config(
        cc,
        paths,
        config_path,
        StartupOptions {
            fading_enabled: false,
            offline: true,
            tile_access: gt_map::TileAccess::Synthetic,
            storage: Storage::Disabled,
            app_version: TEST_APP_VERSION,
            pending_writes,
            instance_lock,
        },
    )
}

/// Drop `file` into the app and step until the background load thread has
/// finished with it.
///
/// The thread sends a `Completed` message when done and `drain_load_channel`
/// (called at the start of every `ui()` frame) removes the job. A dropped
/// recording is looked up in the recording history before its job starts, so
/// the wait covers that lookup too.
pub fn drop_file_and_wait_for_load(harness: &mut Harness<App>, file: TestDroppedFile) {
    harness.input_mut().dropped_files.push(Arc::new(file));
    harness.step();
    assert!(
        harness.step_until(|harness| harness.state().loader.loading_jobs.is_empty()
            && !harness.state().association_batches.has_recording_work()),
        "the background load did not finish"
    );
    pin_the_load_time_the_overlay_shows(harness);
}

/// Give every job the load overlay still lists [`TEST_LOAD_ELAPSED_SECS`].
fn pin_the_load_time_the_overlay_shows(harness: &mut Harness<'_, App>) {
    for job in &mut harness.state_mut().loader.finishing_jobs {
        job.elapsed_secs = TEST_LOAD_ELAPSED_SECS;
    }
}

/// Drop a recording history already holds, answer the prompt it raises with
/// "Load from disk", and step until the recording is in the view.
pub fn drop_a_stored_recording_and_load_it_from_disk(
    harness: &mut Harness<App>,
    file: TestDroppedFile,
) {
    let loaded_before = harness.state().shared.borrow().loaded_files.len();
    harness.input_mut().dropped_files.push(Arc::new(file));
    harness.step();
    step_until_the_prompt_over_stored_recordings_is_drawn(harness);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len()
            > loaded_before
            && harness.state().loader.loading_jobs.is_empty()),
        "the recording did not load from disk"
    );
    pin_the_load_time_the_overlay_shows(harness);
}

/// Step until the prompt over the recordings history holds is drawn where the
/// user sees it: an anchored dialog takes its position on the pass after it
/// opens, and a click aimed at the pass before that misses its buttons.
pub fn step_until_the_prompt_over_stored_recordings_is_drawn(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness
            .state()
            .pending_recordings_already_in_history
            .is_some()),
        "the drop of a recording history holds raised no prompt"
    );
    harness.run_steps(3);
}

/// Step the harness repeatedly until the query worker's result has landed.
pub fn step_until_query_result(harness: &mut Harness<App>) {
    assert!(
        harness.step_until(|harness| harness.state().query_window.matches().is_some()),
        "the query worker produced no result"
    );
}

pub fn step_until_a_recording_is_loaded(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "no recording reached the view"
    );
}

pub fn step_until_a_log_is_loaded(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness.state().logs.len() == 1),
        "no log reached the viewer"
    );
}

#[derive(Clone, Copy)]
pub enum ResultsTable {
    Matches,
    Points,
    PoppedOutMatches,
}

pub fn topmost_point_row(harness: &Harness<'_, App>) -> (egui::Rect, String) {
    time_rows(harness, ResultsTable::Points)
        .into_iter()
        .next()
        .expect("the results list point rows")
}

/// The points table aligns its time column with the match caption.
/// The matches table indents its times after the swatch, number and track columns.
pub fn time_rows(harness: &Harness<'_, App>, table: ResultsTable) -> Vec<(egui::Rect, String)> {
    let indented = match table {
        ResultsTable::Matches => true,
        ResultsTable::Points => false,
        ResultsTable::PoppedOutMatches => {
            return rows_in_reading_order(time_cells_in_window(harness, MATCH_LIST_WINDOW_TITLE));
        }
    };
    let left_edge = harness.get_by_label_contains("Match ").rect().left();
    rows_in_reading_order(
        time_cells_in_window(harness, QUERY_WINDOW_TITLE)
            .into_iter()
            .filter(|(rect, _)| (rect.left() > left_edge + INDENT_TOLERANCE_PX) == indented)
            .collect(),
    )
}

fn time_cells_in_window(harness: &Harness<'_, App>, title: &str) -> Vec<(egui::Rect, String)> {
    harness
        .get_by_role_and_label(Role::Window, title)
        .query_all(bare_time_labels())
        .filter_map(|node| Some((node.rect(), node.accesskit_node().value()?)))
        .collect()
}

/// A match row has separate start and end time cells.
fn rows_in_reading_order(mut cells: Vec<(egui::Rect, String)>) -> Vec<(egui::Rect, String)> {
    cells.sort_by(|(a, _), (b, _)| {
        a.top()
            .total_cmp(&b.top())
            .then_with(|| a.left().total_cmp(&b.left()))
    });
    cells.dedup_by(|(a, _), (b, _)| (a.top() - b.top()).abs() < INDENT_TOLERANCE_PX);
    cells
}

fn bare_time_labels<'a>() -> By<'a> {
    By::new()
        .role(Role::Label)
        .predicate(|node: &AccessKitNode<'_>| {
            node.value().is_some_and(|value| {
                value.len() >= ROW_TIME_LEN
                    && value.contains(':')
                    && value
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
            })
        })
}

/// The fixed version string injected in place of the real crate version in
/// tests, so every version-bearing UI snapshot stays stable across release
/// bumps. The one placeholder for the whole app (the About dialog and the
/// update prompt both flow through it).
pub const TEST_APP_VERSION: &str = "0.0.0-test";

/// The load time that every finished job reports in a test, replacing the
/// value that the app measures. The load overlay prints it as `0.4s`. The
/// measured value counts the frames that the background thread ran for, and
/// the machine's load changes that count from run to run.
pub const TEST_LOAD_ELAPSED_SECS: f32 = 0.4;

pub const QUERY_WINDOW_TITLE: &str = "Query";

const ROW_TIME_LEN: usize = "14:00:19".len();

/// The tolerance for identifying an indented column or cells in the same row.
const INDENT_TOLERANCE_PX: f32 = 2.0;
