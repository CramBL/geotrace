//! What the plot draws for a sensor channel whose sample timestamps step
//! backwards, and the two gates the marks follow: the Channels section and the
//! setting behind them.

use chrono::{DateTime, TimeDelta, Utc};
use gt_plot::PlotState;
use gt_types::{Channel, FileSource, LoadedFile, NavPoint, TimeRange};
use rustc_hash::FxHashMap;
use support::{DrawnPlot, PlotSources, at_second, plot_area};

mod support;

/// Fixes in the recording, one per second.
const FIX_COUNT: usize = 60;

/// A recording of one track at 1 Hz carrying a scalar channel sampled at the
/// same rate, whose timestamps step back by ten seconds halfway through.
fn recording_with_a_backward_time_step() -> LoadedFile {
    let points: Vec<NavPoint> =
        gt_test_utils::fixtures::nav_points_from(at_second(0), FIX_COUNT, 1);
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

impl MarkGates {
    /// A harness that has drawn the plot over the recording under these gates.
    fn draw(self) -> DrawnPlot {
        let Self {
            show_channels,
            mark_backward_time_steps,
        } = self;
        let mut plot = PlotState::default();
        plot.show_channels = show_channels;
        plot.mark_backward_time_steps = mark_backward_time_steps;
        support::drawn_plot(
            vec![recording_with_a_backward_time_step()],
            PlotSources::default(),
            plot,
        )
    }
}

#[test]
fn the_marks_draw_with_the_channels_revealed() {
    let mut marked = MarkGates {
        show_channels: true,
        mark_backward_time_steps: true,
    }
    .draw();
    let mut unmarked = MarkGates {
        show_channels: true,
        mark_backward_time_steps: false,
    }
    .draw();
    let pixels_per_point = marked.harness.inner.ctx.pixels_per_point();

    let with_marks = marked
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");
    let without_marks = unmarked
        .harness
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
    let mut marks_on = MarkGates {
        show_channels: false,
        mark_backward_time_steps: true,
    }
    .draw();
    let mut marks_off = MarkGates {
        show_channels: false,
        mark_backward_time_steps: false,
    }
    .draw();
    let pixels_per_point = marks_on.harness.inner.ctx.pixels_per_point();

    let with_setting = marks_on
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");
    let without_setting = marks_off
        .harness
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
