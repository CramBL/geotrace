mod error;
pub use error::LoadError;

const STAGE_READING: &str = "Reading…";
const STAGE_PARSING: &str = "Parsing…";
const STAGE_CONVERTING: &str = "Converting…";
const STAGE_SEGMENTING: &str = "Segmenting…";

use std::fs::File;
use std::path::Path;

use nav_types::satellites::{Constellation, Satellite, Satellites};
use nav_types::time_types::{GpsTime, SysTime};
use nav_types::{CustomMarker, LoadedFile, MarkerIcon, NavPoint, TimePositionVelocity};
use naview_sdk::degree;
use naview_sdk::{
    Constellation as SdkConstellation, Marker as SdkMarker, MarkerIcon as SdkMarkerIcon, NavFile,
    Satellite as SdkSatellite, SatelliteReport,
};

/// Load a `.nvd` file from `path`, segment it into trips, and return a fully
/// populated `LoadedFile`.
pub fn load_file(path: impl AsRef<Path>) -> Result<LoadedFile, LoadError> {
    load_file_with_progress(path, |_, _| {})
}

/// Parse a `.nvd` file from raw bytes (e.g. delivered via drag-and-drop on Wayland).
pub fn load_bytes(bytes: &[u8], filename: String) -> Result<LoadedFile, LoadError> {
    load_bytes_with_progress(bytes, filename, |_, _| {})
}

/// Like [`load_file`] but calls `progress(fraction, stage)` at key milestones so
/// the caller can drive a progress bar. `fraction` is in `[0.0, 1.0]`.
pub fn load_file_with_progress(
    path: impl AsRef<Path>,
    progress: impl Fn(f32, &'static str),
) -> Result<LoadedFile, LoadError> {
    let path = path.as_ref();
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("unknown"))
        .to_owned();
    progress(0.05, STAGE_READING);
    let file = File::open(path)?;
    progress(0.20, STAGE_PARSING);
    let nav_file = NavFile::read(file)?;
    progress(0.65, STAGE_CONVERTING);
    let (points, markers) = from_nav_file(&nav_file)?;
    progress(0.90, STAGE_SEGMENTING);
    let loaded = nav_types::segment::build_loaded_file(filename, &points, &markers);
    Ok(loaded)
}

/// Like [`load_bytes`] but calls `progress(fraction, stage)` at key milestones.
pub fn load_bytes_with_progress(
    bytes: &[u8],
    filename: String,
    progress: impl Fn(f32, &'static str),
) -> Result<LoadedFile, LoadError> {
    progress(0.15, STAGE_PARSING);
    let nav_file = NavFile::read(bytes)?;
    progress(0.60, STAGE_CONVERTING);
    let (points, markers) = from_nav_file(&nav_file)?;
    progress(0.90, STAGE_SEGMENTING);
    let loaded = nav_types::segment::build_loaded_file(filename, &points, &markers);
    Ok(loaded)
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

        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(sdk_point.fix.effective_gps_time()))
            .lat(sdk_point.fix.lat)
            .lon(sdk_point.fix.lon)
            .maybe_heading(sdk_point.fix.heading)
            .maybe_velocity(sdk_point.fix.speed)
            .maybe_sys_time(sdk_point.fix.sys_time.map(SysTime::from_utc))
            .maybe_eph_m(sdk_point.fix.eph_m.map(|v| v as f32))
            .build();

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

    let gps_time = report.gps_time.map(GpsTime::from_utc);
    let sys_time = report.sys_time.map(SysTime::from_utc);
    Satellites::new(gps_time, sys_time, satellites)
}

fn convert_marker(m: &SdkMarker) -> CustomMarker {
    CustomMarker::new(
        m.annotation.time,
        m.annotation.label.as_deref().unwrap_or("").to_owned(),
        m.annotation.icon.map_or(MarkerIcon::Pin, convert_icon),
        m.lat,
        m.lon,
        None,
    )
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
            .gps_time(time)
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
                .gps_time(t0)
                .lat(Angle::new::<degree>(51.5))
                .lon(Angle::new::<degree>(-0.1))
                .heading(Angle::new::<degree>(270.0))
                .speed(Velocity::new::<meter_per_second>(12.5))
                .build(),
        );
        let (nav_points, _) = build(b.finish().unwrap()).unwrap();
        assert_eq!(nav_points.len(), 1);
        let tpv = nav_points[0].tpv;
        assert_eq!(tpv.time().utc(), t0);
        assert_eq!(tpv.lat().get::<degree>(), 51.5);
        assert_eq!(tpv.lon().get::<degree>(), -0.1);
        assert_eq!(tpv.heading().map(|h| h.get::<degree>()), Some(270.0));
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
                .gps_time(base())
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
                .gps_time(t0)
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
                .gps_time(base())
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
                .gps_time(base())
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
