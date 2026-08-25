//! What the space weather warning states: the affected tracks, and the level
//! each metric warns at.

use crate::reference::ReferenceDocument;

/// One metric's row in the popup behind the map's environment warning icon:
/// the level at which the metric raises a warning, and the material the row's
/// link opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningLevelExplanation {
    pub trigger: String,
    pub reference: ReferenceDocument,
}

/// The space weather warning of one loaded track: the track as the rest of the
/// app names it, and one line per environment metric that reached its
/// disturbance level over it, each stating the value it reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackSpaceWeatherWarning {
    pub track_label: String,
    pub lines: Vec<String>,
    /// Whether [`Self::lines`] states a TEC deviation, which every surface
    /// listing the track closes with the caveat about its 27-day reference.
    pub states_tec_deviation: bool,
}
