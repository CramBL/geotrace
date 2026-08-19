//! What the loaded logs' filters put on the map: the entries each filter
//! selected, where they were recorded, and the colour that filter draws them
//! in.
//!
//! A match takes its position from the recording its log is associated
//! against, a log being a layer over time: an entry with no fix inside the
//! association window has no position and draws nothing.

use std::sync::Arc;

use gt_logfile::ParsedLog;
use gt_types::MercPoint;

/// Session-unique identity of a loaded log, handed out by `LoadedLogs`.
///
/// Stable while the log stays loaded, and never handed out again once it is
/// unloaded. The hexagon under the cursor names its log by this, and the
/// viewer resolves that back to the log's rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadedLogId(u64);

impl LoadedLogId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// The colour a group of log matches draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogMatchColor {
    /// The colour reserved for the filter being typed.
    LiveFilter,

    /// A layer chip's palette slot. `shared` marks a slot held by more than
    /// one chip, which the map draws with a doubled outline.
    LayerSlot { index: usize, shared: bool },
}

/// One entry a filter matched, at the position it was recorded at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogMatch {
    pub merc: MercPoint,

    /// Index into [`ParsedLog::entries`] of the layer's log.
    pub entry_index: usize,
}

/// The log a layer's matches were read out of.
#[derive(Debug, Clone, PartialEq)]
pub struct LogMatchSource {
    pub id: LoadedLogId,
    pub parsed: Arc<ParsedLog>,
}

/// The matches of one filter, in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct LogMatchLayer {
    pub color: LogMatchColor,
    pub log: LogMatchSource,
    pub matches: Vec<LogMatch>,
}

/// Every loaded log's map contribution, in draw order: later layers draw over
/// earlier ones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogMatches {
    layers: Vec<LogMatchLayer>,
}

impl LogMatches {
    pub fn from_layers(layers: Vec<LogMatchLayer>) -> Self {
        Self { layers }
    }

    pub fn layers(&self) -> &[LogMatchLayer] {
        &self.layers
    }

    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|layer| layer.matches.is_empty())
    }

    /// Matches across every layer, which the display toggle counts. An entry
    /// matched by two filters counts once per filter: each draws its own
    /// hexagon.
    pub fn match_count(&self) -> usize {
        self.layers.iter().map(|layer| layer.matches.len()).sum()
    }
}
