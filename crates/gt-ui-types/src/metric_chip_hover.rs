//! The three lines an environment metric's plot chip shows on hover.

use std::fmt;

use crate::reference::ReferenceDocument;

/// The hover of a chip whose values GeoTrace downloads from an archive: what
/// the metric is, where it comes from, and where to read more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricChipHover {
    pub definition: String,
    pub source_cadence_and_scale: String,
    pub reference: ReferenceDocument,
}

impl MetricChipHover {
    /// Every metric's hover ends with this phrasing, stating the settings link
    /// that opens its reference window.
    pub fn reference_line(&self) -> String {
        format!("More: '{}' in Settings.", self.reference.link_question)
    }
}

impl fmt::Display for MetricChipHover {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.definition)?;
        writeln!(f, "{}", self.source_cadence_and_scale)?;
        write!(f, "{}", self.reference_line())
    }
}
