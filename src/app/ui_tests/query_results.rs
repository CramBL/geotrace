use std::collections::BTreeMap;
use std::ops::Range;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use egui_phosphor::regular::COPY as ICON_COPY;
use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use gt_jam::wire::HexObservation;
use gt_store::{EnvironmentArchive, JamStore};
use gt_test_utils::{By, DEMO_BYTES, HarnessInteraction as _, TestHarness};
use gt_types::{DataCategory, FileIdx, TrackIdx, TrackRef};
use gt_ui_theme::MIDDLE_DOT;
use gt_ui_types::{DataPointRef, DisplayCategory, MapScope, PointVisibility};
use rstest::rstest;

use crate::app::App;
use crate::app::environment_storage;
use crate::app::query;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests::{self, ACCEL_HIGH_RANGES, MANY_MATCH_QUERY, QUERY_WINDOW_TITLE};

/// Query results gray out when the data they were computed from changes -
/// here via a global-filter edit - and recover when it changes back.
#[test]
fn query_results_go_stale_when_the_filter_changes() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    let stale_after_run = harness
        .state()
        .query_window
        .matches()
        .expect("run produced matches")
        .stale;
    assert!(!stale_after_run, "fresh results are not stale");

    // A minimum-distance filter changes the evaluated track set.
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .min_distance_km = Some(uom::si::f64::Length::new::<uom::si::length::kilometer>(
        999.0,
    ));
    harness.run_steps(3);
    let matches_stale = harness
        .state()
        .query_window
        .matches()
        .expect("results kept while stale")
        .stale;
    assert!(matches_stale, "results gray out when the filter changes");

    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .min_distance_km = None;
    harness.run_steps(3);
    let stale_after_revert = harness
        .state()
        .query_window
        .matches()
        .expect("results kept")
        .stale;
    assert!(!stale_after_revert, "reverting the filter un-grays results");
}

/// The results gray out on a filter edit made with the query window closed,
/// since the map draws the last run's matches whether or not it is open.
#[test]
fn query_results_go_stale_with_the_query_window_closed() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    harness.state_mut().query_window.open = false;
    harness.run_steps(3);
    let stale_after_closing = harness
        .state()
        .query_window
        .matches()
        .expect("closing the window keeps the results")
        .stale;
    assert!(!stale_after_closing, "closing alone leaves results current");

    // A minimum-distance filter changes the evaluated track set.
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .min_distance_km = Some(uom::si::f64::Length::new::<uom::si::length::kilometer>(
        999.0,
    ));
    harness.run_steps(3);
    let matches_stale = harness
        .state()
        .query_window
        .matches()
        .expect("results kept while stale")
        .stale;
    assert!(
        matches_stale,
        "results gray out with the query window closed"
    );
}

/// Gives every loaded track interference query values: installs a scheduler
/// whose archive values the cell each loaded fix sits in.
fn install_interference_archive_covering_loaded_fixes(
    harness: &mut Harness<'_, App>,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<JamStore>()
        .expect("archive");
    let mut observations_by_day: BTreeMap<chrono::NaiveDate, Vec<HexObservation>> = BTreeMap::new();
    {
        let state = harness.state();
        let shared = state.shared.borrow();
        for file in shared.loaded_files.files() {
            for track in &file.tracks {
                for point in track.placed_points().into_iter().flat_map(|p| p.iter()) {
                    let (latitude, longitude) = point.resolved_position();
                    let Some(cell) = gt_jam::dataset::cell_at(latitude, longitude) else {
                        continue;
                    };
                    let observations = observations_by_day
                        .entry(point.fix.tpv.time().utc().date_naive())
                        .or_default();
                    if !observations
                        .iter()
                        .any(|observation| observation.cell == cell)
                    {
                        observations.push(HexObservation {
                            cell,
                            good: 90,
                            bad: 10,
                        });
                    }
                }
            }
        }
    }
    for (day, observations) in observations_by_day {
        environment_storage::archive_one_day(
            &store,
            EnvironmentArchive::AircraftInterference.day_insert_registration(day),
            |archive| archive.insert_day(day, "host", chrono::Utc::now(), &observations),
        );
    }
    ui_tests::install_interference_scheduler(harness, &store);
    dir
}

/// A run over a recording with archived interference stays live: the
/// per-track interference values the app hands the query fingerprint every
/// frame keep their `Arc` identity while the archive holds them.
#[test]
fn query_results_over_archived_interference_stay_fresh() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );
    let _archive = install_interference_archive_covering_loaded_fixes(&mut harness);

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h".to_owned());
    }
    harness.run_steps(3);
    assert!(
        !harness.state().jamming.query_values().is_empty(),
        "the loaded track must carry interference values for the fingerprint to compare"
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness);
    harness.run_steps(5);

    assert!(
        !harness
            .state()
            .query_window
            .matches()
            .expect("run produced matches")
            .stale,
        "its results stay live: nothing changed since the run"
    );
}

/// `snap_error` evaluates over a completed run's values without any network
/// step: points with a value match the draw query, and a re-snap grays the
/// results out through the fingerprint.
#[test]
fn query_matches_on_snap_error_after_a_run() {
    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    test_util::snap::inject_completed_run(&mut harness, track);

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where snap_error >= 2 m | draw".to_owned());
    }
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness);
    harness.run_steps(3);
    let matches = harness
        .state()
        .query_window
        .matches()
        .expect("run produced results");
    assert!(!matches.stale, "fresh results are not stale");
    assert!(
        matches
            .draws
            .first()
            .is_some_and(|layer| !layer.ranges.is_empty()),
        "snapped points must match the snap_error draw"
    );

    // A re-snap produces new values: the results gray out.
    test_util::snap::inject_completed_run(&mut harness, track);
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .query_window
            .matches()
            .expect("results kept while stale")
            .stale,
        "a new run must gray snap_error results out"
    );
}

/// Clicking a point row pins that point, like a point row in the side panel:
/// the map then owns a pinned popup for it.
#[test]
fn query_point_row_click_pins_its_point() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    harness.run_steps(3);

    let first_row = topmost_point_row(&harness).0.center();
    harness.press_drag_release(first_row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);

    let sticky = harness
        .state()
        .shared
        .borrow()
        .highlight
        .sticky
        .expect("the row click pins its point");
    assert_eq!(
        sticky.track,
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    );
    assert_eq!(sticky.category, gt_types::DataCategory::Tpv);
}

/// What the map does with the fix the plot-hover cross-highlight names, judged
/// by the map's own hit-test. `None` when the highlight has no fix.
fn highlighted_fix_visibility(harness: &Harness<'_, App>) -> Option<PointVisibility> {
    let app = harness.state();
    let shared = app.shared.borrow();
    let (fi, ti, pi) = shared.highlight.plot_hover_point?;
    let scope = MapScope {
        files: shared.loaded_files.files(),
        visibility: shared.tree.visibility(),
        filter: &shared.filter,
        display_mask: shared.display_mask,
        query_matches: app.query_window.matches(),
    };
    Some(scope.point_visibility(DataPointRef {
        track: TrackRef::new(fi, ti),
        category: DataCategory::Tpv,
        point_index: pi,
    }))
}

/// A point `fraction_across` of the way from the plot pane's left edge to its
/// right, at half its height. The pane holds the whole recording's span.
fn plot_pane_point(harness: &Harness<'_, App>, fraction_across: f32) -> egui::Pos2 {
    let app = harness.state();
    let rect = app
        .tiles_tree
        .tiles
        .rect(app.plot_tile_id)
        .expect("the plot pane is laid out");
    egui::pos2(
        rect.left() + rect.width() * fraction_across,
        rect.center().y,
    )
}

const PLOT_PANE_MIDDLE: f32 = 0.5;

/// Well inside the later half of the span, which
/// [`the_plot_cursor_over_a_hidden_stretch_reaches_for_no_drawn_fix`] hides.
const PLOT_PANE_LATE: f32 = 0.9;

/// Frames a cross-highlight takes to reach the map after the pointer moved.
const HIGHLIGHT_SETTLE_FRAMES: usize = 2;

/// The guard the row cases below rest on: hovering a point row cross-highlights
/// that row's fix, which the map draws.
#[test]
fn a_hovered_point_row_highlights_its_own_fix() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");

    let first_row = topmost_point_row(&harness).0.center();
    harness.hover_at_and_settle(first_row, HIGHLIGHT_SETTLE_FRAMES);

    assert_eq!(
        highlighted_fix_visibility(&harness),
        Some(PointVisibility::Shown)
    );
}

#[rstest]
#[case::a_query_hid_the_fix("points | where velocity > 1 km/h | hide", |_: &Harness<'_, App>| {})]
#[case::the_display_mask_hides_the_track_points(
    "points | where velocity > 1 km/h",
    |harness: &Harness<'_, App>| {
        harness
            .state()
            .shared
            .borrow_mut()
            .display_mask
            .set_visible(DisplayCategory::TrackPoints, false);
    }
)]
fn a_hovered_point_row_highlights_no_fix_the_map_leaves_out(
    #[case] query: &str,
    #[case] leave_the_fix_out: fn(&Harness<'_, App>),
) {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, query);
    leave_the_fix_out(&harness);
    harness.run_steps(3);

    let first_row = topmost_point_row(&harness).0.center();
    harness.hover_at_and_settle(first_row, HIGHLIGHT_SETTLE_FRAMES);

    assert_eq!(highlighted_fix_visibility(&harness), None);
}

/// The guard the plot-cursor cases below rest on: the cursor cross-highlights
/// the fix it is over, which the map draws.
#[test]
fn the_plot_cursor_highlights_the_fix_it_is_over() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.run_steps(3);

    let middle = plot_pane_point(&harness, PLOT_PANE_MIDDLE);
    harness.hover_at_and_settle(middle, HIGHLIGHT_SETTLE_FRAMES);

    assert_eq!(
        highlighted_fix_visibility(&harness),
        Some(PointVisibility::Shown)
    );
}

#[rstest]
#[case::a_query_hid_the_fixes(|harness: &mut Harness<'static, App>| {
    ui_tests::run_query(harness, "points | where velocity > 1 km/h | hide");
})]
#[case::the_display_mask_hides_the_track_points(|harness: &mut Harness<'static, App>| {
    harness
        .state()
        .shared
        .borrow_mut()
        .display_mask
        .set_visible(DisplayCategory::TrackPoints, false);
})]
#[case::the_track_hides_its_fixes(|harness: &mut Harness<'static, App>| {
    harness
        .state()
        .shared
        .borrow_mut()
        .tree
        .set_category_visible(
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            DataCategory::Tpv,
            false,
        );
})]
fn the_plot_cursor_over_fixes_the_map_leaves_out_highlights_nothing(
    #[case] leave_the_fixes_out: fn(&mut Harness<'static, App>),
) {
    let mut harness = ui_tests::app_with_query_window_open();
    leave_the_fixes_out(&mut harness);
    // The query window sits over the plot pane the cursor has to reach.
    harness.state_mut().query_window.open = false;
    harness.run_steps(3);

    let middle = plot_pane_point(&harness, PLOT_PANE_MIDDLE);
    harness.hover_at_and_settle(middle, HIGHLIGHT_SETTLE_FRAMES);

    assert_eq!(highlighted_fix_visibility(&harness), None);
}

/// The query below hides the later half of the span and leaves the earlier half
/// drawn: the recording's fixes climb north by 0.0002 degrees each.
#[test]
fn the_plot_cursor_over_a_hidden_stretch_reaches_for_no_drawn_fix() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where lat > 51.506 deg | hide");
    harness.state_mut().query_window.open = false;
    harness.run_steps(3);

    let late = plot_pane_point(&harness, PLOT_PANE_LATE);
    harness.hover_at_and_settle(late, HIGHLIGHT_SETTLE_FRAMES);

    assert_eq!(highlighted_fix_visibility(&harness), None);
}

/// The matches table lists a track, times and counts, and the points table
/// stretches its striping across the window: neither may widen the window,
/// which would cover the map beside it.
#[test]
fn a_run_leaves_the_query_window_at_its_default_width() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    harness.run_steps(5);

    let width = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open")
        .width();
    assert!(
        width <= query::DEFAULT_WINDOW_WIDTH,
        "the results widened the window to {width}"
    );
}

/// Neither the results, a tab switch nor scrolling may make the window taller:
/// a window that claimed the height it could have would cover the plot below
/// it.
#[test]
fn the_results_leave_the_query_window_at_its_default_height() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let height_of = |harness: &Harness<'_, App>| {
        harness
            .window_rect(QUERY_WINDOW_TITLE)
            .expect("the query window is open")
            .height()
    };
    let with_results = height_of(&harness);
    assert!(
        with_results <= query::DEFAULT_WINDOW_HEIGHT,
        "the results grew the window to {with_results}"
    );

    harness.get_by_label("Examples").click();
    harness.run_steps(5);
    harness.get_by_label("Results").click();
    harness.run_steps(5);
    let after_tabs = height_of(&harness);
    assert!(
        after_tabs <= query::DEFAULT_WINDOW_HEIGHT,
        "switching tabs grew the window to {after_tabs}"
    );

    let rows = topmost_point_row(&harness).0.center();
    harness.scroll_wheel_at(rows, -RESULTS_WHEEL_POINTS, WHEEL_SETTLE_FRAMES);
    let after_scrolling = height_of(&harness);
    assert!(
        after_scrolling <= query::DEFAULT_WINDOW_HEIGHT,
        "scrolling the rows grew the window to {after_scrolling}"
    );
}

/// The demo recording loaded with `query` run over it, for the tests that
/// drive the results table. Its track matches in several stretches, which one
/// match table then lists.
fn demo_app_with_query_run(query: &str) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    ui_tests::run_query(&mut harness, query);
    harness
}

/// A demo-trip query with two matches of clearly different length, for the
/// tests that pick, sort and frame one of them.
const TWO_MATCH_QUERY: &str = "points | window 10 | where avg(velocity) > 25 km/h";

/// Wheel points sent over the results, past the rows one viewport holds.
const RESULTS_WHEEL_POINTS: f32 = 200.0;

/// Frames a wheel scroll's smooth animation takes to come to rest.
const WHEEL_SETTLE_FRAMES: usize = 12;

/// Motion below which a widget counts as having stayed where it was.
const STATIONARY_TOLERANCE_PX: f32 = 0.5;

/// Length of the bare wall-clock label a point row's time column states.
const ROW_TIME_LEN: usize = "14:00:19".len();

/// Every widget labeled with a bare wall-clock time: the start and end columns
/// of the matches table, and the time column of the points table. A channel
/// run's samples are timed to the millisecond, so the time carries a fraction
/// there.
fn bare_time_labels<'a>() -> By<'a> {
    By::new().role(egui::accesskit::Role::Label).predicate(
        |node: &egui_kittest::kittest::AccessKitNode<'_>| {
            node.value().is_some_and(|value| {
                value.len() >= ROW_TIME_LEN
                    && value.contains(':')
                    && value
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
            })
        },
    )
}

/// The rows one of the results tab's two tables lists, top row first: the rect
/// of the row's first time cell and the time it states.
///
/// The caption stating the picked match starts at the left edge of the tab, and
/// so does the points table's time column. The matches table indents its times
/// behind the swatch, number and track columns.
fn time_rows(harness: &Harness<'_, App>, table: ResultsTable) -> Vec<(egui::Rect, String)> {
    let left_edge = harness.get_by_label_contains("Match ").rect().left();
    rows_in_reading_order(
        time_cells_in_window(harness, QUERY_WINDOW_TITLE)
            .into_iter()
            .filter(|(rect, _)| {
                let indented = rect.left() > left_edge + INDENT_TOLERANCE_PX;
                match table {
                    ResultsTable::Matches => indented,
                    ResultsTable::Points => !indented,
                }
            })
            .collect(),
    )
}

/// The rows the matches table lists in the window it was popped out into, top
/// row first. Every time that window states belongs to a match row.
fn popped_out_match_rows(harness: &Harness<'_, App>) -> Vec<(egui::Rect, String)> {
    rows_in_reading_order(time_cells_in_window(
        harness,
        query::results::MATCH_LIST_WINDOW_TITLE,
    ))
}

/// Every bare wall-clock label the window titled `title` states, with the rect
/// it was laid out in.
fn time_cells_in_window(harness: &Harness<'_, App>, title: &str) -> Vec<(egui::Rect, String)> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .query_all(bare_time_labels())
        .filter_map(|node| Some((node.rect(), node.accesskit_node().value()?)))
        .collect()
}

/// `cells` in reading order: down the rows, and left to right within a row - a
/// match row states a start and an end, of which the start is kept.
fn rows_in_reading_order(mut cells: Vec<(egui::Rect, String)>) -> Vec<(egui::Rect, String)> {
    cells.sort_by(|(a, _), (b, _)| {
        a.top()
            .total_cmp(&b.top())
            .then_with(|| a.left().total_cmp(&b.left()))
    });
    cells.dedup_by(|(a, _), (b, _)| (a.top() - b.top()).abs() < INDENT_TOLERANCE_PX);
    cells
}

/// Every button framing the map that the window titled `title` shows, top to
/// bottom: the run-wide button first, then one button per listed match. The
/// summary strip leads whichever window holds the matches list.
fn map_buttons_in_window<'h>(
    harness: &'h Harness<'_, App>,
    title: &'h str,
) -> Vec<egui_kittest::Node<'h>> {
    let mut buttons: Vec<_> = harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .query_all_by_role_and_label(egui::accesskit::Role::Button, ICON_CROSSHAIR)
        .collect();
    buttons.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    buttons
}

/// The matches listed in the window titled `title`, counted by the button each
/// row carries to frame the map on its match. A window without the list shows
/// neither those buttons nor the strip's run-wide one.
fn listed_match_count(harness: &Harness<'_, App>, title: &str) -> usize {
    map_buttons_in_window(harness, title)
        .len()
        .saturating_sub(1)
}

/// The summary strip's button, which frames the map on every match of the run.
fn run_wide_map_button<'h>(harness: &'h Harness<'_, App>) -> egui_kittest::Node<'h> {
    *map_buttons_in_window(harness, QUERY_WINDOW_TITLE)
        .first()
        .expect("the results tab states what the run matched")
}

/// The button match row `index` carries to frame the map on that one match.
fn match_row_map_button<'h>(harness: &'h Harness<'_, App>, index: usize) -> egui_kittest::Node<'h> {
    *map_buttons_in_window(harness, QUERY_WINDOW_TITLE)
        .get(index + 1)
        .expect("the results tab lists that match")
}

/// How far a cell may sit from the tab's left edge and still count as starting
/// there, and how far two cells' tops may differ and still be one row.
const INDENT_TOLERANCE_PX: f32 = 2.0;

/// Which of the results tab's two tables a test reads.
#[derive(Clone, Copy)]
enum ResultsTable {
    Matches,
    Points,
}

/// The topmost point row's time cell and the time it states.
fn topmost_point_row(harness: &Harness<'_, App>) -> (egui::Rect, String) {
    time_rows(harness, ResultsTable::Points)
        .into_iter()
        .next()
        .expect("the results list point rows")
}

/// The time stated by the topmost point row on display.
fn topmost_row_time(harness: &Harness<'_, App>) -> String {
    topmost_point_row(harness).1
}

/// The start time of the topmost match row on display.
fn topmost_match_row(harness: &Harness<'_, App>) -> (egui::Rect, String) {
    time_rows(harness, ResultsTable::Matches)
        .into_iter()
        .next()
        .expect("the results list matches")
}

/// The wheel over the points table scrolls its rows, and neither the matches
/// table above it nor the editor above the tab strip moves with them: the two
/// tables scroll on their own.
#[test]
fn the_wheel_over_the_results_scrolls_its_rows() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let run_button_top = |harness: &Harness<'_, App>| {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
            .rect()
            .top()
    };
    let first_time = topmost_row_time(&harness);
    let run_button_before = run_button_top(&harness);
    let first_match_before = topmost_match_row(&harness).0.top();

    let rows = topmost_point_row(&harness).0.center();
    harness.scroll_wheel_at(rows, -RESULTS_WHEEL_POINTS, WHEEL_SETTLE_FRAMES);

    let scrolled_time = topmost_row_time(&harness);
    assert!(
        scrolled_time > first_time,
        "the wheel scrolled to later rows: {first_time} then {scrolled_time}"
    );
    let editor_shift = (run_button_top(&harness) - run_button_before).abs();
    assert!(
        editor_shift < STATIONARY_TOLERANCE_PX,
        "the editor above the tabs moved {editor_shift} px with the rows"
    );
    let match_shift = (topmost_match_row(&harness).0.top() - first_match_before).abs();
    assert!(
        match_shift < STATIONARY_TOLERANCE_PX,
        "the matches table moved {match_shift} px with the point rows"
    );
}

/// A run with more matches than the matches table shows at once scrolls them
/// under its own header, leaving the points table below it where it is.
#[test]
fn the_wheel_over_the_matches_scrolls_only_them() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    let first_match = topmost_match_row(&harness);
    let first_point_before = topmost_point_row(&harness);

    harness.scroll_wheel_at(
        first_match.0.center(),
        -RESULTS_WHEEL_POINTS,
        WHEEL_SETTLE_FRAMES,
    );

    let scrolled_match = topmost_match_row(&harness).1;
    assert!(
        scrolled_match > first_match.1,
        "the wheel scrolled to later matches: {} then {scrolled_match}",
        first_match.1
    );
    assert_eq!(
        topmost_point_row(&harness).1,
        first_point_before.1,
        "the points of the picked match stay where they are"
    );
}

/// Pixels the splitter is dragged to give the matches table the height of the
/// points table below it, and the far longer drag that runs into either clamp.
const SPLITTER_DRAG_PX: f32 = 120.0;
const SPLITTER_DRAG_PAST_THE_CLAMP_PX: f32 = 400.0;

/// The centre of the splitter band, read again after every drag: it sits where
/// the matches table now ends.
fn splitter_center(harness: &Harness<'_, App>) -> egui::Pos2 {
    harness
        .get_by_label(query::results::SPLITTER_LABEL)
        .rect()
        .center()
}

/// Dragging the splitter down gives the matches table the height the points
/// table had, listing the matches that were scrolled out of it. The window it
/// is in keeps its size: the splitter only divides what the tab already has.
#[test]
fn dragging_the_splitter_lists_more_matches() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    let listed = |harness: &Harness<'_, App>| {
        time_rows(harness, ResultsTable::Matches)
            .into_iter()
            .map(|(_, time)| time)
            .collect::<Vec<_>>()
    };
    let before = listed(&harness);
    let window_before = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open")
        .size();

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, SPLITTER_DRAG_PX),
        4,
    );
    harness.run_steps(3);

    let after = listed(&harness);
    assert!(
        after.len() > before.len(),
        "the drag listed more matches: {before:?} then {after:?}"
    );
    assert!(
        after.starts_with(&before),
        "the matches already listed stayed where they were: {before:?} then {after:?}"
    );
    let window_after = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open")
        .size();
    assert!(
        (window_after - window_before).length() < STATIONARY_TOLERANCE_PX,
        "the drag resized the window from {window_before:?} to {window_after:?}"
    );
}

/// A double-click on the splitter puts the boundary back where the tab opened
/// it.
#[test]
fn double_clicking_the_splitter_puts_the_matches_table_back() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    let default_rows = time_rows(&harness, ResultsTable::Matches).len();

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, SPLITTER_DRAG_PX),
        4,
    );
    harness.run_steps(3);
    let dragged_rows = time_rows(&harness, ResultsTable::Matches).len();
    assert!(
        dragged_rows > default_rows,
        "the drag listed more matches: {default_rows} then {dragged_rows}"
    );

    let splitter = splitter_center(&harness);
    harness.double_click_at(splitter);
    harness.run_steps(3);

    assert_eq!(
        time_rows(&harness, ResultsTable::Matches).len(),
        default_rows,
        "the double-click listed the matches the tab opened with again"
    );
}

/// However far the splitter is dragged, both tables keep rows on display:
/// neither can be collapsed to its header.
#[test]
fn the_splitter_keeps_rows_of_both_tables_on_display() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, -SPLITTER_DRAG_PAST_THE_CLAMP_PX),
        4,
    );
    harness.run_steps(3);
    let matches_left = time_rows(&harness, ResultsTable::Matches).len();
    assert!(
        matches_left >= query::results_split::MIN_SPLIT_ROWS,
        "dragging to the top left {matches_left} matches on display"
    );

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, SPLITTER_DRAG_PAST_THE_CLAMP_PX),
        4,
    );
    harness.run_steps(3);
    let points_left = time_rows(&harness, ResultsTable::Points).len();
    assert!(
        points_left >= query::results_split::MIN_SPLIT_ROWS,
        "dragging to the bottom left {points_left} point rows on display"
    );
}

/// The pop-out button moves the matches into a window of their own, leaving the
/// results tab to the picked match's rows. Closing that window puts them back.
#[test]
fn popping_the_matches_out_moves_them_into_their_own_window() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let matches = listed_match_count(&harness, QUERY_WINDOW_TITLE);
    assert_eq!(matches, 2, "the results tab lists the run's matches");

    ui_tests::pop_out_button(&harness).click();
    harness.run_steps(5);

    assert_eq!(
        listed_match_count(&harness, query::results::MATCH_LIST_WINDOW_TITLE),
        matches,
        "every match moved into the popped-out window"
    );
    assert_eq!(
        listed_match_count(&harness, QUERY_WINDOW_TITLE),
        0,
        "the results tab lists none of them any more"
    );
    assert_eq!(
        harness.query_all_by_label_contains("Match 1 ").count(),
        1,
        "the caption over the point rows stays in the results tab"
    );

    harness
        .get_by_role_and_label(
            egui::accesskit::Role::Window,
            query::results::MATCH_LIST_WINDOW_TITLE,
        )
        .get_by_label("Close window")
        .click();
    harness.run_steps(5);

    assert_eq!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Window,
                query::results::MATCH_LIST_WINDOW_TITLE
            )
            .count(),
        0,
        "closing the window took it off the screen"
    );
    assert_eq!(
        listed_match_count(&harness, QUERY_WINDOW_TITLE),
        matches,
        "the matches are listed in the results tab again"
    );
}

/// The popped-out window keeps the size it opened at however long it stays on
/// screen: a table claiming the height it could have would grow it by one
/// spacing every frame.
#[test]
fn the_popped_out_matches_window_keeps_its_default_size() {
    // More matches than the window can list, so its table fills the height it was
    // given.
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    ui_tests::pop_out_button(&harness).click();
    harness.run_steps(5);

    let size_of = |harness: &Harness<'_, App>| {
        harness
            .window_rect(query::results::MATCH_LIST_WINDOW_TITLE)
            .expect("the matches list opened its own window")
            .size()
    };
    let opened = size_of(&harness);
    assert!(
        opened.x <= query::results::MATCH_LIST_WINDOW_WIDTH
            && opened.y <= query::results::MATCH_LIST_WINDOW_HEIGHT,
        "the matches window opened at {opened:?}"
    );

    harness.run_steps(30);
    let settled = size_of(&harness);
    assert!(
        (settled - opened).length() < STATIONARY_TOLERANCE_PX,
        "the matches window grew from {opened:?} to {settled:?}"
    );
}

/// The popped-out window and the results tab share what the tab kept: a match
/// picked in the window lists its rows in the query window.
#[test]
fn a_match_picked_in_the_popped_out_window_lists_its_points_in_the_query_window() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let first_point = topmost_row_time(&harness);

    ui_tests::pop_out_button(&harness).click();
    harness.run_steps(5);

    let second_match = popped_out_match_rows(&harness)
        .get(1)
        .map(|(rect, _)| rect.center())
        .expect("the popped-out window lists a second match");
    harness.press_drag_release(second_match, egui::Vec2::ZERO, 1);
    harness.run_steps(3);

    assert_eq!(
        harness.query_all_by_label_contains("Match 2 ").count(),
        1,
        "the caption in the query window names the picked match"
    );
    let second_point = topmost_row_time(&harness);
    assert!(
        second_point > first_point,
        "the query window lists the second match's points: {first_point} then {second_point}"
    );
}

/// The tab strip shows one list at a time: the history tab replaces the
/// results, and the results tab shows them again.
#[test]
fn the_history_tab_replaces_the_results() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    assert!(
        !map_buttons_in_window(&harness, QUERY_WINDOW_TITLE).is_empty(),
        "the results tab opens on the run"
    );

    harness.get_by_label("Query history").click();
    harness.run_steps(3);
    assert!(
        map_buttons_in_window(&harness, QUERY_WINDOW_TITLE).is_empty(),
        "the history tab replaces the results"
    );
    assert_eq!(
        harness
            .query_all_by_label_contains("points | where velocity")
            .count(),
        1,
        "the history lists the query that ran"
    );

    harness.get_by_label("Results").click();
    harness.run_steps(3);
    assert!(
        !map_buttons_in_window(&harness, QUERY_WINDOW_TITLE).is_empty(),
        "the results tab shows them again"
    );
}

/// Neither scrolling the results nor switching tabs may widen the window over
/// the map: a tab's scroll area uses the width the window has, never more.
#[test]
fn scrolling_and_switching_tabs_leave_the_query_window_at_its_default_width() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let rows = topmost_point_row(&harness).0.center();
    harness.scroll_wheel_at(rows, -RESULTS_WHEEL_POINTS, WHEEL_SETTLE_FRAMES);

    let width_of = |harness: &Harness<'_, App>| {
        harness
            .window_rect(QUERY_WINDOW_TITLE)
            .expect("the query window is open")
            .width()
    };
    let scrolled = width_of(&harness);
    assert!(
        scrolled <= query::DEFAULT_WINDOW_WIDTH,
        "scrolling the results widened the window to {scrolled}"
    );

    harness.get_by_label("Examples").click();
    harness.run_steps(5);
    let switched = width_of(&harness);
    assert!(
        switched <= query::DEFAULT_WINDOW_WIDTH,
        "the examples tab widened the window to {switched}"
    );
}

/// Height a row and the gap under it take, as a tolerance on where the last
/// listed row ends.
const ROW_HEIGHT_ALLOWANCE: f32 = 40.0;

/// The points table reaches down to the bottom of the window: it takes what
/// the matches table above it leaves.
#[test]
fn the_results_fill_the_rest_of_the_window() {
    let harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let window = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open");
    let lowest_row = time_rows(&harness, ResultsTable::Points)
        .into_iter()
        .map(|(rect, _)| rect.bottom())
        .fold(f32::MIN, f32::max);

    assert!(
        window.bottom() - lowest_row < ROW_HEIGHT_ALLOWANCE,
        "the rows stop at {lowest_row} in a window ending at {}",
        window.bottom()
    );
}

/// Clicking a match lists its rows in the points table below, and the caption
/// there states the match on display.
#[test]
fn clicking_a_match_lists_its_points() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    assert_eq!(
        harness.query_all_by_label_contains("Match 1 ").count(),
        1,
        "the first match is picked until another one is"
    );
    let first_point = topmost_row_time(&harness);

    let second_match = time_rows(&harness, ResultsTable::Matches)
        .get(1)
        .map(|(rect, _)| rect.center())
        .expect("the run lists a second match");
    harness.press_drag_release(second_match, egui::Vec2::ZERO, 1);
    harness.run_steps(3);

    assert_eq!(
        harness.query_all_by_label_contains("Match 2 ").count(),
        1,
        "the caption names the picked match"
    );
    let second_point = topmost_row_time(&harness);
    assert!(
        second_point > first_point,
        "the points table lists the second match: {first_point} then {second_point}"
    );
}

/// A click on a rendered column header reaches the sort. `MatchSort::clicked`
/// has its own tests for the sort order.
#[test]
fn a_column_header_click_sorts_the_matches() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);

    // The two matches of the run are told apart by how long each one ran. The
    // run lists the long match first. The first click keeps that order and the
    // second click reverses it.
    for _ in 0..2 {
        matches_sort_header(&harness, "points").click();
        harness.run_steps(3);
    }

    assert!(
        harness.get_by_label("0:11").rect().top() < harness.get_by_label("1:01").rect().top(),
        "the clicks on the points header did not reach the sort"
    );
}

/// The header of the matches table that sorts the list by `title`'s column.
fn matches_sort_header<'h>(
    harness: &'h Harness<'_, App>,
    title: &'h str,
) -> egui_kittest::Node<'h> {
    harness.get_by_role_and_label(egui::accesskit::Role::Button, title)
}

/// Space activates the focused sort header the way a click on it does.
#[test]
fn space_on_a_focused_column_header_sorts_the_matches() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    matches_sort_header(&harness, "points").focus();
    harness.run_steps(1);

    // The first activation sorts largest first, the order the run already
    // lists in, and the second reverses it.
    for _ in 0..2 {
        harness
            .input_mut()
            .events
            .push(ui_tests::key_press(egui::Key::Space));
        harness.run_steps(3);
    }

    assert!(
        harness.get_by_label("0:11").rect().top() < harness.get_by_label("1:01").rect().top(),
        "the short match is listed above the long one"
    );
}

/// A sort header requests the pointing hand on hover.
#[test]
fn a_column_header_requests_the_pointing_hand() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);

    let header = matches_sort_header(&harness, "points").rect().center();
    harness.hover_at_and_settle(header, 5);

    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
}

/// A match's own map button frames the map on that one match, tighter than the
/// run-wide button frames every match of the run.
#[test]
fn a_match_row_frames_the_map_on_that_match() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);

    run_wide_map_button(&harness).click();
    harness.run_steps(3);
    let framed_run = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the run's matches");

    // The second match is the shorter of the two, so framing it narrows the
    // viewport whichever way the first one framed.
    match_row_map_button(&harness, 1).click();
    harness.run_steps(3);
    let framed_match = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the one match");

    assert!(
        framed_match.lon_max - framed_match.lon_min < framed_run.lon_max - framed_run.lon_min,
        "one match frames tighter than the whole run: \
         {framed_match:?} against {framed_run:?}"
    );
}

/// Hovering a value column's header explains the metric it holds, out of the
/// same catalog the editor documents that metric from.
#[test]
fn hovering_a_column_header_explains_its_metric() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");

    let header = harness.get_by_label("velocity").rect().center();
    harness.hover_at_and_settle(header, 5);

    assert_eq!(
        harness.query_all_by_label_contains("ground speed").count(),
        1,
        "the header hover states what the metric measures"
    );
}

/// Every line of the results strip requests the cursor that matches what it
/// does: a line that is neither text entry nor a control keeps the default
/// cursor.
#[rstest::rstest]
// The summary states what the run left out on hover.
#[case::run_summary(demo_query_run, "2 matches", egui::CursorIcon::Help)]
// The caption and the stale note explain nothing and do nothing on click.
#[case::match_caption(demo_query_run, "Match 1 ", egui::CursorIcon::Default)]
#[case::stale_note(
    stale_demo_query_run,
    "Data changed since this run",
    egui::CursorIcon::Default
)]
fn the_results_strip_requests_a_cursor_that_matches_what_each_line_does(
    #[case] harness_with_run: fn() -> Harness<'static, App>,
    #[case] label: &str,
    #[case] expected: egui::CursorIcon,
) {
    let mut harness = harness_with_run();

    let line = harness.get_by_label_contains(label).rect().center();
    harness.hover_at_and_settle(line, 5);

    assert_eq!(
        harness.output().platform_output.cursor_icon,
        expected,
        "hovering {label:?} should request {expected:?}"
    );
}

/// The demo recording with a two-match run over it, and a minimum-distance
/// filter set after that run to leave its results stale.
fn stale_demo_query_run() -> Harness<'static, App> {
    let mut harness = demo_query_run();
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .min_distance_km = Some(uom::si::f64::Length::new::<uom::si::length::kilometer>(
        999.0,
    ));
    harness.run_steps(3);
    harness
}

fn demo_query_run() -> Harness<'static, App> {
    demo_app_with_query_run(TWO_MATCH_QUERY)
}

/// "Copy as TSV" writes the whole run to the clipboard: a header line stating
/// each column in its unit, then one line per matched point.
#[test]
fn copying_a_query_result_writes_a_tab_separated_table() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h | draw");

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, ICON_COPY)
        .click();
    harness.run_steps(1);

    let copied = copied_text(&harness);
    let matches = harness
        .state()
        .query_window
        .matches()
        .expect("the run produced matches");
    let matched_points: usize = matches.draws.first().map_or(0, |layer| {
        layer.ranges.values().flatten().map(Range::len).sum()
    });
    let mut lines = copied.lines();
    assert_eq!(lines.next(), Some("match\tpoint\ttime\tvelocity (km/h)"));
    assert_eq!(lines.next(), Some("1\t0\t11:33:20\t22.0"));
    assert_eq!(
        copied.lines().count(),
        matched_points + 1,
        "one line per matched point, under the header"
    );
}

/// The copy follows the order the matches table lists: sorting the matches
/// smallest first copies the smaller match's rows ahead of the larger one's.
#[test]
fn copying_after_sorting_writes_the_matches_in_the_listed_order() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    // The first click on the points header sorts largest first, the second
    // smallest first.
    for _ in 0..2 {
        matches_sort_header(&harness, "points").click();
        harness.run_steps(3);
    }

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, ICON_COPY)
        .click();
    harness.run_steps(1);

    let copied = copied_text(&harness);
    let first_row = copied.lines().nth(1).expect("the copy lists rows");
    assert!(
        first_row.starts_with("2\t"),
        "the smaller match is copied first: {first_row}"
    );
}

/// The text the app last put on the clipboard.
fn copied_text(harness: &Harness<'_, App>) -> String {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyImage(_) => None,
        })
        .expect("nothing was copied")
}

/// The accel fixture with the query window open over it.
fn accel_app_with_query_window() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    harness
}

/// The accel fixture with a channel-source query run over it: two stretches of
/// matched samples for one table to list.
fn channel_app_with_query_run() -> Harness<'static, App> {
    let mut harness = accel_app_with_query_window();
    ui_tests::run_query(&mut harness, "@accel | where @accel.x > 1 g");
    harness
}

/// The x component the crafted stretches reach, as the listing prints it in the
/// g the fixture declares.
const ACCEL_HIGH_X_CELL: &str = "1.500";

/// The accel fixture with a points query whose table holds an aggregate over
/// the channel. Each match then has the samples that aggregate reduced under
/// it.
fn accel_points_query_with_aggregate() -> Harness<'static, App> {
    let mut harness = accel_app_with_query_window();
    ui_tests::run_query(
        &mut harness,
        "points | window 1 | where max(@accel.x) > 1 g | table time, max(@accel.x)",
    );
    harness
}

/// Open the samples listed under the picked match.
fn open_samples_listing(harness: &mut Harness<'_, App>) {
    harness
        .get_by_label_contains(query::aggregate_samples::TOGGLE_LABEL)
        .click();
    harness.run_steps(3);
}

/// A points match's aggregate reduced the channel samples inside the match's
/// time extent. The listing holds as many samples as the match holds points:
/// the fixture records one sample per fix.
#[test]
fn expanding_a_points_match_lists_the_samples_its_aggregate_reduced() {
    let mut harness = accel_points_query_with_aggregate();
    let stretch = ACCEL_HIGH_RANGES.first().expect("two crafted stretches");

    open_samples_listing(&mut harness);

    harness.get_by_label_contains(&format!("@accel {MIDDLE_DOT} {} samples", stretch.len()));
    assert!(
        harness
            .query_all_by_label_contains(ACCEL_HIGH_X_CELL)
            .count()
            > 0,
        "the listing states each sample's x component in the unit the track declared"
    );
}

/// The listing counts every sample the aggregate reduced and draws the rows it
/// has room for, leaving the rest to its scroll bar.
#[test]
fn a_listing_of_more_samples_than_it_shows_draws_only_the_rows_that_fit() {
    let mut harness = accel_points_query_with_aggregate();
    let stretch = ACCEL_HIGH_RANGES.first().expect("two crafted stretches");

    open_samples_listing(&mut harness);

    // egui lays out the row straddling the bottom edge of the scroll area
    // alongside the rows fully inside it.
    let drawn = harness
        .query_all_by_label_contains(ACCEL_HIGH_X_CELL)
        .count();
    assert!(
        (1..=query::aggregate_samples::VISIBLE_SAMPLE_ROWS + 1).contains(&drawn),
        "the listing drew {drawn} of the {} samples it counts",
        stretch.len()
    );
}

/// On a channel source the aggregate reduced the match's own rows of the
/// source timeline, and those are what the listing holds.
#[test]
fn expanding_a_channel_match_lists_the_samples_of_its_matched_rows() {
    let mut harness = accel_app_with_query_window();
    ui_tests::run_query(
        &mut harness,
        "@accel | window 1 | where max(@accel.x) > 1 g | table max(@accel.x)",
    );
    let stretch = ACCEL_HIGH_RANGES.first().expect("two crafted stretches");

    open_samples_listing(&mut harness);

    harness.get_by_label_contains(&format!("@accel {MIDDLE_DOT} {} samples", stretch.len()));
}

/// A query whose table has no aggregate over a channel has nothing to list,
/// and its control says so.
#[test]
fn a_match_whose_query_reduces_no_channel_states_why_it_lists_nothing() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let toggle = harness.get_by_label_contains(query::aggregate_samples::TOGGLE_LABEL);
    assert!(
        toggle.accesskit_node().is_disabled(),
        "a match with no channel aggregate lists no samples"
    );

    let center = toggle.rect().center();
    harness.hover_at_and_settle(center, 5);

    assert_eq!(
        harness
            .query_all_by_label_contains("No aggregate column of this query reduces a channel")
            .count(),
        1,
        "the disabled control states why"
    );
}

/// A channel run lists a match per stretch of matched samples, and picking one
/// lists its samples below.
#[test]
fn a_channel_run_lists_a_match_per_stretch_of_samples() {
    let mut harness = channel_app_with_query_run();
    for stretch in ACCEL_HIGH_RANGES {
        assert_eq!(
            harness
                .query_all_by_role_and_label(
                    egui::accesskit::Role::Label,
                    &stretch.len().to_string()
                )
                .count(),
            1,
            "every matched stretch states how many samples it holds"
        );
    }
    let first_sample = topmost_row_time(&harness);

    let second_match = time_rows(&harness, ResultsTable::Matches)
        .get(1)
        .map(|(rect, _)| rect.center())
        .expect("the run lists a second stretch");
    harness.press_drag_release(second_match, egui::Vec2::ZERO, 1);
    harness.run_steps(3);

    let second_sample = topmost_row_time(&harness);
    assert!(
        second_sample > first_sample,
        "the samples table lists the second stretch: {first_sample} then {second_sample}"
    );
}

/// "Copy as TSV" writes a channel run the way it writes a points query: a
/// header line stating each column in the unit the track declared, then one
/// line per matched sample.
#[test]
fn copying_a_channel_result_writes_a_tab_separated_sample_table() {
    let mut harness = channel_app_with_query_run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, ICON_COPY)
        .click();
    harness.run_steps(1);

    let copied = copied_text(&harness);
    let mut lines = copied.lines();
    assert_eq!(
        lines.next(),
        Some("match\tsample\ttime\tx (g)\ty (g)\tz (g)")
    );
    assert_eq!(
        lines.next(),
        Some("1\t60\t11:34:20.000\t1.500\t0.100\t0.980")
    );
    let samples: usize = ACCEL_HIGH_RANGES.iter().map(Range::len).sum();
    assert_eq!(
        copied.lines().count(),
        samples + 1,
        "one line per matched sample, under the header"
    );
}

/// A channel match's map button is grayed out, stating on hover that a sample
/// has no position of its own.
#[test]
fn a_channel_match_states_why_it_cannot_frame_the_map() {
    let mut harness = channel_app_with_query_run();
    let button = match_row_map_button(&harness, 0);
    assert!(
        button.accesskit_node().is_disabled(),
        "a sample range cannot frame the map"
    );

    let center = button.rect().center();
    harness.hover_at_and_settle(center, 5);
    assert_eq!(
        harness
            .query_all_by_label_contains("Channel samples have no position")
            .count(),
        1,
        "the disabled button states why"
    );
}

/// The query results' run-wide map button frames the map on what the run drew:
/// the viewport narrows from the whole recording to the matched stretches.
#[test]
fn the_run_wide_map_button_frames_the_query_matches() {
    let mut harness = demo_app_with_query_run("points | where velocity > 25 km/h");

    let whole_trip = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the loaded recording");

    run_wide_map_button(&harness).click();
    harness.run_steps(3);

    let framed_matches = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the matches");
    assert!(
        framed_matches.lon_max - framed_matches.lon_min < whole_trip.lon_max - whole_trip.lon_min,
        "the matched stretches frame tighter than the whole trip: \
         {framed_matches:?} against {whole_trip:?}"
    );
}

/// Hovering a row of the matches table cross-highlights the whole match: its
/// range lands in `hover_match` (the map halo band and the plot time band read
/// it) and the match's track gets hover focus.
#[test]
fn query_match_row_hover_highlights_the_match() {
    use gt_ui_types::HighlightScope;

    let gtd_bytes = ui_tests::minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h".to_owned());
    }
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness);
    harness.run_steps(3);

    let match_row = topmost_match_row(&harness).0.center();
    harness.hover_at(match_row);
    harness.run_steps(2);

    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let highlight = harness.state().shared.borrow().highlight;
    let hover_match = highlight.hover_match.expect("the row hover sets the match");
    assert_eq!(hover_match.track, track);
    assert!(
        hover_match.start < hover_match.end,
        "the hovered match covers a non-empty range"
    );
    assert_eq!(
        highlight.hover,
        Some(HighlightScope::Track(track)),
        "the match's track gets hover focus"
    );

    // Pointer off the row: the cross-highlight clears the next frame.
    harness.hover_at(egui::pos2(1.0, 1.0));
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .highlight
            .hover_match
            .is_none(),
        "the highlight clears when the pointer leaves the row"
    );
}

/// Snapshot of the query window end to end over the demo trip: highlighted
/// editor text, the tab strip on its results, a run whose matches draw as
/// halos on the map, the run summary, and the match table filling the window.
#[test]
fn snapshot_app_query_window() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window.set_text(
        "points\n| window 10\n| where avg(velocity) > 25 km/h # demo\n| table time, velocity"
            .to_owned(),
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    // The run executes on a worker thread, so step until its results land.
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    let match_count: usize = {
        let app = harness.inner.state();
        app.query_window.matches().map_or(0, |m| {
            m.draws
                .iter()
                .flat_map(|d| d.ranges.values())
                .map(Vec::len)
                .sum()
        })
    };
    assert!(match_count > 0, "the demo trip has stretches above 25 km/h");

    let history_len = harness.inner.state().query_window.history().len();
    assert_eq!(history_len, 1, "the run above is recorded in history");

    ui_tests::assert_the_capture_covers_the_map(&mut harness, "app_query_window");
    harness.snapshot_loose("app_query_window");
}

/// The same run with its matches popped out: the list fills a window of its
/// own and the results tab is left to the picked match's rows.
#[test]
fn snapshot_app_query_matches_window() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window.set_text(TWO_MATCH_QUERY.to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    ui_tests::pop_out_button(&harness.inner).click();
    harness.inner.run_steps(10);

    harness.snapshot_loose("app_query_matches_window");
}

/// The query editor under the light theme, so the syntax-highlight colours
/// (keywords, numbers, identifiers, comments) are verified on the white editor
/// background where the dark-tuned palette was unreadable. Focused on the
/// editor: it sets highlighted text but does not run the query.
#[test]
fn snapshot_app_query_editor_light() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    harness.inner.ctx.set_theme(egui::ThemePreference::Light);
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    // Exercises every token class: keywords, numeric literals, a unit,
    // identifiers, and a comment.
    app.query_window.set_text(
        "points\n| window 10\n| where avg(velocity) > 25 km/h # keep the fast bits\n| table time, velocity"
            .to_owned(),
    );
    harness.inner.run_steps(8);

    harness.snapshot_loose("app_query_editor_light");
}

/// Hovering a match header in the results table: the map draws the highlight
/// blue halo band over the matched stretch and the plot shades the match's
/// time span.
#[test]
fn snapshot_app_query_match_hover() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window
        .set_text("points | window 10 | where avg(velocity) > 25 km/h".to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // Hover the larger match's row. The cross-highlight lands on the map and
    // plot a frame later.
    let match_row = topmost_match_row(&harness.inner).0.center();
    harness.inner.hover_at(match_row);
    harness.inner.run_steps(10);

    let hover_match = harness.inner.state().shared.borrow().highlight.hover_match;
    assert!(
        hover_match.is_some(),
        "hovering the header cross-highlights the match"
    );

    harness.snapshot_loose("app_query_match_hover");
}

/// A channel-source query end to end: filtering on a vector channel's
/// component (`@accel.x`) runs standalone, the results list the matched
/// samples in one table (time plus each component) under a name row per
/// matched stretch, and the map halos the track segments those samples cover.
#[test]
fn snapshot_app_query_channel_source() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window
        .set_text("@accel | where @accel.x > 1 g".to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // The matches table lists a row per crafted stretch, the samples of the
    // first one fill the table below it, and the map draws halos over the
    // segments those samples cover.
    let first_stretch = ACCEL_HIGH_RANGES.first().expect("two crafted stretches");
    harness.inner.get_by_label_contains(&format!(
        "Match 1 {MIDDLE_DOT} {} samples",
        first_stretch.len()
    ));

    harness.snapshot_loose("app_query_channel_source");
}

/// A points-source query that references a channel: the window's time span
/// collects the `@accel` samples, `max(norm(@accel))` reduces them, and the
/// matches land as point ranges - the results table shows the query's metric
/// columns and the map halos the matched stretches. The mixed flow, distinct
/// from the standalone channel source above.
#[test]
fn snapshot_app_query_points_with_channel() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    // Hard-maneuver detection: the fixture's baseline norm sits near 1 g
    // (gravity on z), the crafted stretches reach ~1.8 g.
    app.query_window.set_text(
        "points | window 10 | where max(norm(@accel)) > 1.2 g | table time, velocity".to_owned(),
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // The high-accel stretches match as point ranges on the track.
    let matches = harness
        .inner
        .state()
        .query_window
        .matches()
        .expect("the run produced matches")
        .clone();
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    assert!(
        !matches.draws[0].ranges_for(track).is_empty(),
        "the matched windows halo the track"
    );

    // Park the pointer off the table so the hovered-match cross-highlight (its
    // own snapshot) does not blend into this one.
    harness.inner.hover_at(egui::pos2(1.0, 1.0));
    harness.inner.run_steps(5);

    harness.snapshot_loose("app_query_points_with_channel");
}

/// A match expanded to the samples behind its aggregate column: the results tab
/// lists the `@accel` samples the match's `max(@accel.x)` reduced between the
/// line stating the match and the match's own rows.
#[test]
fn snapshot_app_query_match_samples() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window.set_text(
        "points | window 1 | where max(@accel.x) > 1 g | table time, max(@accel.x)".to_owned(),
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    harness
        .inner
        .get_by_label_contains(query::aggregate_samples::TOGGLE_LABEL)
        .click();
    harness.inner.run_steps(5);

    // No hover text covers the listing with the pointer parked off the tab.
    harness.inner.hover_at(egui::pos2(1.0, 1.0));
    harness.inner.run_steps(5);

    harness.snapshot_loose("app_query_match_samples");
}

/// Several queries compose in one editor: a `hide` filter plus two colored
/// `draw` layers, evaluated in sequence over the demo trip.
#[test]
fn snapshot_query_pipeline() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        // Hide the slow stretches, then outline the fast and the very-fast
        // survivors in two colors.
        app.query_window.set_text(
            "points | where velocity < 20 km/h | hide\n\n\
             points | where velocity > 30 km/h | draw\n\n\
             points | where velocity > 80 km/h | draw"
                .to_owned(),
        );
    }
    harness.inner.run_steps(5);
    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    test_util::harness::step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    {
        let matches = harness
            .inner
            .state()
            .query_window
            .matches()
            .expect("the pipeline produced a result");
        assert!(
            !matches.hidden.is_empty(),
            "the hide query removed the slow points"
        );
        assert_eq!(matches.draws.len(), 2, "two draw queries, two halo layers");
    }

    harness.snapshot_loose("query_pipeline");
}
