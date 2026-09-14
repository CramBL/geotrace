//! The write-path satellite entry.

use crate::{GtdConstellation, GtdOptF32};

/// A satellite entry within a report (write path, input from C).
///
/// Pass an array of these to `gtd_builder_add_satellite_report()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatellite {
    /// GNSS constellation. A @ref GtdConstellation value.
    pub constellation: u32,
    /// Pseudo-random noise number (satellite ID).
    pub prn: u32,
    /// Non-zero if this satellite contributed to the position fix.
    pub in_fix: u8,
    /// Elevation above the horizon in degrees [0, 90].
    pub elevation_deg: GtdOptF32,
    /// Azimuth from true north in degrees [0, 360).
    pub azimuth_deg: GtdOptF32,
    /// Signal-to-noise ratio in dB·Hz, `GTD_NONE_F32` without a measurement. The builder writes a
    /// present value unchanged: pass `GTD_NONE_F32` for a reading for which
    /// `gtd_snr_is_no_data_sentinel()` returns 1.
    pub snr_dbhz: GtdOptF32,
}

impl GtdSatellite {
    /// `None` when `constellation` is a value no [`GtdConstellation`] variant
    /// declares.
    pub(crate) fn to_sdk_satellite(self) -> Option<geotrace_sdk::Satellite> {
        let constellation = GtdConstellation::from_abi_value(self.constellation)?;
        Some(
            geotrace_sdk::Satellite::builder()
                .constellation(geotrace_sdk::Constellation::from(constellation))
                .prn(self.prn)
                .in_fix(self.in_fix != 0)
                .maybe_elevation(self.elevation_deg.to_opt())
                .maybe_azimuth(self.azimuth_deg.to_opt())
                .maybe_snr(self.snr_dbhz.to_opt())
                .build(),
        )
    }
}
