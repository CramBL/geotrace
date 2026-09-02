//! The `satellite` group of `geotrace.h`: the write-path satellite entry.

use crate::{GtdConstellation, GtdOptF64};

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
