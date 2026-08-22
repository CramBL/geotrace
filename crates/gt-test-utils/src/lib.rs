pub mod fixtures;
#[cfg(feature = "snapshot")]
pub mod interaction;
#[cfg(feature = "ionex")]
pub mod ionex_fixtures;
pub mod log_fixtures;
#[cfg(feature = "snapshot")]
pub mod snapshot_harness;
pub mod transport;

pub use fixtures::{
    SyntheticGtdSpec, empty_file_metadata, empty_track_metadata, latlon_at_meters,
    loaded_track_with_points, marker_test_data, nav_data_with_gap, nav_point_at_meters,
    nav_points_from, nav_points_walking_from, nav_test_data, single_nav_point, stationary_nav_data,
    synthetic_gtd_bytes, synthetic_gtd_bytes_with_channels,
};
#[cfg(feature = "snapshot")]
pub use interaction::HarnessInteraction;
pub use log_fixtures::{
    SyntheticLogSpec, SyntheticLogTimestamps, synthetic_journald_log, synthetic_log_start,
};
#[cfg(feature = "snapshot")]
pub use snapshot_harness::{By, Queryable, TestHarness, TestHarnessBuilder};
pub use transport::{ScriptedTransport, TransportAnswer, UrlPrefixAnswers};

pub const GOLD_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/gold_dataset/gold.gtd"
));
pub const DEMO_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/demo_trip/demo_trip.gtd"
));

/// Asserts that a slice or Vec exactly matches a sequence of patterns, one per element.
///
/// Fails if the length doesn't match or any element fails its pattern.
/// Uses `assert_matches!` internally for each element, so failure messages include
/// the matched value when `Debug` is implemented.
///
/// # Example
/// ```
/// use gt_test_utils::assert_matches_sequence;
/// let v = vec![Some(1u32), None, Some(3)];
/// assert_matches_sequence!(v, [Some(_), None, Some(_)]);
/// ```
#[macro_export]
macro_rules! assert_matches_sequence {
    ($seq:expr, [$($pat:pat),* $(,)?]) => {{
        let seq = $seq;
        let expected_len = [$(stringify!($pat)),*].len();
        assert_eq!(
            seq.len(),
            expected_len,
            "sequence length mismatch: expected {expected_len}, got {}",
            seq.len()
        );
        let mut _idx = 0usize;
        $(
            assert!(
                matches!(&seq[_idx], $pat),
                "element [{}] did not match pattern `{}`",
                _idx,
                stringify!($pat)
            );
            _idx += 1;
        )*
    }};
}
