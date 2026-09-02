//! The `builder` group of `geotrace.h`: adding nav fixes, satellite reports, markers and channels.

use std::ffi::c_char;

use geotrace_sdk::{
    Angle, Annotation, Channel, ChannelUnit, EventMarker, EventMarkerColor, EventMarkerStyle,
    SatelliteReport, Velocity,
};

use super::GtdFileBuilder;
use crate::error::{self, GtdStatus};
use crate::timestamp;
use crate::{GtdChannel, GtdMarkerIcon, GtdOptF64, GtdSatellite, GtdTimestamp};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_nav_fix(
    b: *mut GtdFileBuilder,
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
    lat_deg: f64,
    lon_deg: f64,
    heading_deg: GtdOptF64,
    speed_mps: GtdOptF64,
    eph_m: GtdOptF64,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        b.recorder_mut().add_nav_fix(geotrace_sdk::NavFix {
            gps_time: timestamp::ts_to_datetime(gps_time),
            sys_time: timestamp::ts_to_datetime(sys_time),
            lat: Angle::degrees(lat_deg),
            lon: Angle::degrees(lon_deg),
            heading: heading_deg.to_opt().map(Angle::degrees),
            speed: speed_mps.to_opt().map(Velocity::meter_per_second),
            eph_m: eph_m.to_opt(),
        });
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_satellite_report(
    b: *mut GtdFileBuilder,
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
    sats: *const GtdSatellite,
    n_sats: usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        if n_sats > 0 && sats.is_null() {
            error::set_last_error("sats is null but n_sats > 0");
            return GtdStatus::ErrNullArgument;
        }
        let tracked: Vec<geotrace_sdk::Satellite> = if n_sats == 0 {
            Vec::new()
        } else {
            // SAFETY: sats is non-null (checked above), `n_sats` is the element count
            let slice = unsafe { std::slice::from_raw_parts(sats, n_sats) };
            slice.iter().map(|s| s.to_sdk_satellite()).collect()
        };
        b.recorder_mut().add_satellite_report(SatelliteReport {
            gps_time: timestamp::ts_to_datetime(gps_time),
            sys_time: timestamp::ts_to_datetime(sys_time),
            tracked,
        });
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_annotation(
    b: *mut GtdFileBuilder,
    time: GtdTimestamp,
    label: *const c_char,
    icon: GtdMarkerIcon,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let Some(ann_time) = timestamp::ts_to_datetime(time) else {
            error::set_last_error("annotation time must not be gtd_ts_none()");
            return GtdStatus::ErrNullArgument;
        };
        let annotation = match Annotation::builder()
            .time(ann_time)
            .maybe_label(cstr_opt!(label).map(str::to_owned))
            .maybe_icon(icon.to_marker_icon())
            .build()
        {
            Ok(annotation) => annotation,
            Err(e) => {
                let status = error::status_for_error(&e);
                error::set_last_error(e);
                return status;
            }
        };
        b.recorder_mut().add_annotation(annotation);
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_event_marker(
    b: *mut GtdFileBuilder,
    variant_path: *const c_char,
    sys_time: GtdTimestamp,
    annotation: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let path = cstr!(variant_path);
        let ann = cstr_opt!(annotation).map(str::to_owned);
        let Some(dt) = timestamp::ts_to_datetime(sys_time) else {
            error::set_last_error("event marker sys_time must not be gtd_ts_none()");
            return GtdStatus::ErrNullArgument;
        };
        let marker = match EventMarker::builder()
            .variant_path(path)
            .sys_time(dt)
            .maybe_annotation(ann)
            .build()
        {
            Ok(m) => m,
            Err(e) => {
                let status = error::status_for_event_marker_error(&e);
                error::set_last_error(e);
                return status;
            }
        };
        b.recorder_mut().add_event_marker(marker);
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_event_marker_style(
    b: *mut GtdFileBuilder,
    variant_path: *const c_char,
    icon: GtdMarkerIcon,
    color_hex: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let path = cstr!(variant_path);
        let icon_choice = icon.to_icon_choice();
        let color = match cstr_opt!(color_hex) {
            Some(hex) => EventMarkerColor::Hex(hex.to_owned()),
            None => EventMarkerColor::Auto,
        };
        b.recorder_mut().add_event_marker_style(EventMarkerStyle {
            variant_path: path.to_owned(),
            icon: icon_choice,
            color,
        });
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_channel(
    b: *mut GtdFileBuilder,
    channel: *const GtdChannel,
) -> GtdStatus {
    // SAFETY: this forwards the caller's pointers unchanged.
    unsafe { gtd_builder_add_channel_with_unit_mode(b, channel, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_channel_with_unit_mode(
    b: *mut GtdFileBuilder,
    channel: *const GtdChannel,
    unit_mode: u32,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let ch = nonnull_ref!(channel);

        let name = cstr!(ch.name);
        let unit_label = cstr_opt!(ch.unit);
        let unit = match parse_channel_unit(unit_label, unit_mode) {
            Ok(unit) => unit,
            Err(status) => return status,
        };
        let description = cstr_opt!(ch.description).map(str::to_owned);

        // A null or empty component list is a scalar channel.
        let components = if ch.n_components == 0 {
            None
        } else if ch.components.is_null() {
            error::set_last_error("components is null but n_components > 0");
            return GtdStatus::ErrNullArgument;
        } else {
            // SAFETY: components is non-null with `n_components` elements (checked).
            let ptrs = unsafe { std::slice::from_raw_parts(ch.components, ch.n_components) };
            let mut labels = Vec::with_capacity(ch.n_components);
            for &ptr in ptrs {
                if ptr.is_null() {
                    error::set_last_error("a component label pointer is null");
                    return GtdStatus::ErrNullArgument;
                }
                // SAFETY: `ptr` is a non-null C string (checked).
                match unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str() {
                    Ok(label) => labels.push(label.to_owned()),
                    Err(_) => {
                        error::set_last_error("a component label is not valid UTF-8");
                        return GtdStatus::ErrUtf8;
                    }
                }
            }
            Some(labels)
        };

        if ch.n_times > 0 && ch.times.is_null() {
            error::set_last_error("times is null but n_times > 0");
            return GtdStatus::ErrNullArgument;
        }
        let time_slice = if ch.n_times == 0 {
            &[][..]
        } else {
            // SAFETY: times is non-null with `n_times` elements (checked above).
            unsafe { std::slice::from_raw_parts(ch.times, ch.n_times) }
        };
        let mut times = Vec::with_capacity(ch.n_times);
        for ts in time_slice {
            let Some(dt) = timestamp::ts_to_datetime(*ts) else {
                error::set_last_error("channel timestamps must not be gtd_ts_none()");
                return GtdStatus::ErrNullArgument;
            };
            times.push(dt);
        }

        if ch.n_values > 0 && ch.values.is_null() {
            error::set_last_error("values is null but n_values > 0");
            return GtdStatus::ErrNullArgument;
        }
        let values = if ch.n_values == 0 {
            Vec::new()
        } else {
            // SAFETY: values is non-null with `n_values` elements (checked above).
            unsafe { std::slice::from_raw_parts(ch.values, ch.n_values) }.to_vec()
        };

        let built = Channel::builder()
            .name(name)
            .maybe_unit(unit)
            .maybe_period(ch.period_deg.to_opt().map(Angle::degrees))
            .maybe_description(description)
            .maybe_components(components)
            .times(times)
            .values(values)
            .build();
        match built {
            Ok(built) => {
                b.recorder_mut().add_channel(built);
                GtdStatus::Ok
            }
            Err(e) => {
                error::set_last_error(e);
                GtdStatus::ErrInvalidChannel
            }
        }
    })
}

fn parse_channel_unit(
    label: Option<&str>,
    unit_mode: u32,
) -> Result<Option<ChannelUnit>, GtdStatus> {
    let Some(label) = label else {
        return Ok(None);
    };
    let parsed = match unit_mode {
        0 => label.parse(),
        1 => ChannelUnit::custom(label),
        _ => {
            error::set_last_error("unit_mode is not a valid GtdChannelUnitMode");
            return Err(GtdStatus::ErrInvalidChannel);
        }
    };
    parsed.map(Some).map_err(|error| {
        error::set_last_error(error);
        GtdStatus::ErrInvalidChannel
    })
}
