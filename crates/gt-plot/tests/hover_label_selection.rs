//! Which item the plot picks when several lie within reach of the pointer,
//! and which sample the label it draws names.

#![expect(
    clippy::expect_used,
    reason = "the fixture helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use gt_flare::{MarkedFlare, SolarFlare};
use gt_plot::PlotState;
use gt_solar::GeomagneticIndex;
use gt_types::{
    Channel, FileIdx, FileSource, LoadedFile, MetricKind, NavPoint, TimeRange, TrackIdx, TrackRef,
};
use gt_ui_types::{GeomagneticPoint, IndexContextSample, TecContextSample, TecPoint};
use rstest::rstest;
use rustc_hash::FxHashMap;
use strum::IntoEnumIterator as _;
use support::{DrawnPlot, PlotPosition, PlotSources, at_second};

mod support;

const CHANNEL_NAME: &str = "Incline";
const SECOND_CHANNEL_NAME: &str = "Brake pressure";

/// Fixes of the recording whose velocity line the plot draws downsampled: far
/// past the ~2 samples per pixel the finest level would hand over.
const DOWNSAMPLED_FIX_COUNT: usize = 12_000;

/// Value between the two constant channel lines of
/// [`snapshot_the_plot_labels_the_line_it_added_last`], which sets them about
/// 3 points apart on screen.
const CHANNEL_LINE_GAP_VALUE: f64 = 0.15;

/// The constant velocity of every fix [`gt_test_utils::nav_points_from`]
/// builds, in km/h, which is where the velocity line is drawn.
const FIXTURE_VELOCITY_KMH: f64 = 15.0;

/// A scalar channel sampled at `times`, one value per sample.
fn scalar_channel(name: &str, times: Vec<DateTime<Utc>>, values: Vec<f64>) -> Channel {
    Channel {
        name: name.to_owned(),
        unit: None,
        period: None,
        description: None,
        components: Vec::new(),
        values,
        times,
    }
}

/// A recording of one track of `fix_count` fixes `step_secs` apart from the
/// first fix, carrying `channels`.
fn recording(fix_count: usize, step_secs: i64, channels: Vec<Channel>) -> LoadedFile {
    let points: Vec<NavPoint> = gt_test_utils::nav_points_from(at_second(0), fix_count, step_secs);
    let last_offset = (fix_count as i64 - 1) * step_secs;
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.metadata.time_range = TimeRange::new(at_second(0), at_second(last_offset));
    track.metadata.duration = TimeDelta::seconds(last_offset);
    track.channels = channels;
    LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdBytes([].into()),
        load_warnings: Vec::new(),
    }
}

/// The one track of the one recording every scene loads.
fn the_track() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

/// One archived flare peaking `offset_secs` after the first fix.
fn flare_peaking_at(offset_secs: i64) -> MarkedFlare {
    let peak = at_second(offset_secs);
    MarkedFlare {
        flare: SolarFlare {
            id: format!("{peak}-FLR-001"),
            begin: peak - TimeDelta::minutes(28),
            peak,
            end: Some(peak + TimeDelta::minutes(23)),
            classification: "X2.2".parse().expect("a published class"),
            source_location: None,
            active_region: None,
        },
        receiver_side: None,
    }
}

/// What one case draws: the recording, which metric lines are on, whether the
/// Channels section is open, and what the archives hold over the span.
struct PlotScene {
    file: LoadedFile,
    shown_metrics: Vec<MetricKind>,
    show_channels: bool,
    sources: PlotSources,
}

impl PlotScene {
    /// `file` with every metric line off, the Channels section collapsed, and
    /// the archives empty.
    fn of(file: LoadedFile) -> Self {
        Self {
            file,
            shown_metrics: Vec::new(),
            show_channels: false,
            sources: PlotSources::default(),
        }
    }

    fn showing(mut self, metrics: &[MetricKind]) -> Self {
        self.shown_metrics = metrics.to_vec();
        self
    }

    fn with_the_channels_revealed(mut self) -> Self {
        self.show_channels = true;
        self
    }

    fn with_a_flare_peaking_at(mut self, offset_secs: i64) -> Self {
        self.sources.solar_flares = vec![flare_peaking_at(offset_secs)];
        self
    }

    /// `index` archived over the plot's span, with one valued point on the
    /// track so the index's chip enables.
    fn with_an_archived_index(
        mut self,
        index: GeomagneticIndex,
        samples: Vec<IndexContextSample>,
    ) -> Self {
        let line = match index {
            GeomagneticIndex::Hp30 => &mut self.sources.context_lines.geomagnetic.hp30,
            GeomagneticIndex::Kp => &mut self.sources.context_lines.geomagnetic.kp,
        };
        *line = Arc::new(samples);
        let point = GeomagneticPoint {
            x_secs: at_second(0).timestamp() as f64,
            hp30: (index == GeomagneticIndex::Hp30).then_some(1.0),
            kp: (index == GeomagneticIndex::Kp).then_some(1.0),
        };
        self.sources
            .geomagnetic
            .points_by_track
            .insert(the_track(), Arc::new(vec![point]));
        self
    }

    /// TEC archived over the plot's span, with one valued point on the track
    /// so the TEC chip enables.
    fn with_archived_tec(mut self, samples: Vec<TecContextSample>) -> Self {
        self.sources.context_lines.tec = Arc::new(samples);
        self.sources.tec.points_by_track.insert(
            the_track(),
            Arc::new(vec![TecPoint {
                x_secs: at_second(0).timestamp() as f64,
                tecu: Some(10.0),
            }]),
        );
        self
    }

    fn draw(self) -> DrawnPlot {
        let Self {
            file,
            shown_metrics,
            show_channels,
            sources,
        } = self;
        let mut plot = PlotState::default();
        plot.show_channels = show_channels;
        for kind in MetricKind::iter() {
            plot.metric_vis.set(kind, shown_metrics.contains(&kind));
        }
        support::drawn_plot(vec![file], sources, plot)
    }
}

/// The tooltip of the archived X2.2 flare, which both flare cases below
/// assert.
const THE_FLARE_HOVER_LABEL: &str = "X2.2 solar flare\n\
    R3 strong radio blackout\n\
    Peaked at 2024-01-15T12:00 (UTC)\n\
    Began 2024-01-15T11:32, ended 2024-01-15T12:23\n\
    The catalog lists every flare the Sun produced, so a flare raises the \
    ionization above a receiver only where the Sun was up at the time.";

/// The label names the channel's first sample and the rulers draw at it, far
/// to the left of the pointer. The channel's two samples draw one segment
/// across the whole recording, and the pointer rests a third of the way along
/// it, nearest that first endpoint.
#[test]
fn snapshot_the_plot_labels_the_segment_endpoint_nearest_the_pointer() {
    let channel = scalar_channel(
        CHANNEL_NAME,
        vec![at_second(0), at_second(59)],
        vec![0.0, 10.0],
    );
    let mut plot = PlotScene::of(recording(60, 1, vec![channel]))
        .with_the_channels_revealed()
        .draw();

    let a_third_along_the_segment = plot.screen_position(PlotPosition {
        offset_secs: 20.0,
        y: 10.0 * 20.0 / 59.0,
    });
    plot.hover(a_third_along_the_segment);

    assert_eq!(plot.hover_label(), "Incline\n12:00:00\n0.00");
    plot.snapshot("hover_a_two_sample_channel_segment");
}

/// The label names 12:00:47: the sample nearest the pointer among those the
/// drawn level kept. A recording of 12 000 fixes draws its velocity line from
/// a downsampled mipmap level, and the pointer rests on that line at 12:00:40,
/// a second the recording holds a fix at.
#[test]
fn snapshot_the_plot_labels_the_nearest_sample_of_the_level_it_drew() {
    let mut plot = PlotScene::of(recording(DOWNSAMPLED_FIX_COUNT, 1, Vec::new()))
        .showing(&[MetricKind::Velocity])
        .draw();

    let on_the_line = plot.screen_position(PlotPosition {
        offset_secs: 40.5,
        y: FIXTURE_VELOCITY_KMH,
    });
    plot.hover(on_the_line);

    assert_eq!(
        plot.state().hovered_time,
        Some(at_second(40)),
        "the pointer must rest on a second the recording has a fix at"
    );
    assert_eq!(plot.hover_label(), "Velocity (km/h)\n12:00:47\n15.00");
    plot.snapshot("hover_a_downsampled_line");
}

/// The label names the upper of two channel lines, which the plot added last.
/// The lines lie about 3 points apart, both within egui_plot's interact radius
/// of a pointer 1 point below the lower one.
#[test]
fn snapshot_the_plot_labels_the_line_it_added_last() {
    let sample_times = vec![at_second(0), at_second(59)];
    let lower = scalar_channel(CHANNEL_NAME, sample_times.clone(), vec![1.0, 1.0]);
    let upper = scalar_channel(
        SECOND_CHANNEL_NAME,
        sample_times,
        vec![1.0 + CHANNEL_LINE_GAP_VALUE; 2],
    );
    // The velocity line, far above both, spreads the value axis, which puts
    // the two channel lines a few points apart.
    let mut plot = PlotScene::of(recording(60, 1, vec![lower, upper]))
        .showing(&[MetricKind::Velocity])
        .with_the_channels_revealed()
        .draw();

    let on_the_lower_line = plot.screen_position(PlotPosition {
        offset_secs: 20.0,
        y: 1.0,
    });
    let on_the_upper_line = plot.screen_position(PlotPosition {
        offset_secs: 20.0,
        y: 1.0 + CHANNEL_LINE_GAP_VALUE,
    });
    let gap_px = on_the_lower_line.y - on_the_upper_line.y;
    assert!(
        (2.0..4.0).contains(&gap_px),
        "both lines must lie within one interact radius of the pointer, {gap_px} points apart"
    );
    plot.hover(on_the_lower_line + egui::vec2(0.0, 1.0));

    assert_eq!(plot.hover_label(), "Brake pressure\n12:00:00\n1.15");
    plot.snapshot("hover_between_two_channel_lines");
}

/// The plot draws the flare's label while the pointer rests 1 point from the
/// velocity line and 3 points right of the marker's peak: a custom hover label
/// suppresses egui_plot's own for the frame.
#[test]
fn snapshot_the_plot_labels_a_flare_while_the_pointer_rests_on_a_line() {
    let mut plot = PlotScene::of(recording(60, 1, Vec::new()))
        .showing(&[MetricKind::Velocity])
        .with_a_flare_peaking_at(30)
        .draw();

    let peak_on_the_line = plot.screen_position(PlotPosition {
        offset_secs: 30.0,
        y: FIXTURE_VELOCITY_KMH,
    });
    plot.hover(peak_on_the_line + egui::vec2(3.0, 1.0));

    assert_eq!(plot.hover_label(), THE_FLARE_HOVER_LABEL);
    plot.snapshot("hover_beside_a_flare_marker");
}

/// The label names the flare with the pointer near the top of the plot, far
/// from every line: a marker runs the plot's full height and is hit-tested on
/// x alone.
#[test]
fn snapshot_the_plot_labels_a_flare_from_its_x_alone() {
    let mut plot = PlotScene::of(recording(60, 1, Vec::new()))
        .showing(&[MetricKind::Velocity])
        .with_a_flare_peaking_at(30)
        .draw();

    let peak = plot.screen_position(PlotPosition {
        offset_secs: 30.0,
        y: FIXTURE_VELOCITY_KMH,
    });
    let near_the_top = egui::pos2(peak.x, plot.transform().frame().top() + 10.0);
    plot.hover(near_the_top);

    assert_eq!(plot.hover_label(), THE_FLARE_HOVER_LABEL);
    plot.snapshot("hover_a_flare_marker_above_the_lines");
}

/// The label states the period's value and its start with the pointer resting
/// mid-step: a geomagnetic index holds its value for a whole period.
#[rstest]
#[case::hp30(
    GeomagneticIndex::Hp30,
    "Geomagnetic activity\nHp30 4.667\nActive\n30 minutes from 2024-01-15T12:30:00 (UTC)",
    "hover_mid_step_of_the_hp30_line"
)]
#[case::kp(
    GeomagneticIndex::Kp,
    "Geomagnetic activity\nKp 4.667\nActive\n3 hours from 2024-01-15T15:00:00 (UTC)",
    "hover_mid_step_of_the_kp_line"
)]
fn snapshot_the_plot_labels_the_period_the_pointer_rests_in(
    #[case] index: GeomagneticIndex,
    #[case] expected_label: &str,
    #[case] snapshot_name: &str,
) {
    let period_secs = index.period_length().num_seconds();
    let samples: Vec<IndexContextSample> = [2.0, 4.667, 6.0, 3.0]
        .into_iter()
        .enumerate()
        .map(|(step, value)| IndexContextSample {
            start_secs: at_second(step as i64 * period_secs).timestamp() as f64,
            value: Some(value),
        })
        .collect();
    let kind = match index {
        GeomagneticIndex::Hp30 => MetricKind::Hp30,
        GeomagneticIndex::Kp => MetricKind::Kp,
    };
    // A fix a minute over the four archived periods, which is the span the
    // plot fits its x range to.
    let fixes = usize::try_from(4 * period_secs / 60 + 1).expect("a positive fix count");
    let mut plot = PlotScene::of(recording(fixes, 60, Vec::new()))
        .showing(&[kind])
        .with_an_archived_index(index, samples)
        .draw();

    let mid_second_step = plot.screen_position(PlotPosition {
        offset_secs: 1.5 * period_secs as f64,
        y: 4.667,
    });
    plot.hover(mid_second_step);

    assert_eq!(plot.hover_label(), expected_label);
    plot.snapshot(snapshot_name);
}

/// The label states the value interpolated at the pointer's own instant, which
/// is no epoch of the archive. The TEC line runs from one archived map epoch
/// to the next, and the pointer rests halfway between two of them 2 hours
/// apart.
#[test]
fn snapshot_the_plot_labels_the_tec_interpolated_at_the_pointer() {
    let samples = vec![
        TecContextSample {
            x_secs: at_second(0).timestamp() as f64,
            tecu: Some(10.0),
        },
        TecContextSample {
            x_secs: at_second(7200).timestamp() as f64,
            tecu: Some(20.0),
        },
    ];
    let mut plot = PlotScene::of(recording(121, 60, Vec::new()))
        .showing(&[MetricKind::Tec])
        .with_archived_tec(samples)
        .draw();

    let halfway_between_the_epochs = plot.screen_position(PlotPosition {
        offset_secs: 3600.5,
        y: 15.0,
    });
    plot.hover(halfway_between_the_epochs);

    assert_eq!(
        plot.hover_label(),
        "Ionospheric TEC\n\
         TEC 15.0 TECU\n\
         L1 delay about 2.4 m\n\
         Interpolated between maps at 2024-01-15T13:00:00 (UTC)"
    );
    plot.snapshot("hover_the_tec_line_between_two_epochs");
}

/// The label states the pointer's own time and value: a channel run of one
/// sample draws no line, so the pointer resting on that sample reaches no
/// channel.
#[test]
fn snapshot_the_plot_draws_no_line_and_no_channel_label_for_a_run_of_one_sample() {
    let drawn = scalar_channel(
        CHANNEL_NAME,
        vec![at_second(0), at_second(59)],
        vec![0.0, 10.0],
    );
    let lone_sample = scalar_channel(SECOND_CHANNEL_NAME, vec![at_second(5)], vec![9.0]);
    let mut plot = PlotScene::of(recording(60, 1, vec![drawn, lone_sample]))
        .with_the_channels_revealed()
        .draw();

    let at_the_lone_sample = plot.screen_position(PlotPosition {
        offset_secs: 5.5,
        y: 9.0,
    });
    plot.hover(at_the_lone_sample);

    assert_eq!(plot.hover_label(), "12:00:05\n9.00");
    plot.snapshot("hover_a_channel_run_of_one_sample");
}

/// The label over either run names the channel and the sample the pointer is
/// nearest: a channel whose timestamps step back once is drawn as two lines
/// under one name.
#[test]
fn snapshot_both_runs_of_one_channel_are_labelled_by_the_channel_name() {
    let times: Vec<DateTime<Utc>> = (0..60)
        .map(|sample| {
            if sample < 30 {
                at_second(sample)
            } else {
                at_second(sample - 10)
            }
        })
        .collect();
    let values: Vec<f64> = (0..60)
        .map(|sample| if sample < 30 { 2.0 } else { 8.0 })
        .collect();
    let channel = scalar_channel(CHANNEL_NAME, times, values);
    let mut plot = PlotScene::of(recording(60, 1, vec![channel]))
        .with_the_channels_revealed()
        .draw();

    let on_the_first_run = plot.screen_position(PlotPosition {
        offset_secs: 5.0,
        y: 2.0,
    });
    plot.hover(on_the_first_run);
    assert_eq!(plot.hover_label(), "Incline\n12:00:05\n2.00");

    let on_the_second_run = plot.screen_position(PlotPosition {
        offset_secs: 40.0,
        y: 8.0,
    });
    plot.hover(on_the_second_run);
    assert_eq!(plot.hover_label(), "Incline\n12:00:40\n8.00");
    plot.snapshot("hover_the_second_run_of_a_channel");
}
