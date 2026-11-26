use std::cell::LazyCell;
use std::ops::{Deref, Range};
use std::slice;

use chrono::{DateTime, Utc};
use geotrace_sdk_units::ChannelUnit;
use gt_loaded_files::{LoadedFilesView, SessionTrackRef};
use gt_query::{AggregateColumn, ChannelTimeline, QueryMetric, TableColumn, TimeSpan};
use gt_types::{FixRef, LoadedTrack, PointIdx, TrackRef};

use super::QuerySession;
use crate::provider::{PointResultValues, TrackProvider, TrackQueryData};
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

pub struct CurrentRetainedResults<'a>(RetainedResults<'a>);

impl<'a> Deref for CurrentRetainedResults<'a> {
    type Target = RetainedResults<'a>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl QuerySession {
    /// Filter, visibility and series freshness reflect the last [Self::refresh_staleness] call.
    pub fn retained_results<'a>(
        &'a self,
        files: LoadedFilesView<'a>,
    ) -> Result<CurrentRetainedResults<'a>, RetainedResultError> {
        let retained = self.displayed_results(files)?;
        if retained.results.stale() {
            return Err(RetainedResultError::Stale);
        }
        Ok(CurrentRetainedResults(retained))
    }

    /// Results remain available after filter or series changes when the recording generation matches.
    pub fn displayed_results<'a>(
        &'a self,
        files: LoadedFilesView<'a>,
    ) -> Result<RetainedResults<'a>, RetainedResultError> {
        let completed = self
            .completed
            .as_ref()
            .ok_or(RetainedResultError::Unavailable)?;
        if !completed.fingerprint.matches_loaded_files(files) {
            return Err(RetainedResultError::Stale);
        }
        Ok(RetainedResults {
            results: &completed.results,
            files,
        })
    }
}

impl<'a> RetainedResults<'a> {
    pub fn queries(&self) -> impl Iterator<Item = RetainedQuery<'a>> {
        let count = match self.results {
            RunResults::Channel(_) => 1,
            RunResults::Points(results) => results.queries.len(),
        };
        (0..count).filter_map(|ordinal| self.query(ordinal).ok())
    }

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

    pub fn columns(&self) -> impl Iterator<Item = (usize, RetainedColumn<'a>)> {
        match self.result {
            QuerySource::Channel(channel) => ResultColumns::Channel(channel),
            QuerySource::Points { query, .. } => ResultColumns::Points(&query.columns),
        }
        .columns_with_ordinals()
    }

    pub fn aggregate_columns(&self) -> impl Iterator<Item = (usize, &'a AggregateColumn)> {
        self.columns().filter_map(|(ordinal, column)| match column {
            RetainedColumn::Aggregate { column, .. } => Some((ordinal, column)),
            RetainedColumn::Value(_) => None,
        })
    }

    pub fn first_match(&self) -> Option<MatchPosition> {
        (self.track_count() > 0).then(MatchPosition::default)
    }

    pub fn matches(&self) -> impl Iterator<Item = RetainedMatch<'a>> {
        let at = |position| self.match_at(position).ok();
        std::iter::successors(self.first_match().and_then(at), move |matched| {
            matched.next_position().and_then(at)
        })
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
                        loaded,
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
            position,
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
    position: MatchPosition,
    track: TrackRef,
    reference: SessionTrackRef,
    matched: &'a MatchValues,
    source: MatchSource<'a>,
    extent: MatchExtent<'a>,
    next: Option<MatchPosition>,
}

impl<'a> RetainedMatch<'a> {
    pub fn position(&self) -> MatchPosition {
        self.position
    }

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
            MatchSource::Points { loaded, data, .. } => {
                Some(PointResultValues::new(&loaded.points, data))
            }
        };
        Ok(RetainedRows {
            source: self.source,
            track: self.track,
            aggregates: self.aggregates(),
            range: start..self.matched.rows.end,
            point_values,
        })
    }

    pub fn reduced_channels<'names>(
        &self,
        channels: &'names [&'names str],
    ) -> impl Iterator<Item = (&'names str, ReducedChannelSamples<'a>)> {
        let span = LazyCell::new(|| match self.source {
            MatchSource::Points { loaded, .. } => {
                loaded.points.get(self.range()).and_then(|points| {
                    TimeSpan::covering(
                        points
                            .iter()
                            .map(|point| point.tpv.time().as_secs_f64_with_subseconds()),
                    )
                })
            }
            MatchSource::Channel { .. } => None,
        });
        channels.iter().filter_map(move |&channel| {
            self.reduced_channel(channel, || *span)
                .map(|preview| (channel, preview))
        })
    }

    fn reduced_channel(
        &self,
        channel: &str,
        span: impl FnOnce() -> Option<TimeSpan>,
    ) -> Option<ReducedChannelSamples<'a>> {
        match self.source {
            MatchSource::Channel { result, track } => {
                (result.channel == channel).then(|| ReducedChannelSamples {
                    components: &result.components,
                    unit: track.unit.as_ref(),
                    samples: track.timeline.slice_rows(self.range()),
                })
            }
            MatchSource::Points { loaded, .. } => {
                let recorded = loaded
                    .channels
                    .iter()
                    .find(|candidate| candidate.name == channel);
                let samples = recorded
                    .and_then(|recorded| {
                        let span = span()?;
                        Some(
                            TrackProvider::new(&loaded.points, slice::from_ref(recorded), None)
                                .channel_span_timeline(channel, span),
                        )
                    })
                    .unwrap_or_default();
                Some(ReducedChannelSamples {
                    components: recorded.map_or(&[], |recorded| recorded.components.as_slice()),
                    unit: recorded.and_then(|recorded| recorded.unit.as_ref()),
                    samples,
                })
            }
        }
    }
}

pub struct ReducedChannelSamples<'a> {
    pub components: &'a [String],
    pub unit: Option<&'a ChannelUnit>,
    pub samples: ChannelTimeline,
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

pub enum RetainedColumn<'a> {
    Aggregate {
        index: usize,
        column: &'a AggregateColumn,
    },
    Value(RowColumn<'a>),
}

enum ResultColumns<'a> {
    Channel(&'a ChannelResults),
    Points(&'a [TableColumn]),
}

impl<'a> ResultColumns<'a> {
    fn columns_with_ordinals(self) -> impl Iterator<Item = (usize, RetainedColumn<'a>)> {
        let (points, components, aggregates) = match self {
            Self::Channel(channel) => (
                &[][..],
                channel.component_labels(),
                channel.aggregate_columns.as_slice(),
            ),
            Self::Points(columns) => (columns, &[][..], &[][..]),
        };
        points
            .iter()
            .enumerate()
            .scan(0, |aggregate_index, (ordinal, column)| {
                let column = match column {
                    TableColumn::Metric(metric) => {
                        RetainedColumn::Value(RowColumn::Metric(*metric))
                    }
                    TableColumn::Aggregate(column) => {
                        let index = *aggregate_index;
                        *aggregate_index += 1;
                        RetainedColumn::Aggregate { index, column }
                    }
                };
                Some((ordinal, column))
            })
            .chain(components.iter().enumerate().map(|(ordinal, name)| {
                (
                    ordinal,
                    RetainedColumn::Value(RowColumn::ChannelComponent(name)),
                )
            }))
            .chain(aggregates.iter().enumerate().map(move |(index, column)| {
                (
                    components.len() + index,
                    RetainedColumn::Aggregate { index, column },
                )
            }))
    }
}

pub struct RetainedRows<'a> {
    source: MatchSource<'a>,
    track: TrackRef,
    aggregates: &'a [Option<f64>],
    range: Range<usize>,
    point_values: Option<PointResultValues<'a>>,
}

impl<'a> RetainedRows<'a> {
    pub fn columns(&self) -> impl Iterator<Item = (usize, RowColumn<'a>)> {
        match self.source {
            MatchSource::Channel { result, .. } => ResultColumns::Channel(result),
            MatchSource::Points { columns, .. } => ResultColumns::Points(columns),
        }
        .columns_with_ordinals()
        .filter_map(|(ordinal, column)| match column {
            RetainedColumn::Aggregate { .. } => None,
            RetainedColumn::Value(value) => Some((ordinal, value)),
        })
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
        self.nth(0)
    }

    fn nth(&mut self, offset: usize) -> Option<Self::Item> {
        Some(RetainedRow {
            index: self.indices.nth(offset)?,
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

    pub fn fix(&self) -> Option<FixRef> {
        matches!(self.rows.source, MatchSource::Points { .. })
            .then(|| FixRef::new(self.rows.track, PointIdx::new(self.index)))
    }

    pub fn aggregate(&self, index: usize) -> Option<f64> {
        self.rows.aggregates.get(index).copied().flatten()
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
        loaded: &'a LoadedTrack,
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
            Self::Points { loaded, .. } => {
                loaded
                    .points
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
            Self::Points { loaded, .. } => Some(ResultTime::PointUtc(
                loaded.points.get(index)?.tpv.time().utc(),
            )),
        }
    }
}

#[cfg(test)]
mod tests;
