//! The scope of the plot's show/hide-all button: the shown metrics, the
//! channels while the Channels section is open, and the solar flare markers
//! while a flare is archived over the span the plot shows.

use chrono::{DateTime, Utc};
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use gt_flare::MarkedFlare;
use gt_plot::PlotState;
use gt_test_utils::Queryable as _;
use gt_types::{LoadedFile, MetricKind};
use rstest::rstest;
use support::{DrawnPlot, PlotSources};

mod support;

/// Fixes in the recording, one per second.
const FIX_COUNT: usize = 60;

/// The one channel the recording carries.
const CHANNEL_NAME: &str = "Incline";

/// A channel of a recording this test never loads.
const UNLOADED_CHANNEL_NAME: &str = "Brake pressure";

/// Where the archived flare peaks, in seconds from the first fix: inside the
/// recording, so the flare chip enables.
const FLARE_PEAK_SECS: i64 = 30;

/// A recording of one track at 1 Hz carrying a scalar channel sampled at the
/// same rate.
fn recording_with_a_channel() -> LoadedFile {
    let times: Vec<DateTime<Utc>> = (0..FIX_COUNT as i64).map(support::at_second).collect();
    let channel = gt_test_utils::fixtures::scalar_channel(
        CHANNEL_NAME,
        None,
        times.clone(),
        vec![1.0; times.len()],
    );
    support::recording(support::fixes(FIX_COUNT, 1), vec![channel])
}

/// What the plot draws the recording under: whether the Channels section is
/// open, and which flares the archive holds for the days in view.
struct PlotScene {
    show_channels: bool,
    solar_flares: Vec<MarkedFlare>,
}

impl PlotScene {
    /// Both series beside the metrics in scope: the Channels section open and
    /// a flare archived.
    fn with_channels_and_a_flare() -> Self {
        Self {
            show_channels: true,
            solar_flares: vec![support::flare_peaking_at(FLARE_PEAK_SECS)],
        }
    }

    /// A harness that has drawn the plot over the recording in this scene.
    fn draw(self) -> DrawnPlot {
        let Self {
            show_channels,
            solar_flares,
        } = self;
        let mut plot = PlotState::default();
        plot.show_channels = show_channels;
        support::drawn_plot(
            vec![recording_with_a_channel()],
            PlotSources {
                solar_flares,
                ..PlotSources::default()
            },
            plot,
        )
    }
}

impl DrawnPlot {
    /// The icon on the show/hide-all button: the crossed-out eye while every
    /// series in scope is visible, the plain eye otherwise.
    fn show_hide_all_icon(&self) -> &'static str {
        if self.harness.inner.query_by_label(ICON_EYE_SLASH).is_some() {
            ICON_EYE_SLASH
        } else {
            ICON_EYE
        }
    }

    fn click_show_hide_all(&mut self) {
        let icon = self.show_hide_all_icon();
        self.harness.inner.get_by_label(icon).click();
        self.run();
    }

    /// The state a click reaches from the default: every series in scope
    /// visible, then every one of them hidden.
    fn show_all_then_hide_all(&mut self) {
        self.click_show_hide_all();
        self.click_show_hide_all();
    }
}

#[test]
fn showing_all_shows_a_hidden_metric_channel_and_the_flare_markers() {
    let mut plot = PlotScene::with_channels_and_a_flare().draw();
    plot.state_mut().metric_vis.set(MetricKind::Velocity, false);
    plot.state_mut().channel_vis.set(CHANNEL_NAME, false);
    plot.state_mut().show_solar_flares = false;
    plot.run();

    plot.click_show_hide_all();

    assert!(plot.state().metric_vis.field(MetricKind::Velocity));
    assert!(plot.state().channel_vis.is_visible(CHANNEL_NAME));
    assert!(plot.state().show_solar_flares);
}

#[test]
fn hiding_all_hides_the_metrics_the_channel_and_the_flare_markers() {
    let mut plot = PlotScene::with_channels_and_a_flare().draw();

    plot.show_all_then_hide_all();

    assert!(!plot.state().metric_vis.field(MetricKind::Velocity));
    assert!(!plot.state().channel_vis.is_visible(CHANNEL_NAME));
    assert!(!plot.state().show_solar_flares);
}

/// A hidden series in scope leaves the button offering to show all, whichever
/// series it is.
#[rstest]
#[case::everything_visible(HiddenSeries::Nothing, ICON_EYE_SLASH)]
#[case::a_hidden_channel(HiddenSeries::Channel, ICON_EYE)]
#[case::hidden_flare_markers(HiddenSeries::FlareMarkers, ICON_EYE)]
fn the_icon_states_what_a_click_changes(#[case] hidden: HiddenSeries, #[case] expected_icon: &str) {
    let mut plot = PlotScene::with_channels_and_a_flare().draw();
    plot.click_show_hide_all();

    match hidden {
        HiddenSeries::Nothing => {}
        HiddenSeries::Channel => plot.state_mut().channel_vis.set(CHANNEL_NAME, false),
        HiddenSeries::FlareMarkers => plot.state_mut().show_solar_flares = false,
    }
    plot.run();

    assert_eq!(plot.show_hide_all_icon(), expected_icon);
}

/// Which series is hidden while every other one in scope is visible.
#[derive(Clone, Copy)]
enum HiddenSeries {
    Nothing,
    Channel,
    FlareMarkers,
}

/// A collapsed Channels section offers no channel chip, which leaves its
/// channels out of scope.
#[test]
fn hiding_all_leaves_the_channels_of_a_collapsed_section_visible() {
    let mut plot = PlotScene {
        show_channels: false,
        solar_flares: vec![support::flare_peaking_at(FLARE_PEAK_SECS)],
    }
    .draw();

    plot.show_all_then_hide_all();

    assert!(plot.state().channel_vis.is_visible(CHANNEL_NAME));
    assert!(!plot.state().show_solar_flares);
}

/// With no flare archived over the span the plot shows, the flare chip is
/// disabled and its markers are out of scope.
#[test]
fn hiding_all_leaves_the_markers_of_a_disabled_flare_chip_shown() {
    let mut plot = PlotScene {
        show_channels: true,
        solar_flares: Vec::new(),
    }
    .draw();

    plot.show_all_then_hide_all();

    assert!(plot.state().show_solar_flares);
    assert!(!plot.state().channel_vis.is_visible(CHANNEL_NAME));
}

/// A hide-all writes an entry for the loaded channels alone, so a channel of
/// a recording loaded later is visible.
#[test]
fn hiding_all_writes_no_entry_for_a_channel_that_is_not_loaded() {
    let mut plot = PlotScene::with_channels_and_a_flare().draw();

    plot.show_all_then_hide_all();

    assert_eq!(
        plot.state().channel_vis.entries(),
        vec![(CHANNEL_NAME.to_owned(), false)]
    );
    assert!(plot.state().channel_vis.is_visible(UNLOADED_CHANNEL_NAME));
}

/// A metric with no chip in the row is out of scope: an advanced metric while
/// its section is collapsed, and a per-constellation metric of a constellation
/// the recording has no satellite of.
#[test]
fn hiding_all_leaves_a_metric_with_no_chip_untouched() {
    let mut plot = PlotScene::with_channels_and_a_flare().draw();

    plot.show_all_then_hide_all();

    assert!(plot.state().metric_vis.field(MetricKind::UtilAll));
    assert!(plot.state().metric_vis.field(MetricKind::GpsSeen));
}
