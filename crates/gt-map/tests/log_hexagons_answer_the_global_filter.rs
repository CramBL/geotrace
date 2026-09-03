//! What the global filter keeps off the map's log-match layer: the hexagons of
//! a recording the filter rejects, the entries its time window hides, and the
//! count the display toggle states beside "Log matches".
//!
//! A log takes its positions from one recording: the global filter keeps or
//! hides that recording's hexagons the way it keeps or hides its fixes. The
//! log's own filter chips still select which lines match.

mod support;

use chrono::Duration;
use gt_filter::GlobalFilter;
use gt_map::display_counts::{DisplayCounts, SuppliedCounts};
use gt_types::LoadedFile;
use gt_ui_types::{
    DisplayCategory, EventMarkerVisibility, GeneratedMarkerVisibility, LogMatchGlyph, LogMatches,
    TrackDataVisibility,
};
use support::{
    FRAMES_TO_SETTLE, Frame, HeadlessMap, WALKING_STEP_DEGREES, a_log_over, a_recording_of, epoch,
    fix_position, matches_over, window_ending_at,
};

/// Longitude between consecutive fixes of a recording made in one spot, about
/// 6 cm. Every hexagon over such a recording collapses into one cluster.
const STANDING_STEP_DEGREES: f64 = 0.000_001;

/// Fixes of the recording the cases draw, one a minute apart.
const FIX_COUNT: usize = 30;

/// The map every case drives: `files` framed with no filter active over
/// `FRAMES_TO_SETTLE` frames, holding `matches`, with `filter` set after that.
///
/// The app narrows the window the same way, while the user is looking at the
/// track. The camera stays where those frames put it.
fn map_framed_on(
    files: &[LoadedFile],
    matches: LogMatches,
    filter: GlobalFilter,
) -> HeadlessMap<'_> {
    let mut map = HeadlessMap::new(files, GlobalFilter::default());
    map.set_log_matches(matches);
    for _ in 0..FRAMES_TO_SETTLE {
        map.draw(&Frame::default());
    }
    map.set_filter(filter);
    map
}

/// What the map published about the hexagon the pointer was on.
struct PointedGlyphs {
    hovered: Option<LogMatchGlyph>,
    clicked: Option<LogMatchGlyph>,
}

/// Shapes one frame paints over `files` under `filter`, with `matches` handed
/// to the map, after the frames that framed the recording unfiltered.
///
/// A layer that must draw nothing shows up as a difference against the same
/// frame without it: the count is the whole frame's.
fn shapes_with(files: &[LoadedFile], filter: GlobalFilter, matches: LogMatches) -> usize {
    map_framed_on(files, matches, filter).draw(&Frame::default())
}

/// Points at the hexagon sitting on the fix at `fix_index`, which the camera
/// is centred on, under `filter`, and clicks it.
///
/// The move and the click go in separate frames: egui reads a widget's hover
/// against the rect the previous frame laid out.
fn pointing_at_the_hexagon_on_fix(
    files: &[LoadedFile],
    fix_index: usize,
    filter: GlobalFilter,
    matches: LogMatches,
) -> PointedGlyphs {
    let target = support::viewport_center();
    let mut map = map_framed_on(files, matches, filter);
    map.center_on(fix_position(files, fix_index));
    map.draw(&Frame {
        events: vec![egui::Event::PointerMoved(target)],
        ..Frame::default()
    });
    map.draw(&Frame::default());
    let hovered = map.hovered_log_glyph();
    map.draw(&Frame {
        events: [true, false]
            .into_iter()
            .map(|pressed| egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            })
            .collect(),
        ..Frame::default()
    });
    PointedGlyphs {
        hovered,
        clicked: map.clicked_log_glyph(),
    }
}

/// A recording the filter rejects puts nothing on the map, and the hexagons of
/// the log anchored to it are part of that nothing: they sit on its fixes.
#[rstest::rstest]
#[case::the_time_window_is_disjoint_from_the_recording(GlobalFilter {
    time_start: Some(epoch() + Duration::hours(5)),
    ..GlobalFilter::default()
})]
#[case::the_recording_is_shorter_than_the_minimum_duration(GlobalFilter {
    min_duration: Some(Duration::hours(5)),
    ..GlobalFilter::default()
})]
fn no_log_hexagon_is_drawn_for_a_recording_the_filter_rejects(#[case] filter: GlobalFilter) {
    let files = a_recording_of(FIX_COUNT, STANDING_STEP_DEGREES);
    let log = a_log_over(&files);

    assert_eq!(
        shapes_with(&files, filter, matches_over(&files, &log, 0..FIX_COUNT)),
        shapes_with(&files, filter, LogMatches::default()),
        "the hexagons of a filtered-out recording put ink on the map"
    );
}

/// The time window ends the recorded track at the last fix it keeps, and the
/// hexagons of the entries logged after that end there too. The cursor finds
/// nothing where a hidden entry was recorded: a hexagon the map does not draw
/// takes no pointer.
#[test]
fn no_log_hexagon_is_hovered_for_an_entry_the_time_window_hides() {
    /// Fixes of a recording walking east, each hexagon its own on screen.
    const WALKING_FIX_COUNT: usize = 4;

    /// The last fix the window keeps, two fixes before the one the pointer
    /// goes to.
    const LAST_KEPT: usize = 1;

    let files = a_recording_of(WALKING_FIX_COUNT, WALKING_STEP_DEGREES);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, 0..WALKING_FIX_COUNT);

    let pointed = pointing_at_the_hexagon_on_fix(
        &files,
        WALKING_FIX_COUNT - 1,
        window_ending_at(LAST_KEPT),
        matches,
    );

    assert_eq!(
        pointed.hovered, None,
        "a hexagon of an entry the time window hides took the pointer"
    );
}

/// A hexagon the filter removed is not under the cursor either: the map
/// publishes nothing for the log viewer to mark, and a click where the hexagon
/// was opens no log.
#[rstest::rstest]
#[case::the_time_window_is_disjoint_from_the_recording(GlobalFilter {
    time_start: Some(epoch() + Duration::hours(5)),
    ..GlobalFilter::default()
})]
#[case::the_recording_is_shorter_than_the_minimum_duration(GlobalFilter {
    min_duration: Some(Duration::hours(5)),
    ..GlobalFilter::default()
})]
fn no_log_hexagon_is_hovered_or_clicked_on_a_recording_the_filter_rejects(
    #[case] filter: GlobalFilter,
) {
    let files = a_recording_of(FIX_COUNT, STANDING_STEP_DEGREES);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, 0..FIX_COUNT);

    let pointed = pointing_at_the_hexagon_on_fix(&files, 0, filter, matches);

    assert_eq!(
        pointed.hovered, None,
        "a hexagon of a filtered-out recording took the pointer"
    );
    assert_eq!(
        pointed.clicked, None,
        "a click landed on a hexagon of a filtered-out recording"
    );
}

/// A cluster stands for the entries it collapsed, and an entry the time window
/// hides is not one of them: the hexagon lists and counts the rest.
#[test]
fn a_hexagon_stands_for_no_entry_the_time_window_hides() {
    /// Entries the window keeps, from the first to this fix.
    const LAST_KEPT: usize = 14;

    let files = a_recording_of(FIX_COUNT, STANDING_STEP_DEGREES);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, 0..FIX_COUNT);

    let pointed = pointing_at_the_hexagon_on_fix(&files, 0, window_ending_at(LAST_KEPT), matches);

    assert_eq!(
        pointed.hovered.map(|glyph| glyph.entry_indices),
        Some((0..=LAST_KEPT).collect()),
        "the hexagon stood for entries the time window hides"
    );
}

/// The count leaves out what the filter removed: the display toggle states how
/// many hexagons the map would draw.
#[rstest::rstest]
#[case::the_recording_is_shorter_than_the_minimum_duration(GlobalFilter {
    min_duration: Some(Duration::hours(5)),
    ..GlobalFilter::default()
}, 0)]
#[case::the_time_window_keeps_the_first_fifteen_entries(window_ending_at(14), 15)]
fn the_log_match_count_states_the_hexagons_the_filter_keeps(
    #[case] filter: GlobalFilter,
    #[case] expected: usize,
) {
    let files = a_recording_of(FIX_COUNT, STANDING_STEP_DEGREES);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, 0..FIX_COUNT);

    let counts = DisplayCounts::compute(
        &files,
        &TrackDataVisibility::from_loaded(&files),
        &filter,
        &EventMarkerVisibility::default(),
        &GeneratedMarkerVisibility::default(),
        None,
        SuppliedCounts {
            log_matches: Some(&matches),
            ..SuppliedCounts::default()
        },
    );

    assert_eq!(counts.get(DisplayCategory::LogMatches), expected);
}

/// This guards the oracle the cases above rely on: a recording the filter
/// keeps draws its hexagons, and the one under the cursor takes the pointer.
#[test]
fn a_log_hexagon_of_a_kept_recording_is_drawn() {
    let files = a_recording_of(FIX_COUNT, STANDING_STEP_DEGREES);
    let log = a_log_over(&files);

    assert!(
        shapes_with(
            &files,
            GlobalFilter::default(),
            matches_over(&files, &log, 0..FIX_COUNT)
        ) > shapes_with(&files, GlobalFilter::default(), LogMatches::default()),
        "the hexagons of a kept recording put no ink on the map"
    );
}

/// The other half of that oracle: the pointer reaches the hexagon at the
/// centre of the viewport, and a click on it opens its log.
#[test]
fn a_log_hexagon_of_a_kept_recording_takes_the_pointer() {
    let files = a_recording_of(FIX_COUNT, STANDING_STEP_DEGREES);
    let log = a_log_over(&files);
    let matches = matches_over(&files, &log, 0..FIX_COUNT);

    let pointed = pointing_at_the_hexagon_on_fix(&files, 0, GlobalFilter::default(), matches);

    assert_eq!(
        (pointed.hovered.is_some(), pointed.clicked.is_some()),
        (true, true),
        "the hexagon of a kept recording took neither the hover nor the click"
    );
}
