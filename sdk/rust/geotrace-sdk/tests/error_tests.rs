use geotrace_sdk::{
    Angle, DateTime, Duration, Error, NavFile, NavFileBuilder, NavFix, NavFixTime, Utc,
    VariantPathField,
};
use hdf5_pure::{AttrValue, FileBuilder, GroupBuilder};
use rstest::rstest;

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn minimal_gtd_bytes() -> Vec<u8> {
    let t = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t))
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .heading(Angle::degrees(90.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t + Duration::seconds(60)))
            .lat(Angle::degrees(51.6))
            .lon(Angle::degrees(-0.2))
            .heading(Angle::degrees(270.0))
            .build(),
    );
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    nav_file.write(&mut bytes).expect("write");
    bytes
}

#[test]
fn read_non_hdf5_bytes_returns_error() {
    let garbage = b"this is not an HDF5 file at all";
    let result = NavFile::read(garbage.as_slice());
    assert!(
        result.is_err(),
        "expected an error reading non-HDF5 bytes, got Ok"
    );
}

#[test]
fn read_empty_bytes_returns_error() {
    let result = NavFile::read([].as_slice());
    assert!(result.is_err(), "expected an error reading empty bytes");
}

#[test]
fn read_truncated_hdf5_magic_returns_error() {
    // HDF5 magic is "\x89HDF\r\n\x1a\n" - truncate after 4 bytes.
    let truncated = b"\x89HDF";
    let result = NavFile::read(truncated.as_slice());
    assert!(
        result.is_err(),
        "expected an error reading truncated HDF5 header"
    );
}

#[test]
fn valid_gtd_round_trips_without_error() {
    let bytes = minimal_gtd_bytes();
    let nav_file = NavFile::read(bytes.as_slice()).expect("valid .gtd bytes must parse");
    assert_eq!(nav_file.nav_points().len(), 2);
}

/// A file that is valid HDF5 but has no `geotrace_version` attribute should fail.
#[test]
fn missing_version_attribute_returns_error() {
    let mut fb = FileBuilder::new();
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("lat").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("lon").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("heading")
        .with_f64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[])
        .with_shape(&[0]);
    fb.add_group(np.finish());
    let bytes = fb.finish().expect("build");

    let result = NavFile::read(bytes.as_slice());
    assert!(
        result.is_err(),
        "expected an error reading a file without geotrace_version"
    );
}

/// A file that uses an unrecognised version string should return UnsupportedVersion.
#[test]
fn unrecognised_version_string_returns_unsupported_version() {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("99".into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("lat").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("lon").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("heading")
        .with_f64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[])
        .with_shape(&[0]);
    fb.add_group(np.finish());
    let bytes = fb.finish().expect("build");

    let result = NavFile::read(bytes.as_slice());
    assert!(
        matches!(result, Err(Error::UnsupportedVersion { ref version }) if version == "99"),
        "expected UnsupportedVersion(\"99\"), got: {result:?}"
    );
}

/// The `gps_time_us` and `sys_time_us` a `.gtd` file stores for one nav point,
/// `u64::MAX` standing for an absent one.
struct StoredNavPointTimestampsUs {
    gps: u64,
    sys: u64,
}

fn add_nav_points_group(fb: &mut FileBuilder, rows: &[StoredNavPointTimestampsUs]) {
    let count = rows.len();
    let shape = [count as u64];
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&vec![0; count])
        .with_shape(&shape);
    np.create_dataset("gps_time_us")
        .with_u64_data(&rows.iter().map(|row| row.gps).collect::<Vec<u64>>())
        .with_shape(&shape);
    np.create_dataset("sys_time_us")
        .with_u64_data(&rows.iter().map(|row| row.sys).collect::<Vec<u64>>())
        .with_shape(&shape);
    np.create_dataset("lat")
        .with_f64_data(&vec![51.5; count])
        .with_shape(&shape);
    np.create_dataset("lon")
        .with_f64_data(&vec![-0.1; count])
        .with_shape(&shape);
    np.create_dataset("heading")
        .with_f64_data(&vec![f64::NAN; count])
        .with_shape(&shape);
    np.create_dataset("speed_mps")
        .with_f64_data(&vec![f64::NAN; count])
        .with_shape(&shape);
    fb.add_group(np.finish());
}

#[test]
fn nav_point_without_either_timestamp_fails_the_read() {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    add_nav_points_group(
        &mut fb,
        &[StoredNavPointTimestampsUs {
            gps: u64::MAX,
            sys: u64::MAX,
        }],
    );
    let bytes = fb.finish().expect("build");

    let result = NavFile::read(bytes.as_slice());
    let Err(error @ Error::FixWithoutTimestamp { record: 0 }) = result else {
        panic!("expected FixWithoutTimestamp at record 0, got: {result:?}");
    };
    assert_eq!(
        error.to_string(),
        "nav point 0 has neither a receiver nor a host timestamp"
    );
}

#[test]
fn satellite_report_without_either_timestamp_fails_the_read() {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    add_nav_points_group(
        &mut fb,
        &[StoredNavPointTimestampsUs {
            gps: 0,
            sys: u64::MAX,
        }],
    );
    add_satellite_reports_group(
        &mut fb,
        &[SatelliteReportRow {
            nav_point_idx: 0,
            gps_time_us: u64::MAX,
            sys_time_us: u64::MAX,
        }],
    );
    add_tracked_satellites_group(&mut fb, &[0]);

    let bytes = fb.finish().expect("build");

    let result = NavFile::read(bytes.as_slice());
    let Err(error @ Error::ReportWithoutTimestamp { report: 0 }) = result else {
        panic!("expected ReportWithoutTimestamp at report 0, got: {result:?}");
    };
    assert_eq!(
        error.to_string(),
        "satellite report 0 has neither a receiver nor a host timestamp"
    );
}

const RECEIVER_TIME_US: u64 = 1_700_000_000_000_000;

/// A microsecond count past chrono's latest representable instant, and not the
/// count [`u64::MAX`] reserved for an absent timestamp.
const TIME_PAST_THE_UTC_RANGE_US: u64 = i64::MAX.cast_unsigned();

fn version_2_file() -> FileBuilder {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    fb
}

fn file_with_one_nav_point(timestamps: StoredNavPointTimestampsUs) -> FileBuilder {
    let mut fb = version_2_file();
    add_nav_points_group(&mut fb, &[timestamps]);
    fb
}

fn file_with_two_nav_points() -> FileBuilder {
    let mut fb = version_2_file();
    add_nav_points_group(&mut fb, &[under_lock(), under_lock()]);
    fb
}

fn under_lock() -> StoredNavPointTimestampsUs {
    StoredNavPointTimestampsUs {
        gps: RECEIVER_TIME_US,
        sys: u64::MAX,
    }
}

fn finish(fb: FileBuilder) -> Vec<u8> {
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    fb.finish().expect("build")
}

fn variant_path_row(path: &str) -> Vec<u8> {
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    let field = VariantPathField::new(path).expect("a path the field holds");
    field.encode_row().to_vec()
}

struct EventMarkerRow<'a> {
    sys_time_us: u64,
    variant_path: &'a str,
}

fn add_one_event_marker_group(
    fb: &mut FileBuilder,
    EventMarkerRow {
        sys_time_us,
        variant_path,
    }: EventMarkerRow<'_>,
) {
    let mut em = fb.create_group("event_markers");
    em.create_dataset("sys_time_us")
        .with_u64_data(&[sys_time_us])
        .with_shape(&[1]);
    em.create_dataset("lat")
        .with_f64_data(&[51.5])
        .with_shape(&[1]);
    em.create_dataset("lon")
        .with_f64_data(&[-0.1])
        .with_shape(&[1]);
    em.create_dataset("variant_path")
        .with_u8_data(&variant_path_row(variant_path))
        .with_shape(&[1, 256]);
    em.create_dataset("annotation")
        .with_u8_data(&[0u8; 512])
        .with_shape(&[1, 512]);
    fb.add_group(em.finish());
}

fn add_one_event_marker_style_group(fb: &mut FileBuilder, variant_path: &str) {
    let mut styles = fb.create_group("event_marker_styles");
    styles
        .create_dataset("variant_path")
        .with_u8_data(&variant_path_row(variant_path))
        .with_shape(&[1, 256]);
    styles
        .create_dataset("icon_name")
        .with_u8_data(&[0u8; 32])
        .with_shape(&[1, 32]);
    styles
        .create_dataset("color_hex")
        .with_u8_data(&[0u8; 8])
        .with_shape(&[1, 8]);
    fb.add_group(styles.finish());
}

/// One row of a crafted `sat_reports` table, `u64::MAX` standing for an absent
/// timestamp.
struct SatelliteReportRow {
    nav_point_idx: u64,
    gps_time_us: u64,
    sys_time_us: u64,
}

impl SatelliteReportRow {
    fn under_lock(nav_point_idx: u64) -> Self {
        Self {
            nav_point_idx,
            gps_time_us: RECEIVER_TIME_US,
            sys_time_us: u64::MAX,
        }
    }
}

fn add_satellite_reports_group(fb: &mut FileBuilder, rows: &[SatelliteReportRow]) {
    let shape = [rows.len() as u64];
    let mut sr = fb.create_group("sat_reports");
    sr.create_dataset("nav_point_idx")
        .with_u64_data(
            &rows
                .iter()
                .map(|row| row.nav_point_idx)
                .collect::<Vec<u64>>(),
        )
        .with_shape(&shape);
    sr.create_dataset("gps_time_us")
        .with_u64_data(&rows.iter().map(|row| row.gps_time_us).collect::<Vec<u64>>())
        .with_shape(&shape);
    sr.create_dataset("sys_time_us")
        .with_u64_data(&rows.iter().map(|row| row.sys_time_us).collect::<Vec<u64>>())
        .with_shape(&shape);
    fb.add_group(sr.finish());
}

fn add_tracked_satellites_group(fb: &mut FileBuilder, sat_report_indices: &[u64]) {
    let count = sat_report_indices.len();
    let shape = [count as u64];
    let mut ts = fb.create_group("tracked_sats");
    ts.create_dataset("sat_report_idx")
        .with_u64_data(sat_report_indices)
        .with_shape(&shape);
    ts.create_dataset("constellation")
        .with_u8_data(&vec![0; count])
        .with_shape(&shape);
    ts.create_dataset("prn")
        .with_u32_data(&vec![1; count])
        .with_shape(&shape);
    ts.create_dataset("in_fix")
        .with_u8_data(&vec![1; count])
        .with_shape(&shape);
    ts.create_dataset("elevation")
        .with_f32_data(&vec![45.0; count])
        .with_shape(&shape);
    ts.create_dataset("azimuth")
        .with_f32_data(&vec![90.0; count])
        .with_shape(&shape);
    ts.create_dataset("snr")
        .with_f32_data(&vec![38.0; count])
        .with_shape(&shape);
    fb.add_group(ts.finish());
}

#[test]
fn event_marker_without_a_timestamp_fails_the_read() {
    let mut fb = file_with_one_nav_point(under_lock());
    add_one_event_marker_group(
        &mut fb,
        EventMarkerRow {
            sys_time_us: u64::MAX,
            variant_path: "power/boot",
        },
    );

    let result = NavFile::read(finish(fb).as_slice());
    let Err(error @ Error::EventMarkerWithoutTimestamp { record: 0 }) = result else {
        panic!("expected EventMarkerWithoutTimestamp at record 0, got: {result:?}");
    };
    assert_eq!(error.to_string(), "event marker 0 has no timestamp");
}

fn event_marker_with_an_empty_variant_path() -> Vec<u8> {
    let mut fb = file_with_one_nav_point(under_lock());
    add_one_event_marker_group(
        &mut fb,
        EventMarkerRow {
            sys_time_us: RECEIVER_TIME_US,
            variant_path: "",
        },
    );
    finish(fb)
}

fn event_marker_style_with_an_empty_variant_path() -> Vec<u8> {
    let mut fb = file_with_one_nav_point(under_lock());
    add_one_event_marker_style_group(&mut fb, "");
    finish(fb)
}

#[rstest]
#[case::event_marker(
    event_marker_with_an_empty_variant_path(),
    "event_markers/variant_path: record 0 is empty"
)]
#[case::event_marker_style(
    event_marker_style_with_an_empty_variant_path(),
    "event_marker_styles/variant_path: record 0 is empty"
)]
fn an_empty_variant_path_fails_the_read(#[case] bytes: Vec<u8>, #[case] expected: &str) {
    let result = NavFile::read(bytes.as_slice());
    let Err(error @ Error::EmptyField { .. }) = result else {
        panic!("expected EmptyField, got: {result:?}");
    };
    assert_eq!(error.to_string(), expected);
}

fn tracked_satellite_pointing_past_the_reports() -> Vec<u8> {
    let mut fb = file_with_one_nav_point(under_lock());
    add_satellite_reports_group(&mut fb, &[SatelliteReportRow::under_lock(0)]);
    add_tracked_satellites_group(&mut fb, &[3]);
    finish(fb)
}

fn report_pointing_past_the_nav_points() -> Vec<u8> {
    let mut fb = file_with_one_nav_point(under_lock());
    add_satellite_reports_group(&mut fb, &[SatelliteReportRow::under_lock(3)]);
    add_tracked_satellites_group(&mut fb, &[0]);
    finish(fb)
}

#[rstest]
#[case::tracked_satellite(
    tracked_satellite_pointing_past_the_reports(),
    "tracked_sats/sat_report_idx: record 0 holds index 3, past the row count of sat_reports (1)"
)]
#[case::satellite_report(
    report_pointing_past_the_nav_points(),
    "sat_reports/nav_point_idx: record 0 holds index 3, past the row count of nav_points (1)"
)]
fn an_index_past_the_table_it_points_into_fails_the_read(
    #[case] bytes: Vec<u8>,
    #[case] expected: &str,
) {
    let result = NavFile::read(bytes.as_slice());
    let Err(error @ Error::IndexPastTable { .. }) = result else {
        panic!("expected IndexPastTable, got: {result:?}");
    };
    assert_eq!(error.to_string(), expected);
}

fn nav_point_timestamp_past_the_utc_range() -> Vec<u8> {
    finish(file_with_one_nav_point(StoredNavPointTimestampsUs {
        gps: TIME_PAST_THE_UTC_RANGE_US,
        sys: u64::MAX,
    }))
}

fn marker_timestamp_past_the_utc_range() -> Vec<u8> {
    let mut fb = file_with_one_nav_point(under_lock());
    let mut markers = fb.create_group("markers");
    markers
        .create_dataset("time")
        .with_i64_data(&[i64::MAX])
        .with_shape(&[1]);
    markers
        .create_dataset("lat")
        .with_f64_data(&[51.5])
        .with_shape(&[1]);
    markers
        .create_dataset("lon")
        .with_f64_data(&[-0.1])
        .with_shape(&[1]);
    markers
        .create_dataset("icon")
        .with_u8_data(&[0])
        .with_shape(&[1]);
    markers
        .create_dataset("label")
        .with_u8_data(&[0u8; 256])
        .with_shape(&[1, 256]);
    fb.add_group(markers.finish());
    finish(fb)
}

#[rstest]
#[case::nav_point(
    nav_point_timestamp_past_the_utc_range(),
    "nav_points/gps_time_us: record 0 holds 9223372036854775807 microseconds, past the range a UTC timestamp covers"
)]
#[case::marker(
    marker_timestamp_past_the_utc_range(),
    "markers/time: record 0 holds 9223372036854775807 microseconds, past the range a UTC timestamp covers"
)]
fn a_timestamp_past_the_utc_range_fails_the_read(#[case] bytes: Vec<u8>, #[case] expected: &str) {
    let result = NavFile::read(bytes.as_slice());
    let Err(error @ Error::TimestampOutOfRange { .. }) = result else {
        panic!("expected TimestampOutOfRange, got: {result:?}");
    };
    assert_eq!(error.to_string(), expected);
}

#[test]
fn a_file_without_gps_time_us_reads_the_time_axis_as_the_receiver_timestamp() {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[RECEIVER_TIME_US.cast_signed()])
        .with_shape(&[1]);
    np.create_dataset("lat")
        .with_f64_data(&[51.5])
        .with_shape(&[1]);
    np.create_dataset("lon")
        .with_f64_data(&[-0.1])
        .with_shape(&[1]);
    np.create_dataset("heading")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    fb.add_group(np.finish());

    let nav_file = NavFile::read(finish(fb).as_slice()).expect("a v1 file must read");
    let expected = DateTime::from_timestamp_micros(RECEIVER_TIME_US.cast_signed());
    assert_eq!(
        nav_file.nav_points().first().map(|np| np.fix.time),
        expected.map(NavFixTime::Receiver)
    );
}

#[test]
fn a_gps_time_us_of_a_string_datatype_fails_the_read() {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[0])
        .with_shape(&[1]);
    np.create_dataset("gps_time_us")
        .with_strings(&["1700000000000000"])
        .expect("a string dataset")
        .with_shape(&[1]);
    np.create_dataset("lat")
        .with_f64_data(&[51.5])
        .with_shape(&[1]);
    np.create_dataset("lon")
        .with_f64_data(&[-0.1])
        .with_shape(&[1]);
    np.create_dataset("heading")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    fb.add_group(np.finish());

    let result = NavFile::read(finish(fb).as_slice());
    assert!(
        matches!(result, Err(Error::Hdf5(_))),
        "expected an Hdf5 error, got: {result:?}"
    );
}

struct TableWithAShortDataset<'a> {
    short: &'a str,
}

impl TableWithAShortDataset<'_> {
    fn rows(&self, dataset: &str) -> usize {
        if dataset == self.short { 1 } else { 2 }
    }

    fn u8_dataset(&self, group: &mut GroupBuilder, dataset: &str, value: u8) {
        let rows = self.rows(dataset);
        group
            .create_dataset(dataset)
            .with_u8_data(&vec![value; rows])
            .with_shape(&[rows as u64]);
    }

    fn u32_dataset(&self, group: &mut GroupBuilder, dataset: &str, value: u32) {
        let rows = self.rows(dataset);
        group
            .create_dataset(dataset)
            .with_u32_data(&vec![value; rows])
            .with_shape(&[rows as u64]);
    }

    fn u64_dataset(&self, group: &mut GroupBuilder, dataset: &str, value: u64) {
        let rows = self.rows(dataset);
        group
            .create_dataset(dataset)
            .with_u64_data(&vec![value; rows])
            .with_shape(&[rows as u64]);
    }

    fn f32_dataset(&self, group: &mut GroupBuilder, dataset: &str, value: f32) {
        let rows = self.rows(dataset);
        group
            .create_dataset(dataset)
            .with_f32_data(&vec![value; rows])
            .with_shape(&[rows as u64]);
    }

    fn f64_dataset(&self, group: &mut GroupBuilder, dataset: &str, value: f64) {
        let rows = self.rows(dataset);
        group
            .create_dataset(dataset)
            .with_f64_data(&vec![value; rows])
            .with_shape(&[rows as u64]);
    }
}

fn nav_points_with_a_short_dataset(short: &str) -> Vec<u8> {
    let table = TableWithAShortDataset { short };
    let mut fb = version_2_file();
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[0, 0])
        .with_shape(&[2]);
    table.u64_dataset(&mut np, "gps_time_us", RECEIVER_TIME_US);
    table.u64_dataset(&mut np, "sys_time_us", u64::MAX);
    table.f64_dataset(&mut np, "lat", 51.5);
    table.f64_dataset(&mut np, "lon", -0.1);
    table.f64_dataset(&mut np, "heading", f64::NAN);
    table.f64_dataset(&mut np, "speed_mps", f64::NAN);
    table.f64_dataset(&mut np, "eph_m", f64::NAN);
    fb.add_group(np.finish());
    finish(fb)
}

fn satellite_reports_with_a_short_dataset(short: &str) -> Vec<u8> {
    let table = TableWithAShortDataset { short };
    let mut fb = file_with_two_nav_points();
    let mut sr = fb.create_group("sat_reports");
    sr.create_dataset("nav_point_idx")
        .with_u64_data(&[0, 1])
        .with_shape(&[2]);
    table.u64_dataset(&mut sr, "gps_time_us", RECEIVER_TIME_US);
    table.u64_dataset(&mut sr, "sys_time_us", u64::MAX);
    fb.add_group(sr.finish());
    add_tracked_satellites_group(&mut fb, &[0, 1]);
    finish(fb)
}

/// A file whose `sat_reports` holds the v1 `time` dataset, with one row where
/// `nav_point_idx` has two.
fn satellite_reports_with_a_short_v1_time_dataset() -> Vec<u8> {
    let mut fb = file_with_two_nav_points();
    let mut sr = fb.create_group("sat_reports");
    sr.create_dataset("nav_point_idx")
        .with_u64_data(&[0, 1])
        .with_shape(&[2]);
    sr.create_dataset("time")
        .with_i64_data(&[RECEIVER_TIME_US.cast_signed()])
        .with_shape(&[1]);
    fb.add_group(sr.finish());
    add_tracked_satellites_group(&mut fb, &[0, 1]);
    finish(fb)
}

fn tracked_satellites_with_a_short_dataset(short: &str) -> Vec<u8> {
    let table = TableWithAShortDataset { short };
    let mut fb = file_with_two_nav_points();
    add_satellite_reports_group(
        &mut fb,
        &[
            SatelliteReportRow::under_lock(0),
            SatelliteReportRow::under_lock(1),
        ],
    );
    let mut ts = fb.create_group("tracked_sats");
    ts.create_dataset("sat_report_idx")
        .with_u64_data(&[0, 1])
        .with_shape(&[2]);
    table.u8_dataset(&mut ts, "constellation", 0);
    table.u32_dataset(&mut ts, "prn", 1);
    table.u8_dataset(&mut ts, "in_fix", 1);
    table.f32_dataset(&mut ts, "elevation", 45.0);
    table.f32_dataset(&mut ts, "azimuth", 90.0);
    table.f32_dataset(&mut ts, "snr", 38.0);
    fb.add_group(ts.finish());
    finish(fb)
}

#[rstest]
#[case::nav_point_sys_time_us(
    nav_points_with_a_short_dataset("sys_time_us"),
    r#"dataset "sys_time_us" in group "nav_points": expected 2 rows but found 1"#
)]
#[case::nav_point_eph_m(
    nav_points_with_a_short_dataset("eph_m"),
    r#"dataset "eph_m" in group "nav_points": expected 2 rows but found 1"#
)]
#[case::satellite_report_gps_time_us(
    satellite_reports_with_a_short_dataset("gps_time_us"),
    r#"dataset "gps_time_us" in group "sat_reports": expected 2 rows but found 1"#
)]
#[case::satellite_report_sys_time_us(
    satellite_reports_with_a_short_dataset("sys_time_us"),
    r#"dataset "sys_time_us" in group "sat_reports": expected 2 rows but found 1"#
)]
#[case::satellite_report_time(
    satellite_reports_with_a_short_v1_time_dataset(),
    r#"dataset "time" in group "sat_reports": expected 2 rows but found 1"#
)]
#[case::tracked_satellite_constellation(
    tracked_satellites_with_a_short_dataset("constellation"),
    r#"dataset "constellation" in group "tracked_sats": expected 2 rows but found 1"#
)]
#[case::tracked_satellite_prn(
    tracked_satellites_with_a_short_dataset("prn"),
    r#"dataset "prn" in group "tracked_sats": expected 2 rows but found 1"#
)]
#[case::tracked_satellite_in_fix(
    tracked_satellites_with_a_short_dataset("in_fix"),
    r#"dataset "in_fix" in group "tracked_sats": expected 2 rows but found 1"#
)]
#[case::tracked_satellite_elevation(
    tracked_satellites_with_a_short_dataset("elevation"),
    r#"dataset "elevation" in group "tracked_sats": expected 2 rows but found 1"#
)]
#[case::tracked_satellite_azimuth(
    tracked_satellites_with_a_short_dataset("azimuth"),
    r#"dataset "azimuth" in group "tracked_sats": expected 2 rows but found 1"#
)]
#[case::tracked_satellite_snr(
    tracked_satellites_with_a_short_dataset("snr"),
    r#"dataset "snr" in group "tracked_sats": expected 2 rows but found 1"#
)]
fn a_dataset_shorter_than_its_table_fails_the_read(#[case] bytes: Vec<u8>, #[case] expected: &str) {
    let result = NavFile::read(bytes.as_slice());
    let Err(error @ Error::ShapeMismatch { .. }) = result else {
        panic!("expected ShapeMismatch, got: {result:?}");
    };
    assert_eq!(error.to_string(), expected);
}

fn a_dataset_where_an_optional_group_belongs(name: &str) -> Vec<u8> {
    let mut fb = file_with_one_nav_point(under_lock());
    fb.create_dataset(name).with_i64_data(&[0]).with_shape(&[1]);
    finish(fb)
}

#[rstest]
#[case::sat_reports("sat_reports")]
#[case::markers("markers")]
#[case::event_markers("event_markers")]
#[case::event_marker_styles("event_marker_styles")]
#[case::channels("channels")]
fn a_dataset_where_an_optional_group_belongs_fails_the_read(#[case] name: &str) {
    let result = NavFile::read(a_dataset_where_an_optional_group_belongs(name).as_slice());
    let Err(error @ Error::Hdf5(_)) = result else {
        panic!("expected an Hdf5 error, got: {result:?}");
    };
    assert_eq!(
        error.to_string(),
        format!("HDF5 error: not a group: {name}")
    );
}

#[test]
fn a_file_without_a_markers_group_reads_with_no_markers() {
    let bytes = finish(file_with_one_nav_point(under_lock()));

    let nav_file = NavFile::read(bytes.as_slice()).expect("a file with only nav points must read");

    assert!(nav_file.markers().is_empty());
}
