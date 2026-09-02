//! The write-path satellite entry.

use crate::{GtdConstellation, GtdOptF64};

/// A satellite entry within a report (write path, input from C).
///
/// Pass an array of these to `gtd_builder_add_satellite_report()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatellite {
    /// GNSS constellation.
    pub constellation: GtdConstellation,
    /// Pseudo-random noise number (satellite ID).
    pub prn: u32,
    /// Non-zero if this satellite contributed to the position fix.
    pub in_fix: u8,
    /// Elevation above the horizon in degrees [0, 90].
    pub elevation_deg: GtdOptF64,
    /// Azimuth from true north in degrees [0, 360).
    pub azimuth_deg: GtdOptF64,
    /// Signal-to-noise ratio in dB·Hz.
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
