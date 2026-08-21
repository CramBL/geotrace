//! What raises the environment warning, one row per metric.

use crate::reference::ReferenceDocument;

/// One metric's row in the popup behind the map's environment warning icon:
/// the level at which the metric raises a warning, and the material the row's
/// link opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningLevelExplanation {
    pub trigger: String,
    pub reference: ReferenceDocument,
}
