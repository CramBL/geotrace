//! What the plot draws for a sensor channel whose sample timestamps step
//! backwards, and the two gates the marks follow: the Channels section and the
//! setting behind them.

use chrono::{DateTime, TimeDelta, Utc};
use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingNames;
use gt_plot::{ArchiveOverlays, PlotState};
use gt_test_utils::TestHarness;
use gt_types::{Channel, FileSource, LoadedFile, NavPoint, TimeRange};
use gt_ui_types::{
    ContextLines, GeomagneticSeries, JammingSeries, SnapErrorSeries, TecSeries, TrackDataVisibility,
};
use rustc_hash::FxHashMap;

/// 2024-01-15 12:00:00 UTC, the first fix of the recording below.
const FIRST_FIX_SECS: i64 = 1_705_320_000;

/// Fixes in the recording, one per second.
const FIX_COUNT: usize = 60;

/// Plot size in points. Wide enough that a chip row and a plot both lay out.
const PLOT_SIZE: egui::Vec2 = egui::vec2(700.0, 400.0);

fn at_second(offset: i64) -> DateTime<Utc> {
    DateTime::UNIX_EPOCH + TimeDelta::seconds(FIRST_FIX_SECS + offset)
}

/// A recording of one track at 1 Hz carrying a scalar channel sampled at the
/// same rate, whose timestamps step back by ten seconds halfway through.
fn recording_with_a_backward_time_step() -> LoadedFile {
    let points: Vec<NavPoint> = gt_test_utils::nav_points_from(at_second(0), FIX_COUNT, 1);
    let times: Vec<DateTime<Utc>> = (0..FIX_COUNT as i64)
        .map(|i| {
            if i < 30 {
                at_second(i)
            } else {
                at_second(i - 10)
            }
        })
        .collect();
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.metadata.time_range = TimeRange::new(at_second(0), at_second(FIX_COUNT as i64 - 1));
    track.metadata.duration = TimeDelta::seconds(FIX_COUNT as i64 - 1);
    track.channels = vec![Channel {
        name: "Incline".to_owned(),
        unit: None,
        period: None,
        description: None,
        components: Vec::new(),
        values: times.iter().map(|_| 1.0).collect(),
        times,
    }];
    LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdBytes([].into()),
        load_warnings: Vec::new(),
    }
}

/// The two gates the marks follow.
#[derive(Clone, Copy)]
struct MarkGates {
    show_channels: bool,
    mark_backward_time_steps: bool,
}

/// A harness that has drawn the plot over the recording under `gates`.
fn drawn_plot(gates: MarkGates) -> TestHarness<'static, PlotState> {
    let files = [recording_with_a_backward_time_step()];
    let names = RecordingNames::default();
    let visibility = TrackDataVisibility::from_loaded(&files);
    let filter = GlobalFilter::default();
    let snap_error = SnapErrorSeries::default();
    let jamming = JammingSeries::default();
    let geomagnetic = GeomagneticSeries::default();
    let tec = TecSeries::default();
    let context_lines = ContextLines::default();
    let mut plot = PlotState::default();
    plot.show_channels = gates.show_channels;
    plot.mark_backward_time_steps = gates.mark_backward_time_steps;
    plot.rebuild_all(&files);

    let mut harness = TestHarness::builder().size(PLOT_SIZE).ui_state(
        move |ui, plot: &mut PlotState| {
            gt_plot::show_track_plot(
                ui,
                &files,
                &names,
                &visibility,
                &filter,
                None,
                None,
                None,
                None,
                &snap_error,
                &jamming,
                &geomagnetic,
                &tec,
                ArchiveOverlays {
                    context_lines: &context_lines,
                    solar_flares: &[],
                },
                plot,
            );
        },
        plot,
    );
    harness.run();
    harness
}

/// The area every comparison below reads: the whole plot, marks included.
fn plot_area() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, PLOT_SIZE)
}

#[test]
fn the_marks_draw_with_the_channels_revealed() {
    let mut marked = drawn_plot(MarkGates {
        show_channels: true,
        mark_backward_time_steps: true,
    });
    let mut unmarked = drawn_plot(MarkGates {
        show_channels: true,
        mark_backward_time_steps: false,
    });
    let pixels_per_point = marked.inner.ctx.pixels_per_point();

    let with_marks = marked.inner.render().expect("the harness renders a frame");
    let without_marks = unmarked
        .inner
        .render()
        .expect("the harness renders a frame");

    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &with_marks,
            &without_marks,
            plot_area(),
            pixels_per_point
        ),
        "the channel's backward time step must reach the plot"
    );
}

/// The marks annotate the channel lines, which the collapsed Channels section
/// leaves off the plot.
#[test]
fn a_collapsed_channels_section_draws_no_mark() {
    let mut marks_on = drawn_plot(MarkGates {
        show_channels: false,
        mark_backward_time_steps: true,
    });
    let mut marks_off = drawn_plot(MarkGates {
        show_channels: false,
        mark_backward_time_steps: false,
    });
    let pixels_per_point = marks_on.inner.ctx.pixels_per_point();

    let with_setting = marks_on
        .inner
        .render()
        .expect("the harness renders a frame");
    let without_setting = marks_off
        .inner
        .render()
        .expect("the harness renders a frame");

    assert!(
        !gt_test_utils::snapshot_harness::pixels_differ(
            &with_setting,
            &without_setting,
            plot_area(),
            pixels_per_point
        ),
        "the setting must draw nothing while the Channels section is collapsed"
    );
}
