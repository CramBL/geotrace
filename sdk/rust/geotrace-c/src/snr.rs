//! The signal-to-noise ratio convention of the `.gtd` format.

use geotrace_sdk_units::snr;

/// Whether @p snr_dbhz is the SNR some receiver firmware sends when it has no measurement:
/// 99 dB·Hz, within the tolerance of the Rust SDK's `snr::is_no_data_sentinel`.
///
/// @param snr_dbhz SNR in dB·Hz.
///
/// @return 1 for such a reading, 0 for any other.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_snr_is_no_data_sentinel(snr_dbhz: f32) -> u8 {
    u8::from(snr::is_no_data_sentinel(snr_dbhz))
}
