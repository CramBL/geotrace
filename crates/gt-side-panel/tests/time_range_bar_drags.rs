//! What a drag on one of the filter panel's two time range bars selects, and
//! what the label under the bar states about it.

mod support;

use chrono::{DateTime, Duration, Utc};
use gt_filter::GlobalFilter;
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use rstest::rstest;
use support::{
    FIXES_PER_TRACK, PanelState, bar_rects, harness, label_texts, point_on_track, recording, utc,
};

/// A recording of two tracks ten hours apart, and a window over the later one
/// only.
///
/// The active range is then a hundredth of the full range, which is the
/// condition the panel draws the second bar under.
fn harness_with_a_window_on_the_later_track() -> TestHarness<'static, PanelState> {
    let file = recording(
        "two_stints.gtd",
        &[
            (utc(0, 0, 0), FIXES_PER_TRACK),
            (utc(10, 0, 0), FIXES_PER_TRACK),
        ],
    );
    let mut harness = harness(vec![file]);
    harness.state_mut().filter.time_start = Some(utc(10, 0, 10));
    harness.state_mut().filter.time_end = Some(utc(10, 0, 40));
    harness.run();
    harness
}

/// The active range bar's left end is not the start of the recording: its span
/// is the active range with a little padding, not the whole loaded time range.
/// A drag to that end must not clear the window's start.
#[test]
fn a_drag_to_the_left_end_of_the_active_range_bar_keeps_the_earlier_track_filtered_out() {
    let mut harness = harness_with_a_window_on_the_later_track();
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
        .expect("the recording has two tracks");
    assert!(
        !gt_filter::track_passes_filter(earlier_track, &state.filter),
        "the window starts at {:?}",
        state.filter.time_start
    );
}

/// The window here runs the whole loaded time range, from the first fix of the
/// earlier track to the last fix of the later one: neither bound is set. The
/// active range bar's own viewport starts ten hours later.
#[test]
fn the_active_range_bar_states_the_ends_of_the_window_and_not_of_its_viewport() {
    let file = recording(
        "two_stints.gtd",
        &[(utc(0, 0, 0), 5), (utc(10, 0, 0), FIXES_PER_TRACK)],
    );
    let mut harness = harness(vec![file]);
    // The earlier track lasts four seconds and the later one 59: a minimum
    // duration between the two narrows the active range to the later track
    // while both window bounds stay absent.
    harness.state_mut().filter.min_duration = Some(Duration::seconds(30));
    harness.run();
    let labels = label_texts(&harness, "01/01");
    assert_eq!(
        labels.get(1).map(String::as_str),
        Some("01/01 00:00 — 01/01 10:00"),
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

    fn instant_before_the_drag(self) -> DateTime<Utc> {
        match self {
            Self::Start => utc(0, 3, 0),
            Self::End => utc(0, 7, 0),
        }
    }
}

/// A press grabs the handle nearest it and the drag holds that handle until
/// the release. The pointer crosses the other handle within a single frame,
/// which is what a drag over a long distance does.
#[rstest]
#[case::the_end_handle_dragged_left_past_the_start(
    DragAlongTheTrack { press: 0.70, release: 0.05 },
    WindowBound::Start,
)]
#[case::the_start_handle_dragged_right_past_the_end(
    DragAlongTheTrack { press: 0.30, release: 0.95 },
    WindowBound::End,
)]
fn a_drag_past_the_other_handle_moves_only_the_bound_it_grabbed(
    #[case] drag: DragAlongTheTrack,
    #[case] kept: WindowBound,
) {
    // Ten minutes of fixes, over which 00:03:00 and 00:07:00 sit three tenths
    // and seven tenths of the way along the bar.
    let file = recording("one_stint.gtd", &[(utc(0, 0, 0), 600)]);
    let mut harness = harness(vec![file]);
    harness.state_mut().filter.time_start = Some(WindowBound::Start.instant_before_the_drag());
    harness.state_mut().filter.time_end = Some(WindowBound::End.instant_before_the_drag());
    harness.run();
    let bar = *bar_rects(&harness).first().expect("the bar is on screen");

    let from = point_on_track(bar, drag.press);
    harness
        .inner
        .press_drag_release(from, point_on_track(bar, drag.release) - from, 1);
    harness.run();

    let held = kept.instant_before_the_drag();
    let filter = &harness.state().filter;
    assert_eq!(
        kept.of(filter),
        Some(held),
        "the bound the press left alone"
    );
    assert_eq!(
        kept.other().of(filter),
        Some(held),
        "the grabbed bound stops at the bound it passed"
    );
}

/// A drag on either bar clamps the handle it moves against the other handle,
/// whichever bar set that other handle.
#[test]
fn every_drag_across_both_bars_keeps_the_window_start_at_or_before_its_end() {
    let file = recording(
        "two_stints.gtd",
        &[
            (utc(0, 0, 0), FIXES_PER_TRACK),
            (utc(10, 0, 0), FIXES_PER_TRACK),
        ],
    );
    let mut harness = harness(vec![file]);
    harness.run();

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
                assert!(start <= end, "the window runs {start:?} to {end:?}");
            }
        }
    }
}
