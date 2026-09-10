use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration as StdDuration;

use egui_kittest::{Harness, kittest::Queryable as _};
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use gt_log_view::LoadedLog;
use gt_store::RecordingsHandle;
use gt_test_utils::{
    By, HarnessInteraction as _, SyntheticGtdSpec, SyntheticLogSpec, SyntheticLogTimestamps,
    TestHarness,
};
use gt_types::FileIdx;

use crate::app::App;
use crate::app::log_viewer;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// A journald-shaped log in the ISO form: its lines carry the year, so what the
/// viewer draws does not depend on the year the test runs in.
fn synthetic_log(approx_bytes: usize) -> String {
    gt_test_utils::synthetic_journald_log(SyntheticLogSpec {
        approx_bytes,
        seed: 7,
        timestamps: SyntheticLogTimestamps::Iso8601Space,
    })
}

/// Drops a log and confirms the association dialog it raises: the log is left
/// associated with the recording the dialog preselected.
fn drop_log_and_associate_it(harness: &mut Harness<App>, text: &str, name: &str) {
    ui_tests::drop_log_and_wait_for_load(harness, text, name);
    harness.run_steps(3);
    harness
        .get_by_label(log_viewer::association_dialog::CONFIRM_LABEL)
        .click();
    harness.run_steps(3);
}

fn app_with_a_log_loaded() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    ui_tests::drop_log_and_wait_for_load(&mut harness, &synthetic_log(64 * 1024), "navsyncd.log");
    harness.run_steps(3);
    harness
}

fn parse_summary_of_the_shown_log(harness: &Harness<App>) -> String {
    harness
        .state()
        .shown_log()
        .map(|log| log.parse_summary_line())
        .unwrap_or_default()
}

#[test]
fn a_log_that_finished_loading_opens_the_viewer_on_its_parse_summary() {
    let harness = app_with_a_log_loaded();

    assert!(
        harness.state().log_viewer.open,
        "the viewer opens by itself"
    );
    assert_eq!(harness.state().logs.len(), 1);
    let summary = parse_summary_of_the_shown_log(&harness);
    assert!(
        summary.starts_with("ISO 8601 · "),
        "the summary names the detected format, got {summary:?}"
    );
    harness.get_by_label(summary.as_str());
}

/// One log per content: dropping a text the session already holds opens the
/// loaded log and keeps the one copy.
#[test]
fn dropping_a_log_the_session_already_holds_shows_the_loaded_one() {
    let mut harness = app_with_a_log_loaded();
    let loaded = harness.state().logs.first_id();
    harness.state_mut().log_viewer.open = false;

    ui_tests::drop_log_and_wait_for_load(
        &mut harness,
        &synthetic_log(64 * 1024),
        "copy-of-navsyncd.log",
    );
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness.state().logs.first_id(),
        loaded,
        "the loaded log kept its identity"
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .map(gt_log_view::LoadedLog::name),
        Some("navsyncd.log"),
        "the viewer shows the log the session holds, under the name it loaded with"
    );
    assert!(harness.state().log_viewer.open, "the viewer opens on it");
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "the drop raises a toast naming the loaded log"
    );
}

/// A drag over the app covers it with the hint the empty viewer shows.
#[test]
fn a_drag_over_the_app_shows_the_hint_naming_every_way_a_log_gets_in() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    assert!(
        harness.query_by_label(log_viewer::LOG_LOAD_HINT).is_none(),
        "nothing is being dragged yet"
    );

    harness
        .input_mut()
        .hovered_files
        .push(egui::HoveredFile::default());
    harness.step();

    harness.get_by_label(log_viewer::LOG_LOAD_HINT);
}

/// A log line whose timestamp the pasted-log name is taken from.
const PASTED_LOG: &str = "2026-01-01 14:02:11 navsyncd: uploaded 2 recordings\n";

/// Pastes `text` as Ctrl+V does, and runs until the load it started has
/// finished.
fn paste_and_wait_for_load(harness: &mut Harness<App>, text: &str) {
    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(text.to_owned()));
    harness.step();
    assert!(
        harness.step_until(|harness| harness.state().loader.loading_jobs.is_empty()),
        "the background load did not finish"
    );
}

/// Ctrl+V with nothing focused loads the clipboard text as a log, named after
/// the first entry it anchored.
#[test]
fn pasting_log_text_loads_it_named_after_its_first_entry() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();

    paste_and_wait_for_load(&mut harness, PASTED_LOG);
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness
            .state()
            .first_log()
            .map(gt_log_view::LoadedLog::name),
        Some("pasted 14:02:11")
    );
    assert!(harness.state().log_viewer.open);
}

/// A paste while a text field holds focus belongs to that field, and loads
/// nothing.
#[test]
fn pasting_into_a_focused_field_reaches_the_field_and_loads_no_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().query_window.open = true;
    harness.run_steps(2);
    ui_tests::focus_query_editor_at_end(&harness, "");
    harness.run_steps(2);

    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(PASTED_LOG.to_owned()));
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    assert!(
        harness.state().query_window.text().contains("navsyncd"),
        "the paste reached the editor, got {:?}",
        harness.state().query_window.text()
    );
}

/// An empty clipboard has no log in it.
#[test]
fn pasting_empty_text_loads_no_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();

    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(String::new()));
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    assert!(harness.state().loader.loading_jobs.is_empty());
}

/// A log carrying a byte that is not UTF-8 loads, and its summary states what
/// reading it as text cost.
#[test]
fn dropping_a_log_that_is_not_utf8_states_the_replacement_in_its_summary() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(
            b"2026-01-01 14:02:11 navsyncd: caf\xe9 open\n".as_slice(),
            "navsyncd.log",
        ),
    );
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    let summary = parse_summary_of_the_shown_log(&harness);
    assert!(
        summary.ends_with("1 byte replaced"),
        "the summary states the lossy decode, got {summary:?}"
    );
    harness.get_by_label(summary.as_str());
}

/// With no recording loaded a log is still fully readable: nothing asks the
/// user which recording it belongs to, and it puts nothing on the map.
#[test]
fn a_log_loaded_without_a_recording_stays_untargeted_and_raises_no_dialog() {
    let mut harness = app_with_a_log_loaded();

    assert!(harness.state().association_dialog.is_none());
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(gt_log_view::LoadedLog::associated_recording),
        None
    );
    assert_eq!(harness.state_mut().log_map_match_count(), 0);
    harness.get_by_label(parse_summary_of_the_shown_log(&harness).as_str());
}

#[test]
fn the_menu_bar_icon_closes_and_reopens_the_viewer() {
    let mut harness = app_with_a_log_loaded();

    harness.get_by_label(ICON_ARTICLE).click();
    harness.run_steps(2);
    assert!(!harness.state().log_viewer.open);

    harness.get_by_label(ICON_ARTICLE).click();
    harness.run_steps(2);
    assert!(harness.state().log_viewer.open);
}

#[test]
fn clicking_the_parse_summary_unfolds_the_boots_and_the_service_table() {
    let mut harness = app_with_a_log_loaded();
    assert!(
        harness.query_by_label("Boots").is_none(),
        "the summary panel starts folded away"
    );

    let summary = parse_summary_of_the_shown_log(&harness);
    harness.get_by_label(summary.as_str()).click();
    harness.run_steps(3);

    harness.get_by_label("Boots");
    harness.get_by_label("Service summary");
    // What the fixture's exporter summary block states about the device.
    harness.get_by_label("nav-devkit-mk2");
    harness.get_by_label("hal-powerd");
}

/// The viewer over a journald-shaped log: the selector row with its parse
/// summary, the summary panel unfolded onto the boot timeline and the
/// exporter's service table, the line table, and the footer's association
/// controls.
#[test]
fn snapshot_app_log_viewer() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(
        &mut harness.inner,
        &synthetic_log(64 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    let summary = parse_summary_of_the_shown_log(&harness.inner);
    harness.inner.get_by_label(summary.as_str()).click();
    harness.inner.run_steps(8);

    harness.snapshot_loose("app_log_viewer");
}

/// Types `text` into the log viewer's live filter and runs until the scan it
/// starts has landed. The field is focused by its own id: the app renders text
/// fields of its own behind the window.
fn type_into_log_filter(harness: &mut TestHarness<'_, App>, text: &str) {
    ui_tests::type_into_log_filter_of(&mut harness.inner, text);
}

/// Writes `text` into the live filter and keeps it as a chip.
fn add_log_filter(harness: &mut TestHarness<'_, App>, text: &str) {
    ui_tests::add_log_filter_in(&mut harness.inner, text);
}

const PASSES_THE_POOL_HOLDS_A_FILTER_SCAN_QUEUED: u64 = 20;

/// Holds every thread of [`gt_logfile::log_worker_pool`] busy, leaving the
/// filter scans spawned onto it queued until the pool is released.
struct OccupiedLogWorkerPool {
    release: Arc<Barrier>,
}

impl OccupiedLogWorkerPool {
    fn occupy() -> Self {
        let pool = gt_logfile::log_worker_pool().expect("the log worker pool builds");
        let workers = pool.current_num_threads();
        let occupied = Arc::new(Barrier::new(workers + 1));
        let release = Arc::new(Barrier::new(workers + 1));
        for _ in 0..workers {
            let occupied = Arc::clone(&occupied);
            let release = Arc::clone(&release);
            pool.spawn(move || {
                occupied.wait();
                release.wait();
            });
        }
        occupied.wait();
        Self { release }
    }

    /// Releases the pool once `ctx` has run `passes` further passes, holding a
    /// queued scan for the same number of frames on every machine.
    fn release_after_passes(self, ctx: &egui::Context, passes: u64) {
        let ctx = ctx.clone();
        let release_at = ctx.cumulative_pass_nr().saturating_add(passes);
        thread::Builder::new()
            .name("release-log-worker-pool".to_owned())
            .spawn(move || {
                while ctx.cumulative_pass_nr() < release_at {
                    thread::sleep(StdDuration::from_millis(1));
                }
                self.release.wait();
            })
            .expect("the releasing thread spawns");
    }
}

/// The scan a keystroke starts stays pending until the worker pool runs it,
/// and the filter row draws [`log_viewer::filters::PENDING_NOTE`] until it
/// lands.
#[test]
fn the_log_filter_wait_runs_until_the_scan_the_keystroke_started_lands() {
    let mut harness = app_with_a_log_loaded();
    OccupiedLogWorkerPool::occupy()
        .release_after_passes(&harness.ctx, PASSES_THE_POOL_HOLDS_A_FILTER_SCAN_QUEUED);

    ui_tests::focus_the_live_log_filter(&mut harness);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("kernel".to_owned()));
    harness.run_steps(1);
    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| log.filters().is_query_pending()),
        "the keystroke frame started a scan"
    );
    harness.run_steps(2);
    harness.get_by_label(log_viewer::filters::PENDING_NOTE);

    ui_tests::run_until_the_log_filter_scans_land(&mut harness);

    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| !log.filters().is_query_pending()),
        "the wait ran until the scan landed"
    );
    assert!(
        harness
            .query_by_label(log_viewer::filters::PENDING_NOTE)
            .is_none(),
        "the note goes with the scan"
    );
}

/// The viewer filtering a journald-shaped log: the live filter with its match
/// count and the term it highlights in the table, a layer chip with its colour
/// swatch and gutter bars, and a refine chip narrowing the table to what it
/// matched.
#[test]
fn snapshot_app_log_viewer_filters() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(
        &mut harness.inner,
        &synthetic_log(64 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    add_log_filter(&mut harness, "kernel");
    add_log_filter(&mut harness, "rotated");
    add_log_filter(&mut harness, "rc=-110");
    // The last chip added is the one furthest right in the chip row.
    harness
        .inner
        .nth_matching(By::new().label(ICON_PLUS_CIRCLE), 2)
        .click();
    ui_tests::run_until_the_log_filter_scans_land(&mut harness.inner);
    type_into_log_filter(&mut harness, "retries");

    harness.snapshot_loose("app_log_viewer_filters");
}

/// The association dialog over a freshly loaded log: the loaded recordings
/// ranked by how much of the log each ran alongside, the one that missed it
/// grayed, and the attach tickbox live for a recording the history database
/// holds.
#[test]
fn snapshot_log_association_dialog() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history = crate::app::history_db::HistoryWorker::spawn(
        RecordingsHandle::Owner(ui_tests::open_temporary_history_database(
            &dir.path().join("geotrace.h5"),
        )),
        egui::Context::default(),
        gt_pending_writes::PendingWrites::default(),
    );
    harness.inner.state_mut().sync_db_path();
    harness.inner.state_mut().history.hide_path();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_a_day_after_the_log("drive.gtd"),
    );
    ui_tests::drop_log_and_wait_for_load(
        &mut harness.inner,
        &synthetic_log(64 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_label(log_viewer::association_dialog::ATTACH_LABEL)
        .click();
    harness.inner.run_steps(5);

    harness.snapshot_loose("log_association_dialog");
}

/// A recording from a day the log does not cover, which the dialog lists as a
/// choice that would leave every line unassociated.
fn recording_a_day_after_the_log(name: &str) -> TestDroppedFile {
    TestDroppedFile::bytes(
        gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
            start: gt_test_utils::synthetic_log_start() + chrono::Duration::days(1),
            point_count: 300,
            step_secs: 1,
            start_lat_deg: 48.2,
            start_lon_deg: 11.6,
            lat_step_deg: 0.00004,
            lon_step_deg: 0.00009,
            heading_deg: 75.0,
            speed_kmh: 42.0,
            eph_m: 2.2,
            sats_seen: 12,
            sats_in_fix: 9,
        }),
        name,
    )
}

/// The map draws what a filter selected, and stops when the log that owns the
/// filter is hidden.
#[test]
fn a_layer_chip_puts_the_lines_it_matched_on_the_map() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(&mut harness.inner, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.inner.run_steps(5);
    assert_eq!(
        harness.inner.state_mut().log_map_match_count(),
        0,
        "a loaded log draws nothing until a filter selects lines"
    );

    add_log_filter(&mut harness, "gnss");

    let matched = harness.inner.state_mut().log_map_match_count();
    assert!(matched > 0, "the chip's lines reach the map");

    let loaded = harness.inner.state().logs.first_id();
    if let Some(log) = loaded.and_then(|id| harness.inner.state_mut().logs.get_mut_by_id(id)) {
        log.set_visible(false);
    }
    harness.inner.run_steps(2);
    assert_eq!(
        harness.inner.state_mut().log_map_match_count(),
        0,
        "hiding the log takes its layer off the map"
    );
}

/// The map under a filtered log: a layer chip's hexagons along the recording,
/// clustered where the lines are dense, with the live filter's own colour over
/// them. The viewer is closed so the map it draws on is visible.
#[test]
fn snapshot_app_log_map_hexagons() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    // A log long enough to span the whole recording, so its hexagons run the
    // length of the track the map frames.
    drop_log_and_associate_it(
        &mut harness.inner,
        &synthetic_log(384 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    add_log_filter(&mut harness, "kernel");
    type_into_log_filter(&mut harness, "bus-off");
    harness.inner.get_by_label(ICON_ARTICLE).click();
    harness.inner.run_steps(5);

    ui_tests::assert_the_capture_covers_the_map(&mut harness, "app_log_map_hexagons");
    harness.snapshot_loose("app_log_map_hexagons");
}

/// A recording the history database does not hold is identified by the session
/// identity it keeps for as long as it stays loaded: unloading it takes the logs
/// anchored to it with it.
#[test]
fn unloading_a_recording_outside_history_unloads_the_logs_anchored_to_it() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        ui_tests::recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(&mut harness, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.run_steps(3);
    assert!(
        matches!(
            harness.state().first_log().and_then(LoadedLog::anchor_key),
            Some(gt_log_view::RecordingKey::Session(_))
        ),
        "the recording is in no history database"
    );

    harness.state_mut().shared.borrow_mut().tree.pending_unload =
        Some(vec![gt_side_panel::NodeKey::File(FileIdx::new(0))]);
    harness.run_steps(3);

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
    assert_eq!(harness.state().logs.len(), 0);
}

#[test]
fn choosing_a_target_in_the_footer_associates_the_log_against_it() {
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
    ui_tests::drop_log_and_wait_for_load(&mut harness, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.run_steps(3);
    // The footer is the fallback for a log left untargeted, which cancelling
    // the association dialog is one way to reach.
    harness.get_by_label("Cancel").click();
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(|log| log.associated_recording()),
        None,
        "two overlapping recordings leave the choice to the user"
    );

    harness.get_by_label("Associated with");
    harness.get(By::new().value(gt_ui_theme::EM_DASH)).click();
    harness.run_steps(2);
    // The side panel lists the same recording, so take the row the combo
    // popup opened at the bottom of the viewer.
    harness
        .bottommost_matching(By::new().label("walk_b.gtd"))
        .click();
    harness.run_steps(3);

    assert!(
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_entry_count() > 0),
        "picking a target associates the log against it right away"
    );
}
