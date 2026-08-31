//! How the editor buffer's blank lines and whitespace determine what counts as
//! a query, since that is what `split_queries` returns and everything
//! downstream relies on.

use gt_query_map_harness::{Dataset, MapScenario, TrackSpec};
use rstest::rstest;

fn scenario() -> MapScenario {
    MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0, 5.0,
    ])))
}

/// Every messy-but-legitimate way to write two queries lands the same two
/// chunks and the same map, so whitespace is never load-bearing.
#[rstest]
#[case::one_blank_line(
    "points | where velocity > 30 km/h | draw\n\npoints | where velocity < 10 km/h | hide"
)]
#[case::three_blank_lines(
    "points | where velocity > 30 km/h | draw\n\n\n\npoints | where velocity < 10 km/h | hide"
)]
#[case::spaces_and_tabs_between(
    "points | where velocity > 30 km/h | draw\n   \n\t\npoints | where velocity < 10 km/h | hide"
)]
#[case::leading_and_trailing_blanks(
    "\n\n points | where velocity > 30 km/h | draw\n\npoints | where velocity < 10 km/h | hide\n\n\n"
)]
#[case::crlf_endings(
    "points | where velocity > 30 km/h | draw\r\n\r\npoints | where velocity < 10 km/h | hide\r\n"
)]
#[case::continuation_lines(
    "points\n| where velocity > 30 km/h\n| draw\n\npoints\n| where velocity < 10 km/h\n| hide"
)]
#[case::indented_continuation_lines(
    "points\n    | where velocity > 30 km/h\n    | draw\n\npoints\n    | where velocity < 10 km/h\n    | hide"
)]
#[case::comment_paragraph_between(
    "points | where velocity > 30 km/h | draw\n\n# the slow points are noise\n\npoints | where velocity < 10 km/h | hide"
)]
fn whitespace_between_queries_never_changes_the_result(#[case] text: &str) {
    let mut scenario = scenario();
    scenario.run(text);
    insta::allow_duplicates! {
        insta::assert_snapshot!(scenario.picture(), @"
        track.gtd#0  x00x
        counts: shown 2, halos 1
        ");
    }
}

/// A blank line inside what was one query splits it in two, and the tail is not
/// a query on its own - the editor must say so instead of running half of it.
#[test]
fn a_blank_line_inside_a_query_splits_it_into_a_failing_chunk() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h\n\n| draw");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 2
      0..33 ok
      35..41 error: a query starts with a source: points or a channel like @accel
    run: rejected
    ");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
    counts: shown 4, halos 0
    ");
}

/// No trailing newline is the normal state of a buffer being typed in.
#[test]
fn a_buffer_without_a_trailing_newline_runs() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..40 ok
    run: completed
      1 match on 1 track
    ");
}

/// Only whitespace is not a query, and neither is only a comment: nothing runs
/// and the map stays untouched. The panel's `run: rejected` line is
/// `RunAttempt::Rejected`'s display text.
#[rstest]
#[case::blank("   \n\t\n  ")]
#[case::comment_only("# nothing to see here\n")]
#[case::empty("")]
fn a_buffer_with_no_query_is_rejected(#[case] text: &str) {
    let mut scenario = scenario();
    scenario.run(text);
    insta::allow_duplicates! {
        insta::assert_snapshot!(scenario.panel(), @"
        chunks: 0
        run: rejected
        ");
    }
    assert!(
        scenario.matches().is_none(),
        "a rejected run leaves the map alone"
    );
}
