//! The scope of the plot's show/hide-all button: the shown metrics, the
//! channels while the Channels section is open, and the solar flare markers
//! while a flare is archived over the span the plot shows.

use chrono::{DateTime, TimeDelta, Utc};
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use gt_filter::GlobalFilter;
use gt_flare::{MarkedFlare, SolarFlare};
use gt_loaded_files::RecordingNames;
use gt_plot::{ArchiveOverlays, PlotState};
use gt_test_utils::{Queryable as _, TestHarness};
use gt_types::{Channel, FileSource, LoadedFile, MetricKind, NavPoint, TimeRange};
use gt_ui_types::{
    ContextLines, GeomagneticSeries, JammingSeries, SnapErrorSeries, TecSeries, TrackDataVisibility,
};
use rstest::rstest;
use rustc_hash::FxHashMap;

/// 2024-01-15 12:00:00 UTC, the first fix of the recording below.
const FIRST_FIX_SECS: i64 = 1_705_320_000;

/// Fixes in the recording, one per second.
const FIX_COUNT: usize = 60;

/// The one channel the recording carries.
const CHANNEL_NAME: &str = "Incline";

/// A channel of a recording this test never loads.
const UNLOADED_CHANNEL_NAME: &str = "Brake pressure";

/// Plot size in points. Wide enough that a chip row and a plot both lay out.
const PLOT_SIZE: egui::Vec2 = egui::vec2(700.0, 400.0);

fn at_second(offset: i64) -> DateTime<Utc> {
    DateTime::UNIX_EPOCH + TimeDelta::seconds(FIRST_FIX_SECS + offset)
}

/// A recording of one track at 1 Hz carrying a scalar channel sampled at the
/// same rate.
fn recording_with_a_channel() -> LoadedFile {
    let points: Vec<NavPoint> = gt_test_utils::nav_points_from(at_second(0), FIX_COUNT, 1);
    let times: Vec<DateTime<Utc>> = (0..FIX_COUNT as i64).map(at_second).collect();
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.metadata.time_range = TimeRange::new(at_second(0), at_second(FIX_COUNT as i64 - 1));
    track.metadata.duration = TimeDelta::seconds(FIX_COUNT as i64 - 1);
    track.channels = vec![Channel {
        name: CHANNEL_NAME.to_owned(),
        unit: None,
        period: None,
        description: None,
        components: Vec::new(),
        values: vec![1.0; times.len()],
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

/// One flare peaking during the recording, which enables the flare chip.
#[expect(
    clippy::expect_used,
    reason = "the fixture helpers beside the tests are not covered by clippy's in-test relaxations"
)]
fn archived_flare() -> MarkedFlare {
    let peak = at_second(30);
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
            solar_flares: vec![archived_flare()],
        }
    }
}

/// A harness that has drawn the plot over the recording in `scene`.
fn drawn_plot(scene: PlotScene) -> TestHarness<'static, PlotState> {
    let PlotScene {
        show_channels,
        solar_flares,
    } = scene;
    let files = [recording_with_a_channel()];
    let names = RecordingNames::default();
    let visibility = TrackDataVisibility::from_loaded(&files);
    let filter = GlobalFilter::default();
    let snap_error = SnapErrorSeries::default();
    let jamming = JammingSeries::default();
    let geomagnetic = GeomagneticSeries::default();
    let tec = TecSeries::default();
    let context_lines = ContextLines::default();
    let mut plot = PlotState::default();
    plot.show_channels = show_channels;
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
                    solar_flares: &solar_flares,
                },
                plot,
            );
        },
        plot,
    );
    harness.run();
    harness
}

/// The icon on the show/hide-all button: the crossed-out eye while every
/// series in scope is visible, the plain eye otherwise.
fn show_hide_all_icon(harness: &TestHarness<'_, PlotState>) -> &'static str {
    if harness.inner.query_by_label(ICON_EYE_SLASH).is_some() {
        ICON_EYE_SLASH
    } else {
        ICON_EYE
    }
}

fn click_show_hide_all(harness: &mut TestHarness<'_, PlotState>) {
    let icon = show_hide_all_icon(harness);
    harness.inner.get_by_label(icon).click();
    harness.run();
}

/// The state a click reaches from the default: every series in scope visible,
/// then every one of them hidden.
fn show_all_then_hide_all(harness: &mut TestHarness<'_, PlotState>) {
    click_show_hide_all(harness);
    click_show_hide_all(harness);
}

#[test]
fn showing_all_shows_a_hidden_metric_channel_and_the_flare_markers() {
    let mut harness = drawn_plot(PlotScene::with_channels_and_a_flare());
    harness
        .state_mut()
        .metric_vis
        .set(MetricKind::Velocity, false);
    harness.state_mut().channel_vis.set(CHANNEL_NAME, false);
    harness.state_mut().show_solar_flares = false;
    harness.run();

    click_show_hide_all(&mut harness);

    assert!(harness.state().metric_vis.field(MetricKind::Velocity));
    assert!(harness.state().channel_vis.is_visible(CHANNEL_NAME));
    assert!(harness.state().show_solar_flares);
}

#[test]
fn hiding_all_hides_the_metrics_the_channel_and_the_flare_markers() {
    let mut harness = drawn_plot(PlotScene::with_channels_and_a_flare());

    show_all_then_hide_all(&mut harness);

    assert!(!harness.state().metric_vis.field(MetricKind::Velocity));
    assert!(!harness.state().channel_vis.is_visible(CHANNEL_NAME));
    assert!(!harness.state().show_solar_flares);
}

/// A hidden series in scope leaves the button offering to show all, whichever
/// series it is.
#[rstest]
#[case::everything_visible(HiddenSeries::Nothing, ICON_EYE_SLASH)]
#[case::a_hidden_channel(HiddenSeries::Channel, ICON_EYE)]
#[case::hidden_flare_markers(HiddenSeries::FlareMarkers, ICON_EYE)]
fn the_icon_states_what_a_click_changes(#[case] hidden: HiddenSeries, #[case] expected_icon: &str) {
    let mut harness = drawn_plot(PlotScene::with_channels_and_a_flare());
    click_show_hide_all(&mut harness);

    match hidden {
        HiddenSeries::Nothing => {}
        HiddenSeries::Channel => harness.state_mut().channel_vis.set(CHANNEL_NAME, false),
        HiddenSeries::FlareMarkers => harness.state_mut().show_solar_flares = false,
    }
    harness.run();

    assert_eq!(show_hide_all_icon(&harness), expected_icon);
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
    let mut harness = drawn_plot(PlotScene {
        show_channels: false,
        solar_flares: vec![archived_flare()],
    });

    show_all_then_hide_all(&mut harness);

    assert!(harness.state().channel_vis.is_visible(CHANNEL_NAME));
    assert!(!harness.state().show_solar_flares);
}

/// With no flare archived over the span the plot shows, the flare chip is
/// disabled and its markers are out of scope.
#[test]
fn hiding_all_leaves_the_markers_of_a_disabled_flare_chip_shown() {
    let mut harness = drawn_plot(PlotScene {
        show_channels: true,
        solar_flares: Vec::new(),
    });

    show_all_then_hide_all(&mut harness);

    assert!(harness.state().show_solar_flares);
    assert!(!harness.state().channel_vis.is_visible(CHANNEL_NAME));
}

/// A hide-all writes an entry for the loaded channels alone, so a channel of
/// a recording loaded later is visible.
#[test]
fn hiding_all_writes_no_entry_for_a_channel_that_is_not_loaded() {
    let mut harness = drawn_plot(PlotScene::with_channels_and_a_flare());

    show_all_then_hide_all(&mut harness);

    assert_eq!(
        harness.state().channel_vis.entries(),
        vec![(CHANNEL_NAME.to_owned(), false)]
    );
    assert!(
        harness
            .state()
            .channel_vis
            .is_visible(UNLOADED_CHANNEL_NAME)
    );
}

/// A metric with no chip in the row is out of scope: an advanced metric while
/// its section is collapsed, and a per-constellation metric of a constellation
/// the recording has no satellite of.
#[test]
fn hiding_all_leaves_a_metric_with_no_chip_untouched() {
    let mut harness = drawn_plot(PlotScene::with_channels_and_a_flare());

    show_all_then_hide_all(&mut harness);

    assert!(harness.state().metric_vis.field(MetricKind::UtilAll));
    assert!(harness.state().metric_vis.field(MetricKind::GpsSeen));
}
