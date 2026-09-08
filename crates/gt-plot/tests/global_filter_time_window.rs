//! What the plot draws and reports while the global filter's time window is
//! narrower than the loaded recordings: the view it fits, the chips it
//! offers, and the fix the map highlight lands on.

use chrono::{DateTime, TimeDelta, Utc};
use gt_filter::GlobalFilter;
use gt_plot::PlotState;
use gt_test_utils::Queryable as _;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{FileIdx, LoadedFile, NavPoint, PointIdx, TrackIdx};
use gt_ui_types::TrackDataVisibility;
use support::PlotSources;

mod support;

/// A recording of one track, `count` fixes at 1 Hz from `start_offset`
/// seconds, each carrying a satellite report of `constellation`.
fn recording(start_offset: i64, count: usize, constellation: Constellation) -> LoadedFile {
    let points: Vec<NavPoint> =
        gt_test_utils::fixtures::nav_points_from(support::at_second(start_offset), count, 1)
            .into_iter()
            .map(|point| {
                let report = Satellites::new(
                    Some(point.tpv.time()),
                    None,
                    vec![Satellite::new(
                        constellation,
                        1,
                        Some(45.0),
                        Some(120.0),
                        Some(38.0),
                        true,
                    )],
                );
                NavPoint::new(point.tpv, Some(report))
            })
            .collect();
    support::recording(points, Vec::new())
}

/// The recording with one scalar channel named `channel_name` on each of its
/// tracks, sampled once per fix.
fn with_channel(mut file: LoadedFile, channel_name: &str) -> LoadedFile {
    for track in &mut file.tracks {
        let times: Vec<DateTime<Utc>> = track
            .points
            .iter()
            .map(|point| point.tpv.time().utc())
            .collect();
        let values = vec![1.0; times.len()];
        track.channels = vec![gt_test_utils::fixtures::scalar_channel(
            channel_name,
            None,
            times,
            values,
        )];
    }
    file
}

/// A window from `start` to `end`, both in seconds from the first fix.
fn window(start: i64, end: i64) -> GlobalFilter {
    GlobalFilter {
        time_start: Some(support::at_second(start)),
        time_end: Some(support::at_second(end)),
        ..GlobalFilter::default()
    }
}

/// The sources of a plot the window narrows, with every archive empty.
fn under(filter: GlobalFilter) -> PlotSources {
    PlotSources {
        filter,
        ..PlotSources::default()
    }
}

/// A window of one minute over an hour-long recording resets to the minute,
/// not to the hour. The view fits the data the plot draws, and the plot draws
/// only the fixes inside the time window.
#[test]
fn the_view_fits_the_time_window_rather_than_the_whole_recording() {
    let files = vec![recording(0, 3600, Constellation::Gps)];
    let plot = support::drawn_plot(files, under(window(1800, 1860)), PlotState::default());

    let shown = plot
        .state()
        .visible_x_range()
        .expect("the plot has drawn once");
    let span_secs = shown.end() - shown.start();
    assert!(
        span_secs < 120.0,
        "the view spans {span_secs} s for a window of 60 s"
    );
}

/// The constellations of a recording the time window leaves out reach no chip
/// in the row: a per-constellation chip states that the data on the plot holds
/// that constellation.
#[test]
fn a_recording_outside_the_time_window_offers_no_constellation_chip() {
    let files = vec![
        recording(0, 60, Constellation::Gps),
        recording(7200, 60, Constellation::Qzss),
    ];
    let plot = support::drawn_plot(files, under(window(0, 60)), PlotState::default());

    assert!(
        plot.harness.inner.query_by_label("QZSS seen").is_none(),
        "the QZSS chip belongs to a recording the window leaves out"
    );
}

/// The Channels section toggle renders when a track on the plot carries
/// channels, and the channel chips come from the same union: a recording the
/// time window leaves out must reveal neither.
#[test]
fn a_recording_outside_the_time_window_reveals_no_channels_section() {
    let files = vec![
        recording(0, 60, Constellation::Gps),
        with_channel(recording(7200, 60, Constellation::Gps), "Brake pressure"),
    ];
    let plot = support::drawn_plot(files, under(window(0, 60)), PlotState::default());

    assert!(
        plot.harness
            .inner
            .query_by_label_contains("Channels")
            .is_none(),
        "the channel belongs to a recording the window leaves out"
    );
}

/// The fix the plot cursor cross-highlights lies inside the time window, never
/// on the fix nearest in time that the window left out. The map draws only the
/// fixes inside the window.
#[test]
fn the_cross_highlight_lands_on_a_fix_inside_the_time_window() {
    let files = [recording(0, 60, Constellation::Gps)];
    let visibility = TrackDataVisibility::from_loaded(&files);

    let closest =
        gt_plot::find_closest_tpv(&files, &visibility, &window(10, 20), support::at_second(50));

    assert_eq!(
        closest,
        Some((FileIdx::new(0), TrackIdx::new(0), PointIdx::new(20))),
        "the last fix inside the window is the closest one to 50 s"
    );
}

/// A track the filter rejects for a reason other than time contributes no
/// fix to the cross-highlight either.
#[test]
fn a_track_below_the_minimum_duration_holds_no_cross_highlight() {
    let files = [recording(0, 60, Constellation::Gps)];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let filter = GlobalFilter {
        min_duration: Some(TimeDelta::hours(1)),
        ..GlobalFilter::default()
    };

    assert_eq!(
        gt_plot::find_closest_tpv(&files, &visibility, &filter, support::at_second(30)),
        None
    );
}

/// The plot redraws its lines when the window's end moves in by less than the
/// level cache's own hysteresis threshold. Fixes leave the window on such a
/// move, and the hysteresis is there for a view the user is panning, not for
/// the filter.
#[test]
fn a_small_move_of_the_window_end_redraws_the_lines() {
    let files = vec![recording(0, 3600, Constellation::Gps)];
    let mut plot = support::drawn_plot(files, under(window(0, 3000)), PlotState::default());
    let pixels_per_point = plot.harness.inner.ctx.pixels_per_point();
    let before = plot
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");

    plot.sources_mut().filter.time_end = Some(support::at_second(2960));
    plot.harness.inner.run_steps(2);
    let after = plot
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");

    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            support::plot_area(),
            pixels_per_point
        ),
        "the last 40 s of every line left the window and must leave the plot"
    );
}

/// The lines are the only thing that can change while map-to-plot sync holds
/// the view still. This pins the redraw on the fixes the window keeps, with no
/// help from the extent the plot would otherwise re-fit to.
#[test]
fn a_pinned_view_redraws_its_lines_when_the_window_end_moves() {
    let files = vec![recording(0, 3600, Constellation::Gps)];
    let mut plot = support::drawn_plot(
        files,
        under(window(0, 3000)).pinned_to_map_view(0..=3600),
        PlotState::default(),
    );
    let pixels_per_point = plot.harness.inner.ctx.pixels_per_point();
    let view = plot.state().visible_x_range();
    let before = plot
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");

    plot.sources_mut().filter.time_end = Some(support::at_second(2960));
    plot.harness.inner.run_steps(2);
    let after = plot
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");

    assert_eq!(
        plot.state().visible_x_range(),
        view,
        "the map sync pins the view across the window move"
    );
    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            support::plot_area(),
            pixels_per_point
        ),
        "the last 40 s of every line left the window and must leave the plot"
    );
}
