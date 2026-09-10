//! What the plot draws for a track's clock offset. The offset sits on the
//! shared y-axis while the axis fits it. A marker sits at the nearer edge of
//! the axis while the offset lies outside.

use std::ops::Range;

use chrono::Duration;
use gt_plot::{EDGE_MARKER_INSET, PlotState};
use gt_types::LoadedFile;
use support::{DrawnPlot, PlotPosition, PlotSources};

mod support;

/// Fixes in the recording, one per second.
const FIX_COUNT: usize = 60;

/// Fixes of the crowded recording, one per second. Their markers fall closer
/// together than a glyph is wide at the width of the drawn plot.
const CROWDED_FIX_COUNT: usize = 600;

/// Seconds between the fixes of the drifting-clock recording: one fix per
/// minute over [`FIX_COUNT`] minutes.
const DRIFT_STEP_SECS: i64 = 60;

/// How much further the drifting host clock runs past the receiver's own with
/// every fix, in milliseconds.
const DRIFT_PER_FIX_MS: i64 = 100;

/// How far the host clock of a healthy logger runs past the receiver's own, in
/// milliseconds. The excursion recordings depart from this baseline and return
/// to it.
const BASELINE_HOST_AHEAD_MS: i64 = 200;

/// The fixes of the excursion recordings whose host clock departs from the
/// baseline, halfway along the track.
const EXCURSION_FIXES: Range<usize> = 20..30;

/// How far the host clock departs from the baseline over [`EXCURSION_FIXES`],
/// in either direction.
const EXCURSION_HOURS: i64 = 1;

/// How long the tracker suspended before it stamped its last fix on resume.
/// The clock offset of that fix is the whole suspend.
const SUSPEND_HOURS: i64 = 2;

/// Above the recording's velocity of 15 km/h and its heading of 45°, the two
/// values the shared y-axis fits without the clock offset line.
const VELOCITY_AND_HEADING_LIMIT: f64 = 100.0;

/// The host clock of a tracker that came up with its real-time clock unset
/// runs 56 years behind the receiver. Its stamps fall on 1968-01-15, while the
/// receiver reports the recording's own day.
fn host_clock_before_the_unix_epoch() -> Duration {
    Duration::days(-20_454)
}

/// A host clock a day past the receiver's own. That offset is
/// [`gt_analysis::clock_offset::MAX_PLOTTED_OFFSET_S`], which is the widest
/// offset on the shared y-axis.
fn host_clock_at_the_edge_of_the_band() -> Duration {
    Duration::days(1)
}

/// A recording of one track at 1 Hz whose host clock runs `host_ahead` past
/// the receiver's own on every fix.
fn recording_with_a_host_clock(host_ahead: Duration) -> LoadedFile {
    recording_of_fixes_with_a_host_clock(FIX_COUNT, host_ahead)
}

/// [`recording_with_a_host_clock`] over `fix_count` fixes.
fn recording_of_fixes_with_a_host_clock(fix_count: usize, host_ahead: Duration) -> LoadedFile {
    support::recording(
        gt_test_utils::fixtures::nav_points_with_a_host_clock_from(
            support::at_second(0),
            fix_count,
            1,
            host_ahead,
        ),
        Vec::new(),
    )
}

/// A recording of one track at 1 Hz whose host clock runs
/// [`BASELINE_HOST_AHEAD_MS`] past the receiver's own. Over
/// [`EXCURSION_FIXES`] it runs `host_ahead_in_the_excursion` past it, and it
/// returns to the baseline after.
fn recording_with_an_excursion(host_ahead_in_the_excursion: Duration) -> LoadedFile {
    let host_ahead: Vec<Duration> = (0..FIX_COUNT)
        .map(|index| {
            if EXCURSION_FIXES.contains(&index) {
                host_ahead_in_the_excursion
            } else {
                Duration::milliseconds(BASELINE_HOST_AHEAD_MS)
            }
        })
        .collect();
    recording_with_host_clock_offsets(1, &host_ahead)
}

/// A recording of one track at 1 Hz whose host clock runs
/// [`BASELINE_HOST_AHEAD_MS`] past the receiver's own on every fix but the
/// last. The host stamped the last fix [`SUSPEND_HOURS`] past the receiver's
/// time.
fn recording_with_a_departure_on_the_last_fix() -> LoadedFile {
    let host_ahead: Vec<Duration> = (0..FIX_COUNT)
        .map(|index| {
            if index == FIX_COUNT - 1 {
                Duration::hours(SUSPEND_HOURS)
            } else {
                Duration::milliseconds(BASELINE_HOST_AHEAD_MS)
            }
        })
        .collect();
    recording_with_host_clock_offsets(1, &host_ahead)
}

/// A recording of one track whose fixes are `step_secs` apart. The fix at
/// index `i` has a host timestamp `host_ahead[i]` past the receiver's own
/// time.
fn recording_with_host_clock_offsets(step_secs: i64, host_ahead: &[Duration]) -> LoadedFile {
    support::recording(
        gt_test_utils::fixtures::nav_points_with_host_clock_offsets_from(
            support::at_second(0),
            step_secs,
            host_ahead,
        ),
        Vec::new(),
    )
}

/// A recording of one track per entry of `host_ahead`, each of [`FIX_COUNT`]
/// fixes at 1 Hz. Track `i` runs its own length after track `i - 1` ends, and
/// its host clock runs `host_ahead[i]` past the receiver's own.
fn recording_of_a_track_per_host_clock(host_ahead: &[Duration]) -> LoadedFile {
    let tracks = host_ahead
        .iter()
        .enumerate()
        .map(|(index, &ahead)| {
            let points = gt_test_utils::fixtures::nav_points_with_a_host_clock_from(
                support::at_second((index * 2 * FIX_COUNT) as i64),
                FIX_COUNT,
                1,
                ahead,
            );
            let mut track = gt_test_utils::loaded_track_with_points(points);
            track.metadata.duration = track.metadata.time_range.duration();
            track
        })
        .collect();
    gt_test_utils::loaded_file_with_tracks(tracks)
}

fn drawn_with_a_host_clock(host_ahead: Duration) -> DrawnPlot {
    drawn_over(recording_with_a_host_clock(host_ahead))
}

/// A harness that has drawn the plot over `recording`, under the default
/// sources and the default plot state.
fn drawn_over(recording: LoadedFile) -> DrawnPlot {
    support::drawn_plot(
        vec![recording],
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

#[test]
fn a_baseline_at_the_edge_of_the_band_stays_on_the_line() {
    let host_ahead = host_clock_at_the_edge_of_the_band();
    let drawn = drawn_with_a_host_clock(host_ahead);

    let (y_min, _) = visible_y_range(&drawn);

    let offset_ms = -(host_ahead.num_milliseconds() as f64);
    assert!(
        y_min <= offset_ms,
        "the axis follows the clock offset line down to {offset_ms} ms, got {y_min}"
    );
}

#[test]
fn snapshot_a_host_clock_drifting_by_seconds_draws_on_the_line() {
    let host_ahead: Vec<Duration> = (0..FIX_COUNT as i64)
        .map(|index| Duration::milliseconds(index * DRIFT_PER_FIX_MS))
        .collect();

    let mut drawn = drawn_over(recording_with_host_clock_offsets(
        DRIFT_STEP_SECS,
        &host_ahead,
    ));

    drawn.snapshot("clock_offset_of_a_drifting_host_clock");
}

#[test]
fn snapshot_a_baseline_at_the_edge_of_the_band_draws_on_the_line() {
    let mut drawn = drawn_with_a_host_clock(host_clock_at_the_edge_of_the_band());

    drawn.snapshot("clock_offset_baseline_at_the_edge_of_the_band");
}

#[test]
fn snapshot_a_baseline_past_the_edge_of_the_band_marks_every_fix_at_the_bottom_edge() {
    let mut drawn =
        drawn_with_a_host_clock(host_clock_at_the_edge_of_the_band() + Duration::milliseconds(1));

    drawn.snapshot("clock_offset_baseline_past_the_edge_of_the_band");
}

#[rstest::rstest]
#[case::dark("clock_offset_baseline_before_the_unix_epoch_dark", egui::Theme::Dark)]
#[case::light(
    "clock_offset_baseline_before_the_unix_epoch_light",
    egui::Theme::Light
)]
fn snapshot_a_baseline_before_the_unix_epoch_marks_every_fix_at_the_top_edge(
    #[case] snapshot_name: &str,
    #[case] theme: egui::Theme,
) {
    let mut drawn = support::drawn_plot_in_theme(
        vec![recording_with_a_host_clock(
            host_clock_before_the_unix_epoch(),
        )],
        PlotSources::default(),
        PlotState::default(),
        theme,
    );

    drawn.snapshot(snapshot_name);
}

#[test]
fn snapshot_an_excursion_of_a_host_clock_behind_the_receiver_marks_the_top_edge() {
    let mut drawn = drawn_over(recording_with_an_excursion(Duration::hours(
        -EXCURSION_HOURS,
    )));

    drawn.snapshot("clock_offset_excursion_above_the_axis");
}

#[test]
fn snapshot_an_excursion_of_a_host_clock_ahead_of_the_receiver_marks_the_bottom_edge() {
    let mut drawn = drawn_over(recording_with_an_excursion(Duration::hours(
        EXCURSION_HOURS,
    )));

    drawn.snapshot("clock_offset_excursion_below_the_axis");
}

#[test]
fn snapshot_markers_closer_than_a_glyph_width_draw_one_glyph_apart() {
    let mut drawn = drawn_over(recording_of_fixes_with_a_host_clock(
        CROWDED_FIX_COUNT,
        host_clock_before_the_unix_epoch(),
    ));

    drawn.snapshot("clock_offset_markers_closer_than_a_glyph_width");
}

#[test]
fn snapshot_the_track_on_the_line_sets_the_axis_the_off_scale_track_is_marked_against() {
    let mut drawn = drawn_over(recording_of_a_track_per_host_clock(&[
        host_clock_before_the_unix_epoch(),
        Duration::milliseconds(BASELINE_HOST_AHEAD_MS),
    ]));

    drawn.snapshot("clock_offset_off_scale_track_beside_an_on_scale_track");
}

/// The sample at the view's right boundary is drawn whole: a marker is inset
/// from the top and the bottom edge by [`EDGE_MARKER_INSET`], and from the left
/// and the right boundary by half a glyph.
#[test]
fn snapshot_a_marker_at_the_right_boundary_is_drawn_whole() {
    // The marker at the view's left boundary is drawn whole: the view opens
    // this far before the first fix.
    const LEAD_IN_SECS: i64 = 10;
    let up_to_the_last_fix = -LEAD_IN_SECS..=(FIX_COUNT as i64 - 1);

    let mut drawn = support::drawn_plot(
        vec![recording_with_a_host_clock(
            host_clock_before_the_unix_epoch(),
        )],
        PlotSources::default().pinned_to_map_view(up_to_the_last_fix),
        PlotState::default(),
    );

    drawn.snapshot("clock_offset_marker_at_the_right_boundary");
}

/// The plot draws the clock offset line across the x span of the excursion, and
/// the markers at the top edge above it. A connector runs from the line point
/// before the excursion up to the first marker, from marker to marker along
/// the edge, and back down to the line point after it.
#[test]
fn snapshot_connectors_join_the_markers_to_the_line_they_left() {
    const FIXES_EITHER_SIDE: i64 = 5;
    let around_the_excursion = (EXCURSION_FIXES.start as i64 - FIXES_EITHER_SIDE)
        ..=(EXCURSION_FIXES.end as i64 + FIXES_EITHER_SIDE);

    let mut drawn = support::drawn_plot(
        vec![recording_with_an_excursion(Duration::hours(
            -EXCURSION_HOURS,
        ))],
        PlotSources::default().pinned_to_map_view(around_the_excursion),
        PlotState::default(),
    );

    drawn.snapshot("clock_offset_markers_over_the_line_they_left");
}

/// The departure on the last fix is one sample of a track of [`FIX_COUNT`], far
/// below the tenth of a track's samples that a level shift needs. The plot
/// holds it off the line and marks it at the bottom edge, with a connector up
/// to the line point before it.
#[test]
fn snapshot_a_departure_on_the_last_fix_is_marked_at_the_bottom_edge() {
    let mut drawn = drawn_over(recording_with_a_departure_on_the_last_fix());

    drawn.snapshot("clock_offset_departure_on_the_last_fix");
}

#[test]
fn a_departure_on_the_last_fix_leaves_the_shared_y_axis_to_the_clock_offset_baseline() {
    let drawn = drawn_over(recording_with_a_departure_on_the_last_fix());

    let (y_min, y_max) = visible_y_range(&drawn);

    let baseline_offset_ms = -BASELINE_HOST_AHEAD_MS as f64;
    assert!(
        y_min > 2.0 * baseline_offset_ms && y_max < VELOCITY_AND_HEADING_LIMIT,
        "the baseline of {baseline_offset_ms} ms and the other metrics set the axis, \
         not the departure of {} h, got {y_min}..{y_max}",
        -SUSPEND_HOURS
    );
}

#[test]
fn the_hover_of_a_departure_on_the_last_fix_says_the_recording_ends_there() {
    // The pointer reaches the marker inside the plot area: the view runs past
    // the last fix.
    const LEAD_OUT_SECS: i64 = 10;
    let last_fix = FIX_COUNT as i64 - 1;

    let mut drawn = support::drawn_plot(
        vec![recording_with_a_departure_on_the_last_fix()],
        PlotSources::default().pinned_to_map_view(0..=(last_fix + LEAD_OUT_SECS)),
        PlotState::default(),
    );
    let (y_min, y_max) = visible_y_range(&drawn);
    let marker = drawn.screen_position(PlotPosition {
        offset_secs: last_fix as f64,
        y: y_min + (y_max - y_min) * EDGE_MARKER_INSET,
    });

    drawn.hover(marker);

    let label = drawn.hover_label();
    assert!(
        label.contains("Clock offset excursion"),
        "the marker's tooltip reads {label:?}"
    );
    assert!(
        label.contains("The offset left the track's baseline for 1 sample, and the recording ends"),
        "the marker's tooltip reads {label:?}"
    );
}
