//! The read-path satellite entry.

use crate::{GtdConstellation, GtdOptF64};

/// Satellite data returned by `gtd_nav_file_get_satellite()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatInfo {
    /// GNSS constellation.
    pub constellation: GtdConstellation,
    /// Pseudo-random noise number.
    pub prn: u32,
    /// Non-zero if this satellite contributed to the fix.
    pub in_fix: u8,
    /// Elevation in degrees, if available.
    pub elevation_deg: GtdOptF64,
    /// Azimuth in degrees, if available.
    pub azimuth_deg: GtdOptF64,
    /// SNR in dB·Hz, if available.
    pub snr_dbhz: GtdOptF64,
}
