//! Test helpers shared by the integration test binaries of geotrace-sdk, which
//! import the crate as `test_util`.

use std::path::{Path, PathBuf};

use geotrace_sdk::{
    Angle, AnnotationField, ColorHexField, Constellation, DateTime, Duration, Error, IconNameField,
    MarkerLabelField, NavFile, NavFileBuilder, NavFix, NavFixTime, NavRecorder, Satellite,
    SatelliteReport, Utc, VariantPathField,
};
use hdf5_pure::{AttrValue, FileBuilder};

/// The contents of a `.gtd` file beyond its one nav fix: root attributes, one
/// marker per label row, one event marker per row, one style per row.
/// [`GtdFileContents::into_gtd_bytes`] leaves out the group of an empty list.
#[derive(Default)]
pub struct GtdFileContents {
    pub attrs: Vec<(&'static str, &'static str)>,
    pub marker_labels: Vec<Vec<u8>>,
    pub event_markers: Vec<EventMarkerFieldRows>,
    pub styles: Vec<StyleFieldRows>,
}

impl GtdFileContents {
    /// A `.gtd` file of one nav fix, with these attributes, markers, event
    /// markers and styles. A row that is not UTF-8 has to be assembled here,
    /// since the writer takes a `String` for every field that holds one.
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    pub fn into_gtd_bytes(self) -> Vec<u8> {
        let Self {
            attrs,
            marker_labels,
            event_markers,
            styles,
        } = self;

        let mut fb = FileBuilder::new();
        fb.set_attr("geotrace_version", AttrValue::String("2".into()));
        for (name, value) in attrs {
            fb.set_attr(name, AttrValue::String(value.to_owned()));
        }

        add_nav_points_group_of_one_fix(&mut fb);

        if !marker_labels.is_empty() {
            let count = marker_labels.len();
            let mut markers = fb.create_group("markers");
            markers
                .create_dataset("time")
                .with_i64_data(&vec![base().timestamp_micros(); count])
                .with_shape(&[count as u64]);
            markers
                .create_dataset("lat")
                .with_f64_data(&vec![55.0; count])
                .with_shape(&[count as u64]);
            markers
                .create_dataset("lon")
                .with_f64_data(&vec![12.0; count])
                .with_shape(&[count as u64]);
            markers
                .create_dataset("icon")
                .with_u8_data(&vec![MARKER_ICON_WARNING_CODE; count])
                .with_shape(&[count as u64]);
            markers
                .create_dataset("label")
                .with_u8_data(&marker_labels.concat())
                .with_shape(&[count as u64, MARKER_LABEL_ROW_BYTES as u64]);
            fb.add_group(markers.finish());
        }

        if !event_markers.is_empty() {
            let count = event_markers.len();
            let mut grp = fb.create_group("event_markers");
            grp.create_dataset("sys_time_us")
                .with_u64_data(&vec![base().timestamp_micros().cast_unsigned(); count])
                .with_shape(&[count as u64]);
            grp.create_dataset("lat")
                .with_f64_data(&vec![55.0; count])
                .with_shape(&[count as u64]);
            grp.create_dataset("lon")
                .with_f64_data(&vec![12.0; count])
                .with_shape(&[count as u64]);
            grp.create_dataset("variant_path")
                .with_u8_data(
                    &event_markers
                        .iter()
                        .flat_map(|row| row.variant_path.clone())
                        .collect::<Vec<u8>>(),
                )
                .with_shape(&[count as u64, VARIANT_PATH_ROW_BYTES as u64]);
            grp.create_dataset("annotation")
                .with_u8_data(
                    &event_markers
                        .iter()
                        .flat_map(|row| row.annotation.clone())
                        .collect::<Vec<u8>>(),
                )
                .with_shape(&[count as u64, ANNOTATION_ROW_BYTES as u64]);
            fb.add_group(grp.finish());
        }

        if !styles.is_empty() {
            let count = styles.len();
            let mut grp = fb.create_group("event_marker_styles");
            grp.create_dataset("variant_path")
                .with_u8_data(
                    &styles
                        .iter()
                        .flat_map(|row| row.variant_path.clone())
                        .collect::<Vec<u8>>(),
                )
                .with_shape(&[count as u64, VARIANT_PATH_ROW_BYTES as u64]);
            grp.create_dataset("icon_name")
                .with_u8_data(
                    &styles
                        .iter()
                        .flat_map(|row| row.icon_name.clone())
                        .collect::<Vec<u8>>(),
                )
                .with_shape(&[count as u64, ICON_NAME_ROW_BYTES as u64]);
            grp.create_dataset("color_hex")
                .with_u8_data(
                    &styles
                        .iter()
                        .flat_map(|row| row.color_hex.clone())
                        .collect::<Vec<u8>>(),
                )
                .with_shape(&[count as u64, COLOR_HEX_ROW_BYTES as u64]);
            fb.add_group(grp.finish());
        }

        fb.finish().expect("the assembled file builds")
    }
}

/// The `variant_path` and `annotation` rows of one event marker in
/// [`GtdFileContents`].
pub struct EventMarkerFieldRows {
    pub variant_path: Vec<u8>,
    pub annotation: Vec<u8>,
}

/// The `variant_path`, `icon_name` and `color_hex` rows of one event marker
/// style in [`GtdFileContents`].
pub struct StyleFieldRows {
    pub variant_path: Vec<u8>,
    pub icon_name: Vec<u8>,
    pub color_hex: Vec<u8>,
}

/// A recorder with the one fix `fix_at(0, Lat(55.0), Lon(12.0))`.
pub fn recorder_with_one_fix() -> NavRecorder {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, Lat(55.0), Lon(12.0)));
    recorder
}

/// A fix at `(lat, lon)` heading north, with the receiver time `t_ms(offset_ms)`.
pub fn fix_at(offset_ms: i64, Lat(lat): Lat, Lon(lon): Lon) -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(t_ms(offset_ms)))
        .lat(Angle::degrees(lat))
        .lon(Angle::degrees(lon))
        .heading(Angle::degrees(0.0))
        .build()
}

/// A fix at latitude 0 heading east, with the receiver time `t_ms(offset_ms)`.
pub fn fix_on_the_equator_heading_east(offset_ms: i64, lon: f64) -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(t_ms(offset_ms)))
        .lat(Angle::degrees(0.0))
        .lon(Angle::degrees(lon))
        .heading(Angle::degrees(90.0))
        .build()
}

/// A report with the receiver time `t_ms(offset_ms)` and one satellite in fix.
pub fn report_with(offset_ms: i64, constellation: Constellation, prn: u32) -> SatelliteReport {
    SatelliteReport::builder()
        .time(NavFixTime::Receiver(t_ms(offset_ms)))
        .tracked(vec![
            Satellite::builder()
                .constellation(constellation)
                .prn(prn)
                .in_fix(true)
                .build(),
        ])
        .build()
}

/// [`base`] plus `offset_ms` milliseconds.
pub fn t_ms(offset_ms: i64) -> DateTime<Utc> {
    base() + Duration::milliseconds(offset_ms)
}

/// [`base`] plus `offset_secs` seconds.
pub fn t_s(offset_secs: i64) -> DateTime<Utc> {
    base() + Duration::seconds(offset_secs)
}

/// 2025-05-23 11:33:20 UTC, the instant [`t_ms`] and [`t_s`] count from.
#[expect(clippy::expect_used, reason = "the fixed timestamp is in range")]
pub fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("in range")
}

/// `nav_file` written to bytes and read back.
pub fn round_trip(nav_file: &NavFile) -> Result<NavFile, Error> {
    NavFile::read(to_bytes(nav_file)?.as_slice())
}

/// The bytes the writer writes for `nav_file`.
pub fn to_bytes(nav_file: &NavFile) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;
    Ok(bytes)
}

/// The path of `relative_path` under the repository's `tests/fixtures/`.
pub fn fixture_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures")
        .join(relative_path)
}

/// A [`FileBuilder`] with a `nav_points` group of zero rows and no root
/// attribute.
pub fn file_with_an_empty_nav_points_group() -> FileBuilder {
    let mut fb = FileBuilder::new();
    let mut nav_points = fb.create_group("nav_points");
    nav_points
        .create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    for name in ["lat", "lon", "heading", "speed_mps"] {
        nav_points
            .create_dataset(name)
            .with_f64_data(&[])
            .with_shape(&[0]);
    }
    fb.add_group(nav_points.finish());
    fb
}

/// Adds a `nav_points` group of one fix at [`base`], the fix that
/// [`GtdFileContents::into_gtd_bytes`] writes.
pub fn add_nav_points_group_of_one_fix(fb: &mut FileBuilder) {
    let mut nav_points = fb.create_group("nav_points");
    nav_points
        .create_dataset("time")
        .with_i64_data(&[base().timestamp_micros()])
        .with_shape(&[1]);
    for (name, value) in [
        ("lat", 55.0),
        ("lon", 12.0),
        ("heading", 90.0),
        ("speed_mps", 3.0),
    ] {
        nav_points
            .create_dataset(name)
            .with_f64_data(&[value])
            .with_shape(&[1]);
    }
    fb.add_group(nav_points.finish());
}

/// `content` followed by zero bytes up to `row_bytes`.
pub fn nul_padded_row(content: &[u8], row_bytes: usize) -> Vec<u8> {
    let mut row = content.to_vec();
    row.resize(row_bytes, 0);
    row
}

/// A latitude in degrees.
#[derive(Clone, Copy)]
pub struct Lat(pub f64);

/// A longitude in degrees.
#[derive(Clone, Copy)]
pub struct Lon(pub f64);

/// The width of a `markers/label` row, its terminator included.
pub const MARKER_LABEL_ROW_BYTES: usize = MarkerLabelField::CONTENT_CAPACITY + 1;

/// The width of a `variant_path` row, its terminator included.
pub const VARIANT_PATH_ROW_BYTES: usize = VariantPathField::CONTENT_CAPACITY + 1;

/// The width of an `event_markers/annotation` row, its terminator included.
pub const ANNOTATION_ROW_BYTES: usize = AnnotationField::CONTENT_CAPACITY + 1;

/// The width of an `event_marker_styles/icon_name` row, its terminator included.
pub const ICON_NAME_ROW_BYTES: usize = IconNameField::CONTENT_CAPACITY + 1;

/// The width of an `event_marker_styles/color_hex` row, its terminator included.
pub const COLOR_HEX_ROW_BYTES: usize = ColorHexField::CONTENT_CAPACITY + 1;

const MARKER_ICON_WARNING_CODE: u8 = 4;
