//! The session model of the log layer: the logs loaded alongside the
//! recordings, and the one recording each of them is associated against.
//!
//! A log is a layer over time: the session keeps its full text, and every entry
//! carries the position of the fix nearest in time in the one recording the
//! user pointed the log at. Association reads the fixes of that recording
//! alone, whatever else is loaded.

mod association;
mod loaded_log;
#[cfg(test)]
mod test_fixtures;

pub use association::{AssociationCandidate, AssociationCandidates};
pub use loaded_log::{LoadedLog, LoadedLogs};
