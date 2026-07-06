use std::collections::HashMap;
use std::path::Path;

use crate::builder::{micros_to_datetime, u64_to_opt_datetime};
use crate::error::Error;
use crate::types::{
    Annotation, Channel, Constellation, EventMarkerPoint, EventMarkerStyle, Marker, MarkerIcon,
    Meta, NavFile, NavFix, NavPoint, Satellite, SatelliteReport,
};
use crate::write;
use crate::{Angle, Velocity};
use hdf5_pure::File;
use strum::IntoEnumIterator;

pub(crate) fn parse_hdf5(bytes: Vec<u8>) -> Result<NavFile, Error> {
    let file = File::from_bytes(bytes)?;
    let root = file.root();

    let attrs = root.attrs()?;
    let version = match attrs.get("geotrace_version") {
        Some(hdf5_pure::AttrValue::String(v)) => v.clone(),
        _ => {
            return Err(Error::UnsupportedVersion {
                version: "<missing>".into(),
            });
        }
    };
    if !version.starts_with('1') && !version.starts_with('2') {
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

fn read_meta(attrs: &HashMap<String, hdf5_pure::AttrValue>) -> Meta {
    Meta {
        title: string_attr(attrs, "meta_title"),
        device: string_attr(attrs, "meta_device"),
        notes: string_attr(attrs, "meta_notes"),
        identity: string_attr(attrs, "meta_identity"),
    }
}

fn string_attr(attrs: &HashMap<String, hdf5_pure::AttrValue>, key: &str) -> Option<String> {
    match attrs.get(key) {
        Some(hdf5_pure::AttrValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn f64_attr(attrs: &HashMap<String, hdf5_pure::AttrValue>, key: &str) -> Option<f64> {
    match attrs.get(key) {
        Some(hdf5_pure::AttrValue::F64(v)) => Some(*v),
        _ => None,
    }
}

fn string_array_attr(
    attrs: &HashMap<String, hdf5_pure::AttrValue>,
    key: &str,
) -> Option<Vec<String>> {
    use hdf5_pure::AttrValue::{String as StringAttr, StringArray};
    match attrs.get(key) {
        Some(StringArray(v)) => Some(v.clone()),
        // A single-element string array reads back as a plain `String`, so a
        // one-component vector's `components` attribute arrives this way.
        Some(StringAttr(s)) => Some(vec![s.clone()]),
        _ => None,
    }
}

fn read_nav_points(file: &File) -> Result<Vec<NavPoint>, Error> {
    let grp = file.group("nav_points")?;

    let times = grp.dataset("time")?.read_i64()?;
    let lats = grp.dataset("lat")?.read_f64()?;
    let lons = grp.dataset("lon")?.read_f64()?;
    let headings = grp.dataset("heading")?.read_f64()?;
    let speeds = grp.dataset("speed_mps")?.read_f64()?;

    // sys_time_us and eph_m are absent in older files. Default to sentinel/NaN.
    let sys_times: Vec<u64> = grp
        .dataset("sys_time_us")
        .and_then(|ds| ds.read_u64())
        .unwrap_or_else(|_| vec![u64::MAX; times.len()]);
    let ephs: Vec<f64> = grp
        .dataset("eph_m")
        .and_then(|ds| ds.read_f64())
        .unwrap_or_else(|_| vec![f64::NAN; times.len()]);

    let n = times.len();
    check_len("nav_points", "lat", n, lats.len())?;
    check_len("nav_points", "lon", n, lons.len())?;
    check_len("nav_points", "heading", n, headings.len())?;
    check_len("nav_points", "speed_mps", n, speeds.len())?;

    let nav_points = times
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(headings.iter())
        .zip(speeds.iter())
        .zip(sys_times.iter())
        .zip(ephs.iter())
        .map(
            |(
                (((((time_us, lat_deg), lon_deg), heading_deg), speed_mps), sys_time_us),
                eph_val,
            )| {
                NavPoint {
                    fix: NavFix {
                        gps_time: Some(micros_to_datetime(*time_us)),
                        sys_time: u64_to_opt_datetime(*sys_time_us),
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
                }
            },
        )
        .collect();

    Ok(nav_points)
}

fn attach_satellite_data(
    mut nav_points: Vec<NavPoint>,
    file: &File,
) -> Result<Vec<NavPoint>, Error> {
    let Ok(sat_grp) = file.group("sat_reports") else {
        return Ok(nav_points);
    };

    let nav_point_idx = sat_grp.dataset("nav_point_idx")?.read_u64()?;
    let r = nav_point_idx.len();

    // v2: gps_time_us / sys_time_us (both f64, NaN = absent).
    // v1: single "time" dataset mapped to gps_time. sys_time absent.
    let (report_gps_times, report_sys_times): (Vec<u64>, Vec<u64>) =
        if let Ok(ds) = sat_grp.dataset("gps_time_us") {
            let gps = ds.read_u64()?;
            let sys = sat_grp
                .dataset("sys_time_us")
                .and_then(|d| d.read_u64())
                .unwrap_or_else(|_| vec![u64::MAX; r]);
            (gps, sys)
        } else {
            // v1 file: old "time" (i64) treated as gps_time. No sys_time.
            let times = sat_grp.dataset("time")?.read_i64()?;
            let gps = times.iter().map(|&us| us.cast_unsigned()).collect();
            let sys = vec![u64::MAX; r];
            (gps, sys)
        };

    let ts_grp = file.group("tracked_sats")?;
    let ts_rep_idx = ts_grp.dataset("sat_report_idx")?.read_u64()?;
    let ts_constellation = ts_grp.dataset("constellation")?.read_u8()?;
    let ts_prn = ts_grp.dataset("prn")?.read_u32()?;
    let ts_in_fix = ts_grp.dataset("in_fix")?.read_u8()?;
    let ts_elevation = ts_grp.dataset("elevation")?.read_f32()?;
    let ts_azimuth = ts_grp.dataset("azimuth")?.read_f32()?;
    let ts_snr = ts_grp.dataset("snr")?.read_f32()?;

    let mut tracked_by_report: Vec<Vec<Satellite>> = vec![Vec::new(); r];
    for (&rep_idx, constellation_code, &prn, &in_fix, &elevation, &azimuth, &snr) in ts_rep_idx
        .iter()
        .zip(ts_constellation.iter())
        .zip(ts_prn.iter())
        .zip(ts_in_fix.iter())
        .zip(ts_elevation.iter())
        .zip(ts_azimuth.iter())
        .zip(ts_snr.iter())
        .map(|((((((a, b), c), d), e), f), g)| (a, b, c, d, e, f, g))
    {
        let idx = rep_idx as usize;
        if idx >= r {
            continue;
        }
        let constellation = write::decode_tracked_constellation(*constellation_code)?;
        let sat = Satellite {
            constellation,
            prn,
            in_fix: in_fix != 0,
            elevation: opt_f32(elevation),
            azimuth: opt_f32(azimuth),
            snr: opt_f32(snr),
        };
        if let Some(bucket) = tracked_by_report.get_mut(idx) {
            bucket.push(sat);
        }
    }

    for (i, (&np_idx, (gps_us, sys_us))) in nav_point_idx
        .iter()
        .zip(report_gps_times.iter().zip(report_sys_times.iter()))
        .enumerate()
    {
        if let Some(np) = nav_points.get_mut(np_idx as usize) {
            np.satellites = Some(SatelliteReport {
                gps_time: u64_to_opt_datetime(*gps_us),
                sys_time: u64_to_opt_datetime(*sys_us),
                tracked: tracked_by_report.get(i).cloned().unwrap_or_default(),
            });
        }
    }

    Ok(nav_points)
}

fn read_markers(file: &File) -> Result<Vec<Marker>, Error> {
    let Ok(grp) = file.group("markers") else {
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

    let markers = times
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(icons.iter())
        .zip(label_flat.chunks(256))
        .map(
            |((((time_us, lat_deg), lon_deg), icon_code), label_row)| Marker {
                annotation: Annotation {
                    time: micros_to_datetime(*time_us),
                    label: decode_label(label_row),
                    icon: Some(MarkerIcon::from_u8(*icon_code)),
                },
                lat: Angle::degrees(*lat_deg),
                lon: Angle::degrees(*lon_deg),
            },
        )
        .collect();

    Ok(markers)
}

fn read_event_markers(file: &File) -> Result<Vec<EventMarkerPoint>, Error> {
    let Ok(grp) = file.group("event_markers") else {
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

    let markers = sys_times
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(vp_flat.chunks(256))
        .zip(ann_flat.chunks(512))
        .filter_map(|((((sys_time_us, lat_deg), lon_deg), vp_row), ann_row)| {
            let sys_time = crate::builder::u64_to_opt_datetime(*sys_time_us)?;
            let variant_path = decode_fixed_str(vp_row)?;
            let annotation = decode_fixed_str(ann_row);
            Some(EventMarkerPoint {
                variant_path,
                sys_time,
                lat: Angle::degrees(*lat_deg),
                lon: Angle::degrees(*lon_deg),
                annotation,
            })
        })
        .collect();

    Ok(markers)
}

fn read_event_marker_styles(file: &File) -> Result<Vec<EventMarkerStyle>, Error> {
    let Ok(grp) = file.group("event_marker_styles") else {
        return Ok(Vec::new());
    };

    let vp_flat = grp.dataset("variant_path")?.read_u8()?;
    let icon_flat = grp.dataset("icon_name")?.read_u8()?;
    let color_flat = grp.dataset("color_hex")?.read_u8()?;

    let m = vp_flat.len() / 256;
    check_len("event_marker_styles", "icon_name", m * 32, icon_flat.len())?;
    check_len("event_marker_styles", "color_hex", m * 8, color_flat.len())?;

    let styles = vp_flat
        .chunks(256)
        .zip(icon_flat.chunks(32))
        .zip(color_flat.chunks(8))
        .filter_map(|((vp_row, icon_row), color_row)| {
            let variant_path = decode_fixed_str(vp_row)?;
            let icon = decode_fixed_str(icon_row)
                .and_then(|s| crate::types::MarkerIcon::try_from_lower_case(&s).ok())
                .map_or(
                    crate::types::EventMarkerIconChoice::Auto,
                    crate::types::EventMarkerIconChoice::Icon,
                );
            let color = decode_fixed_str(color_row)
                .filter(|s| s.starts_with('#'))
                .map_or(
                    crate::types::EventMarkerColor::Auto,
                    crate::types::EventMarkerColor::Hex,
                );
            Some(EventMarkerStyle {
                variant_path,
                icon,
                color,
            })
        })
        .collect();

    Ok(styles)
}

/// Read ad-hoc channels from `channels/<name>/`. Absent in files written before
/// channels existed, in which case there are simply none. Channels are returned
/// sorted by name for a deterministic order independent of how the producer
/// added them.
fn read_channels(file: &File) -> Result<Vec<Channel>, Error> {
    let Ok(root) = file.group("channels") else {
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

        channels.push(Channel {
            name,
            unit: string_attr(&attrs, "unit"),
            period: f64_attr(&attrs, "period_deg").map(Angle::degrees),
            description: string_attr(&attrs, "description"),
            components,
            times: times_us.into_iter().map(micros_to_datetime).collect(),
            values,
        });
    }
    channels.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(channels)
}

fn decode_fixed_str(row: &[u8]) -> Option<String> {
    let end = row.iter().position(|&b| b == 0).unwrap_or(row.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(row.get(..end)?).into_owned())
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

fn decode_label(row: &[u8]) -> Option<String> {
    let end = row.iter().position(|&b| b == 0).unwrap_or(row.len());
    if end == 0 {
        return None;
    }
    let s = String::from_utf8_lossy(row.get(..end)?).into_owned();
    Some(s)
}

fn opt_f32(v: f32) -> Option<f32> {
    if v.is_nan() { None } else { Some(v) }
}

pub(crate) fn inspect_path(path: &Path) -> Result<String, Error> {
    use std::fmt::Write as _;

    let file = File::open(path)?;
    let root = file.root();
    let attrs = root.attrs()?;

    let mut out = String::new();
    let sep = "─".repeat(60);

    let version = match attrs.get("geotrace_version") {
        Some(hdf5_pure::AttrValue::String(v)) => v.as_str().to_owned(),
        _ => "<unknown>".into(),
    };
    writeln!(out, "GeoTrace Data File - version {version}").ok();
    writeln!(out, "{sep}").ok();

    // Metadata
    writeln!(out, "Metadata").ok();
    let fmt_meta = |v: Option<String>| v.map_or_else(|| "—".to_owned(), |s| format!("\"{s}\""));
    writeln!(
        out,
        "  title  : {}",
        fmt_meta(string_attr(&attrs, "meta_title"))
    )
    .ok();
    writeln!(
        out,
        "  device : {}",
        fmt_meta(string_attr(&attrs, "meta_device"))
    )
    .ok();
    writeln!(
        out,
        "  notes  : {}",
        fmt_meta(string_attr(&attrs, "meta_notes"))
    )
    .ok();
    writeln!(out).ok();

    let n = inspect_nav_points(&file, &mut out);
    writeln!(out).ok();
    inspect_satellite_reports(&file, n, &mut out);
    writeln!(out).ok();
    inspect_markers(&file, &mut out);
    writeln!(out).ok();
    inspect_channels(&file, &mut out);
    writeln!(out, "{sep}").ok();

    Ok(out)
}

fn inspect_nav_points(file: &File, out: &mut String) -> u64 {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("nav_points") else {
        writeln!(out, "{:<24}0 records", "Nav Points").ok();
        return 0;
    };

    let n = grp
        .dataset("time")
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    writeln!(out, "{:<24}{} records", "Nav Points", fmt_count(n)).ok();

    if n == 0 {
        return 0;
    }

    if let Ok(times) = grp.dataset("time").and_then(|ds| ds.read_i64())
        && let (Some(&first), Some(&last)) = (times.first(), times.last())
    {
        let t0 = micros_to_datetime(first);
        let t1 = micros_to_datetime(last);
        writeln!(
            out,
            "  {:<22}{} → {}",
            "time",
            t0.format("%Y-%m-%dT%H:%M:%SZ"),
            t1.format("%Y-%m-%dT%H:%M:%SZ")
        )
        .ok();
    }

    for (label, ds_name, dec) in &[
        ("lat", "lat", 3usize),
        ("lon", "lon", 3),
        ("heading", "heading", 1),
    ] {
        if let Ok(vals) = grp.dataset(ds_name).and_then(|ds| ds.read_f64()) {
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

    if let Ok(vals) = grp.dataset("speed_mps").and_then(|ds| ds.read_f64()) {
        let (mn, mx, present) = min_max_present_f64(&vals);
        writeln!(
            out,
            "  {:<22}{:.1} – {:.1} m/s  ({} / {} present)",
            "speed",
            mn,
            mx,
            fmt_count(present as u64),
            fmt_count(n)
        )
        .ok();
    }

    n
}

fn inspect_satellite_reports(file: &File, n_nav_points: u64, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(sat_grp) = file.group("sat_reports") else {
        writeln!(out, "{:<24}0 records", "Satellite Reports").ok();
        return;
    };

    // Count via nav_point_idx (present in all versions) rather than a time field
    // whose name changed between v1 ("time") and v2 ("gps_time_us").
    let m = sat_grp
        .dataset("nav_point_idx")
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

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

    let t = file
        .group("tracked_sats")
        .and_then(|g| g.dataset("sat_report_idx"))
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    let avg_t = t as f64 / m as f64;
    writeln!(
        out,
        "  {:<22}{} total  (avg {:.1} per report)",
        "Tracked satellites",
        fmt_count(t),
        avg_t
    )
    .ok();

    if let Ok(ts_grp) = file.group("tracked_sats") {
        if let Ok(codes) = ts_grp.dataset("constellation").and_then(|ds| ds.read_u8()) {
            let list = constellation_names(&codes);
            if !list.is_empty() {
                writeln!(out, "    {:<20}{}", "constellations", list.join(", ")).ok();
            }
        }
        if let Ok(snr_vals) = ts_grp.dataset("snr").and_then(|ds| ds.read_f32()) {
            let (mn, mx, present) = min_max_present_f32(&snr_vals);
            writeln!(
                out,
                "    {:<20}{:.1} – {:.1} dB-Hz  ({} / {} present)",
                "SNR",
                mn,
                mx,
                fmt_count(present as u64),
                fmt_count(t)
            )
            .ok();
        }
    }

    if let Ok(ts_grp) = file.group("tracked_sats")
        && let Ok(in_fix_vals) = ts_grp.dataset("in_fix").and_then(|ds| ds.read_u8())
    {
        let fix_count: u64 = in_fix_vals.iter().filter(|&&v| v != 0).count() as u64;
        let avg_f = fix_count as f64 / m as f64;
        writeln!(
            out,
            "  {:<22}{} total  (avg {:.1} per report)",
            "Fix satellites",
            fmt_count(fix_count),
            avg_f
        )
        .ok();
    }
}

fn inspect_markers(file: &File, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("markers") else {
        writeln!(out, "{:<24}0 records", "Markers").ok();
        return;
    };

    let k = grp
        .dataset("time")
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    writeln!(out, "{:<24}{} records", "Markers", fmt_count(k)).ok();

    if k == 0 {
        return;
    }

    if let Ok(icons) = grp.dataset("icon").and_then(|ds| ds.read_u8()) {
        let hist = icon_histogram(&icons);
        if !hist.is_empty() {
            writeln!(out, "  {:<22}{hist}", "icons").ok();
        }
    }

    if let Ok(label_flat) = grp.dataset("label").and_then(|ds| ds.read_u8()) {
        let preview = label_preview(&label_flat);
        if !preview.is_empty() {
            writeln!(out, "  {:<22}{preview}", "labels").ok();
        }
    }
}

fn inspect_channels(file: &File, out: &mut String) {
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
        let samples = grp
            .dataset("value")
            .and_then(|ds| ds.shape())
            .ok()
            .and_then(|s| s.first().copied())
            .unwrap_or(0);
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

fn min_max_f64(vals: &[f64]) -> (f64, f64) {
    vals.iter()
        .filter(|v| !v.is_nan())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &v| {
            (mn.min(v), mx.max(v))
        })
}

fn min_max_present_f64(vals: &[f64]) -> (f64, f64, usize) {
    let mut mn = f64::INFINITY;
    let mut mx = f64::NEG_INFINITY;
    let mut count = 0usize;
    for &v in vals {
        if !v.is_nan() {
            mn = mn.min(v);
            mx = mx.max(v);
            count += 1;
        }
    }
    (mn, mx, count)
}

fn min_max_present_f32(vals: &[f32]) -> (f32, f32, usize) {
    let mut mn = f32::INFINITY;
    let mut mx = f32::NEG_INFINITY;
    let mut count = 0usize;
    for &v in vals {
        if !v.is_nan() {
            mn = mn.min(v);
            mx = mx.max(v);
            count += 1;
        }
    }
    (mn, mx, count)
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

fn icon_histogram(codes: &[u8]) -> String {
    let mut counts = [0u64; 7];
    for &c in codes {
        if let Some(slot) = counts.get_mut(c as usize) {
            *slot += 1;
        }
    }
    let parts: Vec<String> = counts
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c > 0)
        .map(|(i, &c)| {
            let name = MarkerIcon::from_u8(i as u8).name();
            format!("{name} ×{c}")
        })
        .collect();
    parts.join(", ")
}

fn label_preview(flat: &[u8]) -> String {
    let total = flat.len() / 256;
    let shown = total.min(3);
    let labels: Vec<String> = flat
        .chunks(256)
        .take(shown)
        .map(|row| {
            let end = row.iter().position(|&b| b == 0).unwrap_or(row.len());
            let s = String::from_utf8_lossy(row.get(..end).unwrap_or(&[]));
            format!("\"{s}\"")
        })
        .collect();
    if total > 3 {
        format!("{}, …", labels.join(", "))
    } else {
        labels.join(", ")
    }
}
