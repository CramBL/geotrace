use chrono::{DateTime, Duration};
use geotrace_sdk::{
    Angle, NavFile, NavFileBuilder, NavFix, NavFixTime, PositionProvenance,
    SolutionState as SdkState,
};
use gt_types::{GeneratedMarkerKind, MapFixKind, ResolvedPosition, SolutionState, TrackGeometry};
use rstest::rstest;

#[rstest]
#[case::receiver(false)]
#[case::host(true)]
fn status_only_loss_preserves_clocks_geometry_and_navigation_intervals(#[case] host: bool) {
    let mut recorder = NavFileBuilder::new().open();
    for (second, state) in [
        (0, SdkState::Fix),
        (2, SdkState::NoFix),
        (5, SdkState::Fix),
        (7, SdkState::Fix),
    ] {
        let timestamp = DateTime::UNIX_EPOCH + Duration::seconds(second);
        let time = if host {
            NavFixTime::Host(timestamp)
        } else {
            NavFixTime::Receiver(timestamp)
        };
        let fix = if state == SdkState::NoFix {
            NavFix::status_only(time, state)
        } else {
            NavFix::builder()
                .time(time)
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(second as f64))
                .state(state)
                .build()
        };
        recorder.add(fix);
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    let decoded = NavFile::read(bytes.as_slice()).unwrap();
    let source = decoded
        .nav_points()
        .get(1)
        .expect("status or recovery epoch");
    assert!(source.fix.lat.as_degrees().is_nan());
    assert!(source.fix.lon.as_degrees().is_nan());
    assert_eq!(source.fix.state, Some(SdkState::NoFix));
    assert_eq!(source.fix.position_provenance, None);

    let file = gt_loader::load_bytes(&bytes, "status.gtd".to_owned()).unwrap();
    assert!(file.load_warnings.is_empty(), "{:?}", file.load_warnings);
    assert_eq!(file.tracks.len(), 1);
    let track = file.tracks.first().unwrap();
    assert_eq!(track.points.len(), 4);
    assert_eq!(track.metadata.invalid_position_count, 0);
    let point = track.points.get(1).expect("status or recovery epoch");
    assert!(point.has_unreported_position());
    assert_eq!(point.tpv.position(), None);
    assert_eq!(
        point.tpv.time().utc(),
        DateTime::UNIX_EPOCH + Duration::seconds(2)
    );
    assert_eq!(point.tpv.gps_time().is_none(), host);
    assert_eq!(point.tpv.sys_time().is_some(), host);
    assert_eq!(point.navigation.position_provenance, None);
    assert_eq!(point.map_fix_kind(), MapFixKind::NavigationLoss);
    let placed = track.placed_points().unwrap();
    let status = placed.get(1).expect("status or recovery epoch");
    assert!(matches!(
        status.resolved(),
        ResolvedPosition::Interpolated(_)
    ));
    let (latitude, longitude) = status.resolved_position();
    assert!(latitude.as_degrees().abs() < 1e-9);
    assert!((longitude.as_degrees() - 2.0).abs() < 1e-9);
    for index in [0, 2, 3] {
        assert!(matches!(
            placed.get(index).expect("positioned epoch").resolved(),
            ResolvedPosition::Measured(_)
        ));
    }
    let stats = track.metadata.fix_stats.unwrap();
    assert_eq!(stats.time_with_fix, Duration::seconds(4));
    assert_eq!(stats.time_without_fix, Duration::seconds(3));
    assert_eq!(stats.time_estimated_only, Duration::zero());
    assert_eq!(stats.time_unknown, Duration::zero());
    assert_eq!(stats.fix_loss_count, 1);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(3));
    let markers = &track.generated_markers;
    assert_eq!(markers.len(), 2);
    let loss = markers.first().unwrap();
    assert_eq!(loss.kind, GeneratedMarkerKind::GnssFixLost);
    assert_eq!(loss.time, DateTime::UNIX_EPOCH);
    assert_eq!(
        (loss.lat, loss.lon),
        placed
            .get(0)
            .expect("first positioned epoch")
            .resolved_position()
    );
    let recovery = markers.get(1).expect("status or recovery epoch");
    assert_eq!(recovery.time, DateTime::UNIX_EPOCH + Duration::seconds(5));
    assert_eq!(
        (recovery.lat, recovery.lon),
        placed.get(2).expect("recovery epoch").resolved_position()
    );
    assert_eq!(
        recovery.kind,
        GeneratedMarkerKind::GnssFixRegained {
            fix_lost_duration: Duration::seconds(5)
        }
    );
}

#[rstest]
#[case::no_fix(SdkState::NoFix, SolutionState::NoFix)]
#[case::unknown(SdkState::Unknown, SolutionState::Unknown)]
#[case::fix(SdkState::Fix, SolutionState::Fix)]
#[case::estimated(SdkState::EstimatedOnly, SolutionState::EstimatedOnly)]
fn a_status_only_recording_retains_state_statistics_without_map_geometry(
    #[case] state: SdkState,
    #[case] expected: SolutionState,
) {
    let mut recorder = NavFileBuilder::new().open();
    for second in [0, 2] {
        recorder.add(NavFix::status_only(
            NavFixTime::Receiver(DateTime::UNIX_EPOCH + Duration::seconds(second)),
            state,
        ));
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    let file = gt_loader::load_bytes(&bytes, "status_only.gtd".to_owned()).unwrap();
    assert!(file.load_warnings.is_empty());
    let track = file.tracks.first().unwrap();
    assert_eq!(track.geometry, TrackGeometry::NoValidPosition);
    assert!(track.placed_points().is_none());
    assert_eq!(track.metadata.invalid_position_count, 0);
    assert!(track.generated_markers.is_empty());
    for point in &track.points {
        assert!(point.has_unreported_position());
        assert_eq!(point.resolved_navigation_state().state, expected);
        assert_eq!(point.tpv.position(), None);
    }
    let stats = track.metadata.fix_stats.unwrap();
    assert_eq!(stats.total_duration(), Duration::seconds(2));
    let expected_duration = |bucket| {
        if bucket == expected {
            Duration::seconds(2)
        } else {
            Duration::zero()
        }
    };
    assert_eq!(stats.time_with_fix, expected_duration(SolutionState::Fix));
    assert_eq!(
        stats.time_without_fix,
        expected_duration(SolutionState::NoFix)
    );
    assert_eq!(
        stats.time_estimated_only,
        expected_duration(SolutionState::EstimatedOnly)
    );
    assert_eq!(
        stats.time_unknown,
        expected_duration(SolutionState::Unknown)
    );
}

#[rstest]
#[case::loss(
    f64::NAN,
    f64::NAN,
    Some(SdkState::NoFix),
    None,
    0,
    MapFixKind::NavigationLoss
)]
#[case::fix(
    f64::NAN,
    f64::NAN,
    Some(SdkState::Fix),
    None,
    0,
    MapFixKind::UnreportedPosition
)]
#[case::unknown(
    f64::NAN,
    f64::NAN,
    Some(SdkState::Unknown),
    None,
    0,
    MapFixKind::UnreportedPosition
)]
#[case::estimated(
    f64::NAN,
    f64::NAN,
    Some(SdkState::EstimatedOnly),
    None,
    0,
    MapFixKind::Estimated
)]
#[case::source_provided(
    f64::NAN,
    f64::NAN,
    Some(SdkState::Fix),
    Some(PositionProvenance::SourceProvided),
    2,
    MapFixKind::CoordinateOutOfRange
)]
#[case::synthesized(
    f64::NAN,
    f64::NAN,
    Some(SdkState::NoFix),
    Some(PositionProvenance::SdkSynthesized),
    2,
    MapFixKind::CoordinateOutOfRange
)]
#[case::unclassified(f64::NAN, f64::NAN, None, None, 2, MapFixKind::CoordinateOutOfRange)]
#[case::missing_latitude(
    f64::NAN,
    12.0,
    Some(SdkState::NoFix),
    None,
    1,
    MapFixKind::CoordinateOutOfRange
)]
#[case::missing_longitude(
    55.0,
    f64::NAN,
    Some(SdkState::Fix),
    None,
    1,
    MapFixKind::CoordinateOutOfRange
)]
#[case::infinity(
    f64::INFINITY,
    f64::NEG_INFINITY,
    Some(SdkState::NoFix),
    None,
    2,
    MapFixKind::CoordinateOutOfRange
)]
#[case::out_of_range(
    91.0,
    181.0,
    Some(SdkState::Fix),
    Some(PositionProvenance::SourceProvided),
    2,
    MapFixKind::CoordinateOutOfRange
)]
#[case::positioned(55.0, 12.0, Some(SdkState::Fix), None, 0, MapFixKind::Recorded)]
fn raw_coordinates_only_suppress_warnings_for_the_positionless_convention(
    #[case] latitude: f64,
    #[case] longitude: f64,
    #[case] state: Option<SdkState>,
    #[case] origin: Option<PositionProvenance>,
    #[case] invalid_axes: u8,
    #[case] kind: MapFixKind,
) {
    let source = NavFix::builder()
        .time(NavFixTime::Receiver(DateTime::UNIX_EPOCH))
        .lat(Angle::degrees(latitude))
        .lon(Angle::degrees(longitude))
        .maybe_state(state)
        .position_provenance(origin)
        .used_count(8)
        .build();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(source);
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    let decoded = NavFile::read(bytes.as_slice()).unwrap();
    let decoded = &decoded.nav_points().first().unwrap().fix;
    assert_eq!(decoded.lat.as_degrees().to_bits(), latitude.to_bits());
    assert_eq!(decoded.lon.as_degrees().to_bits(), longitude.to_bits());
    assert_eq!(decoded.state, state);
    assert_eq!(decoded.position_provenance, origin);
    let file = gt_loader::load_bytes(&bytes, "raw.gtd".to_owned()).unwrap();
    let track = file.tracks.first().unwrap();
    let point = track.points.first().unwrap();
    let unreported = kind != MapFixKind::Recorded && invalid_axes == 0;
    assert_eq!(point.has_unreported_position(), unreported);
    assert_eq!(
        track.metadata.invalid_position_count,
        usize::from(invalid_axes > 0)
    );
    assert_eq!(file.load_warnings.is_empty(), invalid_axes == 0);
    assert_eq!(point.map_fix_kind(), kind);
    assert_eq!(point.tpv.lat().as_written().to_bits(), latitude.to_bits());
    assert_eq!(point.tpv.lon().as_written().to_bits(), longitude.to_bits());
    let expected_state = state.map(|state| match state {
        SdkState::Fix => SolutionState::Fix,
        SdkState::NoFix => SolutionState::NoFix,
        SdkState::EstimatedOnly => SolutionState::EstimatedOnly,
        SdkState::Unknown => SolutionState::Unknown,
        _ => panic!("unexpected fixture state"),
    });
    assert_eq!(point.navigation.state, expected_state);
    assert_eq!(point.navigation.used_count, Some(8));
}
