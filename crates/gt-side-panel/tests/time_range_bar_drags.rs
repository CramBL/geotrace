//! What a drag on one of the filter panel's two time range bars selects, and
//! what the label under the bar states about it.

mod support;

use chrono::{DateTime, Duration, Utc};
use gt_filter::GlobalFilter;
use gt_side_panel::filter::{COARSE_BAR_MINIMUM_WINDOW_SPAN, FINE_BAR_MINIMUM_WINDOW_SPAN};
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use gt_types::LoadedFile;
use rstest::rstest;
use support::{
    FIXES_PER_TRACK, PanelState, bar_rects, harness, label_texts, point_on_track, recording, utc,
};

/// A recording of three tracks whose loaded time range spans exactly `days`
/// days: one at the start, one halfway along it, and one of a single fix
/// `days` days after the first.
///
/// A window over the middle track alone leaves an active range of under a
/// minute, which is narrow enough for the panel to split its bars once the
/// loaded time range is over five days.
fn recording_spanning(days: i64) -> LoadedFile {
    let start = utc(0, 0, 0);
    recording(
        "several_days.gtd",
        &[
            (start, FIXES_PER_TRACK),
            (start + Duration::days(days / 2), FIXES_PER_TRACK),
            (start + Duration::days(days), 1),
        ],
    )
}

/// The start of the middle track of [`recording_spanning`] over six days.
fn middle_track() -> DateTime<Utc> {
    utc(0, 0, 0) + Duration::days(3)
}

/// A six-day recording windowed to a day on either side of its middle track,
/// whose bounds the coarse bar draws a third and two thirds along its track.
fn harness_with_a_two_day_window() -> TestHarness<'static, PanelState> {
    let mut harness = harness(vec![recording_spanning(6)]);
    harness.state_mut().filter.time_start = Some(middle_track() - Duration::days(1));
    harness.state_mut().filter.time_end = Some(middle_track() + Duration::days(1));
    harness.run();
    harness
}

/// A six-day recording windowed to half a minute of its middle track. The
/// active range bar spans 83 seconds around that track, over which the
/// window's two bounds sit at 0.27 and 0.63 of its track.
fn harness_with_a_half_minute_window() -> TestHarness<'static, PanelState> {
    let mut harness = harness(vec![recording_spanning(6)]);
    harness.state_mut().filter.time_start = Some(middle_track() + Duration::seconds(10));
    harness.state_mut().filter.time_end = Some(middle_track() + Duration::seconds(40));
    harness.run();
    harness
}

/// The active range bar's left end is not the start of the recording: its span
/// is the active range with a little padding, not the whole loaded time range.
/// A drag to that end must not clear the window's start.
#[test]
fn a_drag_to_the_left_end_of_the_active_range_bar_keeps_the_earlier_track_filtered_out() {
    let mut harness = harness_with_a_half_minute_window();
    let bars = bar_rects(&harness);
    let active_bar = *bars.get(1).expect("the active range bar is on screen");
    let from = point_on_track(active_bar, 0.02);
    harness
        .inner
        .press_drag_release(from, point_on_track(active_bar, -0.02) - from, 1);
    harness.run();
    let state = harness.state();
    let earlier_track = state
        .files
        .first()
        .and_then(|file| file.tracks.first())
        .expect("the recording has three tracks");
    assert!(
        !gt_filter::track_passes_filter(earlier_track, &state.filter),
        "the window starts at {:?}",
        state.filter.time_start
    );
}

/// The window here runs the whole loaded time range, from the first fix of the
/// first track to the last fix of the third: neither bound is set. The active
/// range bar's own viewport spans the middle track three days in.
#[test]
fn the_active_range_bar_states_the_ends_of_the_window_and_not_of_its_viewport() {
    let start = utc(0, 0, 0);
    let file = recording(
        "several_days.gtd",
        &[
            (start, 5),
            (start + Duration::days(3), FIXES_PER_TRACK),
            (start + Duration::days(6), 5),
        ],
    );
    let mut harness = harness(vec![file]);
    // The outer tracks last four seconds and the middle one 59: a minimum
    // duration between the two narrows the active range to the middle track
    // while both window bounds stay absent.
    harness.state_mut().filter.min_duration = Some(Duration::seconds(30));
    harness.run();
    let labels = label_texts(&harness, "01/0");
    assert_eq!(
        labels.get(1).map(String::as_str),
        Some("01/01 00:00 — 01/07 00:00"),
        "the panel's labels of a window bound read {labels:?}"
    );
}

/// One drag along a bar's track, as fractions of the track's own span.
struct DragAlongTheTrack {
    press: f32,
    release: f32,
}

/// One of the two bounds of the time window, each drawn as a handle on every
/// bar.
#[derive(Debug, Clone, Copy)]
enum WindowBound {
    Start,
    End,
}

impl WindowBound {
    fn of(self, filter: &GlobalFilter) -> Option<DateTime<Utc>> {
        match self {
            Self::Start => filter.time_start,
            Self::End => filter.time_end,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Start => Self::End,
            Self::End => Self::Start,
        }
    }
}

/// Presses the bar at `drag.press` and releases at `drag.release`, both as
/// fractions of the bar's track, crossing the distance within a single frame.
fn drag_along_the_bar(
    harness: &mut TestHarness<'static, PanelState>,
    bar: egui::Rect,
    drag: &DragAlongTheTrack,
) {
    let from = point_on_track(bar, drag.press);
    harness
        .inner
        .press_drag_release(from, point_on_track(bar, drag.release) - from, 1);
    harness.run();
}

/// The panel splits into a coarse bar and a fine one only where the loaded
/// time range is over five days, which is the coarse minimum times the ratio
/// every minimum span fits its bar by. A loaded range of exactly five days
/// shows one bar.
#[rstest]
#[case::four_days(4, 1)]
#[case::five_days_exactly(5, 1)]
#[case::six_days(6, 2)]
fn the_active_range_bar_appears_over_a_loaded_range_of_more_than_five_days(
    #[case] days: i64,
    #[case] bars_on_screen: usize,
) {
    let mut harness = harness(vec![recording_spanning(days)]);
    let middle_track = utc(0, 0, 0) + Duration::days(days / 2);
    harness.state_mut().filter.time_start = Some(middle_track + Duration::seconds(10));
    harness.state_mut().filter.time_end = Some(middle_track + Duration::seconds(40));
    harness.run();
    assert_eq!(bar_rects(&harness).len(), bars_on_screen);
}

/// A press grabs the handle nearest it and the drag holds that handle until
/// the release. The pointer crosses the other handle within a single frame,
/// which is what a drag over a long distance does.
#[rstest]
#[case::the_end_handle_dragged_left_past_the_start(
    DragAlongTheTrack { press: 0.67, release: 0.05 },
    WindowBound::End,
)]
#[case::the_start_handle_dragged_right_past_the_end(
    DragAlongTheTrack { press: 0.33, release: 0.95 },
    WindowBound::Start,
)]
fn a_drag_past_the_other_handle_on_the_coarse_bar_stops_a_day_short_of_it(
    #[case] drag: DragAlongTheTrack,
    #[case] moved: WindowBound,
) {
    let mut harness = harness_with_a_two_day_window();
    let held = moved.other().of(&harness.state().filter);
    let bar = *bar_rects(&harness)
        .first()
        .expect("the coarse bar is on screen");

    drag_along_the_bar(&mut harness, bar, &drag);

    let filter = &harness.state().filter;
    assert_eq!(
        moved.other().of(filter),
        held,
        "the bound the press left alone"
    );
    assert_eq!(
        moved.of(filter),
        Some(middle_track()),
        "the grabbed bound stops a day from the bound it passed"
    );
}

#[rstest]
#[case::the_end_handle_dragged_left_past_the_start(
    DragAlongTheTrack { press: 0.63, release: 0.05 },
    WindowBound::End,
    11,
)]
#[case::the_start_handle_dragged_right_past_the_end(
    DragAlongTheTrack { press: 0.27, release: 0.95 },
    WindowBound::Start,
    39,
)]
fn a_drag_past_the_other_handle_on_the_active_range_bar_stops_a_second_short_of_it(
    #[case] drag: DragAlongTheTrack,
    #[case] moved: WindowBound,
    #[case] seconds_into_the_middle_track: i64,
) {
    let mut harness = harness_with_a_half_minute_window();
    let held = moved.other().of(&harness.state().filter);
    let bars = bar_rects(&harness);
    let bar = *bars.get(1).expect("the active range bar is on screen");

    drag_along_the_bar(&mut harness, bar, &drag);

    let filter = &harness.state().filter;
    assert_eq!(
        moved.other().of(filter),
        held,
        "the bound the press left alone"
    );
    assert_eq!(
        moved.of(filter),
        Some(middle_track() + Duration::seconds(seconds_into_the_middle_track)),
        "the grabbed bound stops a second from the bound it passed"
    );
}

/// A recording under the split threshold shows one bar, and that bar is the
/// fine one: the whole window is set through it.
#[rstest]
#[case::the_end_handle_dragged_left_past_the_start(
    DragAlongTheTrack { press: 0.70, release: 0.05 },
    WindowBound::End,
    utc(0, 3, 1),
)]
#[case::the_start_handle_dragged_right_past_the_end(
    DragAlongTheTrack { press: 0.30, release: 0.95 },
    WindowBound::Start,
    utc(0, 6, 59),
)]
fn a_drag_past_the_other_handle_on_the_only_bar_stops_a_second_short_of_it(
    #[case] drag: DragAlongTheTrack,
    #[case] moved: WindowBound,
    #[case] stops_at: DateTime<Utc>,
) {
    // Ten minutes of fixes, over which 00:03:00 and 00:07:00 sit three tenths
    // and seven tenths of the way along the bar.
    let mut harness = harness(vec![recording("one_stint.gtd", &[(utc(0, 0, 0), 600)])]);
    harness.state_mut().filter.time_start = Some(utc(0, 3, 0));
    harness.state_mut().filter.time_end = Some(utc(0, 7, 0));
    harness.run();
    let bars = bar_rects(&harness);
    assert_eq!(bars.len(), 1, "a ten-minute recording is under the split");
    let bar = *bars.first().expect("the bar is on screen");

    drag_along_the_bar(&mut harness, bar, &drag);

    assert_eq!(
        moved.of(&harness.state().filter),
        Some(stops_at),
        "the grabbed bound stops a second from the bound it passed"
    );
}

/// The panel gains its active range bar partway through this drag: the window
/// narrows to the first track alone while the pointer is still down. That
/// turns the bar under the pointer into the coarse one. The bound stops a
/// second from the other, at the minimum the press latched.
#[test]
fn a_drag_keeps_the_minimum_its_press_latched_when_the_panel_gains_a_bar() {
    let mut harness = harness(vec![recording_spanning(6)]);
    harness.state_mut().filter.time_end = Some(utc(0, 0, 0) + Duration::days(5));
    harness.run();
    let bars = bar_rects(&harness);
    assert_eq!(bars.len(), 1, "the active range spans two of the tracks");
    let bar = *bars.first().expect("the bar is on screen");

    drag_along_the_bar(
        &mut harness,
        bar,
        &DragAlongTheTrack {
            press: 0.83,
            release: -0.02,
        },
    );

    assert_eq!(
        harness.state().filter.time_end,
        Some(utc(0, 0, 1)),
        "the grabbed bound stops a second from the start of the loaded time range"
    );
    assert_eq!(
        bar_rects(&harness).len(),
        2,
        "the drag brought the active range bar on screen"
    );
}

/// A window narrower than the coarse bar's minimum comes from the active range
/// bar. A drag on the coarse bar binds the handle it grabbed and leaves the
/// other bound where it is.
#[rstest]
#[case::the_start_handle_dragged_left(
    DragAlongTheTrack { press: 0.48, release: 0.10 },
    WindowBound::Start,
)]
#[case::the_end_handle_dragged_right(
    DragAlongTheTrack { press: 0.60, release: 0.90 },
    WindowBound::End,
)]
fn a_drag_away_from_the_other_handle_moves_only_the_bound_it_grabbed(
    #[case] drag: DragAlongTheTrack,
    #[case] moved: WindowBound,
) {
    let mut harness = harness(vec![recording_spanning(6)]);
    harness.state_mut().filter.time_start = Some(middle_track());
    harness.state_mut().filter.time_end = Some(middle_track() + Duration::hours(12));
    harness.run();
    let held = moved.other().of(&harness.state().filter);
    let bar = *bar_rects(&harness)
        .first()
        .expect("the coarse bar is on screen");

    drag_along_the_bar(&mut harness, bar, &drag);

    let filter = &harness.state().filter;
    assert_eq!(
        moved.other().of(filter),
        held,
        "the bound the press left alone"
    );
    let start = filter.time_start.expect("the window keeps a start");
    let end = filter.time_end.expect("the window keeps an end");
    assert!(
        end - start > COARSE_BAR_MINIMUM_WINDOW_SPAN,
        "the window runs {start:?} to {end:?}"
    );
}

/// A drag on either bar keeps at least the finest minimum between the window's
/// two bounds, whichever bar set the bound it clamps against.
#[test]
fn every_drag_keeps_at_least_the_fine_minimum_between_the_window_bounds() {
    let mut harness = harness_with_a_half_minute_window();

    // Fractions of a bar's track a handle is dragged from and to, alternating
    // between the handle nearer the left end and the one nearer the right.
    let drags = [(1.0, 0.4), (0.0, 0.8), (0.9, 0.1), (0.05, 0.95)];
    for bar_index in [0, 1] {
        for (from, to) in drags {
            let Some(bar) = bar_rects(&harness).get(bar_index).copied() else {
                continue;
            };
            let from = point_on_track(bar, from);
            harness
                .inner
                .press_drag_release(from, point_on_track(bar, to) - from, 4);
            harness.run();
            let filter = &harness.state().filter;
            if let (Some(start), Some(end)) = (filter.time_start, filter.time_end) {
                assert!(
                    end - start >= FINE_BAR_MINIMUM_WINDOW_SPAN,
                    "the window runs {start:?} to {end:?}"
                );
            }
        }
    }
}

#[test]
fn the_label_under_the_coarse_bar_shows_the_bound_after_a_drag_clamps_it() {
    let mut harness = harness_with_a_two_day_window();
    let bar = *bar_rects(&harness)
        .first()
        .expect("the coarse bar is on screen");

    drag_along_the_bar(
        &mut harness,
        bar,
        &DragAlongTheTrack {
            press: 0.67,
            release: 0.05,
        },
    );

    let labels = label_texts(&harness, "01/03");
    assert_eq!(
        labels.first().map(String::as_str),
        Some("01/03 00:00 — 01/04 00:00")
    );
}

/// The coarse bar states on hover that the active range bar sets a window
/// shorter than its own minimum of a day.
#[test]
fn the_coarse_bar_names_the_active_range_bar_on_hover() {
    let mut harness = harness_with_a_two_day_window();
    let bar = *bar_rects(&harness)
        .first()
        .expect("the coarse bar is on screen");

    harness
        .inner
        .hover_at_and_settle(point_on_track(bar, 0.5), 3);

    assert_eq!(
        label_texts(&harness, "active range bar below")
            .first()
            .map(String::as_str),
        Some("Sets a window of 24h or more. The active range bar below sets a shorter one.")
    );
}

/// Only the coarse bar states a minimum on hover: the fine bar sets any window
/// inside the recording.
#[test]
fn the_only_bar_states_no_minimum_on_hover() {
    let mut harness = harness(vec![recording("one_stint.gtd", &[(utc(0, 0, 0), 600)])]);
    harness.run();
    let bar = *bar_rects(&harness).first().expect("the bar is on screen");

    harness
        .inner
        .hover_at_and_settle(point_on_track(bar, 0.5), 3);

    assert!(label_texts(&harness, "active range bar below").is_empty());
}
