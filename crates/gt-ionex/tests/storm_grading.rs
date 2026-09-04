//! Grade the captured May 2024 storm with the planetary ionospheric storm
//! index, from the values JPL published.
//!
//! The index is what the environment warning fires on, so the boundaries in
//! [`gt_ionex::quiet_time`] are checked here against real data: the Gannon
//! storm of 10 to 12 May 2024 is the best documented event the archive covers.

// A helper beside a `#[test]` function is not covered by clippy's in-test
// relaxations, and this file is development-only code.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: helpers beside the tests"
)]

use std::fmt::Write as _;

use chrono::{NaiveDate, TimeDelta};

use gt_ionex::node_series::{CapturedNodeDay, NodeSeriesCapture};
use gt_ionex::quiet_time::{self, IonosphericStormGrade, QuietTimeDeviation, StormGradeRun};
use gt_ionex::{EQUATORIAL_CREST_NODE, EUROPE_NODE, FIXTURE_NODES, NORTH_AMERICA_NODE};

fn day(month: u32, day_of_month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, month, day_of_month).expect("a calendar date")
}

fn capture() -> NodeSeriesCapture {
    gt_ionex::captured_node_series().expect("the node-series capture")
}

/// The deviation of one published value from the median of the same node and
/// offset over the 27 days before its own day, which is what
/// `QuietTimeDeviationCache` computes for a fix.
fn deviation_at(
    capture: &NodeSeriesCapture,
    node: &str,
    day: NaiveDate,
    offset: TimeDelta,
) -> Option<QuietTimeDeviation> {
    let value = capture.value_at_offset(node, day, offset)?;
    let window = capture.background_window(node, day, offset);
    quiet_time::deviation_from_quiet_time(value, &window)
}

/// Every graded epoch of `day`, with the offset it sits at.
fn graded_day(
    capture: &NodeSeriesCapture,
    node: &str,
    day: NaiveDate,
) -> Vec<(TimeDelta, QuietTimeDeviation)> {
    capture
        .day(day)
        .map(CapturedNodeDay::epoch_offsets)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|offset| Some((offset, deviation_at(capture, node, day, offset)?)))
        .collect()
}

/// The storm-grade run the epoch at `offset` stands in, over the whole day's
/// published epochs.
fn storm_grade_run_at(
    capture: &NodeSeriesCapture,
    node: &str,
    day: NaiveDate,
    offset: TimeDelta,
) -> Option<StormGradeRun> {
    let captured = capture.day(day)?;
    let offsets = captured.epoch_offsets();
    let day_epochs: Vec<Option<QuietTimeDeviation>> = offsets
        .iter()
        .map(|offset| deviation_at(capture, node, day, *offset))
        .collect();
    let epoch_index = offsets.iter().position(|epoch| *epoch == offset)?;
    StormGradeRun::containing_epoch(
        &day_epochs,
        epoch_index,
        TimeDelta::seconds(captured.interval_seconds),
    )
}

/// The epoch of `day` standing furthest from its quiet-time median.
fn peak_of_day(
    capture: &NodeSeriesCapture,
    node: &str,
    day: NaiveDate,
) -> (TimeDelta, QuietTimeDeviation) {
    graded_day(capture, node, day)
        .into_iter()
        .max_by(|(_, left), (_, right)| left.log_ratio().abs().total_cmp(&right.log_ratio().abs()))
        .unwrap_or_else(|| panic!("{node} carries no graded epoch on {day}"))
}

fn describe(offset: TimeDelta, deviation: QuietTimeDeviation) -> String {
    format!(
        "{:02}:00 UTC, DTEC {:+.3}, {:+.0} % of the median, {} (W = {})",
        offset.num_hours(),
        deviation.log_ratio(),
        deviation.percent_from_median(),
        deviation.grade(),
        deviation.storm_index_value()
    )
}

/// The peak deviation of every node over the storm and the days before it, as
/// the published maps grade it.
#[test]
fn the_captured_storm_grades_as_published() {
    let capture = capture();
    let mut report = String::new();
    for node in FIXTURE_NODES {
        report.push_str(node.name);
        report.push('\n');
        for day_of_may in 6..=12 {
            let assessed = day(5, day_of_may);
            let (offset, deviation) = peak_of_day(&capture, node.name, assessed);
            let run = storm_grade_run_at(&capture, node.name, assessed, offset)
                .map_or_else(|| "no storm grade".to_owned(), |run| format!("for {run}"));
            writeln!(
                report,
                "  {assessed}  {}, {run}",
                describe(offset, deviation)
            )
            .expect("writing to a string");
        }
    }

    insta::assert_snapshot!("captured_storm_grades", report);
}

/// The day before the flares that opened the event stays under the storm
/// grade at every node and every epoch: ordinary day-to-day variation does
/// not reach the level GeoTrace warns at.
#[test]
fn the_quiet_day_before_the_storm_never_reaches_the_storm_grade() {
    let capture = capture();

    for node in FIXTURE_NODES {
        for (offset, deviation) in graded_day(&capture, node.name, day(5, 8)) {
            assert!(
                !deviation.grade().is_a_storm(),
                "{} on 2024-05-08 reaches {}",
                node.name,
                describe(offset, deviation)
            );
        }
    }
}

/// The storm's positive phase over North America: the archived maps put the
/// afternoon of 10 May, as the main phase began, well above the quiet-time
/// median.
#[test]
fn the_positive_phase_over_north_america_grades_a_storm() {
    let capture = capture();
    let offset = TimeDelta::hours(20);

    let deviation = deviation_at(&capture, NORTH_AMERICA_NODE, day(5, 10), offset)
        .expect("the captured window grades the epoch");

    assert_eq!(
        deviation.grade(),
        IonosphericStormGrade::ModerateStorm,
        "{}",
        describe(offset, deviation)
    );
    assert!(
        deviation.percent_from_median() > 50.0,
        "{}",
        describe(offset, deviation)
    );
}

/// The storm's negative phase on 11 May: every node the capture follows is
/// depleted far below its quiet-time median, which the index grades an
/// intense storm on the same boundaries as an enhancement.
#[test]
fn the_negative_phase_of_11_may_grades_an_intense_storm_everywhere() {
    let capture = capture();

    for node in FIXTURE_NODES {
        let (offset, deviation) = peak_of_day(&capture, node.name, day(5, 11));

        assert_eq!(
            deviation.grade(),
            IonosphericStormGrade::IntenseStorm,
            "{} peaks at {}",
            node.name,
            describe(offset, deviation)
        );
        assert!(
            deviation.percent_from_median() < -50.0,
            "{} peaks at {}",
            node.name,
            describe(offset, deviation)
        );
    }
}

/// The storm is graded from the epoch the main phase began onwards, at every
/// node: the index reads the event whichever side of the median the node's
/// own ionosphere moved.
#[test]
fn every_node_reaches_the_storm_grade_once_the_main_phase_began() {
    let capture = capture();

    for node in FIXTURE_NODES {
        let reached: Vec<String> = graded_day(&capture, node.name, day(5, 10))
            .into_iter()
            .filter(|(offset, deviation)| {
                *offset >= TimeDelta::hours(18) && deviation.grade().is_a_storm()
            })
            .map(|(offset, deviation)| describe(offset, deviation))
            .collect();

        assert!(
            !reached.is_empty(),
            "{} reaches no storm grade from 18:00 UTC on 2024-05-10: {:?}",
            node.name,
            graded_day(&capture, node.name, day(5, 10))
                .into_iter()
                .map(|(offset, deviation)| describe(offset, deviation))
                .collect::<Vec<_>>()
        );
    }
}

/// A day's closing map and the next day's opening map name the same instant
/// and are published in different files, each from its own daily solution.
/// They differ, which is why a value is read from the file of the day it is
/// filed under.
#[test]
fn a_days_closing_map_differs_from_the_next_days_opening_map() {
    let capture = capture();
    let day_end = TimeDelta::hours(24);
    let day_start = TimeDelta::zero();

    let differing = capture
        .days
        .windows(2)
        .filter_map(|pair| match pair {
            [earlier, later] => Some((earlier, later)),
            _ => None,
        })
        .filter(|(earlier, later)| {
            capture.value_at_offset(EUROPE_NODE, earlier.day, day_end)
                != capture.value_at_offset(EUROPE_NODE, later.day, day_start)
        })
        .count();

    assert!(
        differing > capture.days.len() / 2,
        "only {differing} of {} day boundaries differ",
        capture.days.len() - 1
    );
}

/// The capture holds what the tests read it for: every declared node on every
/// day of the window, at the two-hour epochs JPL publishes final maps on.
#[test]
fn the_capture_covers_every_declared_node_over_the_whole_window() {
    let capture = capture();
    let (first, last) = gt_ionex::NODE_SERIES_DAYS;

    assert_eq!(capture.days.first().map(|day| day.day), Some(first));
    assert_eq!(capture.days.last().map(|day| day.day), Some(last));
    for captured in &capture.days {
        assert_eq!(captured.http_status, 200, "{}", captured.day);
        assert_eq!(captured.interval_seconds, 7200, "{}", captured.day);
        for node in FIXTURE_NODES {
            let values = captured.values_tecu.get(node.name);
            assert_eq!(
                values.map(Vec::len),
                Some(13),
                "{} on {}",
                node.name,
                captured.day
            );
            assert!(
                values.is_some_and(|values| values.iter().all(Option::is_some)),
                "{} has a gap on {}",
                node.name,
                captured.day
            );
        }
    }
}

/// The nodes sit where the capture says they do, on the grid JPL publishes.
#[test]
fn the_declared_nodes_sit_on_the_published_grid() {
    for node in FIXTURE_NODES {
        assert!(
            (node.latitude_degrees / 2.5).fract() == 0.0,
            "{} is off the latitude grid",
            node.name
        );
        assert!(
            (node.longitude_degrees / 5.0).fract() == 0.0,
            "{} is off the longitude grid",
            node.name
        );
    }
    assert_eq!(
        FIXTURE_NODES.map(|node| node.name),
        [EUROPE_NODE, NORTH_AMERICA_NODE, EQUATORIAL_CREST_NODE]
    );
}
