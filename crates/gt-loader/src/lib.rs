mod error;
pub use error::LoadError;

/// Derive a stable grouping identity from GTD file metadata.
///
/// Priority:
/// 1. Explicit SDK-supplied identity - returned as-is.
/// 2. `meta_title` and/or `meta_device` - combined as `auto:title::device`,
///    `auto:title`, or `auto:device`.
/// 3. Filename fallback - `auto:<filename>`.
pub fn derive_identity(
    explicit: Option<&str>,
    title: Option<&str>,
    device: Option<&str>,
    filename: &str,
) -> String {
    if let Some(id) = explicit {
        return id.to_owned();
    }
    match (title, device) {
        (Some(t), Some(d)) => format!("auto:{t}::{d}"),
        (Some(t), None) => format!("auto:{t}"),
        (None, Some(d)) => format!("auto:{d}"),
        (None, None) => format!("auto:{filename}"),
    }
}

const STAGE_READING: &str = "Reading…";
const STAGE_PARSING: &str = "Parsing…";
const STAGE_CONVERTING: &str = "Converting…";
const STAGE_SEGMENTING: &str = "Segmenting…";

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use geotrace_sdk::{
    Constellation as SdkConstellation, EventMarkerColor as SdkEventMarkerColor,
    EventMarkerIconChoice as SdkEventMarkerIconChoice, EventMarkerPoint,
    EventMarkerStyle as SdkEventMarkerStyle, Marker as SdkMarker, MarkerIcon as SdkMarkerIcon,
    NavFile, Satellite as SdkSatellite, SatelliteReport, collect_satellite_warnings,
};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::{GpsTime, SysTime};
use gt_types::{
    CustomMarker, EventMarker, EventMarkerStyle, FileSource, Latitude, LoadWarning, LoadedFile,
    Longitude, MarkerColor, MarkerIcon, NavPoint, TimePositionVelocity,
};

fn to_uom_velocity(v: geotrace_sdk::Velocity) -> uom::si::f64::Velocity {
    uom::si::f64::Velocity::new::<uom::si::velocity::meter_per_second>(v.as_meters_per_second())
}

fn to_uom_angle(a: geotrace_sdk::Angle) -> uom::si::f64::Angle {
    uom::si::f64::Angle::new::<uom::si::angle::degree>(a.as_degrees())
}

/// Load a `.gtd` file from `path`, segment it into tracks, and return a fully
/// populated `LoadedFile`.
pub fn load_file(path: impl AsRef<Path>) -> Result<LoadedFile, LoadError> {
    load_file_with_progress(
        path,
        |_, _| {},
        &gt_track_builder::SegmentationConfig::default(),
    )
}

/// Parse a `.gtd` file from raw bytes (e.g. delivered via drag-and-drop on Wayland).
pub fn load_bytes(bytes: &[u8], filename: String) -> Result<LoadedFile, LoadError> {
    load_bytes_with_progress(
        bytes,
        filename,
        |_, _| {},
        &gt_track_builder::SegmentationConfig::default(),
    )
}

/// Like [`load_file`] but calls `progress(fraction, stage)` at key milestones so
/// the caller can drive a progress bar. `fraction` is in `[0.0, 1.0]`.
pub fn load_file_with_progress(
    path: impl AsRef<Path>,
    progress: impl Fn(f32, &'static str),
    config: &gt_track_builder::SegmentationConfig,
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
    let (points, markers, event_markers, event_marker_styles) = from_nav_file(&nav_file)?;
    let load_warnings = satellite_warnings_from_nav_file(&nav_file);
    progress(0.90, STAGE_SEGMENTING);
    let source = FileSource::GtdPath(path.to_path_buf());
    let identity = derive_identity(
        nav_file.meta().identity.as_deref(),
        nav_file.meta().title.as_deref(),
        nav_file.meta().device.as_deref(),
        &filename,
    );
    let loaded = gt_track_builder::build_loaded_file(
        filename,
        identity,
        &points,
        &markers,
        event_markers,
        event_marker_styles,
        config,
        source,
        load_warnings,
    );
    Ok(loaded)
}

/// Like [`load_bytes`] but calls `progress(fraction, stage)` at key milestones.
pub fn load_bytes_with_progress(
    bytes: &[u8],
    filename: String,
    progress: impl Fn(f32, &'static str),
    config: &gt_track_builder::SegmentationConfig,
) -> Result<LoadedFile, LoadError> {
    progress(0.15, STAGE_PARSING);
    let nav_file = NavFile::read(bytes)?;
    progress(0.60, STAGE_CONVERTING);
    let (points, markers, event_markers, event_marker_styles) = from_nav_file(&nav_file)?;
    let load_warnings = satellite_warnings_from_nav_file(&nav_file);
    progress(0.90, STAGE_SEGMENTING);
    let source = FileSource::GtdBytes(Arc::from(bytes));
    let identity = derive_identity(
        nav_file.meta().identity.as_deref(),
        nav_file.meta().title.as_deref(),
        nav_file.meta().device.as_deref(),
        &filename,
    );
    let loaded = gt_track_builder::build_loaded_file(
        filename,
        identity,
        &points,
        &markers,
        event_markers,
        event_marker_styles,
        config,
        source,
        load_warnings,
    );
    Ok(loaded)
}

fn satellite_warnings_from_nav_file(nav_file: &NavFile) -> Vec<LoadWarning> {
    collect_satellite_warnings(
        nav_file
            .nav_points()
            .iter()
            .filter_map(|p| p.satellites.as_ref()),
    )
    .into_iter()
    .map(|w| LoadWarning {
        count: w.count,
        issue: w.issue.to_owned(),
        description: w.description.to_owned(),
    })
    .collect()
}

type NavFileContents = (
    Vec<NavPoint>,
    Vec<CustomMarker>,
    Vec<EventMarker>,
    Vec<EventMarkerStyle>,
);

fn from_nav_file(nav_file: &NavFile) -> Result<NavFileContents, LoadError> {
    let mut nav_points = Vec::with_capacity(nav_file.nav_points().len());

    for (idx, sdk_point) in nav_file.nav_points().iter().enumerate() {
        let lat_deg = sdk_point.fix.lat.as_degrees();
        let lon_deg = sdk_point.fix.lon.as_degrees();

        if !(-90.0..=90.0).contains(&lat_deg) {
            return Err(LoadError::LatitudeOutOfRange { lat: lat_deg, idx });
        }
        if !(-180.0..=180.0).contains(&lon_deg) {
            return Err(LoadError::LongitudeOutOfRange { lon: lon_deg, idx });
        }

        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(sdk_point.fix.effective_gps_time()))
            .lat(Latitude::new(lat_deg))
            .lon(Longitude::new(lon_deg))
            .maybe_heading(sdk_point.fix.heading.map(to_uom_angle))
            .maybe_velocity(sdk_point.fix.speed.map(to_uom_velocity))
            .maybe_sys_time(sdk_point.fix.sys_time.map(SysTime::from_utc))
            .maybe_eph_m(sdk_point.fix.eph_m.map(|v| v as f32))
            .build();

        let satellites = sdk_point.satellites.as_ref().map(convert_satellite_report);

        nav_points.push(NavPoint::new(tpv, satellites));
    }

    let markers = nav_file.markers().iter().map(convert_marker).collect();

    let event_markers = nav_file
        .event_markers()
        .iter()
        .map(convert_event_marker)
        .collect();

    let event_marker_styles = nav_file
        .event_marker_styles()
        .iter()
        .map(convert_event_marker_style)
        .collect();

    Ok((nav_points, markers, event_markers, event_marker_styles))
}

fn convert_event_marker(m: &EventMarkerPoint) -> EventMarker {
    EventMarker::new(
        m.sys_time,
        m.variant_path.clone(),
        m.annotation.clone(),
        Latitude::new(m.lat.as_degrees()),
        Longitude::new(m.lon.as_degrees()),
    )
}

fn convert_event_marker_style(s: &SdkEventMarkerStyle) -> EventMarkerStyle {
    let icon = match s.icon {
        SdkEventMarkerIconChoice::Auto => MarkerIcon::Pin,
        SdkEventMarkerIconChoice::Icon(i) => sdk_icon_to_marker_icon(i),
    };
    let color = match &s.color {
        SdkEventMarkerColor::Auto => {
            gt_types::markers::event_marker_fallback_color(&s.variant_path)
        }
        SdkEventMarkerColor::Hex(hex) => {
            parse_hex_color(hex).unwrap_or(MarkerColor::new(128, 128, 128))
        }
    };
    EventMarkerStyle {
        variant_path: s.variant_path.clone(),
        icon,
        color,
    }
}

fn sdk_icon_to_marker_icon(i: SdkMarkerIcon) -> MarkerIcon {
    match i {
        SdkMarkerIcon::Pin => MarkerIcon::Pin,
        SdkMarkerIcon::Cross => MarkerIcon::Cross,
        SdkMarkerIcon::Circle => MarkerIcon::Circle,
        SdkMarkerIcon::Lightning => MarkerIcon::Lightning,
        SdkMarkerIcon::Warning => MarkerIcon::Warning,
        SdkMarkerIcon::Error => MarkerIcon::Error,
        SdkMarkerIcon::Check => MarkerIcon::Check,
        SdkMarkerIcon::Satellite => MarkerIcon::Satellite,
        SdkMarkerIcon::SatelliteLost => MarkerIcon::SatelliteLost,
        SdkMarkerIcon::Gear => MarkerIcon::Gear,
        SdkMarkerIcon::Refresh => MarkerIcon::Refresh,
        SdkMarkerIcon::Download => MarkerIcon::Download,
        SdkMarkerIcon::Upload => MarkerIcon::Upload,
        SdkMarkerIcon::Wrench => MarkerIcon::Wrench,
    }
}

fn parse_hex_color(hex: &str) -> Option<MarkerColor> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(hex.get(..2)?, 16).ok()?;
    let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
    let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
    Some(MarkerColor::new(r, g, b))
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
        Latitude::new(m.lat.as_degrees()),
        Longitude::new(m.lon.as_degrees()),
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
        SdkMarkerIcon::Satellite => MarkerIcon::Satellite,
        SdkMarkerIcon::SatelliteLost => MarkerIcon::SatelliteLost,
        SdkMarkerIcon::Gear => MarkerIcon::Gear,
        SdkMarkerIcon::Refresh => MarkerIcon::Refresh,
        SdkMarkerIcon::Download => MarkerIcon::Download,
        SdkMarkerIcon::Upload => MarkerIcon::Upload,
        SdkMarkerIcon::Wrench => MarkerIcon::Wrench,
    }
}

#[cfg(test)]
mod identity_tests {
    use super::derive_identity;

    #[test]
    fn explicit_identity_returned_as_is() {
        assert_eq!(
            derive_identity(Some("my-device"), Some("title"), Some("device"), "file.gtd"),
            "my-device"
        );
    }

    #[test]
    fn both_title_and_device_combined() {
        assert_eq!(
            derive_identity(None, Some("MyTitle"), Some("MyDevice"), "file.gtd"),
            "auto:MyTitle::MyDevice"
        );
    }

    #[test]
    fn title_only() {
        assert_eq!(
            derive_identity(None, Some("MyTitle"), None, "file.gtd"),
            "auto:MyTitle"
        );
    }

    #[test]
    fn device_only() {
        assert_eq!(
            derive_identity(None, None, Some("MyDevice"), "file.gtd"),
            "auto:MyDevice"
        );
    }

    #[test]
    fn filename_fallback() {
        assert_eq!(
            derive_identity(None, None, None, "recording.gtd"),
            "auto:recording.gtd"
        );
    }

    #[test]
    fn derive_identity_is_deterministic() {
        let a = derive_identity(None, Some("T"), Some("D"), "f.gtd");
        let b = derive_identity(None, Some("T"), Some("D"), "f.gtd");
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geotrace_sdk::{
        Angle, Annotation, Constellation as SdkConst, DateTime, Duration, MarkerIcon as SdkIcon,
        NavFile, NavFileBuilder, NavFix, Satellite as SdkSat, SatelliteReport, Utc, Velocity,
    };
    use uom::si::velocity::meter_per_second as uom_mps;

    fn base() -> DateTime<Utc> {
        DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is always valid")
    }

    fn minimal_fix(time: DateTime<Utc>) -> NavFix {
        NavFix::builder()
            .gps_time(time)
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build()
    }

    fn build(nav_file: &NavFile) -> Result<(Vec<NavPoint>, Vec<CustomMarker>), LoadError> {
        from_nav_file(nav_file).map(|(pts, markers, _, _)| (pts, markers))
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "direct f64 round-trip comparisons")]
    fn field_by_field_nav_fix() {
        let t0 = base();
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(
            NavFix::builder()
                .gps_time(t0)
                .lat(Angle::degrees(51.5))
                .lon(Angle::degrees(-0.1))
                .heading(Angle::degrees(270.0))
                .speed(Velocity::meter_per_second(12.5))
                .build(),
        );
        let (nav_points, _) = build(&sink.finish().unwrap()).unwrap();
        assert_eq!(nav_points.len(), 1);
        let tpv = nav_points[0].tpv;
        assert_eq!(tpv.time().utc(), t0);
        assert_eq!(tpv.lat().as_degrees(), 51.5);
        assert_eq!(tpv.lon().as_degrees(), -0.1);
        assert_eq!(
            tpv.heading().map(|h| h.get::<uom::si::angle::degree>()),
            Some(270.0)
        );
        assert_eq!(tpv.velocity().map(|v| v.get::<uom_mps>()), Some(12.5));
    }

    #[test]
    fn speed_none_propagation() {
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(minimal_fix(base()));
        let (nav_points, _) = build(&sink.finish().unwrap()).unwrap();
        assert_eq!(nav_points[0].tpv.velocity(), None);
    }

    #[test]
    fn velocity_unit_preservation() {
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .speed(Velocity::meter_per_second(15.0))
                .build(),
        );
        let (nav_points, _) = build(&sink.finish().unwrap()).unwrap();
        assert_eq!(
            nav_points[0].tpv.velocity().map(|v| v.get::<uom_mps>()),
            Some(15.0)
        );
    }

    #[test]
    fn satellite_structure() {
        let t0 = base();
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(minimal_fix(t0));
        sink.add_satellite_report(
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
        let (nav_points, _) = build(&sink.finish().unwrap()).unwrap();
        let sats = nav_points[0].satellites.as_ref().unwrap();
        assert_eq!(sats.satellite_count(), 2);
        assert_eq!(sats.fix_count(), 1);
        let first = sats.satellites().next().unwrap();
        assert_eq!(first.constellation(), Constellation::Gps);
        assert_eq!(first.prn(), 3);
        assert_eq!(first.elevation(), Some(30.0));
        assert_eq!(first.azimuth(), Some(90.0));
        assert_eq!(first.snr().map(|s| s.value()), Some(28.0));
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
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(minimal_fix(t0));
        sink.add_nav_fix(minimal_fix(t1));
        sink.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .build(),
        );
        let (_, markers) = build(&sink.finish().unwrap()).unwrap();
        assert_eq!(markers[0].label, "");
    }

    #[test]
    fn marker_label_empty_string() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(minimal_fix(t0));
        sink.add_nav_fix(minimal_fix(t1));
        sink.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .label(String::new())
                .build(),
        );
        let (_, markers) = build(&sink.finish().unwrap()).unwrap();
        assert_eq!(markers[0].label, "");
    }

    #[test]
    fn marker_icon_none() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(minimal_fix(t0));
        sink.add_nav_fix(minimal_fix(t1));
        sink.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .build(),
        );
        let (_, markers) = build(&sink.finish().unwrap()).unwrap();
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
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(91.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        let nav_file = sink.finish().unwrap();
        let err = from_nav_file(&nav_file).unwrap_err();
        assert!(
            matches!(err, LoadError::LatitudeOutOfRange { lat, idx: 0 } if (lat - 91.0).abs() < 1e-10),
            "expected LatitudeOutOfRange, got: {err:?}"
        );
    }

    #[test]
    fn lon_out_of_range() {
        let mut sink = NavFileBuilder::new().open();
        sink.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(-181.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        let nav_file = sink.finish().unwrap();
        let err = from_nav_file(&nav_file).unwrap_err();
        assert!(
            matches!(err, LoadError::LongitudeOutOfRange { lon, idx: 0 } if (lon - -181.0).abs() < 1e-10),
            "expected LongitudeOutOfRange, got: {err:?}"
        );
    }
}
