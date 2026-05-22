mod error;
pub use error::LoadError;

use nav_types::satellites::{Constellation, Satellite, Satellites};
use nav_types::{CustomMarker, MarkerIcon, NavPoint, TimePositionVelocity};
use naview_sdk::degree;
use naview_sdk::{
    Constellation as SdkConstellation, Marker as SdkMarker, MarkerIcon as SdkMarkerIcon, NavFile,
    Satellite as SdkSatellite, SatelliteReport,
};

/// Load a `.nvd` file from `path`, segment it into trips, and return a fully
/// populated `LoadedFile`.
pub fn load_file(path: impl AsRef<std::path::Path>) -> Result<nav_types::LoadedFile, LoadError> {
    let path = path.as_ref();
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("unknown"))
        .to_owned();
    let file = std::fs::File::open(path)?;
    let nav_file = NavFile::read(file)?;
    let (points, markers) = from_nav_file(&nav_file)?;
    Ok(nav_types::segment::build_loaded_file(
        filename, &points, &markers,
    ))
}

/// Parse a `.nvd` file from raw bytes (e.g. delivered via drag-and-drop on Wayland).
pub fn load_bytes(bytes: &[u8], filename: String) -> Result<nav_types::LoadedFile, LoadError> {
    let nav_file = NavFile::read(bytes)?;
    let (points, markers) = from_nav_file(&nav_file)?;
    Ok(nav_types::segment::build_loaded_file(
        filename, &points, &markers,
    ))
}

fn from_nav_file(nav_file: &NavFile) -> Result<(Vec<NavPoint>, Vec<CustomMarker>), LoadError> {
    let mut nav_points = Vec::with_capacity(nav_file.nav_points().len());

    for (idx, sdk_point) in nav_file.nav_points().iter().enumerate() {
        let lat_deg = sdk_point.fix.lat.get::<degree>();
        let lon_deg = sdk_point.fix.lon.get::<degree>();

        if !(-90.0..=90.0).contains(&lat_deg) {
            return Err(LoadError::LatitudeOutOfRange { lat: lat_deg, idx });
        }
        if !(-180.0..=180.0).contains(&lon_deg) {
            return Err(LoadError::LongitudeOutOfRange { lon: lon_deg, idx });
        }

        let mut b = TimePositionVelocity::build()
            .with_time(sdk_point.fix.time)
            .with_lat(sdk_point.fix.lat)
            .with_lon(sdk_point.fix.lon)
            .with_heading(sdk_point.fix.heading);
        if let Some(v) = sdk_point.fix.speed {
            b = b.with_velocity(v);
        }
        let tpv = b.build();

        let satellites = sdk_point.satellites.as_ref().map(convert_satellite_report);

        nav_points.push(NavPoint::new(tpv, satellites));
    }

    let markers = nav_file.markers().iter().map(convert_marker).collect();

    Ok((nav_points, markers))
}

fn convert_constellation(c: SdkConstellation) -> Constellation {
    match c {
        SdkConstellation::Gps => Constellation::Gps,
        SdkConstellation::Glonass => Constellation::Glonass,
        SdkConstellation::Galileo => Constellation::Galileo,
        SdkConstellation::Beidou => Constellation::Beidou,
    }
}

fn convert_satellite_report(report: &SatelliteReport) -> Satellites {
    let satellites: Vec<Satellite> = report
        .tracked
        .iter()
        .map(|s: &SdkSatellite| {
            Satellite::new(
                convert_constellation(s.constellation),
                s.prn,
                s.elevation,
                s.azimuth,
                s.snr,
                s.in_fix,
            )
        })
        .collect();

    Satellites::new(report.time, satellites)
}

fn convert_marker(m: &SdkMarker) -> CustomMarker {
    CustomMarker {
        time: m.annotation.time,
        label: m.annotation.label.as_deref().unwrap_or("").to_owned(),
        icon: m.annotation.icon.map_or(MarkerIcon::Pin, convert_icon),
        lat: m.lat,
        lon: m.lon,
        color_group: None,
    }
}

fn convert_icon(icon: SdkMarkerIcon) -> MarkerIcon {
    match icon {
        SdkMarkerIcon::Pin => MarkerIcon::Pin,
        SdkMarkerIcon::Cross => MarkerIcon::Cross,
        SdkMarkerIcon::Circle => MarkerIcon::Circle,
        SdkMarkerIcon::Lightning => MarkerIcon::Lightning,
        SdkMarkerIcon::Warning => MarkerIcon::Warning,
        SdkMarkerIcon::Error => MarkerIcon::Error,
        SdkMarkerIcon::Check => MarkerIcon::Check,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naview_sdk::{
        Angle, Annotation, Constellation as SdkConst, DateTime, Duration, MarkerIcon as SdkIcon,
        NavFile, NavFileBuilder, NavFix, Satellite as SdkSat, SatelliteReport, Utc, Velocity,
        degree, meter_per_second,
    };

    fn base() -> DateTime<Utc> {
        DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is always valid")
    }

    fn minimal_fix(time: DateTime<Utc>) -> NavFix {
        NavFix::builder()
            .time(time)
            .lat(Angle::new::<degree>(55.0))
            .lon(Angle::new::<degree>(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build()
    }

    fn build(nav_file: NavFile) -> Result<(Vec<NavPoint>, Vec<CustomMarker>), LoadError> {
        from_nav_file(&nav_file)
    }

    // -----------------------------------------------------------------------

    #[test]
    #[expect(clippy::float_cmp, reason = "direct f64 round-trip comparisons")]
    fn field_by_field_nav_fix() {
        let t0 = base();
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(
            NavFix::builder()
                .time(t0)
                .lat(Angle::new::<degree>(51.5))
                .lon(Angle::new::<degree>(-0.1))
                .heading(Angle::new::<degree>(270.0))
                .speed(Velocity::new::<meter_per_second>(12.5))
                .build(),
        );
        let (nav_points, _) = build(b.finish().unwrap()).unwrap();
        assert_eq!(nav_points.len(), 1);
        let tpv = nav_points[0].tpv;
        assert_eq!(tpv.time(), t0);
        assert_eq!(tpv.lat().get::<degree>(), 51.5);
        assert_eq!(tpv.lon().get::<degree>(), -0.1);
        assert_eq!(tpv.heading().get::<degree>(), 270.0);
        assert_eq!(
            tpv.velocity().map(|v| v.get::<meter_per_second>()),
            Some(12.5)
        );
    }

    #[test]
    fn speed_none_propagation() {
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(minimal_fix(base()));
        let (nav_points, _) = build(b.finish().unwrap()).unwrap();
        assert_eq!(nav_points[0].tpv.velocity(), None);
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "exact unit preservation check")]
    fn velocity_unit_preservation() {
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(
            NavFix::builder()
                .time(base())
                .lat(Angle::new::<degree>(0.0))
                .lon(Angle::new::<degree>(0.0))
                .heading(Angle::new::<degree>(0.0))
                .speed(Velocity::new::<meter_per_second>(15.0))
                .build(),
        );
        let (nav_points, _) = build(b.finish().unwrap()).unwrap();
        assert_eq!(
            nav_points[0]
                .tpv
                .velocity()
                .map(|v| v.get::<meter_per_second>()),
            Some(15.0)
        );
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "exact field preservation")]
    fn satellite_structure() {
        let t0 = base();
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(minimal_fix(t0));
        b.add_satellite_report(
            SatelliteReport::builder()
                .time(t0)
                .tracked(vec![
                    SdkSat::builder()
                        .constellation(SdkConst::Gps)
                        .prn(3u32)
                        .elevation(30.0f32)
                        .azimuth(90.0f32)
                        .snr(28.0f32)
                        .in_fix(true)
                        .build(),
                    SdkSat::builder()
                        .constellation(SdkConst::Galileo)
                        .prn(7u32)
                        .build(),
                ])
                .build(),
        );
        let (nav_points, _) = build(b.finish().unwrap()).unwrap();
        let sats = nav_points[0].satellites.as_ref().unwrap();
        assert_eq!(sats.satellite_count(), 2);
        assert_eq!(sats.fix_count(), 1);
        let first = sats.satellites().next().unwrap();
        assert_eq!(first.constellation(), Constellation::Gps);
        assert_eq!(first.prn(), 3);
        assert_eq!(first.elevation(), Some(30.0));
        assert_eq!(first.azimuth(), Some(90.0));
        assert_eq!(first.snr(), Some(28.0));
    }

    #[test]
    fn constellation_mapping() {
        let consts = [
            (SdkConst::Gps, Constellation::Gps),
            (SdkConst::Glonass, Constellation::Glonass),
            (SdkConst::Galileo, Constellation::Galileo),
            (SdkConst::Beidou, Constellation::Beidou),
        ];
        for (sdk, expected) in consts {
            assert_eq!(convert_constellation(sdk), expected);
        }
    }

    #[test]
    fn marker_label_none() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(minimal_fix(t0));
        b.add_nav_fix(minimal_fix(t1));
        b.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .build(),
        );
        let (_, markers) = build(b.finish().unwrap()).unwrap();
        assert_eq!(markers[0].label, "");
    }

    #[test]
    fn marker_label_empty_string() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(minimal_fix(t0));
        b.add_nav_fix(minimal_fix(t1));
        b.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .label(String::new())
                .build(),
        );
        let (_, markers) = build(b.finish().unwrap()).unwrap();
        assert_eq!(markers[0].label, "");
    }

    #[test]
    fn marker_icon_none() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(minimal_fix(t0));
        b.add_nav_fix(minimal_fix(t1));
        b.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .build(),
        );
        let (_, markers) = build(b.finish().unwrap()).unwrap();
        assert_eq!(markers[0].icon, MarkerIcon::Pin);
    }

    #[test]
    fn marker_icon_some() {
        let pairs = [
            (SdkIcon::Pin, MarkerIcon::Pin),
            (SdkIcon::Cross, MarkerIcon::Cross),
            (SdkIcon::Circle, MarkerIcon::Circle),
            (SdkIcon::Lightning, MarkerIcon::Lightning),
            (SdkIcon::Warning, MarkerIcon::Warning),
            (SdkIcon::Error, MarkerIcon::Error),
            (SdkIcon::Check, MarkerIcon::Check),
        ];
        for (sdk, expected) in pairs {
            assert_eq!(convert_icon(sdk), expected);
        }
    }

    #[test]
    fn lat_out_of_range() {
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(
            NavFix::builder()
                .time(base())
                .lat(Angle::new::<degree>(91.0))
                .lon(Angle::new::<degree>(0.0))
                .heading(Angle::new::<degree>(0.0))
                .build(),
        );
        let nav_file = b.finish().unwrap();
        let err = from_nav_file(&nav_file).unwrap_err();
        assert!(
            matches!(err, LoadError::LatitudeOutOfRange { lat, idx: 0 } if (lat - 91.0).abs() < 1e-10),
            "expected LatitudeOutOfRange, got: {err:?}"
        );
    }

    #[test]
    fn lon_out_of_range() {
        let mut b = NavFileBuilder::new();
        b.add_nav_fix(
            NavFix::builder()
                .time(base())
                .lat(Angle::new::<degree>(0.0))
                .lon(Angle::new::<degree>(-181.0))
                .heading(Angle::new::<degree>(0.0))
                .build(),
        );
        let nav_file = b.finish().unwrap();
        let err = from_nav_file(&nav_file).unwrap_err();
        assert!(
            matches!(err, LoadError::LongitudeOutOfRange { lon, idx: 0 } if (lon - -181.0).abs() < 1e-10),
            "expected LongitudeOutOfRange, got: {err:?}"
        );
    }
}
