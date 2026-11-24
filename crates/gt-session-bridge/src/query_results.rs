use std::num::NonZeroUsize;
use std::ops::Range;

use chrono::{DateTime, Utc};
use gt_loaded_files::LoadedFilesView;
use gt_query::{AggregateColumn, Quantity, TableColumn, Unit};
use gt_query_run::{
    ChannelResults, ChannelTrackResult, MatchValues, PanelQuery, PointResultValues, PointsResults,
    RunResults, TrackQueryData,
};
use gt_types::{NavPoint, TrackRef};
use serde::{Deserialize, Serialize};

use crate::framing::{Frame, MAX_REPLY_BYTES};
use crate::protocol::{ErrorCode, TrackReference};
use crate::query_job::QueryJobId;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GetQueryResults {
    pub job: QueryJobId,
    pub query: usize,
    pub page: QueryPageRequest,
    pub limit: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueryPageRequest {
    Matches {
        start: Option<QueryMatchPosition>,
    },
    Rows {
        matched: QueryMatchPosition,
        start: Option<usize>,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct QueryMatchPosition {
    pub track_ordinal: usize,
    pub match_ordinal: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryResults {
    pub job: QueryJobId,
    pub query: usize,
    pub source: QueryResultSource,
    pub columns: Vec<QueryResultColumn>,
    pub page: QueryResultPage,
}

impl QueryResults {
    pub fn project(
        results: &RunResults,
        files: LoadedFilesView<'_>,
        args: GetQueryResults,
    ) -> Result<Self, ErrorCode> {
        let query = ResultQuery::select(results, args.query)?;
        let (columns, page) = match args.page {
            QueryPageRequest::Matches { start } => query.matches(files, args, start)?,
            QueryPageRequest::Rows { matched, start } => query.rows(files, args, matched, start)?,
        };
        Frame::encode(&columns, MAX_QUERY_COLUMN_BYTES)
            .map_err(|_error| ErrorCode::QueryColumnsTooLarge)?;
        Ok(Self {
            job: args.job,
            query: args.query,
            source: match query {
                ResultQuery::Channel(result) => QueryResultSource::Channel {
                    channel: result.channel.clone(),
                },
                ResultQuery::Points { .. } => QueryResultSource::Points,
            },
            columns,
            page,
        })
    }
}

pub struct QueryMatchRange<'a> {
    pub track: TrackRef,
    pub extent: QueryMatchExtent<'a>,
}

impl<'a> QueryMatchRange<'a> {
    pub fn resolve(
        results: &'a RunResults,
        files: LoadedFilesView<'a>,
        query: usize,
        position: QueryMatchPosition,
    ) -> Result<Self, ErrorCode> {
        let query = ResultQuery::select(results, query)?;
        let track = query.track(files, position.track_ordinal)?;
        let matched = track.matched(position)?;
        let extent = match track.source {
            MatchSource::Channel { channel, track } => QueryMatchExtent::Channel {
                name: &channel.channel,
                times: track
                    .timeline
                    .times
                    .get(matched.rows.clone())
                    .ok_or(ErrorCode::InvalidQueryCursor)?,
            },
            MatchSource::Points { .. } => QueryMatchExtent::Points(matched.rows.clone()),
        };
        Ok(Self {
            track: track.track,
            extent,
        })
    }
}

pub enum QueryMatchExtent<'a> {
    Channel { name: &'a str, times: &'a [f64] },
    Points(Range<usize>),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryResultSource {
    Channel { channel: String },
    Points,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryResultPage {
    Matches {
        matches: Vec<QueryMatch>,
        next_start: Option<QueryMatchPosition>,
    },
    Rows {
        matched: QueryMatchPosition,
        rows: Vec<QueryResultRow>,
        next_start: Option<usize>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryMatch {
    pub position: QueryMatchPosition,
    pub track: TrackReference,
    pub range: Range<usize>,
    pub first_time: Option<QueryResultTime>,
    pub last_time: Option<QueryResultTime>,
    pub aggregates: Vec<QueryResultValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryResultRow {
    pub index: usize,
    pub time: Option<QueryResultTime>,
    pub values: Vec<QueryResultValue>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum QueryResultTime {
    UnixSeconds(f64),
    Utc(DateTime<Utc>),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryResultColumn {
    pub ordinal: usize,
    pub label: String,
    pub unit: Option<String>,
}

impl QueryResultColumn {
    fn aggregate(ordinal: usize, aggregate: &AggregateColumn) -> Self {
        Self::new(ordinal, aggregate.label().to_owned(), aggregate.quantity())
    }

    fn new(ordinal: usize, label: String, quantity: Option<Quantity>) -> Self {
        let unit = quantity.and_then(|quantity| match quantity {
            Quantity::Acceleration => Some(Unit::M_PER_S2.label()),
            Quantity::Angle | Quantity::WrappingAngle => Some(Unit::DEG.label()),
            Quantity::Duration | Quantity::Timestamp => Some(Unit::S.label()),
            Quantity::Length => Some(Unit::M.label()),
            Quantity::Rate => Some(Unit::PER_S.label()),
            Quantity::Speed => Some(Unit::M_PER_S.label()),
            Quantity::Condition | Quantity::Count | Quantity::Index | Quantity::Ratio => None,
        });
        Self {
            ordinal,
            label,
            unit: unit.map(str::to_owned),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QueryResultValue {
    Finite { value: f64 },
    Missing,
    NonFinite,
}

impl From<Option<f64>> for QueryResultValue {
    fn from(value: Option<f64>) -> Self {
        match value {
            Some(value) if value.is_finite() => Self::Finite { value },
            Some(_) => Self::NonFinite,
            None => Self::Missing,
        }
    }
}

enum ResultQuery<'a> {
    Channel(&'a ChannelResults),
    Points {
        results: &'a PointsResults,
        query: &'a PanelQuery,
    },
}

impl<'a> ResultQuery<'a> {
    fn select(results: &'a RunResults, query: usize) -> Result<Self, ErrorCode> {
        if results.stale() {
            return Err(ErrorCode::StaleQueryResults);
        }
        match results {
            RunResults::Channel(channel) if query == 0 => Ok(Self::Channel(channel)),
            RunResults::Channel(_) => Err(ErrorCode::InvalidQueryCursor),
            RunResults::Points(results) => Ok(Self::Points {
                results,
                query: results
                    .queries
                    .get(query)
                    .ok_or(ErrorCode::InvalidQueryCursor)?,
            }),
        }
    }

    fn matches(
        &self,
        files: LoadedFilesView<'_>,
        args: GetQueryResults,
        start: Option<QueryMatchPosition>,
    ) -> Result<(Vec<QueryResultColumn>, QueryResultPage), ErrorCode> {
        let columns = self.aggregate_columns();
        let limit = self::page_limit(args.limit, columns.len())?;
        let mut matches = Vec::new();
        let mut next_start =
            start.or_else(|| (self.track_count() > 0).then(QueryMatchPosition::default));
        while let Some(position) = next_start.filter(|_| matches.len() < limit) {
            let matched_track = self.track(files, position.track_ordinal)?;
            let matched = matched_track.matched(position)?;
            let track = files
                .session_track_ref(matched_track.track)
                .map(|track| TrackReference::new(args.job.session, track))
                .ok_or(ErrorCode::StaleTrack)?;
            let first_time = matched_track.source.time(matched.rows.start);
            let last_time = matched
                .rows
                .end
                .checked_sub(1)
                .and_then(|index| matched_track.source.time(index));
            matches.push(QueryMatch {
                position,
                track,
                range: matched.rows.clone(),
                first_time,
                last_time,
                aggregates: matched.aggregates.iter().copied().map(Into::into).collect(),
            });
            next_start = self.next_match(position, matched_track.matches.len());
        }
        Ok((
            columns,
            QueryResultPage::Matches {
                matches,
                next_start,
            },
        ))
    }

    fn rows(
        &self,
        files: LoadedFilesView<'_>,
        args: GetQueryResults,
        position: QueryMatchPosition,
        start: Option<usize>,
    ) -> Result<(Vec<QueryResultColumn>, QueryResultPage), ErrorCode> {
        let track = self.track(files, position.track_ordinal)?;
        let matched = track.matched(position)?;
        let start = start.unwrap_or(matched.rows.start);
        if !matched.rows.contains(&start) {
            return Err(ErrorCode::InvalidQueryCursor);
        }
        let range = start..matched.rows.end;
        match track.source {
            MatchSource::Channel {
                channel,
                track: samples,
            } => {
                let columns = self::channel_columns(channel, samples)?;
                let component_count = columns.len();
                let from_base = samples
                    .unit
                    .as_ref()
                    .and_then(|unit| unit.as_recognized())
                    .map_or(1.0, Unit::from_base);
                track
                    .source
                    .row_page(position, range, args.limit, columns, |index| {
                        (0..component_count)
                            .map(|component| {
                                samples
                                    .timeline
                                    .value(index, component)
                                    .map(|value| value * from_base)
                                    .into()
                            })
                            .collect()
                    })
            }
            MatchSource::Points {
                points,
                data,
                columns,
            } => {
                let metrics: Vec<_> = columns
                    .iter()
                    .enumerate()
                    .filter_map(|(ordinal, column)| match column {
                        TableColumn::Metric(metric) => Some((ordinal, *metric)),
                        TableColumn::Aggregate(_) => None,
                    })
                    .collect();
                let columns = metrics
                    .iter()
                    .map(|&(ordinal, metric)| {
                        QueryResultColumn::new(ordinal, metric.to_string(), Some(metric.quantity()))
                    })
                    .collect();
                let values = PointResultValues::new(points, data);
                track
                    .source
                    .row_page(position, range, args.limit, columns, |index| {
                        metrics
                            .iter()
                            .map(|&(_, metric)| values.value(metric, index).into())
                            .collect()
                    })
            }
        }
    }

    fn aggregate_columns(&self) -> Vec<QueryResultColumn> {
        match self {
            Self::Channel(channel) => {
                let offset = channel.component_labels().len();
                channel
                    .aggregate_columns
                    .iter()
                    .enumerate()
                    .map(|(index, aggregate)| {
                        QueryResultColumn::aggregate(offset + index, aggregate)
                    })
                    .collect()
            }
            Self::Points { query, .. } => query
                .columns
                .iter()
                .enumerate()
                .filter_map(|(index, column)| match column {
                    TableColumn::Aggregate(aggregate) => {
                        Some(QueryResultColumn::aggregate(index, aggregate))
                    }
                    TableColumn::Metric(_) => None,
                })
                .collect(),
        }
    }

    fn track_count(&self) -> usize {
        match self {
            Self::Channel(result) => result.tracks.len(),
            Self::Points { query, .. } => query.matches.len(),
        }
    }

    fn next_match(
        &self,
        mut position: QueryMatchPosition,
        match_count: usize,
    ) -> Option<QueryMatchPosition> {
        position.match_ordinal += 1;
        if position.match_ordinal == match_count {
            position.match_ordinal = 0;
            position.track_ordinal += 1;
        }
        (position.track_ordinal < self.track_count()).then_some(position)
    }

    fn track(
        &self,
        files: LoadedFilesView<'a>,
        index: usize,
    ) -> Result<MatchedTrack<'a>, ErrorCode> {
        match self {
            Self::Channel(result) => {
                let track = result
                    .tracks
                    .get(index)
                    .ok_or(ErrorCode::InvalidQueryCursor)?;
                Ok(MatchedTrack {
                    track: track.track,
                    matches: &track.matches,
                    source: MatchSource::Channel {
                        channel: result,
                        track,
                    },
                })
            }
            Self::Points { results, query } => {
                let track = query
                    .matches
                    .get(index)
                    .ok_or(ErrorCode::InvalidQueryCursor)?;
                let loaded = track
                    .track
                    .resolve(files.files())
                    .ok_or(ErrorCode::StaleTrack)?;
                Ok(MatchedTrack {
                    track: track.track,
                    matches: &track.matches,
                    source: MatchSource::Points {
                        points: &loaded.points,
                        data: results.track_data(track.track),
                        columns: &query.columns,
                    },
                })
            }
        }
    }
}

struct MatchedTrack<'a> {
    track: TrackRef,
    matches: &'a [MatchValues],
    source: MatchSource<'a>,
}

impl MatchedTrack<'_> {
    fn matched(&self, position: QueryMatchPosition) -> Result<&MatchValues, ErrorCode> {
        self.matches
            .get(position.match_ordinal)
            .ok_or(ErrorCode::InvalidQueryCursor)
    }
}

enum MatchSource<'a> {
    Channel {
        channel: &'a ChannelResults,
        track: &'a ChannelTrackResult,
    },
    Points {
        points: &'a [NavPoint],
        data: Option<&'a TrackQueryData>,
        columns: &'a [TableColumn],
    },
}

impl MatchSource<'_> {
    fn row_page(
        &self,
        matched: QueryMatchPosition,
        range: Range<usize>,
        limit: Option<NonZeroUsize>,
        columns: Vec<QueryResultColumn>,
        values: impl Fn(usize) -> Vec<QueryResultValue>,
    ) -> Result<(Vec<QueryResultColumn>, QueryResultPage), ErrorCode> {
        let limit = self::page_limit(limit, columns.len())?;
        let end = range.start.saturating_add(limit).min(range.end);
        let rows = (range.start..end)
            .map(|index| QueryResultRow {
                index,
                time: self.time(index),
                values: values(index),
            })
            .collect();
        Ok((
            columns,
            QueryResultPage::Rows {
                matched,
                rows,
                next_start: (end < range.end).then_some(end),
            },
        ))
    }

    fn time(&self, index: usize) -> Option<QueryResultTime> {
        match self {
            Self::Channel { track, .. } => track
                .timeline
                .times
                .get(index)
                .copied()
                .filter(|seconds| seconds.is_finite())
                .map(QueryResultTime::UnixSeconds),
            Self::Points { points, .. } => {
                Some(QueryResultTime::Utc(points.get(index)?.tpv.time().utc()))
            }
        }
    }
}

fn channel_columns(
    channel: &ChannelResults,
    track: &ChannelTrackResult,
) -> Result<Vec<QueryResultColumn>, ErrorCode> {
    let labels = channel.component_labels();
    if labels.len() > MAX_QUERY_VALUES_PER_PAGE {
        return Err(ErrorCode::QueryColumnsTooLarge);
    }
    let unit = track.unit.as_ref().map(|unit| unit.label());
    labels
        .iter()
        .try_fold(MAX_QUERY_COLUMN_BYTES, |remaining, label| {
            remaining
                .checked_sub(label.len())?
                .checked_sub(unit.map_or(0, str::len))
        })
        .ok_or(ErrorCode::QueryColumnsTooLarge)?;
    let columns: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(ordinal, label)| QueryResultColumn {
            ordinal,
            label: label.clone(),
            unit: unit.map(str::to_owned),
        })
        .collect();
    Ok(columns)
}

fn page_limit(limit: Option<NonZeroUsize>, columns: usize) -> Result<usize, ErrorCode> {
    if columns > MAX_QUERY_VALUES_PER_PAGE {
        return Err(ErrorCode::QueryColumnsTooLarge);
    }
    Ok(limit
        .map_or(MAX_QUERY_ROWS_PER_PAGE, NonZeroUsize::get)
        .min(MAX_QUERY_ROWS_PER_PAGE)
        .min(MAX_QUERY_VALUES_PER_PAGE / columns.max(1)))
}

const MAX_QUERY_ROWS_PER_PAGE: usize = 100;
const MAX_QUERY_VALUES_PER_PAGE: usize = 2048;
const MAX_QUERY_COLUMN_BYTES: usize = MAX_REPLY_BYTES / 4;

#[cfg(test)]
mod tests;
