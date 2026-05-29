use geotrace_sdk::{
    Angle, Annotation, Constellation as SdkConst, MarkerIcon as SdkIcon, NavFileBuilder, NavFix,
    Satellite as SdkSat, SatelliteReport, Velocity,
};
use gt_types::satellites::Constellation;
use uom::si::angle::degree;

fn sdk_constellation(c: Constellation) -> SdkConst {
    match c {
        Constellation::Gps => SdkConst::Gps,
        Constellation::Glonass => SdkConst::Glonass,
        Constellation::Galileo => SdkConst::Galileo,
        Constellation::Beidou => SdkConst::Beidou,
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
        // Log markers are not stored in .nvd files; map to Pin as a fallback
        gt_types::MarkerIcon::Log => SdkIcon::Pin,
    }
}

#[test]
#[expect(clippy::float_cmp, reason = "exact coordinate round-trip")]
fn round_trip_from_gt_types_test_data() {
    let nav_data = gt_types::nav_test_data();
    let marker_data = gt_types::marker_test_data();

    let mut sink = NavFileBuilder::new().open();

    for np in &nav_data {
        let tpv = np.tpv;
        let fix_b = NavFix::builder()
            .gps_time(tpv.time().utc())
            .lat(Angle::from(tpv.lat()))
            .lon(Angle::from(tpv.lon()))
            .maybe_heading(tpv.heading().map(Angle::from));
        let nav_fix = if let Some(v) = tpv.velocity() {
            fix_b.speed(Velocity::from(v)).build()
        } else {
            fix_b.build()
        };
        sink.add_nav_fix(nav_fix);

        if let Some(sats) = &np.satellites {
            let tracked: Vec<SdkSat> = sats
                .satellites()
                .map(|s| {
                    SdkSat::builder()
                        .constellation(sdk_constellation(s.constellation()))
                        .prn(s.prn())
                        .maybe_elevation(s.elevation())
                        .maybe_azimuth(s.azimuth())
                        .maybe_snr(s.snr())
                        .in_fix(s.in_fix())
                        .build()
                })
                .collect();

            sink.add_satellite_report(
                SatelliteReport::builder()
                    .maybe_gps_time(sats.gps_time().map(|t| t.utc()))
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
            .build();
        sink.add_annotation(ann);
    }

    let nav_file = sink.finish().unwrap();

    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut tmp.as_file(), &bytes).unwrap();

    let loaded = gt_io::load_file(tmp.path()).unwrap();
    let nav_points: Vec<_> = loaded.trips.iter().flat_map(|t| t.points.iter()).collect();
    let markers: Vec<_> = loaded
        .trips
        .iter()
        .flat_map(|t| t.custom_markers.iter())
        .collect();

    assert_eq!(nav_points.len(), nav_data.len());
    assert_eq!(markers.len(), marker_data.len());

    let first = nav_points.first().expect("at least one point");
    assert_eq!(
        first.tpv.lat().get::<degree>(),
        nav_data[0].tpv.lat().get::<degree>()
    );
    assert_eq!(
        first.tpv.lon().get::<degree>(),
        nav_data[0].tpv.lon().get::<degree>()
    );

    let last = nav_points.last().expect("at least one point");
    let last_orig = nav_data.last().expect("nav_data is non-empty");
    assert_eq!(
        last.tpv.lat().get::<degree>(),
        last_orig.tpv.lat().get::<degree>()
    );
    assert_eq!(
        last.tpv.lon().get::<degree>(),
        last_orig.tpv.lon().get::<degree>()
    );

    for (i, (m, orig)) in markers.iter().zip(&marker_data).enumerate() {
        assert_eq!(m.label, orig.label, "marker {i} label mismatch");
    }
}
