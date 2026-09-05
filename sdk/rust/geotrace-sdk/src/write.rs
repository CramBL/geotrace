use chrono::SecondsFormat;
use hdf5_pure::{AttrValue, FileBuilder};

use crate::builder::{datetime_to_micros, opt_datetime_to_u64};
use crate::error::{Error, FieldLocation, MARKER_LABEL_LOCATION};
use crate::fixed_width_string::{
    AnnotationField, ColorHexField, FixedWidthString, FixedWidthStringError, IconNameField,
    MarkerLabelField, VariantPathField,
};
use crate::provenance::{SDK_COMMIT_TIME_ATTR, SDK_GIT_COMMIT_ATTR, SDK_VERSION_ATTR};
use crate::types::{Constellation, EventMarkerColor, MarkerIcon, Meta, NavFile};

/// Number of elements per chunk for 1-D compressed datasets.
///
/// 8 192 elements gives ≥ 8 KiB per chunk across all element sizes used here
/// (1 B for u8, 4 B for f32/u32, 8 B for f64/i64/u64), which is enough for
/// deflate to compress effectively.
const CHUNK_SIZE: u64 = 8_192;

/// The `units` attribute value shared by every microsecond timestamp dataset.
const MICROS_SINCE_EPOCH_UNITS: &str = "microseconds since 1970-01-01T00:00:00Z";

/// Serialise `nav_file` to HDF5 bytes.
pub(crate) fn build_hdf5(nav_file: &NavFile) -> Result<Vec<u8>, Error> {
    let mut fb = FileBuilder::new();

    // Root attributes
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    set_provenance_attrs(&mut fb, &nav_file.meta);
    if let Some(title) = &nav_file.meta.title {
        fb.set_attr("meta_title", AttrValue::String(title.clone()));
    }
    if let Some(device) = &nav_file.meta.device {
        fb.set_attr("meta_device", AttrValue::String(device.clone()));
    }
    if let Some(notes) = &nav_file.meta.notes {
        fb.set_attr("meta_notes", AttrValue::String(notes.clone()));
    }
    if let Some(identity) = &nav_file.meta.identity {
        fb.set_attr("meta_identity", AttrValue::String(identity.clone()));
    }
    if let Some(travel_mode) = &nav_file.meta.travel_mode {
        fb.set_attr(
            "meta_travel_mode",
            AttrValue::String(travel_mode.name().to_owned()),
        );
    }

    write_nav_points(nav_file, &mut fb);
    write_satellite_data(nav_file, &mut fb);
    write_markers(nav_file, &mut fb)?;
    write_event_markers(nav_file, &mut fb)?;
    write_channels(nav_file, &mut fb);

    Ok(fb.finish()?)
}

/// A written file holds the same build stamp as the [`NavFile`] it was written
/// from: [`NavRecorder::finish`](crate::NavRecorder::finish) stamps `meta`
/// before this runs.
fn set_provenance_attrs(fb: &mut FileBuilder, meta: &Meta) {
    if let Some(version) = &meta.sdk_version {
        fb.set_attr(SDK_VERSION_ATTR, AttrValue::String(version.clone()));
    }
    if let Some(commit) = &meta.sdk_git_commit {
        fb.set_attr(SDK_GIT_COMMIT_ATTR, AttrValue::String(commit.clone()));
    }
    if let Some(commit_time) = meta.sdk_commit_time {
        fb.set_attr(
            SDK_COMMIT_TIME_ATTR,
            AttrValue::String(commit_time.to_rfc3339_opts(SecondsFormat::Secs, true)),
        );
    }
}

/// Write ad-hoc channels as `channels/<name>/{time,value}`, with the channel's
/// unit, wrap period, description, and (for vector channels) component labels as
/// attributes on its group.
fn write_channels(nav_file: &NavFile, fb: &mut FileBuilder) {
    let channels = nav_file.channels();
    if channels.is_empty() {
        return;
    }

    let chunk = CHUNK_SIZE.max(1);
    let mut root = fb.create_group("channels");
    for channel in channels {
        let n = channel.times.len() as u64;
        let times: Vec<i64> = channel
            .times
            .iter()
            .copied()
            .map(datetime_to_micros)
            .collect();

        let mut grp = root.create_group(&channel.name);
        if let Some(unit) = &channel.unit {
            grp.set_attr("unit", AttrValue::String(unit.to_string()));
        }
        if let Some(period) = channel.period {
            grp.set_attr("period_deg", AttrValue::F64(period.as_degrees()));
        }
        if let Some(description) = &channel.description {
            grp.set_attr("description", AttrValue::String(description.clone()));
        }
        if channel.is_vector() {
            grp.set_attr(
                "components",
                AttrValue::StringArray(channel.components.clone()),
            );
        }
        grp.create_dataset("time")
            .with_i64_data(&times)
            .with_shape(&[n])
            .set_attr(
                "units",
                AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
            )
            .with_chunks(&[chunk])
            .with_shuffle()
            .with_deflate(6);
        // A scalar channel is a 1-D `[n]` dataset. A vector channel is a 2-D
        // `[n, k]` dataset, so the components stay clock-locked in one column each.
        let value = grp.create_dataset("value").with_f64_data(&channel.values);
        if channel.is_vector() {
            let k = channel.component_count() as u64;
            // Keep ~CHUNK_SIZE elements per chunk by dividing the row count by
            // the column count, mirroring how the string datasets chunk by row.
            value
                .with_shape(&[n, k])
                .with_chunks(&[(chunk / k).max(1), k]);
        } else {
            value.with_shape(&[n]).with_chunks(&[chunk]);
        }
        value.with_shuffle().with_deflate(6);
        root.add_group(grp.finish());
    }
    fb.add_group(root.finish());
}

fn write_nav_points(nav_file: &NavFile, fb: &mut FileBuilder) {
    let points = nav_file.nav_points();
    let n = points.len();

    let times: Vec<i64> = points
        .iter()
        .map(|p| datetime_to_micros(p.fix.effective_gps_time()))
        .collect();
    let gps_times: Vec<u64> = points
        .iter()
        .map(|p| opt_datetime_to_u64(p.fix.gps_time()))
        .collect();
    let sys_times: Vec<u64> = points
        .iter()
        .map(|p| opt_datetime_to_u64(p.fix.sys_time()))
        .collect();
    let lats: Vec<f64> = points.iter().map(|p| p.fix.lat.as_degrees()).collect();
    let lons: Vec<f64> = points.iter().map(|p| p.fix.lon.as_degrees()).collect();
    let headings: Vec<f64> = points
        .iter()
        .map(|p| p.fix.heading.map_or(f64::NAN, |h| h.as_degrees()))
        .collect();
    let speeds: Vec<f64> = points
        .iter()
        .map(|p| p.fix.speed.map_or(f64::NAN, |v| v.as_meters_per_second()))
        .collect();
    let ephs: Vec<f64> = points
        .iter()
        .map(|p| p.fix.eph_m.unwrap_or(f64::NAN))
        .collect();

    let mut grp = fb.create_group("nav_points");

    let chunk = CHUNK_SIZE;

    grp.create_dataset("time")
        .with_i64_data(&times)
        .with_shape(&[n as u64])
        .set_attr(
            "units",
            AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
        )
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("gps_time_us")
        .with_u64_data(&gps_times)
        .with_shape(&[n as u64])
        .set_attr(
            "units",
            AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
        )
        .set_attr("absent_sentinel", AttrValue::String("u64::MAX".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("sys_time_us")
        .with_u64_data(&sys_times)
        .with_shape(&[n as u64])
        .set_attr(
            "units",
            AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
        )
        .set_attr("absent_sentinel", AttrValue::String("u64::MAX".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("lat")
        .with_f64_data(&lats)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("lon")
        .with_f64_data(&lons)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("heading")
        .with_f64_data(&headings)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("speed_mps")
        .with_f64_data(&speeds)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("m/s".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("eph_m")
        .with_f64_data(&ephs)
        .with_shape(&[n as u64])
        .set_attr("units", AttrValue::String("metres".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()))
        .with_chunks(&[chunk])
        .with_shuffle()
        .with_deflate(6);

    fb.add_group(grp.finish());
}

fn write_satellite_data(nav_file: &NavFile, fb: &mut FileBuilder) {
    let mut report_nav_point_idx: Vec<u64> = Vec::new();
    let mut report_gps_times: Vec<u64> = Vec::new();
    let mut report_sys_times: Vec<u64> = Vec::new();

    let mut tracked_rep_idx: Vec<u64> = Vec::new();
    let mut tracked_constellation: Vec<u8> = Vec::new();
    let mut tracked_prn: Vec<u32> = Vec::new();
    let mut tracked_in_fix: Vec<u8> = Vec::new();
    let mut tracked_elevation: Vec<f32> = Vec::new();
    let mut tracked_azimuth: Vec<f32> = Vec::new();
    let mut tracked_snr: Vec<f32> = Vec::new();

    let mut report_idx: u64 = 0;

    for (nav_idx, nav_point) in nav_file.nav_points().iter().enumerate() {
        let Some(report) = &nav_point.satellites else {
            continue;
        };

        report_nav_point_idx.push(nav_idx as u64);
        report_gps_times.push(opt_datetime_to_u64(report.gps_time()));
        report_sys_times.push(opt_datetime_to_u64(report.sys_time()));

        for sat in &report.tracked {
            tracked_rep_idx.push(report_idx);
            tracked_constellation.push(sat.constellation.to_u8());
            tracked_prn.push(sat.prn);
            tracked_in_fix.push(u8::from(sat.in_fix));
            tracked_elevation.push(sat.elevation.unwrap_or(f32::NAN));
            tracked_azimuth.push(sat.azimuth.unwrap_or(f32::NAN));
            tracked_snr.push(sat.snr.unwrap_or(f32::NAN));
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

    let r_chunk = CHUNK_SIZE.max(1);
    let ts_chunk = CHUNK_SIZE.max(1);

    // sat_reports/
    let mut sat_grp = fb.create_group("sat_reports");
    sat_grp
        .create_dataset("nav_point_idx")
        .with_u64_data(&report_nav_point_idx)
        .with_shape(&[r as u64])
        .with_chunks(&[r_chunk])
        .with_shuffle()
        .with_deflate(6);
    sat_grp
        .create_dataset("gps_time_us")
        .with_u64_data(&report_gps_times)
        .with_shape(&[r as u64])
        .set_attr(
            "units",
            AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
        )
        .set_attr("absent_sentinel", AttrValue::String("u64::MAX".into()))
        .with_chunks(&[r_chunk])
        .with_shuffle()
        .with_deflate(6);
    sat_grp
        .create_dataset("sys_time_us")
        .with_u64_data(&report_sys_times)
        .with_shape(&[r as u64])
        .set_attr(
            "units",
            AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
        )
        .set_attr("absent_sentinel", AttrValue::String("u64::MAX".into()))
        .with_chunks(&[r_chunk])
        .with_shuffle()
        .with_deflate(6);
    fb.add_group(sat_grp.finish());

    // tracked_sats/
    let mut ts_grp = fb.create_group("tracked_sats");
    ts_grp
        .create_dataset("sat_report_idx")
        .with_u64_data(&tracked_rep_idx)
        .with_shape(&[ts as u64])
        .with_chunks(&[ts_chunk])
        .with_shuffle()
        .with_deflate(6);
    ts_grp
        .create_dataset("constellation")
        .with_u8_data(&tracked_constellation)
        .with_shape(&[ts as u64])
        .set_attr(
            "encoding",
            AttrValue::String("0=GPS,1=GLONASS,2=Galileo,3=BeiDou".into()),
        )
        .with_chunks(&[ts_chunk])
        .with_deflate(6);
    ts_grp
        .create_dataset("prn")
        .with_u32_data(&tracked_prn)
        .with_shape(&[ts as u64])
        .with_chunks(&[ts_chunk])
        .with_shuffle()
        .with_deflate(6);
    ts_grp
        .create_dataset("in_fix")
        .with_u8_data(&tracked_in_fix)
        .with_shape(&[ts as u64])
        .set_attr(
            "encoding",
            AttrValue::String("0=not_in_fix,1=in_fix".into()),
        )
        .with_chunks(&[ts_chunk])
        .with_deflate(6);
    ts_grp
        .create_dataset("elevation")
        .with_f32_data(&tracked_elevation)
        .with_shape(&[ts as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()))
        .with_chunks(&[ts_chunk])
        .with_shuffle()
        .with_deflate(6);
    ts_grp
        .create_dataset("azimuth")
        .with_f32_data(&tracked_azimuth)
        .with_shape(&[ts as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()))
        .with_chunks(&[ts_chunk])
        .with_shuffle()
        .with_deflate(6);
    ts_grp
        .create_dataset("snr")
        .with_f32_data(&tracked_snr)
        .with_shape(&[ts as u64])
        .set_attr("units", AttrValue::String("dB-Hz".into()))
        .set_attr("nan_means", AttrValue::String("absent".into()))
        .with_chunks(&[ts_chunk])
        .with_shuffle()
        .with_deflate(6);
    fb.add_group(ts_grp.finish());
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

    for marker in markers {
        times.push(datetime_to_micros(marker.annotation.time));
        lats.push(marker.lat.as_degrees());
        lons.push(marker.lon.as_degrees());
        icons.push(marker.annotation.icon.map_or(0, MarkerIcon::to_u8));

        label_flat.extend_from_slice(&encode_field_row(
            MARKER_LABEL_LOCATION,
            MarkerLabelField::new(marker.annotation.label.as_deref().unwrap_or("")),
        )?);
    }

    let k_chunk = CHUNK_SIZE.max(1);

    let mut grp = fb.create_group("markers");
    grp.create_dataset("time")
        .with_i64_data(&times)
        .with_shape(&[k as u64])
        .set_attr(
            "units",
            AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
        )
        .with_chunks(&[k_chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("lat")
        .with_f64_data(&lats)
        .with_shape(&[k as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .with_chunks(&[k_chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("lon")
        .with_f64_data(&lons)
        .with_shape(&[k as u64])
        .set_attr("units", AttrValue::String("degrees".into()))
        .with_chunks(&[k_chunk])
        .with_shuffle()
        .with_deflate(6);
    grp.create_dataset("icon")
        .with_u8_data(&icons)
        .with_shape(&[k as u64])
        .set_attr(
            "encoding",
            AttrValue::String(
                "0=pin,1=cross,2=circle,3=lightning,4=warning,5=error,6=check".into(),
            ),
        )
        .with_chunks(&[k_chunk])
        .with_deflate(6);
    // Label rows are 256 B each. Chunk by 32 rows to stay at ~8 KiB per chunk.
    let label_row_chunk = (CHUNK_SIZE / 256).max(1);
    grp.create_dataset("label")
        .with_u8_data(&label_flat)
        .with_shape(&[k as u64, 256])
        .set_attr(
            "encoding",
            AttrValue::String("UTF-8 null-padded to 256 bytes, max 255 content bytes".into()),
        )
        // Chunk by rows. Shuffle is a no-op for u8 (1-byte elements) so omit it.
        .with_chunks(&[label_row_chunk, 256])
        .with_deflate(6);
    fb.add_group(grp.finish());

    Ok(())
}

fn encode_field_row<const ROW_BYTES: usize>(
    location: FieldLocation,
    field: Result<FixedWidthString<ROW_BYTES>, FixedWidthStringError>,
) -> Result<[u8; ROW_BYTES], Error> {
    field
        .map(|field| field.encode_row())
        .map_err(|source| Error::unwritable_field(location, source))
}

fn write_event_markers(nav_file: &NavFile, fb: &mut FileBuilder) -> Result<(), Error> {
    let em = nav_file.event_markers();
    let styles = nav_file.event_marker_styles();

    if !em.is_empty() {
        let n = em.len();
        let mut sys_times: Vec<u64> = Vec::with_capacity(n);
        let mut lats: Vec<f64> = Vec::with_capacity(n);
        let mut lons: Vec<f64> = Vec::with_capacity(n);
        let mut vp_flat: Vec<u8> = Vec::with_capacity(n * 256);
        let mut ann_flat: Vec<u8> = Vec::with_capacity(n * 512);

        for m in em {
            sys_times.push(m.sys_time.timestamp_micros().cast_unsigned());
            lats.push(m.lat.as_degrees());
            lons.push(m.lon.as_degrees());

            vp_flat.extend_from_slice(&encode_field_row(
                FieldLocation {
                    group: "event_markers",
                    dataset: "variant_path",
                },
                VariantPathField::new(&m.variant_path),
            )?);

            ann_flat.extend_from_slice(&encode_field_row(
                FieldLocation {
                    group: "event_markers",
                    dataset: "annotation",
                },
                AnnotationField::new(m.annotation.as_deref().unwrap_or("")),
            )?);
        }

        let n_chunk = CHUNK_SIZE.max(1);
        let vp_row_chunk = (CHUNK_SIZE / 256).max(1);
        let ann_row_chunk = (CHUNK_SIZE / 512).max(1);

        let mut grp = fb.create_group("event_markers");
        grp.create_dataset("sys_time_us")
            .with_u64_data(&sys_times)
            .with_shape(&[n as u64])
            .set_attr(
                "units",
                AttrValue::String(MICROS_SINCE_EPOCH_UNITS.to_owned()),
            )
            .set_attr("absent_sentinel", AttrValue::String("u64::MAX".into()))
            .with_chunks(&[n_chunk])
            .with_shuffle()
            .with_deflate(6);
        grp.create_dataset("lat")
            .with_f64_data(&lats)
            .with_shape(&[n as u64])
            .set_attr("units", AttrValue::String("degrees".into()))
            .with_chunks(&[n_chunk])
            .with_shuffle()
            .with_deflate(6);
        grp.create_dataset("lon")
            .with_f64_data(&lons)
            .with_shape(&[n as u64])
            .set_attr("units", AttrValue::String("degrees".into()))
            .with_chunks(&[n_chunk])
            .with_shuffle()
            .with_deflate(6);
        // `variant_path`: 256 B rows → chunk at CHUNK_SIZE/256 rows
        grp.create_dataset("variant_path")
            .with_u8_data(&vp_flat)
            .with_shape(&[n as u64, 256])
            .set_attr(
                "encoding",
                AttrValue::String("UTF-8 null-padded to 256 bytes, max 255 content bytes".into()),
            )
            .with_chunks(&[vp_row_chunk, 256])
            .with_deflate(6);
        // annotation: 512 B rows → chunk at CHUNK_SIZE/512 rows
        grp.create_dataset("annotation")
            .with_u8_data(&ann_flat)
            .with_shape(&[n as u64, 512])
            .set_attr(
                "encoding",
                AttrValue::String("UTF-8 null-padded to 512 bytes, empty = no annotation".into()),
            )
            .with_chunks(&[ann_row_chunk, 512])
            .with_deflate(6);
        fb.add_group(grp.finish());
    }

    if !styles.is_empty() {
        let m = styles.len();
        let mut vp_flat: Vec<u8> = Vec::with_capacity(m * 256);
        let mut icon_flat: Vec<u8> = Vec::with_capacity(m * 32);
        let mut color_flat: Vec<u8> = Vec::with_capacity(m * 8);

        for s in styles {
            vp_flat.extend_from_slice(&encode_field_row(
                FieldLocation {
                    group: "event_marker_styles",
                    dataset: "variant_path",
                },
                VariantPathField::new(&s.variant_path),
            )?);

            icon_flat.extend_from_slice(&encode_field_row(
                FieldLocation {
                    group: "event_marker_styles",
                    dataset: "icon_name",
                },
                IconNameField::new(s.icon.wire_name()),
            )?);

            let color_hex = match &s.color {
                EventMarkerColor::Auto => "",
                EventMarkerColor::Hex(hex) | EventMarkerColor::Unrecognized(hex) => hex.as_str(),
            };
            color_flat.extend_from_slice(&encode_field_row(
                FieldLocation {
                    group: "event_marker_styles",
                    dataset: "color_hex",
                },
                ColorHexField::new(color_hex),
            )?);
        }

        let m_chunk = (CHUNK_SIZE / 256).max(1);
        let icon_chunk = (CHUNK_SIZE / 32).max(1);
        let color_chunk = (CHUNK_SIZE / 8).max(1);

        let mut grp = fb.create_group("event_marker_styles");
        grp.create_dataset("variant_path")
            .with_u8_data(&vp_flat)
            .with_shape(&[m as u64, 256])
            .with_chunks(&[m_chunk, 256])
            .with_deflate(6);
        grp.create_dataset("icon_name")
            .with_u8_data(&icon_flat)
            .with_shape(&[m as u64, 32])
            .set_attr(
                "encoding",
                AttrValue::String("lowercase MarkerIcon variant name, null-padded".into()),
            )
            .with_chunks(&[icon_chunk, 32])
            .with_deflate(6);
        grp.create_dataset("color_hex")
            .with_u8_data(&color_flat)
            .with_shape(&[m as u64, 8])
            .set_attr(
                "encoding",
                AttrValue::String("#RRGGBB null-padded to 8 bytes".into()),
            )
            .with_chunks(&[color_chunk, 8])
            .with_deflate(6);
        fb.add_group(grp.finish());
    }

    Ok(())
}

pub(crate) fn decode_tracked_constellation(code: u8) -> Result<Constellation, Error> {
    Constellation::from_u8(code, "tracked_sats/constellation")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{DateTime, Utc};
    use hdf5_pure::File;

    use super::*;
    use crate::provenance::SCRUBBED_SDK_VERSION;

    fn attrs_of_a_file_with(meta: &Meta) -> HashMap<String, AttrValue> {
        let mut fb = FileBuilder::new();
        set_provenance_attrs(&mut fb, meta);
        let file = File::from_bytes(fb.finish().expect("build")).expect("read");
        file.root().attrs().expect("attrs")
    }

    #[test]
    fn a_stamp_with_a_commit_writes_the_commit_and_its_time() {
        let attrs = attrs_of_a_file_with(&Meta {
            sdk_version: Some("0.4.2".to_owned()),
            sdk_git_commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            sdk_commit_time: Some(
                "2026-02-01T15:00:00Z"
                    .parse::<DateTime<Utc>>()
                    .expect("a valid RFC 3339 timestamp"),
            ),
            ..Meta::default()
        });

        assert_eq!(
            attrs.get(SDK_GIT_COMMIT_ATTR).and_then(AttrValue::as_str),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(
            attrs.get(SDK_COMMIT_TIME_ATTR).and_then(AttrValue::as_str),
            Some("2026-02-01T15:00:00Z")
        );
        assert_eq!(
            attrs.get(SDK_VERSION_ATTR).and_then(AttrValue::as_str),
            Some("0.4.2")
        );
    }

    #[test]
    fn a_scrubbed_stamp_writes_the_placeholder_version_and_no_commit() {
        let mut meta = Meta::default();
        meta.stamp_scrubbed_provenance();

        let attrs = attrs_of_a_file_with(&meta);

        assert_eq!(
            attrs.get(SDK_VERSION_ATTR).and_then(AttrValue::as_str),
            Some(SCRUBBED_SDK_VERSION)
        );
        assert!(!attrs.contains_key(SDK_GIT_COMMIT_ATTR));
        assert!(!attrs.contains_key(SDK_COMMIT_TIME_ATTR));
    }

    #[test]
    fn a_file_with_no_stamp_writes_no_provenance_attribute() {
        let attrs = attrs_of_a_file_with(&Meta::default());

        assert!(!attrs.contains_key(SDK_VERSION_ATTR));
        assert!(!attrs.contains_key(SDK_GIT_COMMIT_ATTR));
        assert!(!attrs.contains_key(SDK_COMMIT_TIME_ATTR));
    }
}
