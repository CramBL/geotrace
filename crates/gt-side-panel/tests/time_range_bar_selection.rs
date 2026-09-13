//! What the filter panel's two time range bars span, over the recordings
//! loaded.

use chrono::Duration;
use gt_side_panel::test_util::{self, FIXES_PER_TRACK};
use gt_types::LoadedFile;

/// A recording whose fixes step back an hour and a half in the middle, which
/// segmentation splits into a later track followed by an earlier one.
///
/// Its fixes run from 11:00:00 to 12:30:00, a span of an hour and a half.
fn recording_whose_clock_steps_backwards() -> LoadedFile {
    test_util::recording(
        "clock_step.gtd",
        &[
            (test_util::utc(12, 29, 51), 10),
            (test_util::utc(11, 0, 0), 10),
        ],
    )
}

#[test]
fn the_time_range_heading_states_the_span_of_a_recording_whose_clock_steps_backwards() {
    let mut harness = test_util::harness(vec![recording_whose_clock_steps_backwards()]);
    harness.run();
    assert_eq!(
        test_util::label_text(&harness, "Time range"),
        "Time range — 1h30m"
    );
}

#[test]
fn a_recording_whose_clock_steps_backwards_has_a_time_range_bar() {
    let mut harness = test_util::harness(vec![recording_whose_clock_steps_backwards()]);
    harness.run();
    assert_eq!(
        test_util::bar_rects(&harness).len(),
        1,
        "the panel offers one bar over the whole loaded time range"
    );
}

/// A recording of no fixes at all has no track and covers no span. The bar
/// spans the recordings that hold fixes.
#[test]
fn a_recording_with_no_fixes_stays_out_of_the_time_range_bar() {
    let files = vec![
        test_util::recording("morning.gtd", &[(test_util::utc(0, 0, 0), FIXES_PER_TRACK)]),
        test_util::recording("no_fixes.gtd", &[]),
    ];
    let mut harness = test_util::harness(files);
    harness.run();
    assert_eq!(
        test_util::label_text(&harness, "Time range"),
        "Time range — 59s"
    );
}

/// The active range lies past the end of the range the bars are laid out over,
/// because one recording's tracks are not in time order.
///
/// The two recordings are six days apart, which is over the loaded time range
/// the panel splits its bars at.
#[test]
fn the_active_range_heading_comes_with_a_bar_under_it() {
    let six_days_on = Duration::days(6);
    let files = vec![
        test_util::recording("morning.gtd", &[(test_util::utc(0, 0, 0), FIXES_PER_TRACK)]),
        test_util::recording(
            "clock_step.gtd",
            &[
                (test_util::utc(12, 0, 0) + six_days_on, FIXES_PER_TRACK),
                (test_util::utc(11, 0, 0) + six_days_on, FIXES_PER_TRACK),
            ],
        ),
    ];
    let mut harness = test_util::harness(files);
    harness.state_mut().filter.time_start = Some(test_util::utc(11, 30, 0) + six_days_on);
    harness.run();
    assert_eq!(
        test_util::bar_rects(&harness).len(),
        2,
        "the active range heading reads {:?}",
        test_util::label_text(&harness, "Active range")
    );
}
