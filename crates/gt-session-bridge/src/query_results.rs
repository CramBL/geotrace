use std::num::NonZeroUsize;
use std::ops::Range;

use chrono::{DateTime, Utc};
use gt_query::{AggregateColumn, Quantity, Unit};
use gt_query_run::{
    CurrentRetainedResults, MatchPosition, ResultTime, RetainedQuery, RetainedResultError,
    RowColumn,
};
use serde::{Deserialize, Serialize};

use crate::framing::{Frame, MAX_REPLY_BYTES};
use crate::protocol::{ErrorCode, NumericObservation, TrackReference};
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

impl From<QueryMatchPosition> for MatchPosition {
    fn from(
        QueryMatchPosition {
            track_ordinal,
            match_ordinal,
        }: QueryMatchPosition,
    ) -> Self {
        Self {
            track_ordinal,
            match_ordinal,
        }
    }
}

impl From<MatchPosition> for QueryMatchPosition {
    fn from(
        MatchPosition {
            track_ordinal,
            match_ordinal,
        }: MatchPosition,
    ) -> Self {
        Self {
            track_ordinal,
            match_ordinal,
        }
    }
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
        results: &CurrentRetainedResults<'_>,
        args: GetQueryResults,
    ) -> Result<Self, ErrorCode> {
        let query = results.query(args.query)?;
        let (columns, page) = match args.page {
            QueryPageRequest::Matches { start } => self::matches(&query, args, start)?,
            QueryPageRequest::Rows { matched, start } => {
                self::rows(&query, args.limit, matched, start)?
            }
        };
        Frame::encode(&columns, MAX_QUERY_COLUMN_BYTES)
            .map_err(|_error| ErrorCode::QueryColumnsTooLarge)?;
        Ok(Self {
            job: args.job,
            query: args.query,
            source: query
                .channel_name()
                .map_or(QueryResultSource::Points, |channel| {
                    QueryResultSource::Channel {
                        channel: channel.to_owned(),
                    }
                }),
            columns,
            page,
        })
    }
}

impl From<RetainedResultError> for ErrorCode {
    fn from(error: RetainedResultError) -> Self {
        match error {
            RetainedResultError::InvalidPosition => Self::InvalidQueryCursor,
            RetainedResultError::Stale => Self::StaleQueryResults,
            RetainedResultError::StaleTrack => Self::StaleTrack,
            RetainedResultError::Unavailable => Self::QueryResultsUnavailable,
        }
    }
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
    pub aggregates: Vec<NumericObservation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryResultRow {
    pub index: usize,
    pub time: Option<QueryResultTime>,
    pub values: Vec<NumericObservation>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum QueryResultTime {
    UnixSeconds(f64),
    Utc(DateTime<Utc>),
}

impl From<ResultTime> for QueryResultTime {
    fn from(time: ResultTime) -> Self {
        match time {
            ResultTime::ChannelSeconds(seconds) => Self::UnixSeconds(seconds),
            ResultTime::PointUtc(time) => Self::Utc(time),
        }
    }
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

fn matches(
    query: &RetainedQuery<'_>,
    args: GetQueryResults,
    start: Option<QueryMatchPosition>,
) -> Result<(Vec<QueryResultColumn>, QueryResultPage), ErrorCode> {
    let columns: Vec<_> = query
        .aggregate_columns()
        .map(|(ordinal, aggregate)| QueryResultColumn::aggregate(ordinal, aggregate))
        .collect();
    let limit = self::page_limit(args.limit, columns.len())?;
    let mut matches = Vec::new();
    let mut next = start
        .map(MatchPosition::from)
        .or_else(|| query.first_match());
    while let Some(position) = next.filter(|_| matches.len() < limit) {
        let matched = query.match_at(position)?;
        matches.push(QueryMatch {
            position: position.into(),
            track: TrackReference::new(args.job.session, matched.reference()),
            range: matched.range(),
            first_time: matched.first_time().map(Into::into),
            last_time: matched.last_time().map(Into::into),
            aggregates: matched
                .aggregates()
                .iter()
                .copied()
                .map(Into::into)
                .collect(),
        });
        next = matched.next_position();
    }
    Ok((
        columns,
        QueryResultPage::Matches {
            matches,
            next_start: next.map(Into::into),
        },
    ))
}

fn rows(
    query: &RetainedQuery<'_>,
    limit: Option<NonZeroUsize>,
    position: QueryMatchPosition,
    start: Option<usize>,
) -> Result<(Vec<QueryResultColumn>, QueryResultPage), ErrorCode> {
    let matched = query.match_at(position.into())?;
    let page_rows = matched.rows(start)?;
    let unit = page_rows.channel_unit();
    let label = unit.map(|unit| unit.label());
    let from_base = unit
        .and_then(|unit| unit.as_recognized())
        .map_or(1.0, Unit::from_base);
    let mut remaining = MAX_QUERY_COLUMN_BYTES;
    let columns = page_rows
        .columns()
        .take(MAX_QUERY_VALUES_PER_PAGE + 1)
        .map(|(ordinal, column)| {
            Ok(match column {
                RowColumn::ChannelComponent(component) => {
                    remaining = remaining
                        .checked_sub(component.len())
                        .and_then(|bytes| bytes.checked_sub(label.map_or(0, str::len)))
                        .ok_or(ErrorCode::QueryColumnsTooLarge)?;
                    QueryResultColumn {
                        ordinal,
                        label: component.to_owned(),
                        unit: label.map(str::to_owned),
                    }
                }
                RowColumn::Metric(metric) => {
                    QueryResultColumn::new(ordinal, metric.to_string(), Some(metric.quantity()))
                }
            })
        })
        .collect::<Result<Vec<_>, ErrorCode>>()?;
    let limit = self::page_limit(limit, columns.len())?;
    let mut row_source = (&page_rows).into_iter();
    let rows = row_source
        .by_ref()
        .take(limit)
        .map(|row| QueryResultRow {
            index: row.index(),
            time: row.time().map(Into::into),
            values: columns
                .iter()
                .map(|column| {
                    row.value(column.ordinal)
                        .map(|value| value * from_base)
                        .into()
                })
                .collect(),
        })
        .collect();
    Ok((
        columns,
        QueryResultPage::Rows {
            matched: position,
            rows,
            next_start: row_source.next().map(|row| row.index()),
        },
    ))
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
