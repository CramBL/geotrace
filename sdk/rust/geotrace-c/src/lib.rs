//! C FFI layer for the GeoTrace SDK.
//!
//! This crate is a `cdylib`/`staticlib` - its public surface is the C header
//! `sdk/c/geotrace.h`. Do not add Rust public API here.

#![expect(
    unsafe_code,
    reason = "FFI crate - all extern C functions require unsafe"
)]

#[macro_use]
mod macros;

mod builder;
pub(crate) mod error;
mod nav_file;

use std::ffi::{CStr, c_char};

use geotrace_sdk::ChannelUnit;

pub use error::GtdStatus;

// Re-export the opaque handle types so C sees them at crate root.
pub use builder::GtdFileBuilder;
pub use nav_file::GtdNavFile;

/// Validate and canonicalize a channel unit label.
///
/// # Safety
///
/// `label` must point to a NUL-terminated string, `required_len` must be
/// writable, and a non-null `out` must point to `out_capacity` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_channel_unit_parse(
    label: *const c_char,
    unit_mode: u32,
    out: *mut c_char,
    out_capacity: usize,
    required_len: *mut usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        if label.is_null() || required_len.is_null() {
            error::set_last_error("null pointer argument");
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: label is non-null and must point to a NUL-terminated string.
        let label = match unsafe { CStr::from_ptr(label) }.to_str() {
            Ok(label) => label,
            Err(error) => {
                error::set_last_error(error);
                return GtdStatus::ErrUtf8;
            }
        };
        let parsed = match unit_mode {
            0 => label.parse::<ChannelUnit>(),
            1 => ChannelUnit::custom(label),
            _ => {
                error::set_last_error("unit_mode is not a valid GtdChannelUnitMode");
                return GtdStatus::ErrInvalidChannel;
            }
        };
        let unit = match parsed {
            Ok(unit) => unit,
            Err(error) => {
                error::set_last_error(error);
                return GtdStatus::ErrInvalidChannel;
            }
        };
        let bytes = unit.label().as_bytes();
        // SAFETY: required_len is non-null and writable by the caller.
        unsafe { *required_len = bytes.len().saturating_add(1) };
        if out.is_null() || out_capacity == 0 {
            return GtdStatus::Ok;
        }
        if out_capacity <= bytes.len() {
            error::set_last_error("channel unit output buffer is too small");
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: out points to out_capacity bytes and the capacity was checked.
        // SAFETY: out points to at least bytes.len() writable bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len()) };
        // SAFETY: out_capacity is greater than bytes.len().
        let terminator = unsafe { out.add(bytes.len()) };
        // SAFETY: terminator points within the caller's writable buffer.
        unsafe { *terminator = 0 };
        GtdStatus::Ok
    })
}

/// Timestamp: UTC Unix epoch in microseconds.
/// Use `gtd_ts_none()` for absent timestamps.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdTimestamp {
    pub unix_micros: i64,
}

/// Sentinel value for an absent timestamp.
const TS_NONE_SENTINEL: i64 = i64::MIN;

/// Optional f64 value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdOptF64 {
    pub value: f64,
    pub present: u8,
}

impl GtdOptF64 {
    pub(crate) fn to_opt(self) -> Option<f64> {
        if self.present != 0 {
            Some(self.value)
        } else {
            None
        }
    }
}

/// GNSS constellation identifier.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdConstellation {
    Gps = 0,
    Glonass = 1,
    Galileo = 2,
    Beidou = 3,
    Navic = 4,
    Qzss = 5,
}

impl From<GtdConstellation> for geotrace_sdk::Constellation {
    fn from(c: GtdConstellation) -> Self {
        match c {
            GtdConstellation::Gps => geotrace_sdk::Constellation::Gps,
            GtdConstellation::Glonass => geotrace_sdk::Constellation::Glonass,
            GtdConstellation::Galileo => geotrace_sdk::Constellation::Galileo,
            GtdConstellation::Beidou => geotrace_sdk::Constellation::Beidou,
            GtdConstellation::Navic => geotrace_sdk::Constellation::Navic,
            GtdConstellation::Qzss => geotrace_sdk::Constellation::Qzss,
        }
    }
}

impl From<geotrace_sdk::Constellation> for GtdConstellation {
    fn from(c: geotrace_sdk::Constellation) -> Self {
        match c {
            geotrace_sdk::Constellation::Gps => GtdConstellation::Gps,
            geotrace_sdk::Constellation::Glonass => GtdConstellation::Glonass,
            geotrace_sdk::Constellation::Galileo => GtdConstellation::Galileo,
            geotrace_sdk::Constellation::Beidou => GtdConstellation::Beidou,
            geotrace_sdk::Constellation::Navic => GtdConstellation::Navic,
            geotrace_sdk::Constellation::Qzss => GtdConstellation::Qzss,
        }
    }
}

/// Icon for map markers. GTD_ICON_AUTO (255) uses the application default.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdMarkerIcon {
    Pin = 0,
    Cross = 1,
    Circle = 2,
    Lightning = 3,
    Warning = 4,
    Error = 5,
    Check = 6,
    Satellite = 7,
    SatelliteLost = 8,
    Gear = 9,
    Refresh = 10,
    Download = 11,
    Upload = 12,
    Wrench = 13,
    Auto = 255,
}

impl GtdMarkerIcon {
    pub(crate) fn to_marker_icon(self) -> Option<geotrace_sdk::MarkerIcon> {
        match self {
            Self::Pin => Some(geotrace_sdk::MarkerIcon::Pin),
            Self::Cross => Some(geotrace_sdk::MarkerIcon::Cross),
            Self::Circle => Some(geotrace_sdk::MarkerIcon::Circle),
            Self::Lightning => Some(geotrace_sdk::MarkerIcon::Lightning),
            Self::Warning => Some(geotrace_sdk::MarkerIcon::Warning),
            Self::Error => Some(geotrace_sdk::MarkerIcon::Error),
            Self::Check => Some(geotrace_sdk::MarkerIcon::Check),
            Self::Satellite => Some(geotrace_sdk::MarkerIcon::Satellite),
            Self::SatelliteLost => Some(geotrace_sdk::MarkerIcon::SatelliteLost),
            Self::Gear => Some(geotrace_sdk::MarkerIcon::Gear),
            Self::Refresh => Some(geotrace_sdk::MarkerIcon::Refresh),
            Self::Download => Some(geotrace_sdk::MarkerIcon::Download),
            Self::Upload => Some(geotrace_sdk::MarkerIcon::Upload),
            Self::Wrench => Some(geotrace_sdk::MarkerIcon::Wrench),
            Self::Auto => None,
        }
    }

    pub(crate) fn to_icon_choice(self) -> geotrace_sdk::EventMarkerIconChoice {
        match self.to_marker_icon() {
            Some(icon) => geotrace_sdk::EventMarkerIconChoice::Icon(icon),
            None => geotrace_sdk::EventMarkerIconChoice::Auto,
        }
    }
}

impl From<geotrace_sdk::MarkerIcon> for GtdMarkerIcon {
    fn from(icon: geotrace_sdk::MarkerIcon) -> Self {
        match icon {
            geotrace_sdk::MarkerIcon::Pin => Self::Pin,
            geotrace_sdk::MarkerIcon::Cross => Self::Cross,
            geotrace_sdk::MarkerIcon::Circle => Self::Circle,
            geotrace_sdk::MarkerIcon::Lightning => Self::Lightning,
            geotrace_sdk::MarkerIcon::Warning => Self::Warning,
            geotrace_sdk::MarkerIcon::Error => Self::Error,
            geotrace_sdk::MarkerIcon::Check => Self::Check,
            geotrace_sdk::MarkerIcon::Satellite => Self::Satellite,
            geotrace_sdk::MarkerIcon::SatelliteLost => Self::SatelliteLost,
            geotrace_sdk::MarkerIcon::Gear => Self::Gear,
            geotrace_sdk::MarkerIcon::Refresh => Self::Refresh,
            geotrace_sdk::MarkerIcon::Download => Self::Download,
            geotrace_sdk::MarkerIcon::Upload => Self::Upload,
            geotrace_sdk::MarkerIcon::Wrench => Self::Wrench,
        }
    }
}

/// A satellite entry within a report (write path, input from C).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatellite {
    pub constellation: GtdConstellation,
    pub prn: u32,
    pub in_fix: u8,
    pub elevation_deg: GtdOptF64,
    pub azimuth_deg: GtdOptF64,
    pub snr_dbhz: GtdOptF64,
}

impl GtdSatellite {
    pub(crate) fn to_sdk_satellite(self) -> geotrace_sdk::Satellite {
        geotrace_sdk::Satellite::builder()
            .constellation(geotrace_sdk::Constellation::from(self.constellation))
            .prn(self.prn)
            .in_fix(self.in_fix != 0)
            .maybe_elevation(self.elevation_deg.to_opt().map(|v| v as f32))
            .maybe_azimuth(self.azimuth_deg.to_opt().map(|v| v as f32))
            .maybe_snr(self.snr_dbhz.to_opt().map(|v| v as f32))
            .build()
    }
}

/// A channel to add via `gtd_builder_add_channel` (write path, input from C).
///
/// A scalar channel leaves `components` NULL and `n_components` zero; a vector
/// channel points `components` at `n_components` label strings. `values` is
/// row-major: `n_times` rows of one column (scalar) or `n_components` columns
/// (vector), so `n_values` must equal `n_times * max(n_components, 1)`.
#[repr(C)]
pub struct GtdChannel {
    pub name: *const c_char,
    pub unit: *const c_char,
    pub period_deg: GtdOptF64,
    pub description: *const c_char,
    pub components: *const *const c_char,
    pub n_components: usize,
    pub times: *const GtdTimestamp,
    pub n_times: usize,
    pub values: *const f64,
    pub n_values: usize,
}

/// Channel metadata returned by `gtd_nav_file_get_channel` (read path). Sample
/// timestamps, values, and component labels are fetched separately; a
/// `component_count` of zero marks a scalar channel.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdChannelInfo {
    pub name: [c_char; 256],
    pub has_unit: u8,
    pub unit: [c_char; 64],
    pub period_deg: GtdOptF64,
    pub has_description: u8,
    pub description: [c_char; 1024],
    pub component_count: usize,
    pub sample_count: usize,
}

/// Copy `s` into a fixed C-string buffer, zero-filling and always leaving a
/// trailing NUL (truncating an over-long string rather than overrunning).
pub(crate) fn fill_c_str(dst: &mut [c_char], s: &str) {
    dst.fill(0);
    let cap = dst.len().saturating_sub(1);
    for (slot, byte) in dst.iter_mut().zip(s.bytes().take(cap)) {
        *slot = byte as c_char;
    }
}

/// Nav point data returned by `gtd_nav_file_get_nav_point`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdNavPointInfo {
    pub gps_time: GtdTimestamp,
    pub sys_time: GtdTimestamp,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub heading_deg: GtdOptF64,
    pub speed_mps: GtdOptF64,
    pub eph_m: GtdOptF64,
    pub sat_count: usize,
}

/// Satellite data returned by `gtd_nav_file_get_satellite`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatInfo {
    pub constellation: GtdConstellation,
    pub prn: u32,
    pub in_fix: u8,
    pub elevation_deg: GtdOptF64,
    pub azimuth_deg: GtdOptF64,
    pub snr_dbhz: GtdOptF64,
}

/// Event marker data returned by `gtd_nav_file_get_event_marker`.
#[repr(C)]
pub struct GtdEventMarkerInfo {
    pub variant_path: [c_char; 257],
    pub sys_time: GtdTimestamp,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub has_annotation: u8,
    pub annotation: [c_char; 1024],
}

// ── Timestamp helper functions ──────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

pub(crate) fn ts_from_datetime(dt: chrono::DateTime<chrono::Utc>) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: dt.timestamp_micros(),
    }
}

pub(crate) fn ts_to_datetime(ts: GtdTimestamp) -> Option<chrono::DateTime<chrono::Utc>> {
    if ts.unix_micros == TS_NONE_SENTINEL {
        None
    } else {
        chrono::DateTime::from_timestamp_micros(ts.unix_micros)
    }
}

fn opt_f64_none() -> GtdOptF64 {
    GtdOptF64 {
        value: 0.0,
        present: 0,
    }
}

fn opt_f64_some(v: f64) -> GtdOptF64 {
    GtdOptF64 {
        value: v,
        present: 1,
    }
}

// ── Exported C functions ────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_seconds(secs: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: (secs as i64).saturating_mul(1_000_000),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_millis(ms: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: (ms as i64).saturating_mul(1_000),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_micros(us: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: us as i64,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_nanos(ns: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: (ns / 1_000) as i64,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_none() -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: TS_NONE_SENTINEL,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_is_none(ts: GtdTimestamp) -> u8 {
    u8::from(ts.unix_micros == TS_NONE_SENTINEL)
}

/// Returns the last error message for the current thread, or NULL if none.
/// The pointer is valid until the next SDK call on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_last_error() -> *const c_char {
    error::last_error_ptr()
}
