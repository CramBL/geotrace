//! The stored values that gt-history's and gt-store's tests both write.

use crate::log_attachment::{StoredLogFilter, StoredLogFilterMode};
use crate::{StoredFixPlacementRule, StoredSegmentation, StoredTrackSplitRule};

/// The values the app's `SegmentationConfig::default` stores.
pub fn default_segmentation() -> StoredSegmentation {
    StoredSegmentation {
        track_split_gap_us: 300_000_000,
        track_split_rule: StoredTrackSplitRule::StepInEitherDirection,
        fix_placement_rule: StoredFixPlacementRule::MissingHeadingAndNothingInFix,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    }
}

/// A stack with one chip of each mode, with and without a palette slot.
pub fn log_filters() -> Vec<StoredLogFilter> {
    vec![
        StoredLogFilter {
            text: "gnss".to_owned(),
            regex: false,
            enabled: true,
            mode: StoredLogFilterMode::Layer { color_slot: 3 },
        },
        StoredLogFilter {
            text: "hal-powerd|navsyncd".to_owned(),
            regex: true,
            enabled: false,
            mode: StoredLogFilterMode::Refine,
        },
    ]
}
