//! What the plot draws for a recording whose clock offset baseline lies
//! outside what the shared y-axis shows.

use chrono::Duration;
use gt_plot::{EDGE_MARKER_INSET, PlotState};
use gt_types::LoadedFile;
use support::{DrawnPlot, PlotPosition, PlotSources};

mod support;

/// Fixes in the recording, one per second.
const FIX_COUNT: usize = 60;

/// Above the recording's velocity of 15 km/h and its heading of 45°, the two
/// values the shared y-axis fits without the clock offset line.
const VELOCITY_AND_HEADING_LIMIT: f64 = 100.0;

/// The host clock of a tracker that came up with its real-time clock unset
/// runs 56 years behind the receiver. Its stamps fall on 1968-01-15, while the
/// receiver reports the recording's own day.
fn host_clock_before_the_unix_epoch() -> Duration {
    Duration::days(-20_454)
}

/// A recording of one track at 1 Hz whose host clock runs `host_ahead` past
/// the receiver's own on every fix.
fn recording_with_a_host_clock(host_ahead: Duration) -> LoadedFile {
    support::recording(
        gt_test_utils::fixtures::nav_points_with_a_host_clock_from(
            support::at_second(0),
            FIX_COUNT,
            1,
            host_ahead,
        ),
        Vec::new(),
    )
}

fn drawn_with_a_host_clock(host_ahead: Duration) -> DrawnPlot {
    support::drawn_plot(
        vec![recording_with_a_host_clock(host_ahead)],
        PlotSources::default(),
        PlotState::default(),
    )
}

fn visible_y_range(drawn: &DrawnPlot) -> (f64, f64) {
    let bounds = *drawn.transform().bounds();
    (bounds.min()[1], bounds.max()[1])
}

#[test]
fn a_host_clock_before_the_unix_epoch_leaves_the_shared_y_axis_to_the_other_metrics() {
    let drawn = drawn_with_a_host_clock(host_clock_before_the_unix_epoch());

    let (y_min, y_max) = visible_y_range(&drawn);

    assert!(
        y_min > -VELOCITY_AND_HEADING_LIMIT && y_max < VELOCITY_AND_HEADING_LIMIT,
        "velocity and heading set the axis, got {y_min}..{y_max}"
    );
}

#[test]
fn the_clock_offset_of_a_host_clock_before_the_unix_epoch_is_on_the_marker_at_the_top_edge() {
    let mut drawn = drawn_with_a_host_clock(host_clock_before_the_unix_epoch());
    let (y_min, y_max) = visible_y_range(&drawn);
    let marker = drawn.screen_position(PlotPosition {
        offset_secs: (FIX_COUNT / 2) as f64,
        y: y_max - (y_max - y_min) * EDGE_MARKER_INSET,
    });

    drawn.hover(marker);

    let label = drawn.hover_label();
    assert!(
        label.contains("Clock offset off the plot's scale"),
        "the marker's tooltip reads {label:?}"
    );
    assert!(
        label.contains("1968-01-15 12:00:30"),
        "the marker's tooltip reads {label:?}"
    );
}

/// A host clock a day ahead of the receiver sits at
/// [`gt_analysis::clock_offset::MAX_PLOTTED_OFFSET_S`], the widest offset the
/// shared y-axis shows.
#[test]
fn a_baseline_at_the_edge_of_the_band_stays_on_the_line() {
    let host_ahead = Duration::days(1);
    let drawn = drawn_with_a_host_clock(host_ahead);

    let (y_min, _) = visible_y_range(&drawn);

    let offset_ms = -(host_ahead.num_milliseconds() as f64);
    assert!(
        y_min <= offset_ms,
        "the axis follows the clock offset line down to {offset_ms} ms, got {y_min}"
    );
}
