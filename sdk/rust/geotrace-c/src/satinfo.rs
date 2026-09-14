//! The read-path satellite entry.

use crate::{GtdConstellation, GtdOptF32};

/// Satellite data returned by `gtd_nav_file_get_satellite()`.
///
/// The ranges on @ref elevation_deg and @ref azimuth_deg are data quality
/// expectations. The SDK returns a value outside its range unchanged and counts it
/// in a satellite warning (see `gtd_nav_file_get_satellite_warning()`).
/// Checking a value against its range is the caller's job.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatInfo {
    /// GNSS constellation.
    pub constellation: GtdConstellation,
    /// Pseudo-random noise number.
    pub prn: u32,
    /// Non-zero if this satellite contributed to the fix.
    pub in_fix: u8,
    /// Elevation above the horizon in degrees, expected in [0, 90], if available.
    pub elevation_deg: GtdOptF32,
    /// Azimuth from true north in degrees, expected in [0, 360), if available.
    pub azimuth_deg: GtdOptF32,
    /// SNR in dB-Hz, if available. The reader returns a stored value unchanged, which includes a
    /// reading for which `gtd_snr_is_no_data_sentinel()` returns 1.
    pub snr_dbhz: GtdOptF32,
}
