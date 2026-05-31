pub mod segment;
pub mod spatial;

pub use segment::{SegmentationConfig, build_loaded_file, compute_trip_metadata, segment_trips};
pub use spatial::build_global_tree;
