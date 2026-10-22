use chrono::TimeDelta;
use egui_plot::PlotPoint;
use gt_filter::GlobalFilter;
use gt_plot::test_util::{self, PlotSources};
use gt_plot::{AnalysisConfig, PlotState};
use gt_types::satellites::{Constellation, ConstellationSet, Satellite, Satellites};
use gt_types::tpv::TimePositionVelocity;
use gt_types::{
    FileIdx, GpsTime, LoadedFile, MetricKind, NavPoint, PositionProvenance, SatelliteCoverage,
    SatelliteReportMetadata, SatelliteScope, SatelliteUsageReport, SolutionState,
};
use gt_ui_types::TrackDataVisibility;
use rstest::rstest;

fn sparse_recording() -> LoadedFile {
    let points = test_util::fixes(6, 1)
        .into_iter()
        .enumerate()
        .map(|(index, mut point)| {
            point.tpv = TimePositionVelocity::builder()
                .time(point.tpv.time())
                .lat(point.tpv.lat())
                .lon(point.tpv.lon())
                .build();
            point.navigation.hdop = match index {
                0 => Some(0.9),
                5 => Some(0.0),
                _ => None,
            };
            point.navigation.used_count = (index == 0).then_some(8);
            point.navigation.state = Some(SolutionState::Fix);
            point.navigation.position_provenance = Some(PositionProvenance::SourceProvided);
            point
        })
        .collect();
    test_util::recording(points, Vec::new())
}

fn full_recording() -> LoadedFile {
    let points = test_util::fixes(6, 1)
        .into_iter()
        .map(|point| {
            let satellites = Satellites::new(
                Some(point.tpv.time()),
                None,
                vec![Satellite::new(
                    Constellation::Gps,
                    1,
                    Some(45.0),
                    Some(120.0),
                    Some(38.0),
                    true,
                )],
            );
            NavPoint::new(point.tpv, Some(satellites))
        })
        .collect();
    test_util::recording(points, Vec::new())
}

#[rstest]
#[case::all_samples(None, None, false)]
#[case::missing_gap(Some(1), Some(4), true)]
#[case::single_zero(Some(5), Some(5), false)]
fn dop_availability_requires_a_sample_inside_the_time_window(
    #[case] start: Option<i64>,
    #[case] end: Option<i64>,
    #[case] disabled: bool,
) {
    let sources = PlotSources {
        filter: GlobalFilter::default().with_time_bounds(
            start.map(test_util::at_second),
            end.map(test_util::at_second),
        ),
        ..PlotSources::default()
    };
    let plot = test_util::drawn_plot(vec![sparse_recording()], sources, PlotState::default());
    assert_eq!(plot.metric_chip_disabled("HDOP"), disabled);
    assert!(plot.metric_chip_disabled("VDOP"));
    assert!(plot.metric_chip_disabled("PDOP"));
    assert!(plot.state().metric_vis.field(MetricKind::Hdop));
}

#[test]
fn diagnostics_use_any_displayed_track_and_preserve_selection_across_loads() {
    let mut state = PlotState::default();
    state.show_advanced_metrics = true;
    let mut plot = test_util::drawn_plot(
        vec![sparse_recording(), full_recording()],
        PlotSources::default(),
        state,
    );
    assert!(!plot.metric_chip_disabled("Util all (%)"));
    plot.harness
        .state_mut()
        .visibility
        .show_only_file(FileIdx::new(0));
    plot.run();
    assert!(plot.metric_chip_disabled("Util all (%)"));
    plot.show_all_then_hide_all();
    assert!(plot.state().metric_vis.field(MetricKind::UtilAll));
    let files = vec![full_recording()];
    let state = plot.harness.state_mut();
    state.plot.rebuild_all(&files);
    state.visibility = TrackDataVisibility::from_loaded(&files);
    state.files = files;
    plot.run();
    assert!(!plot.metric_chip_disabled("Util all (%)"));
    assert!(plot.state().metric_vis.field(MetricKind::UtilAll));
}

#[test]
fn changing_analysis_prerequisites_updates_availability_and_preserves_dop_runs() {
    let mut file = full_recording();
    for point in &mut file.tracks[0].points {
        point.navigation.hdop = Some(0.9);
    }
    let mut state = PlotState::default();
    state.show_advanced_metrics = true;
    let mut plot = test_util::drawn_plot(vec![file], PlotSources::default(), state);
    assert!(!plot.metric_chip_disabled("Util all (%)"));
    let state = plot.harness.state_mut();
    state.plot.set_analysis(
        &state.files,
        AnalysisConfig {
            elevation_mask_deg: 60.0,
            ..AnalysisConfig::default()
        },
    );
    plot.run();
    assert!(plot.metric_chip_disabled("Util all (%)"));
    assert!(!plot.metric_chip_disabled("HDOP"));
}

#[rstest]
#[case::reported_zero(Some(0), false)]
#[case::partial_list(None, true)]
fn a_named_scope_exposes_controls_with_independent_count_availability(
    #[case] reported_count: Option<u32>,
    #[case] disabled: bool,
) {
    let mut points = test_util::fixes(2, 1);
    let time = points[0].tpv.time();
    points[0]
        .satellite_evidence
        .usage
        .push(SatelliteUsageReport {
            metadata: SatelliteReportMetadata {
                gps_time: Some(time),
                sys_time: None,
                normalized_time: time,
                input_order: 0,
                scope: SatelliteScope::Constellations(ConstellationSet::from_iter([
                    Constellation::Gps,
                ])),
                coverage: SatelliteCoverage::Partial,
                reported_count,
            },
            satellites: Vec::new(),
        });
    let plot = test_util::drawn_plot(
        vec![test_util::recording(points, Vec::new())],
        PlotSources::default(),
        PlotState::default(),
    );
    assert_eq!(plot.metric_chip_disabled("GPS fix"), disabled);
    assert!(plot.metric_chip_disabled("Sats fix"));
}

#[test]
fn report_samples_after_the_last_fix_enable_their_metric_in_a_fractional_window() {
    let mut points = test_util::fixes(2, 1);
    let time = GpsTime::from_utc(test_util::at_second(1) + TimeDelta::milliseconds(400));
    points[1]
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
                reported_count: Some(0),
            },
            satellites: Vec::new(),
        });
    let sources = PlotSources {
        filter: GlobalFilter::default().with_time_bounds(Some(time.utc()), Some(time.utc())),
        ..PlotSources::default()
    };
    let plot = test_util::drawn_plot(
        vec![test_util::recording(points, Vec::new())],
        sources,
        PlotState::default(),
    );
    assert!(!plot.metric_chip_disabled("Sats fix"));
}

#[rstest]
#[case::dark(egui::Theme::Dark, "sparse_navigation_metrics_dark")]
#[case::light(egui::Theme::Light, "sparse_navigation_metrics_light")]
fn snapshot_sparse_navigation_metrics(#[case] theme: egui::Theme, #[case] snapshot: &str) {
    let mut plot = test_util::drawn_plot_in_theme(
        vec![sparse_recording()],
        PlotSources::default(),
        PlotState::default(),
        theme,
    );
    plot.snapshot(snapshot);
}

#[rstest]
fn navigation_only_samples_enable_measurements_and_disable_satellite_diagnostics(
    #[values(egui::Theme::Dark, egui::Theme::Light)] theme: egui::Theme,
) {
    let recording =
        gt_test_utils::navigation_evidence_fixtures::sparse_navigation_recording().unwrap();
    let mut bytes = Vec::new();
    recording.write(&mut bytes).unwrap();
    let file = gt_loader::load_bytes(&bytes, "navigation_only.gtd".to_owned()).unwrap();
    let mut state = PlotState::default();
    state.show_advanced_metrics = true;
    let mut plot = test_util::drawn_plot_in_theme(vec![file], PlotSources::default(), state, theme);
    for label in ["Sats fix", "HDOP", "Velocity (km/h)", "Heading (°)"] {
        assert!(!plot.metric_chip_disabled(label), "{label}");
    }
    for label in ["Sats seen", "VDOP", "PDOP", "Util all (%)", "EPH (m)"] {
        assert!(plot.metric_chip_disabled(label), "{label}");
    }
    plot.snapshot(&format!(
        "navigation_only_metrics_{}",
        if theme == egui::Theme::Dark {
            "dark"
        } else {
            "light"
        }
    ));
}

#[test]
fn a_single_navigation_only_speed_sample_is_visible_and_selectable() {
    let recording =
        gt_test_utils::navigation_evidence_fixtures::sparse_navigation_recording().unwrap();
    let mut bytes = Vec::new();
    recording.write(&mut bytes).unwrap();
    let file = gt_loader::load_bytes(&bytes, "navigation_only.gtd".to_owned()).unwrap();
    let sample_time = file
        .tracks
        .first()
        .unwrap()
        .points
        .get(1)
        .expect("speed epoch")
        .tpv
        .time();
    let time = sample_time.as_secs_f64();
    let mut plot = test_util::drawn_plot(vec![file], PlotSources::default(), PlotState::default());
    assert!(
        plot.painted_circle_centers()
            .iter()
            .any(|point| (point.x - time).abs() < 0.01 && (point.y - 15.12).abs() < 0.01)
    );
    let sample = plot
        .transform()
        .position_from_point(&PlotPoint::new(time, 15.12));
    plot.hover(sample);
    assert_eq!(plot.hover_label(), "Velocity (km/h)\n12:00:01\n15.12");
    let start = sample_time.utc() + TimeDelta::seconds(4);
    plot.sources_mut().filter = GlobalFilter::default().with_time_bounds(Some(start), None);
    plot.run();
    assert!(plot.metric_chip_disabled("Velocity (km/h)"));
    assert!(
        plot.painted_circle_centers()
            .iter()
            .all(|point| (point.y - 15.12).abs() > 0.01)
    );
    let excluded = plot
        .transform()
        .position_from_point(&PlotPoint::new(time, 15.12));
    plot.hover(excluded);
    assert!(!plot.hover_label().contains("Velocity"));
}
