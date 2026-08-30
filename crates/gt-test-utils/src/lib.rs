pub mod cargo_env;
#[cfg(feature = "archive")]
pub mod day_archive;
pub mod fixtures;
#[cfg(feature = "snapshot")]
pub mod interaction;
#[cfg(feature = "ionex")]
pub mod ionex_fixtures;
pub mod log_fixtures;
pub mod map_tile_fixtures;
pub mod pending_writes;
#[cfg(feature = "snapshot")]
pub mod snapshot_harness;
pub mod transport;
#[cfg(feature = "snapshot")]
pub mod window_fit;

pub use cargo_env::cargo_manifest_dir;
#[cfg(feature = "archive")]
pub use day_archive::{ColumnName, GroupPath};
pub use fixtures::{
    SyntheticGtdSpec, empty_file_metadata, empty_track_metadata, latlon_at_meters,
    loaded_track_with_points, marker_test_data, nav_data_with_gap, nav_point_at_meters,
    nav_points_at_positions, nav_points_from, nav_points_walking_from,
    nav_points_with_a_latitude_out_of_range, nav_points_without_a_valid_position, nav_test_data,
    single_nav_point, stationary_nav_data, synthetic_gtd_bytes, synthetic_gtd_bytes_with_channels,
    track_geometry,
};
#[cfg(feature = "snapshot")]
pub use interaction::HarnessInteraction;
pub use log_fixtures::{
    SyntheticLogSpec, SyntheticLogTimestamps, synthetic_journald_log, synthetic_log_start,
};
pub use map_tile_fixtures::{assert_map_tile_fixture_is_complete, map_tile_fixture_dir};
#[cfg(feature = "snapshot")]
pub use snapshot_harness::{By, Queryable, TestHarness, TestHarnessBuilder};
pub use transport::{ScriptedTransport, TransportAnswer, UrlPrefixAnswers};
#[cfg(feature = "snapshot")]
pub use window_fit::{AuditedWindow, ControlLabel, WindowFitAssertions, oversized_text};

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
