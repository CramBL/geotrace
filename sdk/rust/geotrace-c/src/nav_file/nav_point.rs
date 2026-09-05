//! Nav point data and the accessors that fill it.

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::optf64;
use crate::timestamp;
use crate::{GtdConstellation, GtdOptF64, GtdSatInfo, GtdTimestamp};

/// Navigation fix data returned by `gtd_nav_file_get_nav_point()`.
///
/// All fields are caller-owned (no pointers to SDK memory).
///
/// The ranges on @ref lat_deg, @ref lon_deg and @ref heading_deg, and
/// non-negative @ref speed_mps and @ref eph_m, are data quality expectations,
/// not parse rules.
/// The SDK returns @ref lat_deg and @ref lon_deg unchanged, NaN included.
/// A NaN @ref heading_deg, @ref speed_mps or @ref eph_m is returned as
/// `GTD_NONE_F64`: NaN is how the write path stores an absent one.
/// Checking a value against its range is the caller's job.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdNavPointInfo {
    /// GPS time of the fix. Use `gtd_ts_is_none()` to check.
    pub gps_time: GtdTimestamp,
    /// System (wall-clock) time of the fix.
    pub sys_time: GtdTimestamp,
    /// WGS-84 latitude in degrees, expected in [-90, 90].
    pub lat_deg: f64,
    /// WGS-84 longitude in degrees, expected in [-180, 180].
    pub lon_deg: f64,
    /// Compass heading in degrees, expected in [0, 360), if known.
    pub heading_deg: GtdOptF64,
    /// Ground speed in m/s, expected to be non-negative, if known.
    pub speed_mps: GtdOptF64,
    /// Estimated horizontal position error in metres, expected to be
    /// non-negative, if known.
    pub eph_m: GtdOptF64,
    /// Number of tracked satellites (0 when no satellite report present).
    pub sat_count: usize,
}

/// Return the number of navigation fixes in the file.
///
/// @param file File handle. Returns 0 if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_nav_point_count(file: *const GtdNavFile) -> usize {
    if file.is_null() {
        return 0;
    }
    // SAFETY: file is non-null (checked above)
    unsafe { (*file).file.nav_points().len() }
}

/// Fill @p out with data for the navigation fix at @p index.
///
/// @param file  File handle.
/// @param index Zero-based index. Must be less than `gtd_nav_file_nav_point_count(file)`.
/// @param out   Caller-allocated struct to fill.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if @p index is out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_nav_point(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdNavPointInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(point) = handle.file.nav_points().get(index) else {
            error::set_last_error(format!("nav point index {index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };

        out.gps_time = point
            .fix
            .gps_time()
            .map_or(timestamp::gtd_ts_none(), timestamp::ts_from_datetime);
        out.sys_time = point
            .fix
            .sys_time()
            .map_or(timestamp::gtd_ts_none(), timestamp::ts_from_datetime);
        out.lat_deg = point.fix.lat.as_degrees();
        out.lon_deg = point.fix.lon.as_degrees();
        out.heading_deg = point.fix.heading.map_or(optf64::opt_f64_none(), |h| {
            optf64::opt_f64_some(h.as_degrees())
        });
        out.speed_mps = point.fix.speed.map_or(optf64::opt_f64_none(), |s| {
            optf64::opt_f64_some(s.as_meters_per_second())
        });
        out.eph_m = point
            .fix
            .eph_m
            .map_or(optf64::opt_f64_none(), optf64::opt_f64_some);
        out.sat_count = point.satellites.as_ref().map_or(0, |r| r.tracked.len());

        GtdStatus::GTD_OK
    })
}

/// Fill @p out with satellite data for a specific satellite within a nav fix.
///
/// @param file            File handle.
/// @param nav_point_index Nav fix index.
/// @param satellite_index Satellite index within that fix. Must be less than
///                        `GtdNavPointInfo::sat_count`.
/// @param out             Caller-allocated struct to fill.
///
/// @return `GTD_ERR_NULL_ARGUMENT` if either index is out of range, or the nav
///         fix has no satellite report.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite(
    file: *const GtdNavFile,
    nav_point_index: usize,
    satellite_index: usize,
    out: *mut GtdSatInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let handle = nonnull_ref!(file);
        let out = nonnull_mut!(out);

        let Some(point) = handle.file.nav_points().get(nav_point_index) else {
            error::set_last_error(format!("nav point index {nav_point_index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };

        let Some(report) = &point.satellites else {
            error::set_last_error(format!(
                "nav point {nav_point_index} has no satellite report"
            ));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };

        let Some(sat) = report.tracked.get(satellite_index) else {
            error::set_last_error(format!("satellite index {satellite_index} is out of range"));
            return GtdStatus::GTD_ERR_NULL_ARGUMENT;
        };

        out.constellation = GtdConstellation::from(sat.constellation);
        out.prn = sat.prn;
        out.in_fix = u8::from(sat.in_fix);
        out.elevation_deg = sat.elevation.map_or(optf64::opt_f64_none(), |v| {
            optf64::opt_f64_some(f64::from(v))
        });
        out.azimuth_deg = sat.azimuth.map_or(optf64::opt_f64_none(), |v| {
            optf64::opt_f64_some(f64::from(v))
        });
        out.snr_dbhz = sat.snr.map_or(optf64::opt_f64_none(), |v| {
            optf64::opt_f64_some(f64::from(v))
        });

        GtdStatus::GTD_OK
    })
}
