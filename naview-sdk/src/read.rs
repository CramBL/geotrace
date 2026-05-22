use hdf5_pure::File;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Velocity};
use uom::si::velocity::meter_per_second;

use crate::builder::micros_to_datetime;
use crate::error::Error;
use crate::types::{
    Annotation, FixEntry, Marker, MarkerIcon, Meta, NavFile, NavFix, NavPoint, Satellite,
    SatelliteReport,
};
use crate::write::{decode_fix_constellation, decode_tracked_constellation};

pub(crate) fn parse_hdf5(bytes: Vec<u8>) -> Result<NavFile, Error> {
    let file = File::from_bytes(bytes)?;
    let root = file.root();

    let attrs = root.attrs()?;
    let version = match attrs.get("naview_version") {
        Some(hdf5_pure::AttrValue::String(v)) => v.clone(),
        _ => {
            return Err(Error::UnsupportedVersion {
                version: "<missing>".into(),
            });
        }
    };
    if !version.starts_with('1') {
        return Err(Error::UnsupportedVersion { version });
    }

    let meta = read_meta(&attrs);
    let nav_points = read_nav_points(&file)?;
    let nav_points = attach_satellite_data(nav_points, &file)?;
    let markers = read_markers(&file)?;

    Ok(NavFile {
        meta,
        nav_points,
        markers,
    })
}

fn read_meta(attrs: &std::collections::HashMap<String, hdf5_pure::AttrValue>) -> Meta {
    Meta {
        title: string_attr(attrs, "meta_title"),
        device: string_attr(attrs, "meta_device"),
        notes: string_attr(attrs, "meta_notes"),
    }
}

fn string_attr(
    attrs: &std::collections::HashMap<String, hdf5_pure::AttrValue>,
    key: &str,
) -> Option<String> {
    match attrs.get(key) {
        Some(hdf5_pure::AttrValue::String(s)) => Some(s.clone()),
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
        .map(
            |((((time_us, lat_deg), lon_deg), heading_deg), speed_mps)| NavPoint {
                fix: NavFix {
                    time: micros_to_datetime(*time_us),
                    lat: Angle::new::<degree>(*lat_deg),
                    lon: Angle::new::<degree>(*lon_deg),
                    heading: Angle::new::<degree>(*heading_deg),
                    speed: if speed_mps.is_nan() {
                        None
                    } else {
                        Some(Velocity::new::<meter_per_second>(*speed_mps))
                    },
                },
                satellites: None,
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
    let report_times = sat_grp.dataset("time")?.read_i64()?;

    let r = nav_point_idx.len();
    check_len("sat_reports", "time", r, report_times.len())?;

    let ts_grp = file.group("tracked_sats")?;
    let ts_rep_idx = ts_grp.dataset("sat_report_idx")?.read_u64()?;
    let ts_constellation = ts_grp.dataset("constellation")?.read_u8()?;
    let ts_prn = ts_grp.dataset("prn")?.read_u32()?;
    let ts_elevation = ts_grp.dataset("elevation")?.read_f32()?;
    let ts_azimuth = ts_grp.dataset("azimuth")?.read_f32()?;
    let ts_snr = ts_grp.dataset("snr")?.read_f32()?;

    let fs_grp = file.group("fix_sats")?;
    let fs_rep_idx = fs_grp.dataset("sat_report_idx")?.read_u64()?;
    let fs_constellation = fs_grp.dataset("constellation")?.read_i8()?;
    let fs_prn = fs_grp.dataset("prn")?.read_u32()?;

    let mut tracked_by_report: Vec<Vec<Satellite>> = vec![Vec::new(); r];
    for (&rep_idx, constellation_code, &prn, &elevation, &azimuth, &snr) in ts_rep_idx
        .iter()
        .zip(ts_constellation.iter())
        .zip(ts_prn.iter())
        .zip(ts_elevation.iter())
        .zip(ts_azimuth.iter())
        .zip(ts_snr.iter())
        .map(|(((((a, b), c), d), e), f)| (a, b, c, d, e, f))
    {
        let idx = rep_idx as usize;
        if idx >= r {
            continue;
        }
        let constellation = decode_tracked_constellation(*constellation_code)?;
        let sat = Satellite {
            constellation,
            prn,
            elevation: opt_f32(elevation),
            azimuth: opt_f32(azimuth),
            snr: opt_f32(snr),
        };
        if let Some(bucket) = tracked_by_report.get_mut(idx) {
            bucket.push(sat);
        }
    }

    let mut fix_by_report: Vec<Vec<FixEntry>> = vec![Vec::new(); r];
    for (&rep_idx, &constellation_code, &prn) in fs_rep_idx
        .iter()
        .zip(fs_constellation.iter())
        .zip(fs_prn.iter())
        .map(|((a, b), c)| (a, b, c))
    {
        let idx = rep_idx as usize;
        if idx >= r {
            continue;
        }
        let constellation = decode_fix_constellation(constellation_code)?;
        let entry = FixEntry { constellation, prn };
        if let Some(bucket) = fix_by_report.get_mut(idx) {
            bucket.push(entry);
        }
    }

    for (i, (&np_idx, &rep_time)) in nav_point_idx.iter().zip(report_times.iter()).enumerate() {
        let np = nav_points.get_mut(np_idx as usize);
        if let Some(np) = np {
            np.satellites = Some(SatelliteReport {
                time: micros_to_datetime(rep_time),
                tracked: tracked_by_report.get(i).cloned().unwrap_or_default(),
                fix: fix_by_report.get(i).cloned().unwrap_or_default(),
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
                lat: Angle::new::<degree>(*lat_deg),
                lon: Angle::new::<degree>(*lon_deg),
            },
        )
        .collect();

    Ok(markers)
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

pub(crate) fn inspect_path(path: &std::path::Path) -> Result<String, Error> {
    use std::fmt::Write as _;

    let file = File::open(path)?;
    let root = file.root();
    let attrs = root.attrs()?;

    let mut out = String::new();
    let sep = "─".repeat(60);

    let version = match attrs.get("naview_version") {
        Some(hdf5_pure::AttrValue::String(v)) => v.as_str().to_owned(),
        _ => "<unknown>".into(),
    };
    writeln!(out, "NaView Data File — version {version}").unwrap_or(());
    writeln!(out, "{sep}").unwrap_or(());

    // Metadata
    writeln!(out, "Metadata").unwrap_or(());
    let fmt_meta = |v: Option<String>| v.map_or_else(|| "—".to_owned(), |s| format!("\"{s}\""));
    writeln!(
        out,
        "  title  : {}",
        fmt_meta(string_attr(&attrs, "meta_title"))
    )
    .unwrap_or(());
    writeln!(
        out,
        "  device : {}",
        fmt_meta(string_attr(&attrs, "meta_device"))
    )
    .unwrap_or(());
    writeln!(
        out,
        "  notes  : {}",
        fmt_meta(string_attr(&attrs, "meta_notes"))
    )
    .unwrap_or(());
    writeln!(out).unwrap_or(());

    let n = inspect_nav_points(&file, &mut out);
    writeln!(out).unwrap_or(());
    inspect_satellite_reports(&file, n, &mut out);
    writeln!(out).unwrap_or(());
    inspect_markers(&file, &mut out);
    writeln!(out, "{sep}").unwrap_or(());

    Ok(out)
}

fn inspect_nav_points(file: &File, out: &mut String) -> u64 {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("nav_points") else {
        writeln!(out, "{:<24}0 records", "Nav Points").unwrap_or(());
        return 0;
    };

    let n = grp
        .dataset("time")
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    writeln!(out, "{:<24}{} records", "Nav Points", fmt_count(n)).unwrap_or(());

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
        .unwrap_or(());
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
            .unwrap_or(());
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
        .unwrap_or(());
    }

    n
}

fn inspect_satellite_reports(file: &File, n_nav_points: u64, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(sat_grp) = file.group("sat_reports") else {
        writeln!(out, "{:<24}0 records", "Satellite Reports").unwrap_or(());
        return;
    };

    let m = sat_grp
        .dataset("time")
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
        .unwrap_or(());
    } else {
        writeln!(out, "{:<24}{} records", "Satellite Reports", fmt_count(m)).unwrap_or(());
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
    .unwrap_or(());

    if let Ok(ts_grp) = file.group("tracked_sats") {
        if let Ok(codes) = ts_grp.dataset("constellation").and_then(|ds| ds.read_u8()) {
            let list = constellation_names(&codes);
            if !list.is_empty() {
                writeln!(out, "    {:<20}{}", "constellations", list.join(", ")).unwrap_or(());
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
            .unwrap_or(());
        }
    }

    let f = file
        .group("fix_sats")
        .and_then(|g| g.dataset("sat_report_idx"))
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    let avg_f = f as f64 / m as f64;
    writeln!(
        out,
        "  {:<22}{} total  (avg {:.1} per report)",
        "Fix satellites",
        fmt_count(f),
        avg_f
    )
    .unwrap_or(());
}

fn inspect_markers(file: &File, out: &mut String) {
    use std::fmt::Write as _;

    let Ok(grp) = file.group("markers") else {
        writeln!(out, "{:<24}0 records", "Markers").unwrap_or(());
        return;
    };

    let k = grp
        .dataset("time")
        .and_then(|ds| ds.shape())
        .ok()
        .and_then(|s| s.first().copied())
        .unwrap_or(0);

    writeln!(out, "{:<24}{} records", "Markers", fmt_count(k)).unwrap_or(());

    if k == 0 {
        return;
    }

    if let Ok(icons) = grp.dataset("icon").and_then(|ds| ds.read_u8()) {
        let hist = icon_histogram(&icons);
        if !hist.is_empty() {
            writeln!(out, "  {:<22}{hist}", "icons").unwrap_or(());
        }
    }

    if let Ok(label_flat) = grp.dataset("label").and_then(|ds| ds.read_u8()) {
        let preview = label_preview(&label_flat);
        if !preview.is_empty() {
            writeln!(out, "  {:<22}{preview}", "labels").unwrap_or(());
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
    let mut seen = [false; 4];
    for &c in codes {
        if let Some(slot) = seen.get_mut(c as usize) {
            *slot = true;
        }
    }
    let names: &[&str] = &["GPS", "GLONASS", "Galileo", "BeiDou"];
    seen.iter()
        .enumerate()
        .filter(|&(_, &s)| s)
        .filter_map(|(i, _)| names.get(i).copied())
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
