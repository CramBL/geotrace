//! The matches of one run as table rows: what each row states, the order the
//! table lists them in, and which of them the points table below follows.

use std::cmp::Ordering;
use std::ops::Range;

use chrono::{DateTime, Utc};
use egui::{Align, Layout};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_UP as ICON_CARET_UP;
use gt_query_run::{ChannelResults, PointsResults};
use gt_types::{LoadedFile, TrackRef};
use strum::EnumIter;

use super::column_format;

/// A match's identity across reruns: the track it is on and the row it starts
/// at. A rerun over unchanged data lists the same key, so the selection stays
/// on the match the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MatchKey {
    track: TrackRef,
    first_row: usize,
}

/// One row of the matches table: one match of a points query, or one track's
/// matched stretch of channel samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MatchRow {
    /// The query of the run that matched it: its swatch colour, and the columns
    /// the points table lists its rows under.
    pub(super) query_index: usize,
    /// Position in run order, 1-based, as the `#` column states it.
    pub(super) number: usize,
    pub(super) track: TrackRef,
    /// Indices into the track's row source: its nav points, or the samples of
    /// the channel timeline.
    pub(super) rows: Range<usize>,
    start: Option<DateTime<Utc>>,
    /// When the match's last row was recorded, absent for a single-row match.
    end: Option<DateTime<Utc>>,
    duration_secs: Option<i64>,
}

impl MatchRow {
    pub(super) fn key(&self) -> MatchKey {
        MatchKey {
            track: self.track,
            first_row: self.rows.start,
        }
    }

    /// What the `start`, `end` and `duration` columns state. A single-row match
    /// leaves the last two empty: it has no extent to state.
    pub(super) fn cell_text(&self, column: MatchColumn, files: &[LoadedFile]) -> String {
        let time = |at: Option<DateTime<Utc>>| {
            at.map_or_else(String::new, |at| at.format("%H:%M:%S").to_string())
        };
        match column {
            MatchColumn::Number => format!("{}", self.number),
            MatchColumn::Track => track_label(files, self.track),
            MatchColumn::Start => time(self.start),
            MatchColumn::End => time(self.end),
            MatchColumn::Points => format!("{}", self.rows.len()),
            MatchColumn::Duration => self
                .duration_secs
                .map_or_else(String::new, gt_fmt::format_match_duration),
        }
    }
}

/// The matches of one run, in the order the table lists them.
#[derive(Debug, Default)]
pub(super) struct MatchRows(Vec<MatchRow>);

impl MatchRows {
    /// Every match of a points run, over all of its queries in editor order.
    pub(super) fn of_points(files: &[LoadedFile], results: &PointsResults) -> Self {
        let mut rows = Vec::new();
        for (query_index, query) in results.queries.iter().enumerate() {
            for track_matches in &query.matches {
                let track = track_matches.track;
                let points = track.resolve(files).map(|track| track.points.as_slice());
                let time_of = |index: usize| {
                    points
                        .and_then(|points| points.get(index))
                        .map(|point| point.tpv.time().utc())
                };
                for range in &track_matches.ranges {
                    rows.push(MatchRow {
                        query_index,
                        number: rows.len() + 1,
                        track,
                        start: time_of(range.start),
                        end: gt_fmt::last_index_of_span(range).and_then(time_of),
                        duration_secs: track
                            .resolve(files)
                            .and_then(|track| gt_fmt::match_duration_seconds(track, range)),
                        rows: range.clone(),
                    });
                }
            }
        }
        Self(rows)
    }

    /// Every matched stretch of samples of a channel-source run, over its
    /// tracks in run order.
    pub(super) fn of_channel_samples(results: &ChannelResults) -> Self {
        let mut rows = Vec::new();
        for result in &results.tracks {
            let times = result.timeline.times.as_slice();
            for range in &result.ranges {
                let first = times.get(range.start).copied();
                let last = gt_fmt::last_index_of_span(range)
                    .and_then(|last| times.get(last))
                    .copied();
                rows.push(MatchRow {
                    query_index: 0,
                    number: rows.len() + 1,
                    track: result.track,
                    start: first.and_then(column_format::wall_clock),
                    end: last.and_then(column_format::wall_clock),
                    duration_secs: first.zip(last).map(|(first, last)| (last - first) as i64),
                    rows: range.clone(),
                });
            }
        }
        Self(rows)
    }

    pub(super) fn rows(&self) -> &[MatchRow] {
        &self.0
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The match the points table lists: the selected one while the run still
    /// holds it, else the first row of the table.
    pub(super) fn selected(&self, selected: Option<MatchKey>) -> Option<&MatchRow> {
        selected
            .and_then(|key| self.0.iter().find(|row| row.key() == key))
            .or_else(|| self.0.first())
    }

    pub(super) fn sort(&mut self, sort: MatchSort) {
        sort.apply(&mut self.0);
    }
}

/// A column of the matches table between its swatch and its map button, in the
/// order the table lays them out. Every one of them orders the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub(super) enum MatchColumn {
    Number,
    Track,
    Start,
    End,
    Points,
    Duration,
}

impl MatchColumn {
    /// The column's header text. `samples` names a channel run's rows, which
    /// are samples of a channel timeline rather than nav points.
    pub(super) fn title(self, row_noun: RowNoun) -> &'static str {
        match self {
            Self::Number => "#",
            Self::Track => "track",
            Self::Start => "start",
            Self::End => "end",
            Self::Points => row_noun.plural(),
            Self::Duration => "duration",
        }
    }

    /// The layout this column's cells and its header read in: numbers line up
    /// on their right edge, text on its left.
    pub(super) fn cell_layout(self) -> Layout {
        match self {
            Self::Number | Self::Points | Self::Duration => Layout::right_to_left(Align::Center),
            Self::Track | Self::Start | Self::End => Layout::left_to_right(Align::Center),
        }
    }

    /// The widest text a cell of this column prints, for sizing the column once
    /// instead of measuring every row. A track label longer than this is
    /// clipped where its column ends.
    pub(super) fn widest_cell_text(self) -> &'static str {
        match self {
            Self::Number | Self::Points => "8888",
            Self::Track => "#888",
            Self::Start | Self::End => "88:88:88",
            Self::Duration => "88:88 min",
        }
    }

    /// The direction a first click on this column sorts in: times and run order
    /// read best from the start, magnitudes biggest-first.
    fn initial_direction(self) -> SortDirection {
        match self {
            Self::Number | Self::Track | Self::Start | Self::End => SortDirection::Ascending,
            Self::Points | Self::Duration => SortDirection::Descending,
        }
    }

    /// The header's hover hint for this column in `direction`.
    pub(super) fn order_hint(self, direction: SortDirection) -> &'static str {
        match (self, direction) {
            (Self::Number, SortDirection::Ascending) => "in run order",
            (Self::Number, SortDirection::Descending) => "in reverse run order",
            (Self::Track, SortDirection::Ascending) => "by track, first to last",
            (Self::Track, SortDirection::Descending) => "by track, last to first",
            (Self::Start | Self::End, SortDirection::Ascending) => "earliest first",
            (Self::Start | Self::End, SortDirection::Descending) => "latest first",
            (Self::Points | Self::Duration, SortDirection::Ascending) => "smallest first",
            (Self::Points | Self::Duration, SortDirection::Descending) => "largest first",
        }
    }

    /// Order two matches by this column's value, ascending.
    fn compare(self, a: &MatchRow, b: &MatchRow) -> Ordering {
        match self {
            Self::Number => a.number.cmp(&b.number),
            Self::Track => a.track.cmp(&b.track),
            Self::Start => a.start.cmp(&b.start),
            Self::End => a.end.cmp(&b.end),
            Self::Points => a.rows.len().cmp(&b.rows.len()),
            Self::Duration => a.duration_secs.cmp(&b.duration_secs),
        }
    }
}

/// What one row of a run's tables holds: a nav point of a track, or a sample of
/// a channel timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowNoun {
    Point,
    Sample,
}

impl RowNoun {
    pub(super) fn singular(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Sample => "sample",
        }
    }

    pub(super) fn plural(self) -> &'static str {
        match self {
            Self::Point => "points",
            Self::Sample => "samples",
        }
    }

    /// What a row's index counts from, as the copy tooltip names it.
    pub(super) fn index_source(self) -> &'static str {
        match self {
            Self::Point => "track",
            Self::Sample => "channel's samples",
        }
    }
}

/// Which way a [`MatchColumn`]'s order runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    /// The caret drawn beside the active column's header, pointing the way the
    /// values grow down the list.
    pub(super) fn caret(self) -> &'static str {
        match self {
            Self::Ascending => ICON_CARET_UP,
            Self::Descending => ICON_CARET_DOWN,
        }
    }
}

/// How the matches table is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MatchSort {
    pub(super) column: MatchColumn,
    pub(super) direction: SortDirection,
}

impl Default for MatchSort {
    /// Run order: the queries in editor order, each one's matches in the order
    /// the run listed them.
    fn default() -> Self {
        Self {
            column: MatchColumn::Number,
            direction: SortDirection::Ascending,
        }
    }
}

impl MatchSort {
    /// Apply a header click: clicking the active column reverses it, clicking
    /// another switches to it in the direction that reads most naturally there.
    pub(super) fn clicked(&mut self, column: MatchColumn) {
        *self = if self.column == column {
            Self {
                column,
                direction: self.direction.reversed(),
            }
        } else {
            Self {
                column,
                direction: column.initial_direction(),
            }
        };
    }

    /// The direction a click on `column` sorts in next, for its hover hint.
    pub(super) fn next_direction(self, column: MatchColumn) -> SortDirection {
        if self.column == column {
            self.direction.reversed()
        } else {
            column.initial_direction()
        }
    }

    /// Order `rows` in place. Ties keep run order, which is unique across the
    /// run and independent of the chosen direction.
    fn apply(self, rows: &mut [MatchRow]) {
        rows.sort_by(|a, b| {
            let by_column = match self.direction {
                SortDirection::Ascending => self.column.compare(a, b),
                SortDirection::Descending => self.column.compare(a, b).reverse(),
            };
            by_column.then_with(|| a.number.cmp(&b.number))
        });
    }
}

/// Which track a match is on: its number, led by the recording's filename while
/// several files are loaded, where the number alone would not say which
/// recording is meant.
fn track_label(files: &[LoadedFile], track_ref: TrackRef) -> String {
    match files.len() {
        0 | 1 => format!("#{}", track_ref.index),
        _ => {
            let file = track_ref.fi.get(files).map_or_else(
                || format!("file {}", track_ref.fi),
                |f| f.metadata.filename.clone(),
            );
            format!("{file} #{}", track_ref.index)
        }
    }
}

#[cfg(test)]
mod tests {
    use gt_types::{FileIdx, TrackIdx};
    use rstest::rstest;

    use super::*;

    fn track(index: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(index))
    }

    fn at(hour: u32, minute: u32, second: u32) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(
            i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second),
            0,
        )
    }

    /// Three matches: the first long and early on track 0, the second short and
    /// later on track 1, the third a single point on track 0.
    fn test_rows() -> MatchRows {
        MatchRows(vec![
            MatchRow {
                query_index: 0,
                number: 1,
                track: track(0),
                rows: 0..62,
                start: at(14, 0, 18),
                end: at(14, 1, 19),
                duration_secs: Some(61),
            },
            MatchRow {
                query_index: 0,
                number: 2,
                track: track(1),
                rows: 100..112,
                start: at(14, 2, 19),
                end: at(14, 2, 31),
                duration_secs: Some(12),
            },
            MatchRow {
                query_index: 1,
                number: 3,
                track: track(0),
                rows: 200..201,
                start: at(14, 5, 0),
                end: None,
                duration_secs: None,
            },
        ])
    }

    fn numbers(rows: &MatchRows) -> Vec<usize> {
        rows.rows().iter().map(|row| row.number).collect()
    }

    #[rstest]
    #[case::run_order(MatchColumn::Number, SortDirection::Ascending, [1, 2, 3])]
    #[case::reverse_run_order(MatchColumn::Number, SortDirection::Descending, [3, 2, 1])]
    // Track ties keep run order, whichever way the column runs.
    #[case::track(MatchColumn::Track, SortDirection::Ascending, [1, 3, 2])]
    #[case::track_reversed(MatchColumn::Track, SortDirection::Descending, [2, 1, 3])]
    #[case::start(MatchColumn::Start, SortDirection::Descending, [3, 2, 1])]
    // A single-point match has no end: it sorts before the matches that have
    // one.
    #[case::end(MatchColumn::End, SortDirection::Ascending, [3, 1, 2])]
    #[case::points(MatchColumn::Points, SortDirection::Descending, [1, 2, 3])]
    #[case::points_reversed(MatchColumn::Points, SortDirection::Ascending, [3, 2, 1])]
    #[case::duration(MatchColumn::Duration, SortDirection::Descending, [1, 2, 3])]
    fn sorting_orders_the_matches_by_a_column(
        #[case] column: MatchColumn,
        #[case] direction: SortDirection,
        #[case] expected: [usize; 3],
    ) {
        let mut rows = test_rows();
        rows.sort(MatchSort { column, direction });
        assert_eq!(numbers(&rows), expected);
    }

    #[test]
    fn clicking_a_header_reverses_the_active_column_and_switches_to_any_other() {
        let mut sort = MatchSort::default();
        assert_eq!(sort.column, MatchColumn::Number);

        sort.clicked(MatchColumn::Number);
        assert_eq!(sort.direction, SortDirection::Descending);

        sort.clicked(MatchColumn::Start);
        assert_eq!(
            (sort.column, sort.direction),
            (MatchColumn::Start, SortDirection::Ascending),
            "a time column opens earliest first"
        );
        sort.clicked(MatchColumn::Points);
        assert_eq!(
            (sort.column, sort.direction),
            (MatchColumn::Points, SortDirection::Descending),
            "a magnitude column opens largest first"
        );
    }

    /// A rerun over unchanged data lists the same matches, so the selection
    /// stays where the user put it. A rerun that no longer holds it falls back
    /// to the first row.
    #[test]
    fn the_selection_survives_a_rerun_that_still_lists_it() {
        let rows = test_rows();
        let second = rows.rows().get(1).map(MatchRow::key);
        assert_eq!(rows.selected(second).map(|row| row.number), Some(2));

        let dropped = MatchKey {
            track: track(7),
            first_row: 0,
        };
        assert_eq!(rows.selected(Some(dropped)).map(|row| row.number), Some(1));
        assert_eq!(rows.selected(None).map(|row| row.number), Some(1));
        assert_eq!(MatchRows::default().selected(second), None);
    }

    /// A match states its extent in the columns between its number and its map
    /// button. A single-point match has no end and no duration to state.
    #[rstest]
    #[case(0, MatchColumn::Track, "#0")]
    #[case(0, MatchColumn::Start, "14:00:18")]
    #[case(0, MatchColumn::End, "14:01:19")]
    #[case(0, MatchColumn::Points, "62")]
    #[case(0, MatchColumn::Duration, "1:01 min")]
    #[case(2, MatchColumn::Number, "3")]
    #[case(2, MatchColumn::End, "")]
    #[case(2, MatchColumn::Points, "1")]
    #[case(2, MatchColumn::Duration, "")]
    fn a_match_states_its_track_extent_and_size(
        #[case] index: usize,
        #[case] column: MatchColumn,
        #[case] expected: &str,
    ) {
        let rows = test_rows();
        let row = rows.rows().get(index).expect("the fixture lists the row");
        assert_eq!(row.cell_text(column, &[]), expected);
    }

    /// The track column names the recording as well once several are loaded:
    /// the track number alone would not say which one is meant.
    #[test]
    fn the_track_column_names_the_file_when_several_are_loaded() {
        let files: Vec<LoadedFile> = ["one.gtd", "two.gtd"]
            .into_iter()
            .map(|name| LoadedFile {
                metadata: gt_types::FileMetadata {
                    filename: name.to_owned(),
                    ..gt_test_utils::empty_file_metadata()
                },
                tracks: Vec::new(),
                event_marker_styles: std::collections::HashMap::new(),
                orphaned_event_markers: Vec::new(),
                source: gt_types::FileSource::GtdBytes(std::sync::Arc::from(Vec::<u8>::new())),
                load_warnings: Vec::new(),
            })
            .collect();
        assert_eq!(track_label(&files, track(0)), "one.gtd #0");
        assert_eq!(track_label(&files[..1], track(0)), "#0");
    }
}
