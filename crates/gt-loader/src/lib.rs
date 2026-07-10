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

    // A filename already prefixed with auto: is used as-is.
    if filename.starts_with("auto:") {
        return filename.to_owned();
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
    Constellation as SdkConstellation, EventMarker as SdkEventMarker,
    EventMarkerColor as SdkEventMarkerColor, EventMarkerIconChoice as SdkEventMarkerIconChoice,
    EventMarkerPoint, EventMarkerStyle as SdkEventMarkerStyle, Marker as SdkMarker,
    MarkerIcon as SdkMarkerIcon, NavFile, NavFileBuilder, Satellite as SdkSatellite,
    SatelliteReport, collect_satellite_warnings,
};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::{GpsTime, SysTime};
use gt_types::{
    Channel, CustomMarker, EventMarker, EventMarkerStyle, FileSource, Latitude, LoadWarning,
    LoadedFile, Longitude, MarkerColor, MarkerIcon, NavPoint, TimePositionVelocity,
};

pub struct LoadedGtd {
    pub file: LoadedFile,
    pub identity: String,
}

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
    load_gtd_file_with_progress(path, progress, config).map(|loaded| loaded.file)
}

pub fn load_gtd_file_with_progress(
    path: impl AsRef<Path>,
    progress: impl Fn(f32, &'static str),
    config: &gt_track_builder::SegmentationConfig,
) -> Result<LoadedGtd, LoadError> {
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
    let (points, markers, event_markers, event_marker_styles, channels) = from_nav_file(&nav_file)?;
    let load_warnings = satellite_warnings_from_nav_file(&nav_file);
    progress(0.90, STAGE_SEGMENTING);
    let source = FileSource::GtdPath(path.to_path_buf());
    let identity = derive_identity(
        nav_file.meta().identity.as_deref(),
        nav_file.meta().title.as_deref(),
        nav_file.meta().device.as_deref(),
        &filename,
    );
    let file = gt_track_builder::build_loaded_file(
        filename,
        &points,
        &markers,
        event_markers,
        event_marker_styles,
        &channels,
        config,
        source,
        file_meta_from_nav(&nav_file),
        load_warnings,
    );
    Ok(LoadedGtd { file, identity })
}

/// Capture the recording's SDK file metadata (title/device/notes) for display in
/// the app. The identity is handled separately by [`derive_identity`].
fn file_meta_from_nav(nav_file: &NavFile) -> gt_track_builder::FileMeta {
    let meta = nav_file.meta();
    gt_track_builder::FileMeta {
        title: meta.title.clone(),
        device: meta.device.clone(),
        notes: meta.notes.clone(),
    }
}

/// Like [`load_bytes`] but calls `progress(fraction, stage)` at key milestones.
pub fn load_bytes_with_progress(
    bytes: &[u8],
    filename: String,
    progress: impl Fn(f32, &'static str),
    config: &gt_track_builder::SegmentationConfig,
) -> Result<LoadedFile, LoadError> {
    load_gtd_bytes_with_progress(bytes, filename, progress, config).map(|loaded| loaded.file)
}

pub fn load_gtd_bytes_with_progress(
    bytes: &[u8],
    filename: String,
    progress: impl Fn(f32, &'static str),
    config: &gt_track_builder::SegmentationConfig,
) -> Result<LoadedGtd, LoadError> {
    progress(0.15, STAGE_PARSING);
    let nav_file = NavFile::read(bytes)?;
    progress(0.60, STAGE_CONVERTING);
    let (points, markers, event_markers, event_marker_styles, channels) = from_nav_file(&nav_file)?;
    let load_warnings = satellite_warnings_from_nav_file(&nav_file);
    progress(0.90, STAGE_SEGMENTING);
    let source = FileSource::GtdBytes(Arc::from(bytes));
    let identity = derive_identity(
        nav_file.meta().identity.as_deref(),
        nav_file.meta().title.as_deref(),
        nav_file.meta().device.as_deref(),
        &filename,
    );
    let file = gt_track_builder::build_loaded_file(
        filename,
        &points,
        &markers,
        event_markers,
        event_marker_styles,
        &channels,
        config,
        source,
        file_meta_from_nav(&nav_file),
        load_warnings,
    );
    Ok(LoadedGtd { file, identity })
}

/// Re-encode a `.gtd` recording with the nav points in `drop_ranges` removed.
///
/// Each range is a half-open `[start, end)` slice of the original nav-point
/// sequence - the same index ranges that track segmentation produces (see
/// [`gt_track_builder::segment_tracks`]). The fixes in those ranges and their
/// satellite reports are dropped. File metadata, markers, event markers, and
/// their styles are all preserved. Marker and event-marker positions are
/// re-interpolated from the surviving fixes by the SDK builder, in lenient mode
/// so a marker that ends up outside the surviving time range is clamped with a
/// warning rather than failing the whole re-encode.
///
/// This is the persisted half of a permanent per-track delete: once the new
/// bytes replace the old recording, the dropped points cannot be recovered.
pub fn reencode_dropping_ranges(
    bytes: &[u8],
    drop_ranges: &[std::ops::Range<usize>],
) -> Result<Vec<u8>, LoadError> {
    let nav_file = NavFile::read(bytes)?;
    let point_count = nav_file.nav_points().len();

    // Mark every index that falls inside a dropped range. Ranges past the end
    // are clamped so a stale range can never panic the re-encode.
    let mut dropped = vec![false; point_count];
    for range in drop_ranges {
        let end = range.end.min(point_count);
        for slot in dropped.iter_mut().take(end).skip(range.start) {
            *slot = true;
        }
    }

    let mut recorder = NavFileBuilder::new()
        .with_meta(nav_file.meta().clone())
        .with_lenient_errors()
        .open();

    for (point, drop) in nav_file.nav_points().iter().zip(&dropped) {
        if *drop {
            continue;
        }
        recorder.add_nav_fix(point.fix);
        if let Some(report) = &point.satellites {
            recorder.add_satellite_report(report.clone());
        }
    }

    for marker in nav_file.markers() {
        recorder.add_annotation(marker.annotation.clone());
    }

    for event in nav_file.event_markers() {
        let marker = SdkEventMarker::builder()
            .variant_path(event.variant_path.clone())
            .sys_time(event.sys_time)
            .maybe_annotation(event.annotation.clone())
            .build()?;
        recorder.add_event_marker(marker);
    }

    for style in nav_file.event_marker_styles() {
        recorder.add_event_marker_style(style.clone());
    }

    // Channels are their own time series, independent of the dropped point
    // ranges, so carry them through unchanged. Samples that end up outside every
    // surviving track are dropped when the reencoded file is next segmented.
    for channel in nav_file.channels() {
        recorder.add_channel(channel.clone());
    }

    let rebuilt = recorder.finish()?;
    let mut out = Vec::new();
    rebuilt.write(&mut out)?;
    Ok(out)
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
    Vec<Channel>,
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

    let channels = nav_file.channels().iter().map(convert_channel).collect();

    Ok((
        nav_points,
        markers,
        event_markers,
        event_marker_styles,
        channels,
    ))
}

fn convert_channel(sdk: &geotrace_sdk::Channel) -> Channel {
    Channel {
        name: sdk.name().to_owned(),
        unit: sdk.unit().map(str::to_owned),
        period: sdk.period().map(to_uom_angle),
        description: sdk.description().map(str::to_owned),
        components: sdk.components().to_vec(),
        times: sdk.times().to_vec(),
        values: sdk.values().to_vec(),
    }
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
        SdkEventMarkerIconChoice::Icon(i) => convert_icon(i),
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
        SdkConstellation::Navic => Constellation::Navic,
        SdkConstellation::Qzss => Constellation::Qzss,
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

/// Converts the SDK's wire-format icon to the app's internal `MarkerIcon`.
///
/// The internal type has one variant the SDK doesn't (`Log`, never produced
/// here). This is the only place that maps between the two, so a rename on
/// either side fails to compile here rather than silently desyncing a second,
/// copy-pasted match elsewhere.
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
    use proptest::prelude::*;
    use rstest::rstest;
    use strum::EnumCount;
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
        from_nav_file(nav_file).map(|(pts, markers, _, _, _)| (pts, markers))
    }

    #[test]
    fn loaded_file_carries_channels_on_its_track() {
        let t0 = base();
        let mut recorder = NavFileBuilder::new().open();
        for i in 0..3i64 {
            recorder.add_nav_fix(minimal_fix(t0 + Duration::seconds(i)));
        }
        recorder.add_channel(
            geotrace_sdk::Channel::builder()
                .name("accel")
                .unit("g")
                .period(Angle::degrees(360.0))
                .components(["x", "y", "z"].map(String::from).to_vec())
                .times(vec![t0, t0 + Duration::seconds(2)])
                .values(vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02])
                .build()
                .expect("valid channel"),
        );
        let mut bytes = Vec::new();
        recorder.finish().unwrap().write(&mut bytes).unwrap();

        let file = load_bytes(&bytes, "ride.gtd".to_owned()).unwrap();
        // Three consecutive fixes form one track carrying the channel.
        assert_eq!(file.tracks.len(), 1);
        let channels = &file.tracks[0].channels;
        assert_eq!(channels.len(), 1);
        let accel = &channels[0];
        assert_eq!(accel.name, "accel");
        assert!(accel.is_vector());
        assert_eq!(accel.components, ["x", "y", "z"]);
        assert_eq!(accel.unit.as_deref(), Some("g"));
        assert_eq!(
            accel.period.map(|a| a.get::<uom::si::angle::degree>()),
            Some(360.0)
        );
        assert_eq!(accel.times.len(), 2);
        assert_eq!(accel.values, vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02]);
    }

    #[rstest]
    #[case(Some("Morning ride"), Some("uBlox F9P"), Some("cross-town commute"))]
    #[case(None, None, None)]
    fn loaded_file_carries_sdk_metadata(
        #[case] title: Option<&str>,
        #[case] device: Option<&str>,
        #[case] notes: Option<&str>,
    ) {
        let t0 = base();
        let mut builder = NavFileBuilder::new();
        if let Some(title) = title {
            builder = builder.with_title(title);
        }
        if let Some(device) = device {
            builder = builder.with_device(device);
        }
        if let Some(notes) = notes {
            builder = builder.with_notes(notes);
        }
        let mut recorder = builder.open();
        for i in 0..3i64 {
            recorder.add_nav_fix(minimal_fix(t0 + Duration::seconds(i)));
        }
        let mut bytes = Vec::new();
        recorder.finish().unwrap().write(&mut bytes).unwrap();

        let file = load_bytes(&bytes, "ride.gtd".to_owned()).unwrap();
        assert_eq!(file.metadata.title.as_deref(), title);
        assert_eq!(file.metadata.device.as_deref(), device);
        assert_eq!(file.metadata.notes.as_deref(), notes);
    }

    #[test]
    fn reencode_drops_only_the_given_ranges() {
        let t0 = base();
        // Five fixes one second apart, each at a distinct longitude so we can
        // tell which ones survived.
        let mut recorder = NavFileBuilder::new().open();
        for i in 0..5i64 {
            recorder.add_nav_fix(
                NavFix::builder()
                    .gps_time(t0 + Duration::seconds(i))
                    .lat(Angle::degrees(55.0))
                    .lon(Angle::degrees(i as f64))
                    .heading(Angle::degrees(0.0))
                    .build(),
            );
        }
        let mut bytes = Vec::new();
        recorder.finish().unwrap().write(&mut bytes).unwrap();

        // Drop the middle two points (indices 1 and 2).
        let reencoded =
            reencode_dropping_ranges(&bytes, std::slice::from_ref(&(1usize..3))).unwrap();

        let nav_file = NavFile::read(reencoded.as_slice()).unwrap();
        let lons: Vec<f64> = nav_file
            .nav_points()
            .iter()
            .map(|p| p.fix.lon.as_degrees().round())
            .collect();
        assert_eq!(lons, vec![0.0, 3.0, 4.0]);
    }

    #[test]
    fn reencode_preserves_channels() {
        let t0 = base();
        let mut recorder = NavFileBuilder::new().open();
        for i in 0..5i64 {
            recorder.add_nav_fix(minimal_fix(t0 + Duration::seconds(i)));
        }
        recorder.add_channel(
            geotrace_sdk::Channel::builder()
                .name("accel")
                .unit("g")
                .components(["x", "y", "z"].map(String::from).to_vec())
                .times(vec![t0, t0 + Duration::seconds(4)])
                .values(vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02])
                .build()
                .expect("valid channel"),
        );
        let mut bytes = Vec::new();
        recorder.finish().unwrap().write(&mut bytes).unwrap();

        // Dropping a point range must not drop the channel.
        let reencoded =
            reencode_dropping_ranges(&bytes, std::slice::from_ref(&(1usize..3))).unwrap();
        let nav_file = NavFile::read(reencoded.as_slice()).unwrap();
        assert_eq!(nav_file.channels().len(), 1);
        let accel = &nav_file.channels()[0];
        assert_eq!(accel.name(), "accel");
        assert_eq!(accel.components(), ["x", "y", "z"]);
        assert_eq!(accel.times().len(), 2);
    }

    #[test]
    fn reencode_clamps_ranges_past_the_end() {
        let t0 = base();
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(minimal_fix(t0));
        recorder.add_nav_fix(minimal_fix(t0 + Duration::seconds(1)));
        let mut bytes = Vec::new();
        recorder.finish().unwrap().write(&mut bytes).unwrap();

        // A range that runs past the end must not panic and must keep the rest.
        let reencoded =
            reencode_dropping_ranges(&bytes, std::slice::from_ref(&(1usize..99))).unwrap();
        let nav_file = NavFile::read(reencoded.as_slice()).unwrap();
        assert_eq!(nav_file.nav_points().len(), 1);
    }

    proptest! {
        /// Permanent delete is irreversible, so pin down the drop-range handling
        /// against arbitrary ranges - including reversed, overlapping, and
        /// out-of-bounds ones. Survivors must be exactly the points no (clamped)
        /// range covers. The all-dropped case is a don't-care (the worker deletes
        /// the whole recording instead of re-encoding), so we only require it not
        /// to invent points.
        #[test]
        fn reencode_keeps_exactly_the_undropped_points(
            raw in proptest::collection::vec((0usize..14, 0usize..14), 0..6),
        ) {
            const N: usize = 10;
            // N points with distinct longitudes 0..N, so survivors are identifiable.
            let mut recorder = NavFileBuilder::new().open();
            for i in 0..N {
                recorder.add_nav_fix(
                    NavFix::builder()
                        .gps_time(base() + Duration::seconds(i as i64))
                        .lat(Angle::degrees(55.0))
                        .lon(Angle::degrees(i as f64))
                        .heading(Angle::degrees(0.0))
                        .build(),
                );
            }
            let mut bytes = Vec::new();
            recorder.finish().unwrap().write(&mut bytes).unwrap();

            let ranges: Vec<std::ops::Range<usize>> = raw.iter().map(|&(a, b)| a..b).collect();

            // Independently compute which indices a range covers (half-open, clamped
            // to the point count. Reversed/out-of-bounds ranges cover nothing).
            let mut dropped = [false; N];
            for r in &ranges {
                let end = r.end.min(N);
                for slot in dropped.iter_mut().take(end).skip(r.start) {
                    *slot = true;
                }
            }
            let expected: Vec<f64> = (0..N)
                .filter(|&i| !dropped[i])
                .map(|i| i as f64)
                .collect();

            let result = reencode_dropping_ranges(&bytes, &ranges);
            if expected.is_empty() {
                // Every point dropped: either re-encode errors, or it yields a file
                // with no nav points - never one that resurrects dropped points.
                if let Ok(reencoded) = result {
                    let nav = NavFile::read(reencoded.as_slice()).expect("read back");
                    prop_assert!(nav.nav_points().is_empty());
                }
            } else {
                let reencoded = result.expect("re-encode");
                let nav = NavFile::read(reencoded.as_slice()).expect("read back");
                let lons: Vec<f64> = nav
                    .nav_points()
                    .iter()
                    .map(|p| p.fix.lon.as_degrees().round())
                    .collect();
                prop_assert_eq!(lons, expected);
            }
        }
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "direct f64 round-trip comparisons")]
    fn field_by_field_nav_fix() {
        let t0 = base();
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t0)
                .lat(Angle::degrees(51.5))
                .lon(Angle::degrees(-0.1))
                .heading(Angle::degrees(270.0))
                .speed(Velocity::meter_per_second(12.5))
                .build(),
        );
        let (nav_points, _) = build(&recorder.finish().unwrap()).unwrap();
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
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(minimal_fix(base()));
        let (nav_points, _) = build(&recorder.finish().unwrap()).unwrap();
        assert_eq!(nav_points[0].tpv.velocity(), None);
    }

    #[test]
    fn velocity_unit_preservation() {
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .speed(Velocity::meter_per_second(15.0))
                .build(),
        );
        let (nav_points, _) = build(&recorder.finish().unwrap()).unwrap();
        assert_eq!(
            nav_points[0].tpv.velocity().map(|v| v.get::<uom_mps>()),
            Some(15.0)
        );
    }

    #[test]
    fn satellite_structure() {
        let t0 = base();
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(minimal_fix(t0));
        recorder.add_satellite_report(
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
        let (nav_points, _) = build(&recorder.finish().unwrap()).unwrap();
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
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(minimal_fix(t0));
        recorder.add_nav_fix(minimal_fix(t1));
        recorder.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .build(),
        );
        let (_, markers) = build(&recorder.finish().unwrap()).unwrap();
        assert_eq!(markers[0].label, "");
    }

    #[test]
    fn marker_label_empty_string() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(minimal_fix(t0));
        recorder.add_nav_fix(minimal_fix(t1));
        recorder.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .label(String::new())
                .build(),
        );
        let (_, markers) = build(&recorder.finish().unwrap()).unwrap();
        assert_eq!(markers[0].label, "");
    }

    #[test]
    fn marker_icon_none() {
        let t0 = base();
        let t1 = t0 + Duration::seconds(1);
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(minimal_fix(t0));
        recorder.add_nav_fix(minimal_fix(t1));
        recorder.add_annotation(
            Annotation::builder()
                .time(t0 + Duration::milliseconds(500))
                .build(),
        );
        let (_, markers) = build(&recorder.finish().unwrap()).unwrap();
        assert_eq!(markers[0].icon, MarkerIcon::Pin);
    }

    #[test]
    fn marker_icon_some() {
        // Every `SdkMarkerIcon` variant, paired with the internal `MarkerIcon`
        // it must convert to - `convert_icon` is the only place this
        // correspondence is defined (see its doc comment), so this is a full
        // exhaustiveness check rather than a sample: a wrong mapping for any
        // variant fails here even though the match still compiles.
        //
        // (Internal `MarkerIcon` has one extra variant, `Log`, with no SDK
        // counterpart, so the table is checked against `SdkIcon::COUNT` only.)
        let pairs = [
            (SdkIcon::Pin, MarkerIcon::Pin),
            (SdkIcon::Cross, MarkerIcon::Cross),
            (SdkIcon::Circle, MarkerIcon::Circle),
            (SdkIcon::Lightning, MarkerIcon::Lightning),
            (SdkIcon::Warning, MarkerIcon::Warning),
            (SdkIcon::Error, MarkerIcon::Error),
            (SdkIcon::Check, MarkerIcon::Check),
            (SdkIcon::Satellite, MarkerIcon::Satellite),
            (SdkIcon::SatelliteLost, MarkerIcon::SatelliteLost),
            (SdkIcon::Gear, MarkerIcon::Gear),
            (SdkIcon::Refresh, MarkerIcon::Refresh),
            (SdkIcon::Download, MarkerIcon::Download),
            (SdkIcon::Upload, MarkerIcon::Upload),
            (SdkIcon::Wrench, MarkerIcon::Wrench),
        ];
        assert_eq!(pairs.len(), SdkIcon::COUNT);
        for (sdk, expected) in pairs {
            assert_eq!(convert_icon(sdk), expected);
        }
    }

    #[test]
    fn lat_out_of_range() {
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(91.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        let nav_file = recorder.finish().unwrap();
        let err = from_nav_file(&nav_file).unwrap_err();
        assert!(
            matches!(err, LoadError::LatitudeOutOfRange { lat, idx: 0 } if (lat - 91.0).abs() < 1e-10),
            "expected LatitudeOutOfRange, got: {err:?}"
        );
    }

    #[test]
    fn lon_out_of_range() {
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(-181.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        let nav_file = recorder.finish().unwrap();
        let err = from_nav_file(&nav_file).unwrap_err();
        assert!(
            matches!(err, LoadError::LongitudeOutOfRange { lon, idx: 0 } if (lon - -181.0).abs() < 1e-10),
            "expected LongitudeOutOfRange, got: {err:?}"
        );
    }
}
