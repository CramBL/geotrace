use egui_kittest::{Harness, kittest::Queryable as _};
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use gt_types::LoadWarning;
use rstest::rstest;

use crate::app::App;
use crate::app::frame::{LOADING_OVERLAY_MOST_LISTED_JOBS, LOADING_OVERLAY_WINDOW_ID};
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

#[test]
fn drag_drop_gtd_path_loads_file() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let tmp = tempfile::NamedTempFile::with_suffix(".gtd").expect("create temp file");
    std::io::Write::write_all(&mut tmp.as_file(), &gtd_bytes).expect("write temp gtd");
    let tmp_path = tmp.path().to_path_buf();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(&mut harness, TestDroppedFile::path(tmp_path));

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

#[test]
fn drag_drop_gtd_bytes_loads_file() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

/// Anything that is not a recording goes to the log parser, so binary junk
/// fails as a log: nothing in it contains a timestamp.
#[test]
fn drag_drop_binary_junk_reports_it_is_not_a_recognised_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(b"\xff\xfe\x00binary_junk".as_slice(), "mystery.bin"),
    );

    let error = harness.state().load_error.clone().unwrap_or_default();
    assert!(
        error.starts_with("Not a recognised log: no line has a timestamp in a known format"),
        "got {error:?}"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
    assert_eq!(harness.state().logs.len(), 0);
}

#[test]
fn snapshot_load_warnings_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state().shared.borrow_mut().warnings_popup = Some((
        "ride_2025-05-23.gtd".to_owned(),
        vec![
            LoadWarning {
                count: 3,
                issue: "satellite(s) with PRN 0".to_owned(),
                description: "PRN 0 is reserved and undefined in NMEA".to_owned(),
            },
            LoadWarning {
                count: 2,
                issue: "satellite(s) with elevation > 90°".to_owned(),
                description: "above the zenith; valid NMEA elevation range is [0°, 90°]"
                    .to_owned(),
            },
            LoadWarning {
                count: 5,
                issue: "satellite(s) with SNR ≈ 99 dB-Hz".to_owned(),
                description: "common sentinel value for unavailable signal strength; omit the SNR field when no measurement is available".to_owned(),
            },
        ],
    ));
    harness.run();
    harness.snapshot_with_color_tolerance("load_warnings_dialog");
}

/// Loads still running, which the overlay lists with a progress bar each.
#[derive(Clone, Copy)]
struct RunningJobCount(usize);

/// Loads that have finished, which the overlay lists under the running ones
/// while they fade.
#[derive(Clone, Copy)]
struct FinishedJobCount(usize);

/// The app with the progress overlay in the bottom-right corner, where the
/// map's layer and display toggles also sit.
fn app_with_a_batch_of_load_jobs(
    viewport: egui::Vec2,
    running: RunningJobCount,
    finished: FinishedJobCount,
) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(viewport)
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    let now = harness.inner.ctx.input(|input| input.time);
    let app = harness.state_mut();
    app.loader.loading_jobs = (0..running.0)
        .map(|index| crate::app::loader::LoadingJob {
            id: index as u64,
            filename: format!("ride-2026-05-{:02}.gtd", index + 1),
            progress: 0.2 + 0.15 * (index % 5) as f32,
            stage: crate::app::loader::STAGE_READING,
            started_at: now,
        })
        .collect();
    app.loader.finishing_jobs = (0..finished.0)
        .map(|index| crate::app::loader::FinishedJob {
            filename: format!("walk-2026-04-{:02}.gtd", index + 1),
            elapsed_secs: 1.4,
            completed_at: now,
        })
        .collect();
    harness.inner.run_steps(4);
    harness
}

#[test]
fn snapshot_loading_overlay_past_the_listed_jobs() {
    let mut harness = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(BATCH_FAR_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );
    harness.snapshot_with_color_tolerance("loading_overlay_past_the_listed_jobs");
}

/// The overlay lists [`LOADING_OVERLAY_MOST_LISTED_JOBS`] jobs whatever the
/// batch behind it holds.
#[test]
fn the_loading_overlay_opens_at_one_height_for_every_batch_past_the_listed_jobs() {
    let past = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(BATCH_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );
    let far_past = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(BATCH_FAR_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );

    assert_eq!(
        far_past
            .inner
            .window_rect(LOADING_OVERLAY_WINDOW_ID)
            .expect("the overlay lists the batch")
            .size(),
        past.inner
            .window_rect(LOADING_OVERLAY_WINDOW_ID)
            .expect("the overlay lists the batch")
            .size(),
        "{BATCH_FAR_PAST_THE_LISTED_JOBS} loads made the overlay taller than \
         {BATCH_PAST_THE_LISTED_JOBS} did: it has to list \
         {LOADING_OVERLAY_MOST_LISTED_JOBS} of them and count the rest"
    );
}

/// The line under the listed jobs counts the ones left over, whether those are
/// still running, finished, or both.
#[rstest]
#[case::all_running(BATCH_FAR_PAST_THE_LISTED_JOBS, 0)]
#[case::all_finished(0, BATCH_FAR_PAST_THE_LISTED_JOBS)]
#[case::one_running_over_a_finished_batch(1, BATCH_FAR_PAST_THE_LISTED_JOBS)]
fn the_loading_overlay_counts_the_jobs_it_does_not_list(
    #[case] running: usize,
    #[case] finished: usize,
) {
    let harness = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(running),
        FinishedJobCount(finished),
    );
    let unlisted = running + finished - LOADING_OVERLAY_MOST_LISTED_JOBS;

    assert!(
        harness
            .inner
            .query_by_label(format!("{unlisted} more").as_str())
            .is_some(),
        "the overlay listed {LOADING_OVERLAY_MOST_LISTED_JOBS} of {running} running and \
         {finished} finished jobs without counting the {unlisted} it left out"
    );
}

/// The overlay covers the map's display toggle on a short screen. A press
/// there has to reach the toggle: the overlay has no controls and egui leaves
/// it out of the hit-test.
#[test]
fn the_map_display_toggle_opens_on_a_press_under_the_loading_overlay() {
    let mut harness = app_with_a_batch_of_load_jobs(
        gt_test_utils::window_fit::SHORT_VIEWPORT,
        RunningJobCount(BATCH_FAR_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );
    let toggle = harness
        .inner
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::new(gt_map::DISPLAY_TOGGLE_BUTTON_AREA_ID)))
        .expect("the map draws its display toggle");
    let overlay = harness
        .inner
        .window_rect(LOADING_OVERLAY_WINDOW_ID)
        .expect("the overlay lists the batch");
    assert!(
        overlay.intersects(toggle),
        "the overlay at {overlay:?} does not cover the display toggle at {toggle:?}: the press \
         below would not cross the overlay"
    );

    harness.inner.click_at(toggle.center());
    harness.inner.run_steps(2);

    assert!(
        harness
            .inner
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new(gt_map::DISPLAY_TOGGLE_POPUP_AREA_ID)))
            .is_some(),
        "the press on the display toggle did not open its popup: egui routed it to the loading \
         overlay instead"
    );
}

/// Recordings dropped in one batch, one more than the overlay lists at once.
const BATCH_PAST_THE_LISTED_JOBS: usize = LOADING_OVERLAY_MOST_LISTED_JOBS + 1;

/// Recordings dropped in one batch, far more than the overlay lists at once.
const BATCH_FAR_PAST_THE_LISTED_JOBS: usize = LOADING_OVERLAY_MOST_LISTED_JOBS + 40;

/// 1280x800, where the map fills the top two thirds of the window and the plot
/// the bottom third.
const DESKTOP_VIEWPORT: egui::Vec2 = egui::vec2(1280.0, 800.0);
