//! The signal-to-noise ratio convention of the `.gtd` format: the value some
//! receiver firmware sends when it measured no signal strength.
//!
//! An SDK writes an unavailable SNR as no value at all and counts a reading in
//! this band among its satellite warnings. GeoTrace keeps such a reading and
//! shows it as its own signal-quality class. Both classify a reading here, so
//! one value has one class on either side of a file.

/// The SNR in dB-Hz some receiver firmware sends when it has no measurement.
pub const NO_DATA_SENTINEL_DB_HZ: f32 = 99.0;

/// How far from [`NO_DATA_SENTINEL_DB_HZ`] a reading still counts as that
/// value, in dB-Hz.
pub const NO_DATA_SENTINEL_TOLERANCE_DB_HZ: f32 = 0.5;

/// Whether `snr_db_hz` is the value firmware sends when it has no measurement.
pub fn is_no_data_sentinel(snr_db_hz: f32) -> bool {
    (snr_db_hz - NO_DATA_SENTINEL_DB_HZ).abs() < NO_DATA_SENTINEL_TOLERANCE_DB_HZ
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{NO_DATA_SENTINEL_DB_HZ, is_no_data_sentinel};

    #[rstest]
    #[case::the_value_itself(NO_DATA_SENTINEL_DB_HZ, true)]
    #[case::inside_the_band(99.4, true)]
    #[case::at_the_lower_edge_of_the_band(98.5, false)]
    #[case::at_the_upper_edge_of_the_band(99.5, false)]
    #[case::a_measurement(40.0, false)]
    fn the_band_is_half_a_db_wide_either_side(#[case] snr_db_hz: f32, #[case] expected: bool) {
        assert_eq!(is_no_data_sentinel(snr_db_hz), expected);
    }
}
