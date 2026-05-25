use nav_types::satellites::Constellation;
use naview_sdk::{
    Annotation, Constellation as SdkConst, MarkerIcon as SdkIcon, NavFileBuilder, NavFix,
    Satellite as SdkSat, SatelliteReport, degree,
};

fn sdk_constellation(c: Constellation) -> SdkConst {
    match c {
        Constellation::Gps => SdkConst::Gps,
        Constellation::Glonass => SdkConst::Glonass,
        Constellation::Galileo => SdkConst::Galileo,
        Constellation::Beidou => SdkConst::Beidou,
    }
}

fn sdk_icon(icon: nav_types::MarkerIcon) -> SdkIcon {
    match icon {
        nav_types::MarkerIcon::Pin => SdkIcon::Pin,
        nav_types::MarkerIcon::Cross => SdkIcon::Cross,
        nav_types::MarkerIcon::Circle => SdkIcon::Circle,
        nav_types::MarkerIcon::Lightning => SdkIcon::Lightning,
        nav_types::MarkerIcon::Warning => SdkIcon::Warning,
        nav_types::MarkerIcon::Error => SdkIcon::Error,
        nav_types::MarkerIcon::Check => SdkIcon::Check,
        // Log markers are not stored in .nvd files; map to Pin as a fallback
        nav_types::MarkerIcon::Log => SdkIcon::Pin,
    }
}

#[test]
#[expect(clippy::float_cmp, reason = "exact coordinate round-trip")]
fn round_trip_from_nav_types_test_data() {
    let nav_data = nav_types::nav_test_data();
    let marker_data = nav_types::marker_test_data();

    let mut builder = NavFileBuilder::new();

    for np in &nav_data {
        let tpv = np.tpv;
        let fix_b = NavFix::builder()
            .gps_time(tpv.time().utc())
            .lat(tpv.lat())
            .lon(tpv.lon())
            .maybe_heading(tpv.heading());
        let nav_fix = if let Some(v) = tpv.velocity() {
            fix_b.speed(v).build()
        } else {
            fix_b.build()
        };
        builder.add_nav_fix(nav_fix);

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

            builder.add_satellite_report(
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
        builder.add_annotation(ann);
    }

    let nav_file = builder.finish().unwrap();

    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut tmp.as_file(), &bytes).unwrap();

    let loaded = nav_io::load_file(tmp.path()).unwrap();
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
