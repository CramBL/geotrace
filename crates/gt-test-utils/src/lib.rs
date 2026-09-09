pub mod cargo_env;
pub mod clock_reset_fixtures;
#[cfg(feature = "archive")]
pub mod day_archive;
#[cfg(feature = "snapshot")]
pub mod interaction;
#[cfg(feature = "ionex")]
pub mod ionex_fixtures;
pub mod log_fixtures;
pub mod map_tile_captures;
pub mod pending_writes;
pub mod recording_fixtures;
#[cfg(feature = "snapshot")]
pub mod snapshot_harness;
#[cfg(feature = "tracks")]
pub mod track_fixtures;
#[cfg(feature = "snapshot")]
pub mod window_fit;

pub use cargo_env::cargo_manifest_dir;
pub use clock_reset_fixtures::recording_whose_clock_restarts_at_every_boot;
#[cfg(feature = "archive")]
pub use day_archive::{ColumnName, GroupPath};
/// The fixture builders that need gt-types alone, for the crates that reach
/// them through their dev-dependency on this one.
pub use gt_types::fixtures;
#[cfg(feature = "snapshot")]
pub use interaction::HarnessInteraction;
pub use log_fixtures::{
    SyntheticLogSpec, SyntheticLogTimestamps, after_the_synthetic_log, synthetic_journald_log,
    synthetic_log_start, syslog_journald_log,
};
pub use map_tile_captures::{assert_map_tile_capture_is_complete, map_tile_capture_dir};
pub use recording_fixtures::{
    SyntheticGtdSpec, marker_test_data, nav_test_data, synthetic_gtd_bytes,
    synthetic_gtd_bytes_with_channels,
};
#[cfg(feature = "snapshot")]
pub use snapshot_harness::{By, Node, NodeT, Queryable, TestHarness, TestHarnessBuilder};
#[cfg(feature = "tracks")]
pub use track_fixtures::{
    FileParts, build_file, empty_file_metadata, empty_track_metadata, loaded_file_with_tracks,
    loaded_track_with_points, segmented_recording, track_geometry,
};
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
