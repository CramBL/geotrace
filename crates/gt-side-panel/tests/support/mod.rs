//! Shared fixture construction for the filter panel's time range bar test
//! binaries: the recordings they load, the harness that draws the panel over
//! them, and the readouts they assert on.
//!
//! The panel lays out a heading, the bar under it, and a label of the window's
//! two ends, twice: once over the whole loaded time range, and once over the
//! active range when that range is far narrower than the whole.

#![allow(dead_code, reason = "shared across binaries with different needs")]
#![expect(
    clippy::expect_used,
    reason = "the helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use std::path::PathBuf;

use chrono::{DateTime, TimeZone as _, Utc};
use egui::accesskit::Role;
use egui_kittest::kittest::NodeT as _;
use gt_filter::GlobalFilter;
use gt_side_panel::filter::TRACK_INSET_PX;
use gt_side_panel::{FilterPanelState, render_filter_panel};
use gt_test_utils::{By, Queryable as _, TestHarness};
use gt_types::{LoadedFile, NavPoint};

/// The state the filter panel reads and writes.
pub struct PanelState {
    pub files: Vec<LoadedFile>,
    pub filter: GlobalFilter,
    pub panel: FilterPanelState,
}

/// Fixes per track of the test recordings, at one fix per second.
pub const FIXES_PER_TRACK: usize = 60;

pub fn utc(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
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
pub fn recording(name: &str, tracks: &[(DateTime<Utc>, usize)]) -> LoadedFile {
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

pub fn harness(files: Vec<LoadedFile>) -> TestHarness<'static, PanelState> {
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
/// its rectangle with a sense and leaves the widget info unset. Nothing else
/// the filter panel draws takes that role.
pub fn bar_rects(harness: &TestHarness<'static, PanelState>) -> Vec<egui::Rect> {
    let mut rects: Vec<egui::Rect> = harness
        .inner
        .query_all(By::new().role(Role::Unknown))
        .map(|node| node.rect())
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

/// The text of the one label of the panel containing `needle`.
pub fn label_text(harness: &TestHarness<'static, PanelState>, needle: &str) -> String {
    harness
        .inner
        .get(By::new().label_contains(needle).include_labels())
        .accesskit_node()
        .value()
        .unwrap_or_default()
}

/// The texts of every label of the panel containing `needle`, topmost first.
pub fn label_texts(harness: &TestHarness<'static, PanelState>, needle: &str) -> Vec<String> {
    let mut labels: Vec<(f32, String)> = harness
        .inner
        .query_all(By::new().label_contains(needle).include_labels())
        .map(|node| {
            (
                node.rect().top(),
                node.accesskit_node().value().unwrap_or_default(),
            )
        })
        .collect();
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    labels.into_iter().map(|(_, text)| text).collect()
}

/// A point on the bar `fraction` of the way along its track, which spans the
/// bar inset by [`TRACK_INSET_PX`] at either end.
pub fn point_on_track(bar: egui::Rect, fraction: f32) -> egui::Pos2 {
    let left = bar.left() + TRACK_INSET_PX;
    let width = bar.width() - 2.0 * TRACK_INSET_PX;
    egui::pos2(left + fraction * width, bar.center().y)
}
