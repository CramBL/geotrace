//! The stored values that gt-history's and gt-store's tests both write.

use crate::log_attachment::{StoredLogFilter, StoredLogFilterEffects, StoredLogFilterStack};
use crate::{
    StoredFixPlacementRule, StoredLogFilterCondition, StoredSegmentation, StoredTrackSplitRule,
};

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

pub fn log_filters() -> StoredLogFilterStack {
    StoredLogFilterStack::single_all_group(vec![
        StoredLogFilter {
            group_id: 0,
            condition: StoredLogFilterCondition::Message {
                text: "gnss".to_owned(),
                regex: false,
            },
            effects: StoredLogFilterEffects::Map {
                enabled: true,
                color_slot: 3,
            },
        },
        StoredLogFilter {
            group_id: 0,
            condition: StoredLogFilterCondition::Message {
                text: "hal-powerd|navsyncd".to_owned(),
                regex: true,
            },
            effects: StoredLogFilterEffects::Table { enabled: false },
        },
    ])
}
