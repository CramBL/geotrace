use chrono::Duration;
use geotrace_sdk::{
    Angle, Annotation, Constellation as SdkConst, MarkerIcon as SdkIcon, NavFile, NavFileBuilder,
    NavFix, NavFixTime, RecordedFixTimestamps, Satellite as SdkSat, SatelliteReport, Velocity,
};
use gt_types::satellites::Constellation;
use gt_types::{ResolvedPosition, SatelliteQueryScope, SolutionMethod, SolutionState};
use uom::si::angle::degree;

fn sdk_constellation(c: Constellation) -> SdkConst {
    match c {
        Constellation::Gps => SdkConst::Gps,
        Constellation::Glonass => SdkConst::Glonass,
        Constellation::Galileo => SdkConst::Galileo,
        Constellation::Beidou => SdkConst::Beidou,
        Constellation::Navic => SdkConst::Navic,
        Constellation::Qzss => SdkConst::Qzss,
    }
}

fn sdk_icon(icon: gt_types::MarkerIcon) -> SdkIcon {
    match icon {
        gt_types::MarkerIcon::Pin => SdkIcon::Pin,
        gt_types::MarkerIcon::Cross => SdkIcon::Cross,
        gt_types::MarkerIcon::Circle => SdkIcon::Circle,
        gt_types::MarkerIcon::Lightning => SdkIcon::Lightning,
        gt_types::MarkerIcon::Warning => SdkIcon::Warning,
        gt_types::MarkerIcon::Error => SdkIcon::Error,
        gt_types::MarkerIcon::Check => SdkIcon::Check,
        gt_types::MarkerIcon::Satellite => SdkIcon::Satellite,
        gt_types::MarkerIcon::SatelliteLost => SdkIcon::SatelliteLost,
        gt_types::MarkerIcon::Gear => SdkIcon::Gear,
        gt_types::MarkerIcon::Refresh => SdkIcon::Refresh,
        gt_types::MarkerIcon::Download => SdkIcon::Download,
        gt_types::MarkerIcon::Upload => SdkIcon::Upload,
        gt_types::MarkerIcon::Wrench => SdkIcon::Wrench,
    }
}

#[test]
fn round_trip_from_gt_types_test_data() {
    let nav_data = gt_test_utils::nav_test_data();
    let marker_data = gt_test_utils::marker_test_data();

    let mut recorder = NavFileBuilder::new().open();

    for np in &nav_data {
        let tpv = np.tpv;
        let fix_b = NavFix::builder()
            .time(NavFixTime::Receiver(tpv.time().utc()))
            .lat(Angle::degrees(tpv.lat().as_written()))
            .lon(Angle::degrees(tpv.lon().as_written()))
            .maybe_heading(tpv.heading().map(Angle::from));
        let nav_fix = if let Some(v) = tpv.velocity() {
            fix_b.speed(Velocity::from(v)).build()
        } else {
            fix_b.build()
        };
        recorder.add_nav_fix(nav_fix);

        if let Some(sats) = &np.satellites {
            let tracked: Vec<SdkSat> = sats
                .satellites()
                .map(|s| {
                    SdkSat::builder()
                        .constellation(sdk_constellation(s.constellation()))
                        .prn(s.prn().value())
                        .maybe_elevation(s.elevation())
                        .maybe_azimuth(s.azimuth())
                        .maybe_snr(s.snr().map(|s| s.value()))
                        .in_fix(s.in_fix())
                        .build()
                })
                .collect();

            let recorded = RecordedFixTimestamps {
                gps: sats.gps_time().map(|t| t.utc()),
                sys: sats.sys_time().map(|t| t.utc()),
            };
            let time = NavFixTime::from_recorded(recorded)
                .expect("the test data stamps every report with a receiver time");
            recorder.add_satellite_report(
                SatelliteReport::builder()
                    .time(time)
                    .tracked(tracked)
                    .build(),
            );
        }
    }

    for marker in &marker_data {
        let label_opt = if marker.label.is_empty() {
            None
        } else {
            Some(marker.label.clone())
        };
        let ann = Annotation::builder()
            .time(marker.time)
            .icon(sdk_icon(marker.icon))
            .maybe_label(label_opt)
            .build()
            .expect("the marker label fits its field");
        recorder.add_annotation(ann);
    }

    let nav_file = recorder.finish().unwrap();

    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut tmp.as_file(), &bytes).unwrap();

    let loaded = gt_loader::load_file(tmp.path()).unwrap();
    let nav_points: Vec<_> = loaded.tracks.iter().flat_map(|t| t.points.iter()).collect();
    let markers: Vec<_> = loaded
        .tracks
        .iter()
        .flat_map(|t| t.custom_markers.iter())
        .collect();

    assert_eq!(nav_points.len(), nav_data.len());
    assert_eq!(markers.len(), marker_data.len());

    let first = nav_points.first().expect("at least one point");
    assert_eq!(
        first.tpv.lat().as_written(),
        nav_data[0].tpv.lat().as_written()
    );
    assert_eq!(
        first.tpv.lon().as_written(),
        nav_data[0].tpv.lon().as_written()
    );

    let last = nav_points.last().expect("at least one point");
    let last_orig = nav_data.last().expect("nav_data is non-empty");
    assert_eq!(
        last.tpv.lat().as_written(),
        last_orig.tpv.lat().as_written()
    );
    assert_eq!(
        last.tpv.lon().as_written(),
        last_orig.tpv.lon().as_written()
    );

    for (i, (m, orig)) in markers.iter().zip(&marker_data).enumerate() {
        assert_eq!(m.label, orig.label, "marker {i} label mismatch");
    }
}

#[test]
fn navigation_only_records_preserve_source_facts() {
    let source =
        gt_test_utils::navigation_evidence_fixtures::sparse_navigation_recording().unwrap();
    let mut bytes = Vec::new();
    source.write(&mut bytes).unwrap();
    let decoded = NavFile::read(bytes.as_slice()).unwrap();
    let loaded = gt_loader::load_bytes(&bytes, "navigation_only.gtd".to_owned()).unwrap();
    let track = loaded.tracks.first().unwrap();
    assert_eq!(track.points.len(), 9);
    assert_eq!(track.metadata.invalid_position_count, 0);
    let expected: Vec<_> = decoded
        .nav_points()
        .iter()
        .map(|point| {
            (
                point.fix.time.effective(),
                point.fix.lat.as_degrees().to_bits(),
                point.fix.lon.as_degrees().to_bits(),
                point.fix.state.is_some(),
                point.fix.used_count,
                point.fix.hdop,
            )
        })
        .collect();
    let actual: Vec<_> = track
        .points
        .iter()
        .map(|point| {
            (
                point.tpv.time().utc(),
                point.tpv.lat().as_written().to_bits(),
                point.tpv.lon().as_written().to_bits(),
                point.navigation.state.is_some(),
                point.navigation.used_count,
                point.navigation.hdop,
            )
        })
        .collect();
    assert_eq!(actual, expected);
    assert!(track.points.iter().all(|point| {
        point.satellite_evidence.usage.is_empty()
            && point.satellite_evidence.observations.is_empty()
            && point
                .resolved_seen_count(SatelliteQueryScope::Global)
                .is_none()
            && point.tpv.eph_m().is_none()
    }));
    let status = track.points.get(2).expect("status epoch");
    assert!(status.has_unreported_position());
    assert!(status.tpv.position().is_none());
    assert!(status.navigation.position_provenance.is_none());
    assert!(matches!(
        track.geometry.measured().unwrap().resolved_positions.get(2),
        Some(ResolvedPosition::Interpolated(_))
    ));
    let rtk = track.points.get(1).expect("RTK epoch");
    assert_eq!(rtk.navigation.hdop, Some(0.9));
    assert_eq!(rtk.navigation.method, Some(SolutionMethod::RtkFixed));
    assert_eq!(
        rtk.tpv.heading().unwrap().get::<degree>().to_bits(),
        75.0_f64.to_bits()
    );
}

#[test]
fn navigation_only_records_preserve_state_intervals() {
    let source =
        gt_test_utils::navigation_evidence_fixtures::sparse_navigation_recording().unwrap();
    let mut bytes = Vec::new();
    source.write(&mut bytes).unwrap();
    let loaded = gt_loader::load_bytes(&bytes, "navigation_only.gtd".to_owned()).unwrap();
    let track = loaded.tracks.first().unwrap();
    let states: Vec<_> = track
        .points
        .iter()
        .map(|point| point.resolved_navigation_state().state)
        .collect();
    assert_eq!(
        states,
        vec![
            SolutionState::Fix,
            SolutionState::Fix,
            SolutionState::NoFix,
            SolutionState::Fix,
            SolutionState::Fix,
            SolutionState::Unknown,
            SolutionState::Fix,
            SolutionState::Unknown,
            SolutionState::NoFix,
        ]
    );
    let stats = track.metadata.fix_stats.unwrap();
    assert_eq!(stats.time_with_fix, Duration::seconds(7));
    assert_eq!(stats.time_without_fix, Duration::seconds(3));
    assert_eq!(stats.time_unknown, Duration::seconds(1));
    assert_eq!(stats.fix_loss_count, 2);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(3));
}
