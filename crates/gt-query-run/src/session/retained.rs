use std::ops::Range;

use chrono::{DateTime, Utc};
use geotrace_sdk_units::ChannelUnit;
use gt_loaded_files::{LoadedFilesView, SessionTrackRef};
use gt_query::{AggregateColumn, QueryMetric, TableColumn};
use gt_types::{NavPoint, TrackRef};

use super::QuerySession;
use crate::provider::{PointResultValues, TrackQueryData};
use crate::results::{
    ChannelResults, ChannelTrackResult, MatchValues, PanelQuery, PointsResults, RunResults,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedResultError {
    InvalidPosition,
    Stale,
    StaleTrack,
    Unavailable,
}

pub struct RetainedResults<'a> {
    results: &'a RunResults,
    files: LoadedFilesView<'a>,
}

impl QuerySession {
    /// Filter, visibility and series freshness reflect the last [Self::refresh_staleness] call.
    pub fn retained_results<'a>(
        &'a self,
        files: LoadedFilesView<'a>,
    ) -> Result<RetainedResults<'a>, RetainedResultError> {
        let completed = self
            .completed
            .as_ref()
            .ok_or(RetainedResultError::Unavailable)?;
        if !completed.fingerprint.matches_loaded_files(files) || completed.results.stale() {
            return Err(RetainedResultError::Stale);
        }
        Ok(RetainedResults {
            results: &completed.results,
            files,
        })
    }
}

impl<'a> RetainedResults<'a> {
    pub fn query(&self, ordinal: usize) -> Result<RetainedQuery<'a>, RetainedResultError> {
        let result = match self.results {
            RunResults::Channel(channel) if ordinal == 0 => QuerySource::Channel(channel),
            RunResults::Channel(_) => return Err(RetainedResultError::InvalidPosition),
            RunResults::Points(results) => QuerySource::Points {
                results,
                query: results
                    .queries
                    .get(ordinal)
                    .ok_or(RetainedResultError::InvalidPosition)?,
            },
        };
        Ok(RetainedQuery {
            result,
            files: self.files,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatchPosition {
    pub track_ordinal: usize,
    pub match_ordinal: usize,
}

pub struct RetainedQuery<'a> {
    result: QuerySource<'a>,
    files: LoadedFilesView<'a>,
}

impl<'a> RetainedQuery<'a> {
    pub fn channel_name(&self) -> Option<&'a str> {
        match self.result {
            QuerySource::Channel(result) => Some(result.channel.as_str()),
            QuerySource::Points { .. } => None,
        }
    }

    pub fn aggregate_columns(&self) -> impl Iterator<Item = (usize, &'a AggregateColumn)> {
        let (points, channels, component_count) = match self.result {
            QuerySource::Channel(channel) => (
                &[][..],
                channel.aggregate_columns.as_slice(),
                channel.component_labels().len(),
            ),
            QuerySource::Points { query, .. } => (query.columns.as_slice(), &[][..], 0),
        };
        points
            .iter()
            .enumerate()
            .filter_map(|(ordinal, column)| match column {
                TableColumn::Aggregate(aggregate) => Some((ordinal, aggregate)),
                TableColumn::Metric(_) => None,
            })
            .chain(
                channels
                    .iter()
                    .enumerate()
                    .map(move |(ordinal, aggregate)| (component_count + ordinal, aggregate)),
            )
    }

    pub fn first_match(&self) -> Option<MatchPosition> {
        (self.track_count() > 0).then(MatchPosition::default)
    }

    pub fn match_at(
        &self,
        position: MatchPosition,
    ) -> Result<RetainedMatch<'a>, RetainedResultError> {
        let (track, matches, source) = match self.result {
            QuerySource::Channel(result) => {
                let track = result
                    .tracks
                    .get(position.track_ordinal)
                    .ok_or(RetainedResultError::InvalidPosition)?;
                (
                    track.track,
                    &track.matches,
                    MatchSource::Channel { result, track },
                )
            }
            QuerySource::Points { results, query } => {
                let track = query
                    .matches
                    .get(position.track_ordinal)
                    .ok_or(RetainedResultError::InvalidPosition)?;
                let loaded = track
                    .track
                    .resolve(self.files.files())
                    .ok_or(RetainedResultError::StaleTrack)?;
                (
                    track.track,
                    &track.matches,
                    MatchSource::Points {
                        points: &loaded.points,
                        data: results.track_data(track.track),
                        columns: &query.columns,
                    },
                )
            }
        };
        let matched = matches
            .get(position.match_ordinal)
            .ok_or(RetainedResultError::InvalidPosition)?;
        let reference = self
            .files
            .session_track_ref(track)
            .ok_or(RetainedResultError::StaleTrack)?;
        let extent = source.extent(matched.rows.clone())?;
        let next = if position.match_ordinal + 1 < matches.len() {
            Some(MatchPosition {
                match_ordinal: position.match_ordinal + 1,
                ..position
            })
        } else if position.track_ordinal + 1 < self.track_count() {
            Some(MatchPosition {
                track_ordinal: position.track_ordinal + 1,
                match_ordinal: 0,
            })
        } else {
            None
        };
        Ok(RetainedMatch {
            track,
            reference,
            matched,
            source,
            extent,
            next,
        })
    }

    fn track_count(&self) -> usize {
        match self.result {
            QuerySource::Channel(result) => result.tracks.len(),
            QuerySource::Points { query, .. } => query.matches.len(),
        }
    }
}

enum QuerySource<'a> {
    Channel(&'a ChannelResults),
    Points {
        results: &'a PointsResults,
        query: &'a PanelQuery,
    },
}

#[derive(Debug, PartialEq)]
pub enum MatchExtent<'a> {
    Channel { name: &'a str, times: &'a [f64] },
    Points(Range<usize>),
}

pub struct RetainedMatch<'a> {
    track: TrackRef,
    reference: SessionTrackRef,
    matched: &'a MatchValues,
    source: MatchSource<'a>,
    extent: MatchExtent<'a>,
    next: Option<MatchPosition>,
}

impl<'a> RetainedMatch<'a> {
    pub fn track(&self) -> TrackRef {
        self.track
    }

    pub fn reference(&self) -> SessionTrackRef {
        self.reference
    }

    pub fn range(&self) -> Range<usize> {
        self.matched.rows.clone()
    }

    pub fn aggregates(&self) -> &'a [Option<f64>] {
        &self.matched.aggregates
    }

    pub fn extent(&self) -> &MatchExtent<'a> {
        &self.extent
    }

    pub fn next_position(&self) -> Option<MatchPosition> {
        self.next
    }

    pub fn first_time(&self) -> Option<ResultTime> {
        self.source.time(self.matched.rows.start)
    }

    pub fn last_time(&self) -> Option<ResultTime> {
        self.matched
            .rows
            .end
            .checked_sub(1)
            .and_then(|index| self.source.time(index))
    }

    pub fn rows(&self, start: Option<usize>) -> Result<RetainedRows<'a>, RetainedResultError> {
        let start = start.unwrap_or(self.matched.rows.start);
        if !self.matched.rows.contains(&start) {
            return Err(RetainedResultError::InvalidPosition);
        }
        let point_values = match self.source {
            MatchSource::Channel { .. } => None,
            MatchSource::Points { points, data, .. } => Some(PointResultValues::new(points, data)),
        };
        Ok(RetainedRows {
            source: self.source,
            range: start..self.matched.rows.end,
            point_values,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResultTime {
    ChannelSeconds(f64),
    PointUtc(DateTime<Utc>),
}

pub enum RowColumn<'a> {
    ChannelComponent(&'a str),
    Metric(QueryMetric),
}

pub struct RetainedRows<'a> {
    source: MatchSource<'a>,
    range: Range<usize>,
    point_values: Option<PointResultValues<'a>>,
}

impl<'a> RetainedRows<'a> {
    pub fn columns(&self) -> impl Iterator<Item = (usize, RowColumn<'a>)> {
        let (metrics, components) = match self.source {
            MatchSource::Channel { result, .. } => (&[][..], result.component_labels()),
            MatchSource::Points { columns, .. } => (columns, &[][..]),
        };
        metrics
            .iter()
            .enumerate()
            .filter_map(|(ordinal, column)| match column {
                TableColumn::Metric(metric) => Some((ordinal, RowColumn::Metric(*metric))),
                TableColumn::Aggregate(_) => None,
            })
            .chain(
                components
                    .iter()
                    .enumerate()
                    .map(|(ordinal, label)| (ordinal, RowColumn::ChannelComponent(label.as_str()))),
            )
    }

    pub fn channel_unit(&self) -> Option<&'a ChannelUnit> {
        match self.source {
            MatchSource::Channel { track, .. } => track.unit.as_ref(),
            MatchSource::Points { .. } => None,
        }
    }

    pub fn iter(&self) -> RetainedRowIter<'_, 'a> {
        RetainedRowIter {
            rows: self,
            indices: self.range.clone(),
        }
    }
}

impl<'page, 'data> IntoIterator for &'page RetainedRows<'data> {
    type Item = RetainedRow<'page, 'data>;
    type IntoIter = RetainedRowIter<'page, 'data>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct RetainedRowIter<'page, 'data> {
    rows: &'page RetainedRows<'data>,
    indices: Range<usize>,
}

impl<'page, 'data> Iterator for RetainedRowIter<'page, 'data> {
    type Item = RetainedRow<'page, 'data>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(RetainedRow {
            index: self.indices.next()?,
            rows: self.rows,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl ExactSizeIterator for RetainedRowIter<'_, '_> {}

pub struct RetainedRow<'page, 'data> {
    index: usize,
    rows: &'page RetainedRows<'data>,
}

impl RetainedRow<'_, '_> {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn time(&self) -> Option<ResultTime> {
        self.rows.source.time(self.index)
    }

    pub fn value(&self, column: usize) -> Option<f64> {
        match self.rows.source {
            MatchSource::Channel { track, .. } => track.timeline.value(self.index, column),
            MatchSource::Points { columns, .. } => match columns.get(column)? {
                TableColumn::Metric(metric) => {
                    self.rows.point_values.as_ref()?.value(*metric, self.index)
                }
                TableColumn::Aggregate(_) => None,
            },
        }
    }
}

#[derive(Clone, Copy)]
enum MatchSource<'a> {
    Channel {
        result: &'a ChannelResults,
        track: &'a ChannelTrackResult,
    },
    Points {
        points: &'a [NavPoint],
        data: Option<&'a TrackQueryData>,
        columns: &'a [TableColumn],
    },
}

impl<'a> MatchSource<'a> {
    fn extent(&self, rows: Range<usize>) -> Result<MatchExtent<'a>, RetainedResultError> {
        Ok(match self {
            Self::Channel { result, track } => MatchExtent::Channel {
                name: &result.channel,
                times: track
                    .timeline
                    .times
                    .get(rows)
                    .ok_or(RetainedResultError::InvalidPosition)?,
            },
            Self::Points { points, .. } => {
                points
                    .get(rows.clone())
                    .ok_or(RetainedResultError::InvalidPosition)?;
                MatchExtent::Points(rows)
            }
        })
    }

    fn time(&self, index: usize) -> Option<ResultTime> {
        match self {
            Self::Channel { track, .. } => track
                .timeline
                .times
                .get(index)
                .copied()
                .filter(|seconds| seconds.is_finite())
                .map(ResultTime::ChannelSeconds),
            Self::Points { points, .. } => {
                Some(ResultTime::PointUtc(points.get(index)?.tpv.time().utc()))
            }
        }
    }
}

#[cfg(test)]
mod tests;
