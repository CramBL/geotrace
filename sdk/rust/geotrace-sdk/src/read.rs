use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::builder::ABSENT_TIMESTAMP_MICROS;
use crate::error::{Error, FieldLocation};
use crate::fixed_width_string::{
    AnnotationField, ColorHexField, FixedWidthString, FixedWidthStringError, IconNameField,
    MarkerLabelField, VariantPathField,
};
use crate::format_version::SUPPORTED_FORMAT_VERSIONS;
use crate::provenance;
use crate::size_checked_file::{SizeCheckedFile, SizeCheckedGroup};
use crate::types::{
    Annotation, AnnotationIcon, Channel, Constellation, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerPoint, EventMarkerStyle, Marker, Meta, NavFile, NavFix, NavFixTime, NavPoint,
    RecordedFixTimestamps, Satellite, SatelliteReport, TravelMode,
};
use crate::write;
use crate::{Angle, Velocity};
use geotrace_sdk_units::{ChannelUnit, snr};
use hdf5_pure::AttrValue;
use strum::IntoEnumIterator;

pub(crate) fn parse_hdf5(bytes: Vec<u8>) -> Result<NavFile, Error> {
    let file = SizeCheckedFile::from_bytes(bytes)?;
    let root = file.root();

    let attrs = root.attrs()?;
    let version = match attrs.get("geotrace_version").and_then(AttrValue::as_str) {
        Some(v) => v.to_owned(),
        None => {
            return Err(Error::UnsupportedVersion {
                version: "<missing>".into(),
            });
        }
    };
    let supported = version
        .parse::<u32>()
        .is_ok_and(|number| SUPPORTED_FORMAT_VERSIONS.contains(&number));
    if !supported {
        return Err(Error::UnsupportedVersion { version });
    }

    let meta = read_meta(&attrs);
    let nav_points = read_nav_points(&file)?;
    let nav_points = attach_satellite_data(nav_points, &file)?;
    let markers = read_markers(&file)?;
    let event_markers = read_event_markers(&file)?;
    let event_marker_styles = read_event_marker_styles(&file)?;
    let channels = read_channels(&file)?;

    Ok(NavFile {
        meta,
        nav_points,
        markers,
        event_markers,
        event_marker_styles,
        channels,
    })
}

fn read_meta(attrs: &HashMap<String, AttrValue>) -> Meta {
    Meta {
        title: string_attr(attrs, "meta_title"),
        device: string_attr(attrs, "meta_device"),
        notes: string_attr(attrs, "meta_notes"),
        identity: string_attr(attrs, "meta_identity"),
        travel_mode: string_attr(attrs, "meta_travel_mode").map(|raw| {
            let mode = TravelMode::from_lower_case(&raw);
            if matches!(mode, TravelMode::Unknown(_)) {
                log::warn!("unknown meta_travel_mode value {raw:?}, preserving it as-is");
            }
            mode
        }),
        sdk_version: string_attr(attrs, provenance::SDK_VERSION_ATTR),
        sdk_git_commit: string_attr(attrs, provenance::SDK_GIT_COMMIT_ATTR),
        sdk_commit_time: string_attr(attrs, provenance::SDK_COMMIT_TIME_ATTR)
            .and_then(|raw| provenance::parse_rfc3339("the sdk_commit_time attribute", &raw)),
    }
}

fn string_attr(attrs: &HashMap<String, AttrValue>, key: &str) -> Option<String> {
    attrs
        .get(key)
        .and_then(AttrValue::as_str)
        .map(str::to_owned)
}

fn f64_attr(attrs: &HashMap<String, AttrValue>, key: &str) -> Option<f64> {
    attrs.get(key).and_then(AttrValue::as_f64)
}

fn string_array_attr(attrs: &HashMap<String, AttrValue>, key: &str) -> Option<Vec<String>> {
    attrs
        .get(key)
        .and_then(AttrValue::as_strings)
        .map(<[String]>::to_vec)
}

fn read_nav_points(file: &SizeCheckedFile) -> Result<Vec<NavPoint>, Error> {
    let grp = file.group("nav_points")?;

    let times = grp.dataset("time")?.read_i64()?;
    let lats = grp.dataset("lat")?.read_f64()?;
    let lons = grp.dataset("lon")?.read_f64()?;
    let headings = grp.dataset("heading")?.read_f64()?;
    let speeds = grp.dataset("speed_mps")?.read_f64()?;

    // A file written before `gps_time_us` existed stores only the `time` axis,
    // holding the receiver's timestamp for a fix taken under lock and the host
    // clock's for one taken without. Such a file is read with `time` treated as
    // the receiver's timestamp.
    let gps_times: Vec<u64> = match grp.optional_dataset("gps_time_us")? {
        Some(ds) => ds.read_u64()?,
        None => times.iter().map(|&us| us.cast_unsigned()).collect(),
    };
    // `sys_time_us` and `eph_m` are absent in older files.
    let sys_times: Vec<u64> = match grp.optional_dataset("sys_time_us")? {
        Some(ds) => ds.read_u64()?,
        None => vec![ABSENT_TIMESTAMP_MICROS; times.len()],
    };
    let ephs: Vec<f64> = match grp.optional_dataset("eph_m")? {
        Some(ds) => ds.read_f64()?,
        None => vec![f64::NAN; times.len()],
    };

    let n = times.len();
    check_len("nav_points", "gps_time_us", n, gps_times.len())?;
    check_len("nav_points", "sys_time_us", n, sys_times.len())?;
    check_len("nav_points", "lat", n, lats.len())?;
    check_len("nav_points", "lon", n, lons.len())?;
    check_len("nav_points", "heading", n, headings.len())?;
    check_len("nav_points", "speed_mps", n, speeds.len())?;
    check_len("nav_points", "eph_m", n, ephs.len())?;

    let nav_points = gps_times
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(headings.iter())
        .zip(speeds.iter())
        .zip(sys_times.iter())
        .zip(ephs.iter())
        .enumerate()
        .map(
            |(
                record,
                (
                    (((((gps_time_us, lat_deg), lon_deg), heading_deg), speed_mps), sys_time_us),
                    eph_val,
                ),
            )| {
                let recorded = RecordedFixTimestamps {
                    gps: decode_optional_timestamp(
                        FieldLocation {
                            group: "nav_points",
                            dataset: "gps_time_us",
                        },
                        record,
                        *gps_time_us,
                    )?,
                    sys: decode_optional_timestamp(
                        FieldLocation {
                            group: "nav_points",
                            dataset: "sys_time_us",
                        },
                        record,
                        *sys_time_us,
                    )?,
                };
                let Some(time) = NavFixTime::from_recorded(recorded) else {
                    return Err(Error::FixWithoutTimestamp { record });
                };
                Ok(NavPoint {
                    fix: NavFix {
                        time,
                        lat: Angle::degrees(*lat_deg),
                        lon: Angle::degrees(*lon_deg),
                        heading: if heading_deg.is_nan() {
                            None
                        } else {
                            Some(Angle::degrees(*heading_deg))
                        },
                        speed: if speed_mps.is_nan() {
                            None
                        } else {
                            Some(Velocity::meter_per_second(*speed_mps))
                        },
                        eph_m: if eph_val.is_nan() {
                            None
                        } else {
                            Some(*eph_val)
                        },
                    },
                    satellites: None,
                })
            },
        )
        .collect::<Result<Vec<NavPoint>, Error>>()?;

    Ok(nav_points)
}

fn attach_satellite_data(
    mut nav_points: Vec<NavPoint>,
    file: &SizeCheckedFile,
) -> Result<Vec<NavPoint>, Error> {
    let Some(sat_grp) = file.optional_group("sat_reports")? else {
        return Ok(nav_points);
    };

    let nav_point_idx = sat_grp.dataset("nav_point_idx")?.read_u64()?;
    let r = nav_point_idx.len();

    // v2: `gps_time_us` and `sys_time_us`, both u64 with `u64::MAX` for absent.
    // v1: a single `time` dataset, which the reader treats as the receiver's
    // timestamp, with no host timestamp.
    let (report_gps_times, report_sys_times): (Vec<u64>, Vec<u64>) =
        match sat_grp.optional_dataset("gps_time_us")? {
            Some(ds) => {
                let gps = ds.read_u64()?;
                check_len("sat_reports", "gps_time_us", r, gps.len())?;
                let sys = match sat_grp.optional_dataset("sys_time_us")? {
                    Some(ds) => {
                        let sys = ds.read_u64()?;
                        check_len("sat_reports", "sys_time_us", r, sys.len())?;
                        sys
                    }
                    None => vec![ABSENT_TIMESTAMP_MICROS; r],
                };
                (gps, sys)
            }
            None => {
                let times = sat_grp.dataset("time")?.read_i64()?;
                check_len("sat_reports", "time", r, times.len())?;
                let gps = times.iter().map(|&us| us.cast_unsigned()).collect();
                (gps, vec![ABSENT_TIMESTAMP_MICROS; r])
            }
        };

    let ts_grp = file.group("tracked_sats")?;
    let ts_rep_idx = ts_grp.dataset("sat_report_idx")?.read_u64()?;
    let ts_constellation = ts_grp.dataset("constellation")?.read_u8()?;
    let ts_prn = ts_grp.dataset("prn")?.read_u32()?;
    let ts_in_fix = ts_grp.dataset("in_fix")?.read_u8()?;
    let ts_elevation = ts_grp.dataset("elevation")?.read_f32()?;
    let ts_azimuth = ts_grp.dataset("azimuth")?.read_f32()?;
    let ts_snr = ts_grp.dataset("snr")?.read_f32()?;

    let tracked_rows = ts_rep_idx.len();
    check_len(
        "tracked_sats",
        "constellation",
        tracked_rows,
        ts_constellation.len(),
    )?;
    check_len("tracked_sats", "prn", tracked_rows, ts_prn.len())?;
    check_len("tracked_sats", "in_fix", tracked_rows, ts_in_fix.len())?;
    check_len(
        "tracked_sats",
        "elevation",
        tracked_rows,
        ts_elevation.len(),
    )?;
    check_len("tracked_sats", "azimuth", tracked_rows, ts_azimuth.len())?;
    check_len("tracked_sats", "snr", tracked_rows, ts_snr.len())?;

    let mut tracked_by_report: Vec<Vec<Satellite>> = vec![Vec::new(); r];
    for (record, (&rep_idx, constellation_code, &prn, &in_fix, &elevation, &azimuth, &snr)) in
        ts_rep_idx
            .iter()
            .zip(ts_constellation.iter())
            .zip(ts_prn.iter())
            .zip(ts_in_fix.iter())
            .zip(ts_elevation.iter())
            .zip(ts_azimuth.iter())
            .zip(ts_snr.iter())
            .map(|((((((a, b), c), d), e), f), g)| (a, b, c, d, e, f, g))
            .enumerate()
    {
        let constellation = write::decode_tracked_constellation(*constellation_code)?;
        let sat = Satellite {
            constellation,
            prn,
            in_fix: in_fix != 0,
            elevation: opt_f32(elevation),
            azimuth: opt_f32(azimuth),
            snr: opt_f32(snr),
        };
        row_addressed_by_index(
            &mut tracked_by_report,
            "sat_reports",
            FieldLocation {
                group: "tracked_sats",
                dataset: "sat_report_idx",
            },
            record,
            rep_idx,
        )?
        .push(sat);
    }

    for (report, (&np_idx, (gps_us, sys_us))) in nav_point_idx
        .iter()
        .zip(report_gps_times.iter().zip(report_sys_times.iter()))
        .enumerate()
    {
        let recorded = RecordedFixTimestamps {
            gps: decode_optional_timestamp(
                FieldLocation {
                    group: "sat_reports",
                    dataset: "gps_time_us",
                },
                report,
                *gps_us,
            )?,
            sys: decode_optional_timestamp(
                FieldLocation {
                    group: "sat_reports",
                    dataset: "sys_time_us",
                },
                report,
                *sys_us,
            )?,
        };
        let Some(time) = NavFixTime::from_recorded(recorded) else {
            return Err(Error::ReportWithoutTimestamp { report });
        };
        let np = row_addressed_by_index(
            &mut nav_points,
            "nav_points",
            FieldLocation {
                group: "sat_reports",
                dataset: "nav_point_idx",
            },
            report,
            np_idx,
        )?;
        np.satellites = Some(SatelliteReport {
            time,
            tracked: tracked_by_report.get(report).cloned().unwrap_or_default(),
        });
    }

    Ok(nav_points)
}

fn read_markers(file: &SizeCheckedFile) -> Result<Vec<Marker>, Error> {
    let Some(grp) = file.optional_group("markers")? else {
        return Ok(Vec::new());
    };

    let times = grp.dataset("time")?.read_i64()?;
    let lats = grp.dataset("lat")?.read_f64()?;
    let lons = grp.dataset("lon")?.read_f64()?;
    let icons = grp.dataset("icon")?.read_u8()?;
    let label_flat = grp.dataset("label")?.read_u8()?;

    let k = times.len();
    check_len("markers", "lat", k, lats.len())?;
    check_len("markers", "lon", k, lons.len())?;
    check_len("markers", "icon", k, icons.len())?;
    check_len("markers", "label", k * 256, label_flat.len())?;

    let mut markers = Vec::with_capacity(k);
    for (record, ((((time_us, lat_deg), lon_deg), icon_code), label_row)) in times
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(icons.iter())
        .zip(label_flat.chunks(256))
        .enumerate()
    {
        let label: MarkerLabelField = decode_field_row(
            FieldLocation {
                group: "markers",
                dataset: "label",
            },
            label_row,
        )?;
        let icon = AnnotationIcon::from_wire_code(*icon_code);
        if let AnnotationIcon::Unrecognized(code) = icon {
            log::warn!("unrecognized markers/icon code {code}, preserving it as-is");
        }
        markers.push(Marker {
            annotation: Annotation {
                time: decode_timestamp(
                    FieldLocation {
                        group: "markers",
                        dataset: "time",
                    },
                    record,
                    *time_us,
                )?,
                label: label.into_string_unless_empty(),
                icon,
            },
            lat: Angle::degrees(*lat_deg),
            lon: Angle::degrees(*lon_deg),
        });
    }

    Ok(markers)
}

struct EventMarkerRow<'a> {
    record: usize,
    sys_time_us: u64,
    lat_deg: f64,
    lon_deg: f64,
    variant_path_row: &'a [u8],
    annotation_row: &'a [u8],
}

impl EventMarkerRow<'_> {
    fn decode(self) -> Result<EventMarkerPoint, Error> {
        let variant_path: VariantPathField = decode_field_row(
            FieldLocation {
                group: "event_markers",
                dataset: "variant_path",
            },
            self.variant_path_row,
        )?;
        let annotation: AnnotationField = decode_field_row(
            FieldLocation {
                group: "event_markers",
                dataset: "annotation",
            },
            self.annotation_row,
        )?;
        let Some(sys_time) = decode_optional_timestamp(
            FieldLocation {
                group: "event_markers",
                dataset: "sys_time_us",
            },
            self.record,
            self.sys_time_us,
        )?
        else {
            return Err(Error::EventMarkerWithoutTimestamp {
                record: self.record,
            });
        };
        let Some(variant_path) = variant_path.into_string_unless_empty() else {
            return Err(Error::EmptyField {
                group: "event_markers",
                dataset: "variant_path",
                record: self.record,
            });
        };
        Ok(EventMarkerPoint {
            variant_path,
            sys_time,
            lat: Angle::degrees(self.lat_deg),
            lon: Angle::degrees(self.lon_deg),
            annotation: annotation.into_string_unless_empty(),
        })
    }
}

fn read_event_markers(file: &SizeCheckedFile) -> Result<Vec<EventMarkerPoint>, Error> {
    let Some(grp) = file.optional_group("event_markers")? else {
        return Ok(Vec::new());
    };

    let sys_times = grp.dataset("sys_time_us")?.read_u64()?;
    let lats = grp.dataset("lat")?.read_f64()?;
    let lons = grp.dataset("lon")?.read_f64()?;
    let vp_flat = grp.dataset("variant_path")?.read_u8()?;
    let ann_flat = grp.dataset("annotation")?.read_u8()?;

    let n = sys_times.len();
    check_len("event_markers", "lat", n, lats.len())?;
    check_len("event_markers", "lon", n, lons.len())?;
    check_len("event_markers", "variant_path", n * 256, vp_flat.len())?;
    check_len("event_markers", "annotation", n * 512, ann_flat.len())?;

    let mut markers = Vec::with_capacity(n);
    for (record, ((((sys_time_us, lat_deg), lon_deg), vp_row), ann_row)) in sys_times
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(vp_flat.chunks(256))
        .zip(ann_flat.chunks(512))
        .enumerate()
    {
        markers.push(
            EventMarkerRow {
                record,
                sys_time_us: *sys_time_us,
                lat_deg: *lat_deg,
                lon_deg: *lon_deg,
                variant_path_row: vp_row,
                annotation_row: ann_row,
            }
            .decode()?,
        );
    }

    Ok(markers)
}

fn read_event_marker_styles(file: &SizeCheckedFile) -> Result<Vec<EventMarkerStyle>, Error> {
    let Some(grp) = file.optional_group("event_marker_styles")? else {
        return Ok(Vec::new());
    };

    let vp_flat = grp.dataset("variant_path")?.read_u8()?;
    let icon_flat = grp.dataset("icon_name")?.read_u8()?;
    let color_flat = grp.dataset("color_hex")?.read_u8()?;

    let m = vp_flat.len() / 256;
    check_len("event_marker_styles", "icon_name", m * 32, icon_flat.len())?;
    check_len("event_marker_styles", "color_hex", m * 8, color_flat.len())?;

    let mut styles = Vec::with_capacity(m);
    for (record, ((vp_row, icon_row), color_row)) in vp_flat
        .chunks(256)
        .zip(icon_flat.chunks(32))
        .zip(color_flat.chunks(8))
        .enumerate()
    {
        let variant_path: VariantPathField = decode_field_row(
            FieldLocation {
                group: "event_marker_styles",
                dataset: "variant_path",
            },
            vp_row,
        )?;
        let icon_name: IconNameField = decode_field_row(
            FieldLocation {
                group: "event_marker_styles",
                dataset: "icon_name",
            },
            icon_row,
        )?;
        let color_hex: ColorHexField = decode_field_row(
            FieldLocation {
                group: "event_marker_styles",
                dataset: "color_hex",
            },
            color_row,
        )?;
        let Some(variant_path) = variant_path.into_string_unless_empty() else {
            return Err(Error::EmptyField {
                group: "event_marker_styles",
                dataset: "variant_path",
                record,
            });
        };
        let icon = EventMarkerIconChoice::from_wire_name(icon_name);
        if let EventMarkerIconChoice::Unrecognized(name) = &icon {
            log::warn!(
                "unrecognized event_marker_styles/icon_name value {name:?}, preserving it as-is"
            );
        }
        let color = EventMarkerColor::from_wire_value(color_hex);
        if let EventMarkerColor::Unrecognized(value) = &color {
            log::warn!(
                "unrecognized event_marker_styles/color_hex value {value:?}, preserving it as-is"
            );
        }
        styles.push(EventMarkerStyle {
            variant_path,
            icon,
            color,
        });
    }

    Ok(styles)
}

/// Read ad-hoc channels from `channels/<name>/`. Absent in files written before
/// channels existed, in which case there are simply none. Channels are returned
/// sorted by name for a deterministic order independent of how the producer
/// added them.
fn read_channels(file: &SizeCheckedFile) -> Result<Vec<Channel>, Error> {
    let Some(root) = file.optional_group("channels")? else {
        return Ok(Vec::new());
    };

    let mut channels = Vec::new();
    for name in root.groups()? {
        let grp = root.group(&name)?;
        let attrs = grp.attrs()?;

        let times_us = grp.dataset("time")?.read_i64()?;
        // `read_f64` flattens the 2-D vector dataset row-major, which is exactly
        // the layout `Channel` stores. A vector channel has one column per
        // component, a scalar channel one column.
        let values = grp.dataset("value")?.read_f64()?;
        let components = string_array_attr(&attrs, "components").unwrap_or_default();
        let columns = components.len().max(1);
        let expected = times_us
            .len()
            .checked_mul(columns)
            .ok_or(Error::ShapeMismatch {
                group: "channels",
                dataset: "value",
                expected: usize::MAX,
                actual: values.len(),
            })?;
        check_len("channels", "value", expected, values.len())?;

        let times = times_us
            .into_iter()
            .enumerate()
            .map(|(record, us)| {
                decode_timestamp(
                    FieldLocation {
                        group: "channels",
                        dataset: "time",
                    },
                    record,
                    us,
                )
            })
            .collect::<Result<Vec<DateTime<Utc>>, Error>>()?;

        channels.push(Channel {
            name,
            unit: string_attr(&attrs, "unit").map(ChannelUnit::from_file_label),
            period: f64_attr(&attrs, "period_deg").map(Angle::degrees),
            description: string_attr(&attrs, "description"),
            components,
            times,
            values,
        });
    }
    channels.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(channels)
}

/// The row of `table` that `index` addresses, or [`Error::IndexPastTable`]
/// where `index` is past its end. `index_location` is the group and dataset
/// `index` was read from.
fn row_addressed_by_index<'a, T>(
    table: &'a mut [T],
    table_name: &'static str,
    index_location: FieldLocation,
    record: usize,
    index: u64,
) -> Result<&'a mut T, Error> {
    let table_len = table.len();
    usize::try_from(index)
        .ok()
        .and_then(|row| table.get_mut(row))
        .ok_or(Error::IndexPastTable {
            group: index_location.group,
            dataset: index_location.dataset,
            record,
            index,
            table: table_name,
            table_len,
        })
}

fn decode_timestamp(
    location: FieldLocation,
    record: usize,
    micros: i64,
) -> Result<DateTime<Utc>, Error> {
    DateTime::from_timestamp_micros(micros).ok_or(Error::TimestampOutOfRange {
        group: location.group,
        dataset: location.dataset,
        record,
        micros,
    })
}

/// [`ABSENT_TIMESTAMP_MICROS`] decodes to `None`.
fn decode_optional_timestamp(
    location: FieldLocation,
    record: usize,
    stored: u64,
) -> Result<Option<DateTime<Utc>>, Error> {
    if stored == ABSENT_TIMESTAMP_MICROS {
        return Ok(None);
    }
    decode_timestamp(location, record, stored.cast_signed()).map(Some)
}

fn decode_field_row<const ROW_BYTES: usize>(
    location: FieldLocation,
    row: &[u8],
) -> Result<FixedWidthString<ROW_BYTES>, Error> {
    FixedWidthString::decode_row(row).map_err(|source| Error::UnreadableField {
        group: location.group,
        dataset: location.dataset,
        source,
    })
}

fn check_len(
    group: &'static str,
    dataset: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), Error> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::ShapeMismatch {
            group,
            dataset,
            expected,
            actual,
        })
    }
}

fn opt_f32(v: f32) -> Option<f32> {
    if v.is_nan() { None } else { Some(v) }
}

/// The most rows of a fixed-width string field the summary previews on one
/// line, and the most unreadable rows it lists.
const PREVIEW_ROWS: usize = 3;

/// The most entries a section lists one per line, with `…` where more follow.
const MAX_LISTED_ENTRIES: usize = 20;

const ABSENT_VALUE: &str = "—";

const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// One row of a fixed-width string dataset: its value, or the error
/// [`FixedWidthString::decode_row`] gives for it. [`NavFile::read`] reports
/// that same error as [`Error::UnreadableField`]. `inspect` lists the row and
/// prints the rest of the summary.
type FieldRow<F> = Result<F, FixedWidthStringError>;

pub(crate) fn inspect_path(path: &Path) -> Result<String, Error> {
    use std::fmt::Write as _;

    let file = SizeCheckedFile::open(path)?;
    let root = file.root();
    let attrs = root.attrs()?;

    let mut out = String::new();
    let sep = "─".repeat(60);

    let version = attrs
        .get("geotrace_version")
        .and_then(AttrValue::as_str)
        .unwrap_or("<unknown>");
    writeln!(out, "GeoTrace Data File - version {version}").ok();
    writeln!(out, "{sep}").ok();

    inspect_metadata(&attrs, &mut out);
    writeln!(out).ok();
    let nav_points = inspect_nav_points(&file, &mut out);
    writeln!(out).ok();
    inspect_satellite_reports(&file, nav_points, &mut out);
    writeln!(out).ok();
    inspect_markers(&file, &mut out);
    writeln!(out).ok();
    inspect_event_markers(&file, &mut out);
    writeln!(out).ok();
    inspect_event_marker_styles(&file, &mut out);
    writeln!(out).ok();
    inspect_channels(&file, &mut out);
    writeln!(out, "{sep}").ok();

    Ok(out)
}

fn inspect_metadata(attrs: &HashMap<String, AttrValue>, out: &mut String) {
    use std::fmt::Write as _;

    writeln!(out, "Metadata").ok();

    let quoted = |key| string_attr(attrs, key).map(|value| format!("{value:?}"));
    let bare = |key| string_attr(attrs, key);
    let fields = [
        ("title", quoted("meta_title")),
        ("device", quoted("meta_device")),
        ("notes", quoted("meta_notes")),
        ("identity", quoted("meta_identity")),
        ("travel mode", bare("meta_travel_mode")),
        ("sdk version", bare(provenance::SDK_VERSION_ATTR)),
        ("sdk commit", bare(provenance::SDK_GIT_COMMIT_ATTR)),
        ("sdk commit time", bare(provenance::SDK_COMMIT_TIME_ATTR)),
    ];

    let mut present = fields
        .iter()
        .filter_map(|(label, value)| value.as_deref().map(|value| (label, value)))
        .peekable();
    if present.peek().is_none() {
        writeln!(out, "  {ABSENT_VALUE}").ok();
        return;
    }
    for (label, value) in present {
        writeln!(out, "  {label:<15}: {value}").ok();
    }
}

fn inspect_nav_points(file: &SizeCheckedFile, out: &mut String) -> u64 {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("nav_points") else {
        writeln!(out, "{:<24}0 records", "Nav Points").ok();
        return 0;
    };

    let n = dataset_rows(&grp, "time");

    writeln!(out, "{:<24}{} records", "Nav Points", fmt_count(n)).ok();

    if n == 0 {
        return 0;
    }

    if let Some(times) = grp.dataset("time").ok().and_then(|ds| ds.read_i64().ok())
        && let Some((first, last)) = first_and_last_time(&times)
    {
        writeln!(
            out,
            "  {:<22}{} → {}",
            "time",
            first.format(TIME_FORMAT),
            last.format(TIME_FORMAT)
        )
        .ok();
    }

    for (label, ds_name, dec) in &[
        ("lat", "lat", 3usize),
        ("lon", "lon", 3),
        ("heading", "heading", 1),
    ] {
        if let Some(vals) = grp.dataset(ds_name).ok().and_then(|ds| ds.read_f64().ok()) {
            let (mn, mx) = min_max_f64(&vals);
            writeln!(
                out,
                "  {:<22}{:.dec$}° – {:.dec$}°",
                label,
                mn,
                mx,
                dec = dec
            )
            .ok();
        }
    }

    if let Some(vals) = grp
        .dataset("speed_mps")
        .ok()
        .and_then(|ds| ds.read_f64().ok())
    {
        writeln!(
            out,
            "  {:<22}{}",
            "speed",
            MeasuredRange::of(vals.iter().copied()).line_value(" m/s")
        )
        .ok();
    }

    n
}

fn inspect_satellite_reports(file: &SizeCheckedFile, n_nav_points: u64, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(sat_grp) = file.group("sat_reports") else {
        writeln!(out, "{:<24}0 records", "Satellite Reports").ok();
        return;
    };

    // Counts `nav_point_idx`, the one name every version writes it under. The
    // time field is named `time` in v1 and `gps_time_us` in v2.
    let m = dataset_rows(&sat_grp, "nav_point_idx");

    if n_nav_points > 0 {
        writeln!(
            out,
            "{:<24}{} records  ({} / {} nav points have data)",
            "Satellite Reports",
            fmt_count(m),
            fmt_count(m),
            fmt_count(n_nav_points)
        )
        .ok();
    } else {
        writeln!(out, "{:<24}{} records", "Satellite Reports", fmt_count(m)).ok();
    }

    if m == 0 {
        return;
    }

    let tracked = file.group("tracked_sats").ok();
    let t = tracked
        .as_ref()
        .map_or(0, |grp| dataset_rows(grp, "sat_report_idx"));
    writeln!(
        out,
        "  {:<22}{} total  (avg {:.1} per report)",
        "Tracked satellites",
        fmt_count(t),
        t as f64 / m as f64
    )
    .ok();

    let Some(ts_grp) = tracked else {
        return;
    };

    let constellation_codes = ts_grp
        .dataset("constellation")
        .ok()
        .and_then(|ds| ds.read_u8().ok());
    if let Some(codes) = constellation_codes.as_deref() {
        let list = constellation_names(codes);
        if !list.is_empty() {
            writeln!(out, "    {:<20}{}", "constellations", list.join(", ")).ok();
        }
    }
    if let Some(codes) = constellation_codes.as_deref()
        && let Some(prns) = ts_grp.dataset("prn").ok().and_then(|ds| ds.read_u32().ok())
    {
        let distinct: HashSet<(u8, u32)> =
            codes.iter().copied().zip(prns.iter().copied()).collect();
        writeln!(
            out,
            "    {:<20}{}",
            "distinct satellites",
            fmt_count(distinct.len() as u64)
        )
        .ok();
    }

    for name in ["elevation", "azimuth"] {
        if let Some(degrees) = ts_grp.dataset(name).ok().and_then(|ds| ds.read_f32().ok()) {
            writeln!(
                out,
                "    {:<20}{}",
                name,
                MeasuredRange::of(degrees.iter().copied().map(f64::from)).line_value("°")
            )
            .ok();
        }
    }

    if let Some(snr_values) = ts_grp.dataset("snr").ok().and_then(|ds| ds.read_f32().ok()) {
        writeln!(
            out,
            "    {:<20}{}",
            "SNR",
            MeasuredRange::of(snr_values.iter().copied().map(f64::from)).line_value(" dB-Hz")
        )
        .ok();
        let no_data = snr_values
            .iter()
            .filter(|&&value| snr::is_no_data_sentinel(value))
            .count();
        if no_data > 0 {
            writeln!(
                out,
                "    {:<20}{} / {} readings at {:.0} dB-Hz",
                "no-data SNR",
                fmt_count(no_data as u64),
                fmt_count(snr_values.len() as u64),
                snr::NO_DATA_SENTINEL_DB_HZ
            )
            .ok();
        }
    }

    if let Some(in_fix_vals) = ts_grp
        .dataset("in_fix")
        .ok()
        .and_then(|ds| ds.read_u8().ok())
    {
        let fix_count: u64 = in_fix_vals.iter().filter(|&&v| v != 0).count() as u64;
        writeln!(
            out,
            "  {:<22}{} total  (avg {:.1} per report)",
            "Fix satellites",
            fmt_count(fix_count),
            fix_count as f64 / m as f64
        )
        .ok();
    }
}

fn inspect_markers(file: &SizeCheckedFile, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("markers") else {
        writeln!(out, "{:<24}0 records", "Markers").ok();
        return;
    };

    let k = dataset_rows(&grp, "time");

    writeln!(out, "{:<24}{} records", "Markers", fmt_count(k)).ok();

    if k == 0 {
        return;
    }

    if let Some(icons) = grp.dataset("icon").ok().and_then(|ds| ds.read_u8().ok()) {
        let histogram = icon_histogram(&icons);
        if !histogram.is_empty() {
            writeln!(out, "  {:<22}{histogram}", "icons").ok();
        }
    }

    let labels: Option<Vec<FieldRow<MarkerLabelField>>> = read_field_rows(&grp, "label");
    if let Some(labels) = labels {
        let preview = quoted_values(&labels);
        if !preview.is_empty() {
            writeln!(out, "  {:<22}{}", "labels", preview_list(&preview)).ok();
        }
        write_unreadable_rows(&labels, "label", out);
    }
}

fn inspect_event_markers(file: &SizeCheckedFile, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("event_markers") else {
        writeln!(out, "{:<24}0 records", "Event Markers").ok();
        return;
    };

    let n = dataset_rows(&grp, "sys_time_us");

    writeln!(out, "{:<24}{} records", "Event Markers", fmt_count(n)).ok();

    if n == 0 {
        return;
    }

    if let Some(times) = grp
        .dataset("sys_time_us")
        .ok()
        .and_then(|ds| ds.read_u64().ok())
        && let Some((earliest, latest)) = earliest_and_latest_time(&times)
    {
        writeln!(
            out,
            "  {:<22}{} → {}",
            "time",
            earliest.format(TIME_FORMAT),
            latest.format(TIME_FORMAT)
        )
        .ok();
    }

    let paths: Option<Vec<FieldRow<VariantPathField>>> = read_field_rows(&grp, "variant_path");
    if let Some(paths) = paths {
        let mut markers_per_path: BTreeMap<&str, u64> = BTreeMap::new();
        for path in paths.iter().filter_map(|row| row.as_ref().ok()) {
            *markers_per_path.entry(path.as_str()).or_default() += 1;
        }
        writeln!(
            out,
            "  {:<22}{} distinct",
            "variant paths",
            fmt_count(markers_per_path.len() as u64)
        )
        .ok();
        for (path, markers) in markers_per_path.iter().take(MAX_LISTED_ENTRIES) {
            writeln!(out, "    {path:<19} ×{markers}").ok();
        }
        if markers_per_path.len() > MAX_LISTED_ENTRIES {
            writeln!(out, "    …").ok();
        }
        write_unreadable_rows(&paths, "variant_path", out);
    }

    let annotations: Option<Vec<FieldRow<AnnotationField>>> = read_field_rows(&grp, "annotation");
    if let Some(annotations) = annotations {
        let preview = quoted_values(&annotations);
        if !preview.is_empty() {
            writeln!(out, "  {:<22}{}", "annotations", preview_list(&preview)).ok();
        }
        write_unreadable_rows(&annotations, "annotation", out);
    }
}

fn inspect_event_marker_styles(file: &SizeCheckedFile, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("event_marker_styles") else {
        writeln!(out, "{:<24}0 records", "Event Marker Styles").ok();
        return;
    };

    let paths: Option<Vec<FieldRow<VariantPathField>>> = read_field_rows(&grp, "variant_path");
    let Some(paths) = paths else {
        writeln!(out, "{:<24}unreadable variant_path", "Event Marker Styles").ok();
        return;
    };

    writeln!(
        out,
        "{:<24}{} records",
        "Event Marker Styles",
        fmt_count(paths.len() as u64)
    )
    .ok();

    let icons: Vec<FieldRow<IconNameField>> =
        read_field_rows(&grp, "icon_name").unwrap_or_default();
    let colors: Vec<FieldRow<ColorHexField>> =
        read_field_rows(&grp, "color_hex").unwrap_or_default();

    for (record, path) in paths.iter().enumerate().take(MAX_LISTED_ENTRIES) {
        match path {
            Ok(path) => {
                let label = if path.is_empty() {
                    ABSENT_VALUE
                } else {
                    path.as_str()
                };
                writeln!(
                    out,
                    "  {:<21} {}",
                    label,
                    icon_and_color(icons.get(record), colors.get(record))
                )
                .ok();
            }
            Err(_) => {
                let label = format!("row {record}");
                writeln!(
                    out,
                    "  {label:<21} {}",
                    icon_and_color(icons.get(record), colors.get(record))
                )
                .ok();
            }
        }
    }
    if paths.len() > MAX_LISTED_ENTRIES {
        writeln!(out, "  …").ok();
    }
    write_unreadable_rows(&paths, "variant_path", out);
    write_unreadable_rows(&icons, "icon_name", out);
    write_unreadable_rows(&colors, "color_hex", out);
}

fn inspect_channels(file: &SizeCheckedFile, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(root) = file.group("channels") else {
        writeln!(out, "{:<24}0 channels", "Channels").ok();
        return;
    };

    let mut names = root.groups().unwrap_or_default();
    names.sort();
    writeln!(
        out,
        "{:<24}{} channels",
        "Channels",
        fmt_count(names.len() as u64)
    )
    .ok();
    for name in &names {
        let Ok(grp) = root.group(name) else {
            continue;
        };
        let samples = dataset_rows(&grp, "value");
        let attrs = grp.attrs().ok();
        let unit = attrs
            .as_ref()
            .and_then(|a| string_attr(a, "unit"))
            .map_or_else(String::new, |u| format!(" {u}"));
        writeln!(out, "  {:<22}{} samples{unit}", name, fmt_count(samples)).ok();
        if let Some(components) = attrs
            .as_ref()
            .and_then(|a| string_array_attr(a, "components"))
        {
            writeln!(out, "    {:<20}{}", "components", components.join(", ")).ok();
        }
        if let Some(period_deg) = attrs.as_ref().and_then(|a| f64_attr(a, "period_deg")) {
            writeln!(out, "    {:<20}{period_deg:.1}°", "period").ok();
        }
        if let Some(description) = attrs.as_ref().and_then(|a| string_attr(a, "description")) {
            writeln!(out, "    {:<20}{description:?}", "description").ok();
        }
        if let Some(times) = grp.dataset("time").ok().and_then(|ds| ds.read_i64().ok())
            && let Some((first, last)) = first_and_last_time(&times)
        {
            writeln!(
                out,
                "    {:<20}{} → {}",
                "time",
                first.format(TIME_FORMAT),
                last.format(TIME_FORMAT)
            )
            .ok();
        }
    }
}

/// The rows the dataset declares, `0` where the group holds no such dataset or
/// its shape cannot be read.
fn dataset_rows(grp: &SizeCheckedGroup, name: &str) -> u64 {
    grp.dataset(name)
        .ok()
        .and_then(|ds| ds.shape().ok())
        .and_then(|shape| shape.first().copied())
        .unwrap_or(0)
}

/// The rows of a fixed-width string dataset, each decoded or holding the error
/// the reader returns for it. `None` where the group has no such dataset or it
/// cannot be read.
fn read_field_rows<const ROW_BYTES: usize>(
    grp: &SizeCheckedGroup,
    name: &str,
) -> Option<Vec<FieldRow<FixedWidthString<ROW_BYTES>>>> {
    let flat = grp.dataset(name).ok()?.read_u8().ok()?;
    Some(
        flat.chunks(ROW_BYTES)
            .map(FixedWidthString::decode_row)
            .collect(),
    )
}

/// Every value the rows decode to, quoted, skipping the rows the reader rejects
/// and those holding the empty value the format writes for an absent one.
fn quoted_values<const ROW_BYTES: usize>(
    rows: &[FieldRow<FixedWidthString<ROW_BYTES>>],
) -> Vec<String> {
    rows.iter()
        .filter_map(|row| row.as_ref().ok())
        .filter(|value| !value.is_empty())
        .map(|value| format!("{:?}", value.as_str()))
        .collect()
}

/// Lists the rows of `field` the reader rejects, at most [`PREVIEW_ROWS`] of
/// them.
fn write_unreadable_rows<const ROW_BYTES: usize>(
    rows: &[FieldRow<FixedWidthString<ROW_BYTES>>],
    field: &str,
    out: &mut String,
) {
    use std::fmt::Write as _;

    let unreadable: Vec<(usize, &FixedWidthStringError)> = rows
        .iter()
        .enumerate()
        .filter_map(|(record, row)| row.as_ref().err().map(|error| (record, error)))
        .collect();
    if unreadable.is_empty() {
        return;
    }

    let label = format!("unreadable {field}");
    writeln!(
        out,
        "  {label:<21} {} / {} rows",
        fmt_count(unreadable.len() as u64),
        fmt_count(rows.len() as u64)
    )
    .ok();
    for (record, error) in unreadable.iter().take(PREVIEW_ROWS) {
        let label = format!("row {record}");
        writeln!(out, "    {label:<19} {error}").ok();
    }
    if unreadable.len() > PREVIEW_ROWS {
        writeln!(out, "    …").ok();
    }
}

/// The icon and the color one event marker style sets, as one value. Absent
/// where the style leaves both to the app. For a row the reader rejects, the
/// value states its dataset. [`write_unreadable_rows`] lists that row's error.
fn icon_and_color(
    icon: Option<&FieldRow<IconNameField>>,
    color: Option<&FieldRow<ColorHexField>>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    match icon {
        Some(Ok(name)) => match EventMarkerIconChoice::from_wire_name(name.as_str()) {
            EventMarkerIconChoice::Auto => {}
            EventMarkerIconChoice::Icon(icon) => parts.push(icon.name().to_owned()),
            EventMarkerIconChoice::Unrecognized(name) => {
                parts.push(format!("unrecognized icon {name:?}"));
            }
        },
        Some(Err(_)) => parts.push("unreadable icon_name".to_owned()),
        None => {}
    }
    match color {
        Some(Ok(value)) => match EventMarkerColor::from_wire_value(value.as_str()) {
            EventMarkerColor::Auto => {}
            EventMarkerColor::Hex(hex) => parts.push(hex),
            EventMarkerColor::Unrecognized(value) => {
                parts.push(format!("unrecognized color {value:?}"));
            }
        },
        Some(Err(_)) => parts.push("unreadable color_hex".to_owned()),
        None => {}
    }

    if parts.is_empty() {
        ABSENT_VALUE.to_owned()
    } else {
        parts.join(", ")
    }
}

/// `items` joined with `, `, at most [`PREVIEW_ROWS`] of them, with `…` where
/// more follow.
fn preview_list(items: &[String]) -> String {
    let shown = items
        .iter()
        .take(PREVIEW_ROWS)
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(", ");
    if items.len() > PREVIEW_ROWS {
        format!("{shown}, …")
    } else {
        shown
    }
}

fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut buf = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            buf.push(' ');
        }
        buf.push(ch);
    }
    buf.chars().rev().collect()
}

/// The first and the last timestamp of a microsecond dataset, in the order the
/// file holds them.
fn first_and_last_time(micros: &[i64]) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let first = DateTime::from_timestamp_micros(*micros.first()?)?;
    let last = DateTime::from_timestamp_micros(*micros.last()?)?;
    Some((first, last))
}

/// The earliest and the latest timestamp of a microsecond dataset, skipping the
/// rows holding [`ABSENT_TIMESTAMP_MICROS`].
fn earliest_and_latest_time(micros: &[u64]) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let stamped = || {
        micros
            .iter()
            .copied()
            .filter(|&value| value != ABSENT_TIMESTAMP_MICROS)
    };
    let earliest = DateTime::from_timestamp_micros(stamped().min()?.cast_signed())?;
    let latest = DateTime::from_timestamp_micros(stamped().max()?.cast_signed())?;
    Some((earliest, latest))
}

fn min_max_f64(vals: &[f64]) -> (f64, f64) {
    vals.iter()
        .filter(|v| !v.is_nan())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &v| {
            (mn.min(v), mx.max(v))
        })
}

/// The lowest and the highest value a dataset holds, and how many of its rows
/// hold one at all. A row holding NaN, the `.gtd` encoding of an absent value,
/// is counted and enters neither bound.
struct MeasuredRange {
    min: f64,
    max: f64,
    present: usize,
    rows: usize,
}

impl MeasuredRange {
    fn of(values: impl IntoIterator<Item = f64>) -> Self {
        let mut range = Self {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            present: 0,
            rows: 0,
        };
        for value in values {
            range.rows += 1;
            if !value.is_nan() {
                range.min = range.min.min(value);
                range.max = range.max.max(value);
                range.present += 1;
            }
        }
        range
    }

    /// The range with `unit_suffix` after its high value, and how many rows
    /// hold a value. The absent marker where none do. `unit_suffix` opens with
    /// a space for a unit written with one (` dB-Hz`) and without for one
    /// written without (`°`).
    fn line_value(&self, unit_suffix: &str) -> String {
        if self.present == 0 {
            return format!(
                "{ABSENT_VALUE}  (0 / {} present)",
                fmt_count(self.rows as u64)
            );
        }
        format!(
            "{:.1} – {:.1}{unit_suffix}  ({} / {} present)",
            self.min,
            self.max,
            fmt_count(self.present as u64),
            fmt_count(self.rows as u64)
        )
    }
}

fn constellation_names(codes: &[u8]) -> Vec<&'static str> {
    use strum::EnumCount;
    let mut seen = [false; Constellation::COUNT];
    for &c in codes {
        if let Some(slot) = seen.get_mut(c as usize) {
            *slot = true;
        }
    }
    Constellation::iter()
        .filter(|c| seen.get(c.to_u8() as usize) == Some(&true))
        .map(Constellation::display_name)
        .collect()
}

/// Every `markers/icon` code the dataset holds, with how many markers carry it,
/// in code order. A code outside the [`MarkerIcon`](crate::MarkerIcon) set is
/// listed as itself.
fn icon_histogram(codes: &[u8]) -> String {
    let mut markers_per_code: BTreeMap<u8, u64> = BTreeMap::new();
    for &code in codes {
        *markers_per_code.entry(code).or_default() += 1;
    }
    markers_per_code
        .iter()
        .map(
            |(&code, markers)| match AnnotationIcon::from_wire_code(code) {
                AnnotationIcon::Icon(icon) => format!("{} ×{markers}", icon.name()),
                AnnotationIcon::Unrecognized(code) => {
                    format!("unrecognized code {code} ×{markers}")
                }
            },
        )
        .collect::<Vec<String>>()
        .join(", ")
}
