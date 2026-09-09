//! App constructors for the egui test harness, and the waits that step one
//! until a background job has landed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gt_instance_lock::DataDirectoryLock;
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_test_utils::HarnessInteraction as _;

use crate::app::recording_from_disk::LOAD_FROM_DISK_LABEL;
use crate::app::{App, StartupOptions, Storage};

/// The fixed version string injected in place of the real crate version in
/// tests, so every version-bearing UI snapshot stays stable across release
/// bumps. The one placeholder for the whole app (the About dialog and the
/// update prompt both flow through it).
pub const TEST_APP_VERSION: &str = "0.0.0-test";

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
            && harness.state().recordings_awaiting_a_history_lookup == 0),
        "the background load did not finish"
    );
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
