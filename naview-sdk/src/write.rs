use hdf5_pure::{AttrValue, FileBuilder};
use uom::si::angle::degree;
use uom::si::velocity::meter_per_second;

use crate::builder::datetime_to_micros;
use crate::error::Error;
use crate::types::{Constellation, MarkerIcon, NavFile};

/// Serialise `nav_file` to HDF5 bytes.
pub(crate) fn build_hdf5(nav_file: &NavFile) -> Result<Vec<u8>, Error> {
    let mut fb = FileBuilder::new();

    // Root attributes
    fb.set_attr("naview_version", AttrValue::String("1".into()));
    if let Some(title) = &nav_file.meta.title {
        fb.set_attr("meta_title", AttrValue::String(title.clone()));
    }
    if let Some(device) = &nav_file.meta.device {
        fb.set_attr("meta_device", AttrValue::String(device.clone()));
    }
    if let Some(notes) = &nav_file.meta.notes {
        fb.set_attr("meta_notes", AttrValue::String(notes.clone()));
    }

    write_nav_points(nav_file, &mut fb);
    write_satellite_data(nav_file, &mut fb);
    write_markers(nav_file, &mut fb)?;

    Ok(fb.finish()?)
}

fn write_nav_points(nav_file: &NavFile, fb: &mut FileBuilder) {
    let points = nav_file.nav_points();
    let n = points.len();

    let times: Vec<i64> = points
        .iter()
        .map(|p| datetime_to_micros(p.fix.time))
        .collect();
    let lats: Vec<f64> = points.iter().map(|p| p.fix.lat.get::<degree>()).collect();
    let lons: Vec<f64> = points.iter().map(|p| p.fix.lon.get::<degree>()).collect();
    let headings: Vec<f64> = points
        .iter()
        .map(|p| p.fix.heading.get::<degree>())
        .collect();
    let speeds: Vec<f64> = points
        .iter()
        .map(|p| {
            p.fix
                .speed
                .map_or(f64::NAN, |v| v.get::<meter_per_second>())
        })
        .collect();

    let mut grp = fb.create_group("nav_points");

    grp.create_dataset("time")
        .with_i64_data(&times)
        .with_shape(&[n as u64])
        .set_attr(
            "units",
            AttrValue::String("microseconds since 1970-01-01T00:00:00Z".into()),
        );
    grp.create_dataset("lat")
        .with_f64_data(&lats)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("degrees".into()));
    grp.create_dataset("lon")
        .with_f64_data(&lons)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("degrees".into()));
    grp.create_dataset("heading")
        .with_f64_data(&headings)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("degrees".into()));
    grp.create_dataset("speed_mps")
        .with_f64_data(&speeds)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("m/s".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()));

    fb.add_group(grp.finish());
}

fn write_satellite_data(nav_file: &NavFile, fb: &mut FileBuilder) {
    let mut report_nav_point_idx: Vec<u64> = Vec::new();
    let mut report_times: Vec<i64> = Vec::new();

    let mut tracked_rep_idx: Vec<u64> = Vec::new();
    let mut tracked_constellation: Vec<u8> = Vec::new();
    let mut tracked_prn: Vec<u32> = Vec::new();
    let mut tracked_elevation: Vec<f32> = Vec::new();
    let mut tracked_azimuth: Vec<f32> = Vec::new();
    let mut tracked_snr: Vec<f32> = Vec::new();

    let mut fix_rep_idx: Vec<u64> = Vec::new();
    let mut fix_constellation: Vec<i8> = Vec::new();
    let mut fix_prn: Vec<u32> = Vec::new();

    let mut report_idx: u64 = 0;

    for (nav_idx, nav_point) in nav_file.nav_points().iter().enumerate() {
        let Some(report) = &nav_point.satellites else {
            continue;
        };

        report_nav_point_idx.push(nav_idx as u64);
        report_times.push(datetime_to_micros(report.time));

        for sat in &report.tracked {
            tracked_rep_idx.push(report_idx);
            tracked_constellation.push(sat.constellation.to_u8());
            tracked_prn.push(sat.prn);
            tracked_elevation.push(sat.elevation.unwrap_or(f32::NAN));
            tracked_azimuth.push(sat.azimuth.unwrap_or(f32::NAN));
            tracked_snr.push(sat.snr.unwrap_or(f32::NAN));
        }

        for entry in &report.fix {
            fix_rep_idx.push(report_idx);
            fix_constellation.push(match entry.constellation {
                None => -1i8,
                Some(c) => c.to_u8() as i8,
            });
            fix_prn.push(entry.prn);
        }

        report_idx += 1;
    }

    if report_nav_point_idx.is_empty() {
        return;
    }

    debug_assert!(
        report_nav_point_idx
            .windows(2)
            .all(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| a < b)),
        "nav_point_idx must be strictly increasing"
    );

    let r = report_nav_point_idx.len();
    let ts = tracked_rep_idx.len();
    let fs = fix_rep_idx.len();

    // sat_reports/
    let mut sat_grp = fb.create_group("sat_reports");
    sat_grp
        .create_dataset("nav_point_idx")
        .with_u64_data(&report_nav_point_idx)
        .with_shape(&[r as u64]);
    sat_grp
        .create_dataset("time")
        .with_i64_data(&report_times)
        .with_shape(&[r as u64])
        .set_attr(
            "units",
            AttrValue::String("microseconds since 1970-01-01T00:00:00Z".into()),
        );
    fb.add_group(sat_grp.finish());

    // tracked_sats/
    let mut ts_grp = fb.create_group("tracked_sats");
    ts_grp
        .create_dataset("sat_report_idx")
        .with_u64_data(&tracked_rep_idx)
        .with_shape(&[ts as u64]);
    ts_grp
        .create_dataset("constellation")
        .with_u8_data(&tracked_constellation)
        .with_shape(&[ts as u64])
        .set_attr(
            "encoding",
            AttrValue::String("0=GPS,1=GLONASS,2=Galileo,3=BeiDou".into()),
        );
    ts_grp
        .create_dataset("prn")
        .with_u32_data(&tracked_prn)
        .with_shape(&[ts as u64]);
    ts_grp
        .create_dataset("elevation")
        .with_f32_data(&tracked_elevation)
        .with_shape(&[ts as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()));
    ts_grp
        .create_dataset("azimuth")
        .with_f32_data(&tracked_azimuth)
        .with_shape(&[ts as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()));
    ts_grp
        .create_dataset("snr")
        .with_f32_data(&tracked_snr)
        .with_shape(&[ts as u64])
        .set_attr("units", AttrValue::String("dB-Hz".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()));
    fb.add_group(ts_grp.finish());

    // fix_sats/
    let mut fs_grp = fb.create_group("fix_sats");
    fs_grp
        .create_dataset("sat_report_idx")
        .with_u64_data(&fix_rep_idx)
        .with_shape(&[fs as u64]);
    fs_grp
        .create_dataset("constellation")
        .with_i8_data(&fix_constellation)
        .with_shape(&[fs as u64])
        .set_attr(
            "encoding",
            AttrValue::String("-1=unknown,0=GPS,1=GLONASS,2=Galileo,3=BeiDou".into()),
        );
    fs_grp
        .create_dataset("prn")
        .with_u32_data(&fix_prn)
        .with_shape(&[fs as u64]);
    fb.add_group(fs_grp.finish());
}

fn write_markers(nav_file: &NavFile, fb: &mut FileBuilder) -> Result<(), Error> {
    let markers = nav_file.markers();
    if markers.is_empty() {
        return Ok(());
    }

    let k = markers.len();
    let mut times: Vec<i64> = Vec::with_capacity(k);
    let mut lats: Vec<f64> = Vec::with_capacity(k);
    let mut lons: Vec<f64> = Vec::with_capacity(k);
    let mut icons: Vec<u8> = Vec::with_capacity(k);
    let mut label_flat: Vec<u8> = Vec::with_capacity(k * 256);
    let mut any_truncated = false;

    for marker in markers {
        times.push(datetime_to_micros(marker.annotation.time));
        lats.push(marker.lat.get::<degree>());
        lons.push(marker.lon.get::<degree>());
        icons.push(marker.annotation.icon.map_or(0, MarkerIcon::to_u8));

        let label_str = marker.annotation.label.as_deref().unwrap_or("");
        let truncated_str = truncate_utf8(label_str, 255, &mut any_truncated);
        let truncated_bytes = truncated_str.as_bytes();

        let mut row = [0u8; 256];
        let len = truncated_bytes.len();
        if let Some(dest) = row.get_mut(..len) {
            dest.copy_from_slice(truncated_bytes);
        }
        label_flat.extend_from_slice(&row);
    }

    let mut grp = fb.create_group("markers");
    grp.create_dataset("time")
        .with_i64_data(&times)
        .with_shape(&[k as u64])
        .set_attr(
            "units",
            AttrValue::String("microseconds since 1970-01-01T00:00:00Z".into()),
        );
    grp.create_dataset("lat")
        .with_f64_data(&lats)
        .with_shape(&[k as u64])
        .set_attr("units", AttrValue::String("degrees".into()));
    grp.create_dataset("lon")
        .with_f64_data(&lons)
        .with_shape(&[k as u64])
        .set_attr("units", AttrValue::String("degrees".into()));
    grp.create_dataset("icon")
        .with_u8_data(&icons)
        .with_shape(&[k as u64])
        .set_attr(
            "encoding",
            AttrValue::String(
                "0=pin,1=cross,2=circle,3=lightning,4=warning,5=error,6=check".into(),
            ),
        );
    grp.create_dataset("label")
        .with_u8_data(&label_flat)
        .with_shape(&[k as u64, 256])
        .set_attr(
            "encoding",
            AttrValue::String("UTF-8 null-padded to 256 bytes, max 255 content bytes".into()),
        )
        .set_attr("truncated", AttrValue::I32(i32::from(any_truncated)));
    fb.add_group(grp.finish());

    Ok(())
}

/// Truncate `s` to at most `max_bytes` bytes on a valid UTF-8 boundary.
fn truncate_utf8<'a>(s: &'a str, max_bytes: usize, truncated: &mut bool) -> &'a str {
    if s.len() <= max_bytes {
        return s;
    }
    *truncated = true;
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

pub(crate) fn decode_tracked_constellation(code: u8) -> Result<Constellation, Error> {
    Constellation::from_u8(code, "tracked_sats/constellation")
}

pub(crate) fn decode_fix_constellation(code: i8) -> Result<Option<Constellation>, Error> {
    if code == -1 {
        return Ok(None);
    }
    if code < 0 {
        return Err(Error::UnknownConstellation {
            code: i16::from(code),
            dataset: "fix_sats/constellation",
        });
    }
    Ok(Some(Constellation::from_u8(
        code.cast_unsigned(),
        "fix_sats/constellation",
    )?))
}
