pub mod segment;
pub mod spatial;

pub use segment::{SegmentationConfig, build_loaded_file, compute_track_metadata, segment_tracks};
pub use spatial::build_global_tree;
