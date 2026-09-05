//! Adding nav fixes, satellite reports, markers and channels to a builder.

use std::ffi::c_char;

use geotrace_sdk::{
    Angle, Annotation, Channel, ChannelUnit, EventMarker, EventMarkerColor, EventMarkerStyle,
    NavFixTime, RecordedFixTimestamps, SatelliteReport, Velocity,
};

use super::GtdFileBuilder;
use crate::error::{self, GtdStatus};
use crate::timestamp;
use crate::{GtdChannel, GtdChannelUnitMode, GtdMarkerIcon, GtdOptF64, GtdSatellite, GtdTimestamp};

/// The two timestamp arguments of one `gtd_builder_add_*` call. Each clock has
/// its own field, so a call site cannot transpose the two.
struct TimestampArguments {
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
}

fn nav_fix_time_or_invalid_argument(
    TimestampArguments { gps_time, sys_time }: TimestampArguments,
    what_needs_a_timestamp: &str,
) -> Result<NavFixTime, GtdStatus> {
    let recorded = RecordedFixTimestamps {
        gps: timestamp::ts_to_datetime(gps_time),
        sys: timestamp::ts_to_datetime(sys_time),
    };
    NavFixTime::from_recorded(recorded).ok_or_else(|| {
        error::set_last_error(format!(
            "gps_time and sys_time are both gtd_ts_none(): {what_needs_a_timestamp} needs one"
        ));
        GtdStatus::GTD_ERR_INVALID_ARGUMENT
    })
}

/// Add a GPS navigation fix.
///
/// At least one nav fix is required before `gtd_builder_finish()`.
/// Fixes must be added in ascending time order.
///
/// The ranges named below are data quality expectations, not parse rules.
/// The SDK records a value outside its range, NaN included, as given: a recorder
/// that captured bad data must be able to write it.
/// A NaN @p heading_deg, @p speed_mps or @p eph_m reads back as absent: the SDK
/// stores `GTD_NONE_F64` as NaN.
///
/// @param builder     Builder handle.
/// @param gps_time    GPS time of the fix. Use `gtd_ts_none()` when unavailable.
/// @param sys_time    System (wall-clock) time. Use `gtd_ts_none()` when unavailable.
/// @param lat_deg     WGS-84 latitude in degrees, expected in [-90, 90].
/// @param lon_deg     WGS-84 longitude in degrees, expected in [-180, 180].
/// @param heading_deg Compass heading in degrees, expected in [0, 360), or `GTD_NONE_F64`.
/// @param speed_mps   Ground speed in m/s, expected to be non-negative, or `GTD_NONE_F64`.
/// @param eph_m       Estimated horizontal position error in metres, expected to be
///                    non-negative, or `GTD_NONE_F64`.
///
/// @return `GTD_ERR_INVALID_ARGUMENT` if @p gps_time and @p sys_time are both
///         `gtd_ts_none()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_nav_fix(
    builder: *mut GtdFileBuilder,
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
    lat_deg: f64,
    lon_deg: f64,
    heading_deg: GtdOptF64,
    speed_mps: GtdOptF64,
    eph_m: GtdOptF64,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let time = match nav_fix_time_or_invalid_argument(
            TimestampArguments { gps_time, sys_time },
            "a fix",
        ) {
            Ok(time) => time,
            Err(status) => return status,
        };
        builder.recorder_mut().add_nav_fix(geotrace_sdk::NavFix {
            time,
            lat: Angle::degrees(lat_deg),
            lon: Angle::degrees(lon_deg),
            heading: heading_deg.to_opt().map(Angle::degrees),
            speed: speed_mps.to_opt().map(Velocity::meter_per_second),
            eph_m: eph_m.to_opt(),
        });
        GtdStatus::GTD_OK
    })
}

/// Add a satellite visibility report.
///
/// The report is associated with the nearest preceding nav fix.
/// Passing @p n_sats as zero with a NULL @p sats pointer records an empty report.
///
/// @param builder  Builder handle.
/// @param gps_time GPS time of the report. Use `gtd_ts_none()` when unavailable.
/// @param sys_time System (wall-clock) time of the report.
/// @param sats     Array of @p n_sats satellite entries.
/// @param n_sats   Number of elements in @p sats.
///
/// @return `GTD_ERR_INVALID_ARGUMENT` if @p gps_time and @p sys_time are both
///         `gtd_ts_none()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_satellite_report(
    builder: *mut GtdFileBuilder,
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
    sats: *const GtdSatellite,
    n_sats: usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        if n_sats > 0 && sats.is_null() {
            error::set_last_error("sats is null but n_sats > 0");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        }
        let time = match nav_fix_time_or_invalid_argument(
            TimestampArguments { gps_time, sys_time },
            "a satellite report",
        ) {
            Ok(time) => time,
            Err(status) => return status,
        };
        let tracked: Vec<geotrace_sdk::Satellite> = if n_sats == 0 {
            Vec::new()
        } else {
            // SAFETY: sats is non-null (checked above), `n_sats` is the element count
            let slice = unsafe { std::slice::from_raw_parts(sats, n_sats) };
            slice.iter().map(|s| s.to_sdk_satellite()).collect()
        };
        builder
            .recorder_mut()
            .add_satellite_report(SatelliteReport { time, tracked });
        GtdStatus::GTD_OK
    })
}

/// Add a legacy map-pin annotation (optional label + icon).
///
/// @p time must lie within the nav fix time range unless lenient mode is enabled.
///
/// @param builder Builder handle.
/// @param time    Timestamp of the annotation. Must not be `gtd_ts_none()`.
/// @param label   Human-readable label, or NULL for no label.
/// @param icon    Icon to display. `GTD_ICON_AUTO` uses the application default (Pin).
///
/// @return `GTD_ERR_FIELD_TOO_LONG` if @p label is longer than 255 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_annotation(
    builder: *mut GtdFileBuilder,
    time: GtdTimestamp,
    label: *const c_char,
    icon: GtdMarkerIcon,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let Some(ann_time) = timestamp::ts_to_datetime(time) else {
            error::set_last_error("annotation time must not be gtd_ts_none()");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
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
        builder.recorder_mut().add_annotation(annotation);
        GtdStatus::GTD_OK
    })
}

/// Add a structured event marker.
///
/// Event markers use a hierarchical variant path (e.g. `"system/startup"`) to
/// identify the event type. Paths must be non-empty, consist of alphanumeric
/// segments separated by `/`, and not exceed 255 bytes.
///
/// @param builder      Builder handle.
/// @param variant_path Hierarchical event type path.
/// @param sys_time     Time of the event. Must not be `gtd_ts_none()`.
/// @param annotation   Optional human-readable text. Pass NULL for none.
///
/// @return `GTD_ERR_INVALID_PATH` if @p variant_path is malformed.
/// @return `GTD_ERR_FIELD_TOO_LONG` if @p variant_path is longer than 255 bytes,
///         or @p annotation longer than 511 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_event_marker(
    builder: *mut GtdFileBuilder,
    variant_path: *const c_char,
    sys_time: GtdTimestamp,
    annotation: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let path = cstr!(variant_path);
        let ann = cstr_opt!(annotation).map(str::to_owned);
        let Some(dt) = timestamp::ts_to_datetime(sys_time) else {
            error::set_last_error("event marker sys_time must not be gtd_ts_none()");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
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
        builder.recorder_mut().add_event_marker(marker);
        GtdStatus::GTD_OK
    })
}

/// Register a display style for an event marker variant.
///
/// Styles are per-variant, not per-event. Calling this multiple times for the
/// same path overwrites the previous style.
///
/// @param builder      Builder handle.
/// @param variant_path Hierarchical event type path (same format as in
///                     `gtd_builder_add_event_marker()`).
/// @param icon         Icon to display. `GTD_ICON_AUTO` uses the application default.
/// @param color_hex    Color as an `"#RRGGBB"` string, or NULL for automatic.
///
/// @note The style is checked when the file is written: a @p variant_path past
///       255 bytes or a @p color_hex past 7 bytes fails there with
///       `GTD_ERR_FIELD_TOO_LONG`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_event_marker_style(
    builder: *mut GtdFileBuilder,
    variant_path: *const c_char,
    icon: GtdMarkerIcon,
    color_hex: *const c_char,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let path = cstr!(variant_path);
        let icon_choice = icon.to_icon_choice();
        let color = match cstr_opt!(color_hex) {
            Some(hex) => EventMarkerColor::Hex(hex.to_owned()),
            None => EventMarkerColor::Auto,
        };
        builder
            .recorder_mut()
            .add_event_marker_style(EventMarkerStyle {
                variant_path: path.to_owned(),
                icon: icon_choice,
                color,
            });
        GtdStatus::GTD_OK
    })
}

/// Add a scalar or vector sensor channel.
///
/// The channel keeps its own sample timestamps. It is correlated with the nav
/// track by time at query time, not resampled here. See @ref GtdChannel for the
/// field layout, including the row-major `values` convention.
///
/// @param builder Builder handle.
/// @param channel Channel description. Not retained after the call returns.
///
/// @return `GTD_ERR_INVALID_CHANNEL` if the unit is unrecognized, the name or a
///         component label is malformed, or `values` is not
///         `n_times * max(n_components, 1)` long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_channel(
    builder: *mut GtdFileBuilder,
    channel: *const GtdChannel,
) -> GtdStatus {
    // SAFETY: this forwards the caller's pointers unchanged.
    unsafe { gtd_builder_add_channel_with_unit_mode(builder, channel, 0) }
}

/// Add a channel with an explicit recognized/custom interpretation for its unit.
///
/// This entry point preserves the layout of @ref GtdChannel while allowing a
/// display-only custom label through @ref GTD_CHANNEL_UNIT_CUSTOM.
/// `gtd_builder_add_channel()` is this call with
/// @ref GTD_CHANNEL_UNIT_RECOGNIZED. The label is validated and canonicalized as
/// in @ref gtd_channel_unit_parse, so the file stores the canonical spelling. A
/// NULL @ref GtdChannel::unit adds a channel without a unit, whatever
/// @p unit_mode says.
///
/// @param builder   Builder handle.
/// @param channel   Channel description. Not retained after the call returns.
/// @param unit_mode A @ref GtdChannelUnitMode value.
///
/// @return `GTD_ERR_INVALID_CHANNEL` for an invalid unit/mode combination or
///         malformed channel metadata.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_channel_with_unit_mode(
    builder: *mut GtdFileBuilder,
    channel: *const GtdChannel,
    unit_mode: u32,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
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
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        } else {
            // SAFETY: components is non-null with `n_components` elements (checked).
            let ptrs = unsafe { std::slice::from_raw_parts(ch.components, ch.n_components) };
            let mut labels = Vec::with_capacity(ch.n_components);
            for &ptr in ptrs {
                if ptr.is_null() {
                    error::set_last_error("a component label pointer is null");
                    return GtdStatus::GTD_ERR_NULL_ARGUMENT;
                }
                // SAFETY: `ptr` is a non-null C string (checked).
                match unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str() {
                    Ok(label) => labels.push(label.to_owned()),
                    Err(_) => {
                        error::set_last_error("a component label is not valid UTF-8");
                        return GtdStatus::GTD_ERR_UTF8;
                    }
                }
            }
            Some(labels)
        };

        if ch.n_times > 0 && ch.times.is_null() {
            error::set_last_error("times is null but n_times > 0");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
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
                return GtdStatus::GTD_ERR_NULL_ARGUMENT;
            };
            times.push(dt);
        }

        if ch.n_values > 0 && ch.values.is_null() {
            error::set_last_error("values is null but n_values > 0");
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
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
                builder.recorder_mut().add_channel(built);
                GtdStatus::GTD_OK
            }
            Err(e) => {
                error::set_last_error(e);
                GtdStatus::GTD_ERR_INVALID_CHANNEL
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
    let Some(unit_mode) = GtdChannelUnitMode::from_abi_value(unit_mode) else {
        error::set_last_error("unit_mode is not a valid GtdChannelUnitMode");
        return Err(GtdStatus::GTD_ERR_INVALID_CHANNEL);
    };
    unit_mode.parse_label(label).map(Some).map_err(|error| {
        error::set_last_error(error);
        GtdStatus::GTD_ERR_INVALID_CHANNEL
    })
}
