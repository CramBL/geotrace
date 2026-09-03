//! What the plot draws and reports while the global filter's time window is
//! narrower than the loaded recordings: the view it fits, the chips it
//! offers, and the fix the map highlight lands on.

use std::ops::RangeInclusive;

use chrono::{DateTime, TimeDelta, Utc};
use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingNames;
use gt_plot::{ArchiveOverlays, PlotState};
use gt_test_utils::{Queryable as _, TestHarness};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{Channel, FileIdx, FileSource, LoadedFile, NavPoint, PointIdx, TimeRange, TrackIdx};
use gt_ui_types::{
    ContextLines, GeomagneticSeries, JammingSeries, SnapErrorSeries, TecSeries, TrackDataVisibility,
};
use rustc_hash::FxHashMap;
use support::{PLOT_SIZE, at_second, plot_area};

mod support;

/// A recording of one track, `count` fixes at 1 Hz from `start_offset`
/// seconds, each carrying a satellite report of `constellation`.
fn recording(start_offset: i64, count: usize, constellation: Constellation) -> LoadedFile {
    let points: Vec<NavPoint> = gt_test_utils::nav_points_from(at_second(start_offset), count, 1)
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
    let mut track = gt_test_utils::loaded_track_with_points(points);
    let first = at_second(start_offset);
    let last = at_second(start_offset + count as i64 - 1);
    track.metadata.time_range = TimeRange::new(first, last);
    track.metadata.duration = last - first;
    LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdBytes([].into()),
        load_warnings: Vec::new(),
    }
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
        track.channels = vec![Channel {
            name: channel_name.to_owned(),
            unit: None,
            period: None,
            description: None,
            components: Vec::new(),
            times,
            values,
        }];
    }
    file
}

/// A window from `start` to `end`, both in seconds from the first fix.
fn window(start: i64, end: i64) -> GlobalFilter {
    GlobalFilter {
        time_start: Some(at_second(start)),
        time_end: Some(at_second(end)),
        ..GlobalFilter::default()
    }
}

/// Everything the plot reads besides the recordings and the filter, all of it
/// empty: no archive covers these recordings, and none of them was snapped.
#[derive(Default)]
struct EmptySources {
    snap_error: SnapErrorSeries,
    jamming: JammingSeries,
    geomagnetic: GeomagneticSeries,
    tec: TecSeries,
    context_lines: ContextLines,
}

/// The plot's own state, the filter it draws under and the map viewport it
/// syncs to, so a test can move any of them between two frames the way the
/// app does.
struct PlotUnderFilter {
    plot: PlotState,
    filter: GlobalFilter,
    map_sync_x_range: Option<(f64, f64)>,
}

impl PlotUnderFilter {
    /// A plot under `filter` whose view fits the data it draws.
    fn new(filter: GlobalFilter) -> Self {
        Self {
            plot: PlotState::default(),
            filter,
            map_sync_x_range: None,
        }
    }

    /// A plot under `filter` whose x bounds map-to-plot sync pinned to
    /// `view`, in seconds from the first fix. A pinned view no longer re-fits
    /// to the data.
    fn pinned_to_map_view(filter: GlobalFilter, view: RangeInclusive<i64>) -> Self {
        let seconds = |offset: i64| at_second(offset).timestamp() as f64;
        Self {
            map_sync_x_range: Some((seconds(*view.start()), seconds(*view.end()))),
            ..Self::new(filter)
        }
    }
}

/// Draw the plot over `files` in `state` until the view settles.
fn draw(files: &[LoadedFile], mut state: PlotUnderFilter) -> TestHarness<'_, PlotUnderFilter> {
    let names = RecordingNames::default();
    let visibility = TrackDataVisibility::from_loaded(files);
    let sources = EmptySources::default();
    state.plot.rebuild_all(files);

    let mut harness = TestHarness::builder().size(PLOT_SIZE).ui_state(
        move |ui, state: &mut PlotUnderFilter| {
            gt_plot::show_track_plot(
                ui,
                files,
                &names,
                &visibility,
                &state.filter,
                None,
                None,
                None,
                state.map_sync_x_range,
                &sources.snap_error,
                &sources.jamming,
                &sources.geomagnetic,
                &sources.tec,
                ArchiveOverlays {
                    context_lines: &sources.context_lines,
                    solar_flares: &[],
                },
                &mut state.plot,
            );
        },
        state,
    );
    harness.run();
    harness
}

/// A window of one minute over an hour-long recording resets to the minute,
/// not to the hour. The view fits the data the plot draws, and the plot draws
/// no fix outside the time window.
#[test]
fn the_view_fits_the_time_window_rather_than_the_whole_recording() {
    let files = [recording(0, 3600, Constellation::Gps)];
    let harness = draw(&files, PlotUnderFilter::new(window(1800, 1860)));

    let shown = harness
        .state()
        .plot
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
    let files = [
        recording(0, 60, Constellation::Gps),
        recording(7200, 60, Constellation::Qzss),
    ];
    let harness = draw(&files, PlotUnderFilter::new(window(0, 60)));

    assert!(
        harness.inner.query_by_label("QZSS seen").is_none(),
        "the QZSS chip belongs to a recording the window leaves out"
    );
}

/// The Channels section toggle renders when a track on the plot carries
/// channels, and the channel chips come from the same union: a recording the
/// time window leaves out must reveal neither.
#[test]
fn a_recording_outside_the_time_window_reveals_no_channels_section() {
    let files = [
        recording(0, 60, Constellation::Gps),
        with_channel(recording(7200, 60, Constellation::Gps), "Brake pressure"),
    ];
    let harness = draw(&files, PlotUnderFilter::new(window(0, 60)));

    assert!(
        harness.inner.query_by_label_contains("Channels").is_none(),
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

    let closest = gt_plot::find_closest_tpv(&files, &visibility, &window(10, 20), at_second(50));

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
        gt_plot::find_closest_tpv(&files, &visibility, &filter, at_second(30)),
        None
    );
}

/// The plot redraws its lines when the window's end moves in by less than the
/// level cache's own hysteresis threshold. Fixes leave the window on such a
/// move, and the hysteresis is there for a view the user is panning, not for
/// the filter.
#[test]
fn a_small_move_of_the_window_end_redraws_the_lines() {
    let files = [recording(0, 3600, Constellation::Gps)];
    let mut harness = draw(&files, PlotUnderFilter::new(window(0, 3000)));
    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let plot_area = plot_area();
    let before = harness.inner.render().expect("the harness renders a frame");

    harness.state_mut().filter.time_end = Some(at_second(2960));
    harness.inner.run_steps(2);
    let after = harness.inner.render().expect("the harness renders a frame");

    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            plot_area,
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
    let files = [recording(0, 3600, Constellation::Gps)];
    let mut harness = draw(
        &files,
        PlotUnderFilter::pinned_to_map_view(window(0, 3000), 0..=3600),
    );
    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let plot_area = plot_area();
    let view = harness.state().plot.visible_x_range();
    let before = harness.inner.render().expect("the harness renders a frame");

    harness.state_mut().filter.time_end = Some(at_second(2960));
    harness.inner.run_steps(2);
    let after = harness.inner.render().expect("the harness renders a frame");

    assert_eq!(
        harness.state().plot.visible_x_range(),
        view,
        "the map sync pins the view across the window move"
    );
    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            plot_area,
            pixels_per_point
        ),
        "the last 40 s of every line left the window and must leave the plot"
    );
}
