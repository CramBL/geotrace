//! The `satinfo` group of `geotrace.h`: the read-path satellite entry.

use crate::{GtdConstellation, GtdOptF64};

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
