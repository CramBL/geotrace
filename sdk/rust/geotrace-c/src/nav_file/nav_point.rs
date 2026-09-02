//! The `navpoint` group of `geotrace.h` and the accessors of `navfile_read` that fill it.

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::optf64;
use crate::timestamp;
use crate::{GtdConstellation, GtdOptF64, GtdSatInfo, GtdTimestamp};

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_nav_point_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null (checked above)
    unsafe { (*f).file.nav_points().len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_nav_point(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut GtdNavPointInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(point) = f.file.nav_points().get(idx) else {
            error::set_last_error(format!("nav point index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        out.gps_time = point
            .fix
            .gps_time
            .map_or(timestamp::gtd_ts_none(), timestamp::ts_from_datetime);
        out.sys_time = point
            .fix
            .sys_time
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

        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite(
    f: *const GtdNavFile,
    nav_idx: usize,
    sat_idx: usize,
    out: *mut GtdSatInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(point) = f.file.nav_points().get(nav_idx) else {
            error::set_last_error(format!("nav point index {nav_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        let Some(report) = &point.satellites else {
            error::set_last_error(format!("nav point {nav_idx} has no satellite report"));
            return GtdStatus::ErrNullArgument;
        };

        let Some(sat) = report.tracked.get(sat_idx) else {
            error::set_last_error(format!("satellite index {sat_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
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

        GtdStatus::Ok
    })
}
