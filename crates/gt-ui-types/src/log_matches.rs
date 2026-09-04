//! What the loaded logs' filters put on the map: the entries each filter
//! selected, where they were recorded, and the colour that filter draws them
//! in.
//!
//! A match takes its position from the recording its log is associated
//! against, a log being a layer over time: an entry with no fix inside the
//! association window has no position and draws nothing.

use std::sync::Arc;

use gt_filter::GlobalFilter;
use gt_logfile::ParsedLog;
use gt_types::{FixRef, LoadedFile, MercPoint};

/// Session-unique identity of a loaded log, handed out by `LoadedLogs`.
///
/// Stable while the log stays loaded, and never handed out again once it is
/// unloaded. The hexagon under the cursor identifies its log by this, and the
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

    /// The fix the entry took its position from. The global filter keeps this
    /// match according to that fix's track.
    pub fix: FixRef,
}

/// The log a layer's matches were read out of.
#[derive(Debug, Clone, PartialEq)]
pub struct LogMatchSource {
    pub id: LoadedLogId,
    pub parsed: Arc<ParsedLog>,

    /// The name the map's tooltip shows above this log's lines: the log's name,
    /// with the recording it is anchored to after a middle dot where another
    /// loaded log has the same name. `None` while the session holds one log.
    pub display_name: Option<String>,
}

/// One hexagon on the map: the log it draws matches of, the colour of the
/// filter that selected them, and the entries it groups.
///
/// The map publishes the hexagon under the cursor and the one clicked. The
/// viewer marks the rows of the entries of both, and shows the log of the
/// clicked one.
#[derive(Debug, Clone, PartialEq)]
pub struct LogMatchGlyph {
    pub log: LoadedLogId,
    pub color: LogMatchColor,

    /// Indices into [`ParsedLog::entries`] of the log, ascending.
    pub entry_indices: Vec<usize>,
}

impl LogMatchGlyph {
    pub fn covers(&self, log: LoadedLogId, entry_index: usize) -> bool {
        self.log == log && self.entry_indices.binary_search(&entry_index).is_ok()
    }
}

/// The matches of one filter, in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct LogMatchLayer {
    pub color: LogMatchColor,
    pub log: LogMatchSource,
    pub matches: Vec<LogMatch>,
}

impl LogMatchLayer {
    /// The matches of this layer the global filter keeps, in file order: a
    /// match draws where its entry's timestamp is inside the filter's time
    /// window and its fix's track passes the filter.
    ///
    /// A match whose entry index or fix reference resolves to nothing is kept:
    /// an entry the log no longer holds, or a fix on a track that is no longer
    /// loaded, still draws.
    pub fn matches_passing_filter<'a>(
        &'a self,
        files: &'a [LoadedFile],
        filter: &'a GlobalFilter,
    ) -> impl Iterator<Item = &'a LogMatch> {
        self.matches.iter().filter(move |log_match| {
            self.log
                .parsed
                .entries()
                .get(log_match.entry_index)
                .is_none_or(|entry| gt_filter::point_passes_time_filter(entry.timestamp, filter))
                && log_match
                    .fix
                    .track
                    .resolve(files)
                    .is_none_or(|track| gt_filter::track_passes_filter(track, filter))
        })
    }
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

    /// Matches across every layer. An entry matched by two filters counts once
    /// per filter: each draws its own hexagon.
    pub fn match_count(&self) -> usize {
        self.layers.iter().map(|layer| layer.matches.len()).sum()
    }

    /// [`LogMatches::match_count`] over the matches the global filter keeps,
    /// which the display toggle states beside "Log matches".
    pub fn count_passing_filter(&self, files: &[LoadedFile], filter: &GlobalFilter) -> usize {
        self.layers
            .iter()
            .map(|layer| layer.matches_passing_filter(files, filter).count())
            .sum()
    }
}
