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

use crate::visibility::{self, TrackDataVisibility};

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
    /// A layer chip's palette slot. `shared` marks a slot held by more than
    /// one chip, which the map draws with a doubled outline.
    LayerSlot { index: usize, shared: bool },

    /// The colour reserved for the filter being typed.
    LiveFilter,
}

/// One entry a filter matched, at the position it was recorded at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogMatch {
    pub merc: MercPoint,

    /// Index into [`ParsedLog::entries`] of the layer's log.
    pub entry_index: usize,

    /// The fix the entry took its position from. The tree and the global
    /// filter keep this match according to that fix's track.
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
    /// The matches of this layer the map draws, in file order: a match draws
    /// where its entry's timestamp is inside the filter's time window and its
    /// fix's track is in scope - its file and the track itself enabled in the
    /// side panel tree, and the track passing the filter.
    ///
    /// The track's own "Track" toggle is no gate here: a hexagon is a log
    /// event, and stays on the map where the user hid the line under it.
    ///
    /// A match whose entry index or fix reference resolves to nothing is kept:
    /// an entry the log no longer holds, or a fix on a track that is no longer
    /// loaded, still draws.
    pub fn matches_in_scope<'a>(
        &'a self,
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        filter: &'a GlobalFilter,
    ) -> impl Iterator<Item = &'a LogMatch> {
        self.matches.iter().filter(move |log_match| {
            self.log
                .parsed
                .entries()
                .get(log_match.entry_index)
                .is_none_or(|entry| gt_filter::point_passes_time_filter(entry.timestamp, filter))
                && log_match.fix.track.resolve(files).is_none_or(|_| {
                    visibility::track_in_scope(files, visibility, filter, log_match.fix.track)
                        .is_some()
                })
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

    /// [`LogMatches::match_count`] over the matches of every layer that
    /// [`LogMatchLayer::matches_in_scope`] keeps, which the display toggle
    /// states beside "Log matches".
    pub fn count_in_scope(
        &self,
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
    ) -> usize {
        self.layers
            .iter()
            .map(|layer| layer.matches_in_scope(files, visibility, filter).count())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use gt_types::{FileIdx, PointIdx, TrackIdx, TrackRef};

    use super::*;
    use crate::test_util;

    /// One line per fixture point, timestamped at that point.
    fn a_log_over_the_fixture_track() -> LogMatchSource {
        let text: String = (0..test_util::POINT_COUNT)
            .map(|index| {
                let time = test_util::start() + TimeDelta::seconds(index as i64);
                format!(
                    "{} navsyncd[770]: gnss fix acquired\n",
                    time.format("%Y-%m-%d %H:%M:%S")
                )
            })
            .collect();
        LogMatchSource {
            id: LoadedLogId::new(0),
            parsed: Arc::new(
                gt_logfile::parse_log(text.into(), test_util::start())
                    .expect("the fixture log parses"),
            ),
            display_name: None,
        }
    }

    /// A layer of one match per line of that log, each on the fixture track's
    /// fix of the same index.
    fn a_layer_over_every_line() -> LogMatchLayer {
        LogMatchLayer {
            color: LogMatchColor::LiveFilter,
            log: a_log_over_the_fixture_track(),
            matches: (0..test_util::POINT_COUNT)
                .map(|index| LogMatch {
                    merc: MercPoint { x: 0.5, y: 0.5 },
                    entry_index: index,
                    fix: FixRef::new(test_util::track0(), PointIdx::new(index)),
                })
                .collect(),
        }
    }

    /// What one case withholds a match with.
    struct Gates {
        visibility: TrackDataVisibility,
        filter: GlobalFilter,
    }

    /// The time window opens on the fixture's third line, which leaves the
    /// two lines before it out and keeps the track itself.
    fn window_from_the_third_line(gates: &mut Gates) {
        gates.filter.time_start = Some(test_util::start() + TimeDelta::seconds(2));
    }

    #[rstest::rstest]
    #[case::everything_in_scope(|_: &mut Gates| {}, test_util::POINT_COUNT)]
    #[case::file_unchecked(|gates: &mut Gates| gates.visibility.files[0].enabled = false, 0)]
    #[case::track_unchecked(|gates: &mut Gates| gates.visibility.files[0].tracks[0].enabled = false, 0)]
    #[case::filter_rejects_the_track(
        |gates: &mut Gates| gates.filter.min_duration = Some(TimeDelta::hours(1)),
        0
    )]
    #[case::line_outside_the_time_window(window_from_the_third_line, 2)]
    fn matches_in_scope_applies_the_tree_and_the_filter(
        #[case] withhold: fn(&mut Gates),
        #[case] expected: usize,
    ) {
        let files = test_util::one_track_file();
        let mut gates = Gates {
            visibility: TrackDataVisibility::from_loaded(&files),
            filter: GlobalFilter::default(),
        };
        withhold(&mut gates);

        assert_eq!(
            a_layer_over_every_line()
                .matches_in_scope(&files, &gates.visibility, &gates.filter)
                .count(),
            expected
        );
    }

    #[test]
    fn a_match_on_a_track_that_is_no_longer_loaded_stays_in_scope() {
        let files = test_util::one_track_file();
        let mut layer = a_layer_over_every_line();
        let unloaded = TrackRef::new(FileIdx::new(0), TrackIdx::new(7));
        for log_match in &mut layer.matches {
            log_match.fix = FixRef::new(unloaded, PointIdx::new(0));
        }

        assert_eq!(
            layer
                .matches_in_scope(
                    &files,
                    &TrackDataVisibility::from_loaded(&files),
                    &GlobalFilter::default()
                )
                .count(),
            test_util::POINT_COUNT
        );
    }

    #[test]
    fn count_in_scope_counts_the_matches_of_every_layer_on_a_checked_track() {
        let files = test_util::one_track_file();
        let mut visibility = TrackDataVisibility::from_loaded(&files);
        let filter = GlobalFilter::default();
        let matches =
            LogMatches::from_layers(vec![a_layer_over_every_line(), a_layer_over_every_line()]);

        assert_eq!(
            matches.count_in_scope(&files, &visibility, &filter),
            2 * test_util::POINT_COUNT
        );

        visibility.files[0].tracks[0].enabled = false;
        assert_eq!(matches.count_in_scope(&files, &visibility, &filter), 0);
    }
}
