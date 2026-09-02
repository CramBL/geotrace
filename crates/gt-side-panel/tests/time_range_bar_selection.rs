//! What the filter panel's two time range bars span, over the recordings
//! loaded.
//!
//! The panel lays out a heading, the bar it names, and a label of the window's
//! two ends, twice: once over the whole loaded time range, and once over the
//! active range when that range is far narrower than the whole. [`bar_rects`]
//! reads the bars out of the accessibility tree by their role: a bar carries no
//! label of its own.

#![expect(
    clippy::expect_used,
    reason = "the helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use std::path::PathBuf;

use chrono::{DateTime, TimeZone as _, Utc};
use egui::accesskit::Role;
use egui_kittest::kittest::NodeT as _;
use gt_filter::GlobalFilter;
use gt_side_panel::{FilterPanelState, render_filter_panel};
use gt_test_utils::{By, Queryable as _, TestHarness};
use gt_types::{LoadedFile, NavPoint};

/// The state the filter panel reads and writes.
struct PanelState {
    files: Vec<LoadedFile>,
    filter: GlobalFilter,
    panel: FilterPanelState,
}

/// Fixes per track of the test recordings, at one fix per second.
const FIXES_PER_TRACK: usize = 60;

fn utc(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, hour, minute, second)
        .single()
        .expect("one instant on a day with no clock change")
}

/// A recording holding one track per entry of `tracks`, each of `count` fixes
/// one second apart from its own start.
///
/// Entries further apart than five minutes become tracks of their own, in the
/// order they are written here, not sorted into time order: segmentation
/// splits on a timestamp step of five minutes in either direction.
fn recording(name: &str, tracks: &[(DateTime<Utc>, usize)]) -> LoadedFile {
    let points: Vec<NavPoint> = tracks
        .iter()
        .flat_map(|(start, count)| gt_test_utils::nav_points_from(*start, *count, 1))
        .collect();
    gt_track_builder::build_loaded_file(
        name.to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from(name)),
        gt_track_builder::FileMeta::default(),
        vec![],
    )
}

fn harness(files: Vec<LoadedFile>) -> TestHarness<'static, PanelState> {
    let state = PanelState {
        files,
        filter: GlobalFilter::default(),
        panel: FilterPanelState::default(),
    };
    TestHarness::builder()
        .size(egui::vec2(320.0, 400.0))
        .ui_state(
            |ui, s: &mut PanelState| {
                let reset_requested =
                    render_filter_panel(ui, &s.files, &mut s.filter, &mut s.panel);
                assert!(!reset_requested, "no test here clicks Reset filters");
            },
            state,
        )
}

/// The rectangles of the time range bars on screen, topmost first.
///
/// A bar reaches the accessibility tree under [`Role::Unknown`]: it allocates
/// its rectangle with a sense and reports no widget info. Nothing else the
/// filter panel draws takes that role.
fn bar_rects(harness: &TestHarness<'static, PanelState>) -> Vec<egui::Rect> {
    let mut rects: Vec<egui::Rect> = harness
        .inner
        .query_all(By::new().role(Role::Unknown))
        .map(|node| node.rect())
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

/// The text of the one label of the panel containing `needle`.
fn label_text(harness: &TestHarness<'static, PanelState>, needle: &str) -> String {
    harness
        .inner
        .get(By::new().label_contains(needle).include_labels())
        .accesskit_node()
        .value()
        .unwrap_or_default()
}

/// A recording whose fixes step back an hour and a half in the middle, which
/// segmentation splits into a later track followed by an earlier one.
///
/// Its fixes run from 11:00:00 to 12:30:00, a span of an hour and a half.
fn recording_whose_clock_steps_backwards() -> LoadedFile {
    recording(
        "clock_step.gtd",
        &[(utc(12, 29, 51), 10), (utc(11, 0, 0), 10)],
    )
}

#[test]
fn the_time_range_heading_states_the_span_of_a_recording_whose_clock_steps_backwards() {
    let mut harness = harness(vec![recording_whose_clock_steps_backwards()]);
    harness.run();
    assert_eq!(label_text(&harness, "Time range"), "Time range — 1h30m");
}

#[test]
fn a_recording_whose_clock_steps_backwards_has_a_time_range_bar() {
    let mut harness = harness(vec![recording_whose_clock_steps_backwards()]);
    harness.run();
    assert_eq!(
        bar_rects(&harness).len(),
        1,
        "the panel offers one bar over the whole loaded time range"
    );
}

/// A recording of no fixes at all holds no track and covers no span. The bar
/// spans the recordings that hold fixes.
#[test]
fn a_recording_with_no_fixes_stays_out_of_the_time_range_bar() {
    let files = vec![
        recording("morning.gtd", &[(utc(0, 0, 0), FIXES_PER_TRACK)]),
        recording("no_fixes.gtd", &[]),
    ];
    let mut harness = harness(files);
    harness.run();
    assert_eq!(label_text(&harness, "Time range"), "Time range — 59s");
}

/// The active range lies past the end of the range the bars are laid out over,
/// because one recording's tracks are not in time order.
#[test]
fn the_active_range_heading_comes_with_a_bar_under_it() {
    let files = vec![
        recording("morning.gtd", &[(utc(0, 0, 0), FIXES_PER_TRACK)]),
        recording(
            "clock_step.gtd",
            &[
                (utc(12, 0, 0), FIXES_PER_TRACK),
                (utc(11, 0, 0), FIXES_PER_TRACK),
            ],
        ),
    ];
    let mut harness = harness(files);
    harness.state_mut().filter.time_start = Some(utc(11, 30, 0));
    harness.run();
    assert_eq!(
        bar_rects(&harness).len(),
        2,
        "the active range heading reads {:?}",
        label_text(&harness, "Active range")
    );
}
