//! What the plot draws and reports while the global filter's time window is
//! narrower than the loaded recordings: the view it fits, the chips it
//! offers, and the fix the map highlight lands on.

use std::cell::Cell;
use std::iter;

use chrono::{DateTime, TimeDelta, Utc};
use gt_filter::GlobalFilter;
use gt_plot::PlotState;
use gt_plot::test_util::{self, PlotSources};
use gt_test_utils::Queryable as _;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{
    FileIdx, FixRef, GpsTime, LoadedFile, MetricKind, NavPoint, PointIdx, SatelliteCoverage,
    SatelliteReportMetadata, SatelliteScope, SatelliteUsageReport, TrackIdx,
};
use gt_ui_types::{
    DisplayCategory, DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, MapElementRef,
    MapEligibility, QueryMatches, TrackDataVisibility,
};
use strum::IntoEnumIterator as _;

/// A recording of one track, `count` fixes at 1 Hz from `start_offset`
/// seconds, each carrying a satellite report of `constellation`.
fn recording(start_offset: i64, count: usize, constellation: Constellation) -> LoadedFile {
    let points: Vec<NavPoint> =
        gt_test_utils::fixtures::nav_points_from(test_util::at_second(start_offset), count, 1)
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
    test_util::recording(points, Vec::new())
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
    GlobalFilter::default().with_time_bounds(
        Some(test_util::at_second(start)),
        Some(test_util::at_second(end)),
    )
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
    let plot = test_util::drawn_plot(files, under(window(1800, 1860)), PlotState::default());

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
    let plot = test_util::drawn_plot(files, under(window(0, 60)), PlotState::default());

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
    let plot = test_util::drawn_plot(files, under(window(0, 60)), PlotState::default());

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

    let closest = gt_plot::find_closest_tpv(
        &files,
        &visibility,
        &window(10, 20),
        test_util::at_second(50),
    );

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
    let filter = GlobalFilter::default().with_minimum_duration(Some(TimeDelta::hours(1)));

    assert_eq!(
        gt_plot::find_closest_tpv(&files, &visibility, &filter, test_util::at_second(30)),
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
    let mut plot = test_util::drawn_plot(files, under(window(0, 3000)), PlotState::default());
    let pixels_per_point = plot.harness.inner.ctx.pixels_per_point();
    let before = plot
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");

    plot.sources_mut().filter.set_time_bounds(
        Some(test_util::at_second(0)),
        Some(test_util::at_second(2960)),
    );
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
            test_util::plot_area(),
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
    let mut plot = test_util::drawn_plot(
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

    plot.sources_mut().filter.set_time_bounds(
        Some(test_util::at_second(0)),
        Some(test_util::at_second(2960)),
    );
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
            test_util::plot_area(),
            pixels_per_point
        ),
        "the last 40 s of every line left the window and must leave the plot"
    );
}

#[rstest::rstest]
#[case::query_hidden_nearest(false, false, true, Some(29))]
#[case::hidden_ghost(true, false, false, Some(29))]
#[case::hidden_measured(true, true, true, Some(30))]
#[case::no_present_target(false, true, false, None)]
fn cross_highlight_searches_map_present_candidates(
    #[case] ghost_nearest: bool,
    #[case] hide_measured: bool,
    #[case] show_ghosts: bool,
    #[case] expected: Option<usize>,
) {
    let mut file = recording(0, 60, Constellation::Gps);
    if ghost_nearest {
        file.tracks[0].points[30].satellites = Some(Satellites::new(None, None, Vec::new()));
    }
    let files = [file];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let filter = GlobalFilter::default();
    let generated = GeneratedMarkerVisibility::default();
    let events = EventMarkerVisibility::default();
    let mut query = QueryMatches::default();
    if !ghost_nearest && !hide_measured {
        query
            .hidden
            .insert(test_util::track0(), iter::once(30..31).collect());
    }
    let mut display = DisplayMask::default();
    display.set_visible(DisplayCategory::TrackPoints, !hide_measured);
    display.set_visible(DisplayCategory::GhostFixes, show_ghosts);
    let scope = MapEligibility::new(
        &files,
        &visibility,
        &filter,
        Some(&query),
        &generated,
        &events,
    )
    .with_display_mask(display);
    let preparations = Cell::new(0);
    let closest = gt_plot::find_closest_tpv_matching(&files, test_util::at_second(30), |track| {
        preparations.set(preparations.get() + 1);
        let prepared = scope.prepare_track(track);
        Some(move |point| prepared.draws(MapElementRef::Fix(FixRef::new(track, point))))
    });
    assert_eq!(closest.map(|(_, _, point)| point.as_usize()), expected);
    assert_eq!(preparations.get(), 1);
}

#[test]
fn cross_highlight_searches_another_track_after_a_query_hidden_track() {
    let files = [
        recording(0, 60, Constellation::Gps),
        recording(0, 60, Constellation::Gps),
    ];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let filter = GlobalFilter::default();
    let generated = GeneratedMarkerVisibility::default();
    let events = EventMarkerVisibility::default();
    let mut query = QueryMatches::default();
    query
        .hidden
        .insert(test_util::track0(), iter::once(0..60).collect());
    let scope = MapEligibility::new(
        &files,
        &visibility,
        &filter,
        Some(&query),
        &generated,
        &events,
    )
    .with_display_mask(DisplayMask::default());
    let closest = gt_plot::find_closest_tpv_matching(&files, test_util::at_second(30), |track| {
        let prepared = scope.prepare_track(track);
        Some(move |point| prepared.draws(MapElementRef::Fix(FixRef::new(track, point))))
    });
    assert_eq!(
        closest,
        Some((FileIdx::new(1), TrackIdx::new(0), PointIdx::new(30)))
    );
}

#[test]
fn a_time_window_with_only_an_offset_report_displays_and_hovers_its_sample() {
    let mut points = test_util::fixes(2, 10);
    let time = GpsTime::from_utc(test_util::at_second(10) + TimeDelta::milliseconds(400));
    points
        .last_mut()
        .expect("last fix")
        .satellite_evidence
        .usage
        .push(SatelliteUsageReport {
            metadata: SatelliteReportMetadata {
                gps_time: Some(time),
                sys_time: None,
                normalized_time: time,
                input_order: 0,
                scope: SatelliteScope::Global,
                coverage: SatelliteCoverage::Partial,
                reported_count: Some(8),
            },
            satellites: Vec::new(),
        });
    let filter = GlobalFilter::default().with_time_bounds(
        Some(test_util::at_second(10) + TimeDelta::milliseconds(100)),
        Some(test_util::at_second(10) + TimeDelta::milliseconds(500)),
    );
    let mut state = PlotState::default();
    for kind in MetricKind::iter() {
        state.metric_vis.set(kind, kind == MetricKind::SatsFix);
    }
    let mut plot = test_util::drawn_plot(
        vec![test_util::recording(points, Vec::new())],
        under(filter),
        state,
    );
    let circles = plot.painted_circle_centers();
    assert!(
        circles.iter().any(
            |point| (point.x - time.as_secs_f64_with_subseconds()).abs() < 0.001
                && (point.y - 8.0).abs() < 0.001
        )
    );
    plot.hover(plot.screen_position(test_util::PlotPosition {
        offset_secs: 10.4,
        y: 8.0,
    }));
    assert!(plot.hover_label().contains("12:00:10.400 UTC"));
    plot.sources_mut().filter = GlobalFilter::default().with_time_bounds(
        Some(test_util::at_second(10) + TimeDelta::milliseconds(100)),
        Some(test_util::at_second(10) + TimeDelta::milliseconds(300)),
    );
    plot.run();
    plot.hover(plot.screen_position(test_util::PlotPosition {
        offset_secs: 10.299,
        y: 8.0,
    }));
    assert!(!plot.hover_label().contains("Sats fix"));
}

#[test]
fn sparse_count_samples_draw_isolated_dots_without_a_connecting_line() {
    let mut points = test_util::fixes(6, 1);
    for index in [0, 5] {
        let point = points.get_mut(index).expect("source point");
        point.navigation.used_count = Some(8);
    }
    let mut state = PlotState::default();
    for kind in MetricKind::iter() {
        state.metric_vis.set(kind, kind == MetricKind::SatsFix);
    }
    let plot = test_util::drawn_plot(
        vec![test_util::recording(points, Vec::new())],
        PlotSources::default(),
        state,
    );
    let circles: Vec<_> = plot
        .painted_circle_centers()
        .into_iter()
        .filter(|point| (point.y - 8.0).abs() < 0.01)
        .collect();
    assert_eq!(circles.len(), 2);
    let base = test_util::at_second(0).timestamp() as f64;
    assert!((circles.first().expect("first dot").x - base).abs() < 0.001);
    assert!((circles.last().expect("last dot").x - (base + 5.0)).abs() < 0.001);
    assert!(plot.painted_polylines().iter().all(|line| {
        !line
            .iter()
            .any(|point| (point.y - 8.0).abs() < 0.01 && (point.x - base).abs() < 0.01)
            || !line
                .iter()
                .any(|point| (point.y - 8.0).abs() < 0.01 && (point.x - (base + 5.0)).abs() < 0.01)
    }));
}
