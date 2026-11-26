use std::mem;
use std::num::NonZeroUsize;

use chrono::DateTime;
use gt_query_run::QuerySession;
use gt_query_run::test_util::{self, LoadedState, TEST_EPOCH};
use gt_types::Channel;
use uuid::Uuid;

use super::{
    GetQueryResults, MAX_QUERY_COLUMN_BYTES, QueryMatchPosition, QueryPageRequest, QueryResultPage,
    QueryResultTime, QueryResults,
};
use crate::framing::{self, Frame, MAX_REPLY_BYTES};
use crate::protocol::{ErrorCode, NumericObservation, Reply};
use crate::query_job::QueryJobId;

#[tokio::test]
async fn a_framed_query_result_preserves_channel_time_and_value() {
    let mut channel = test_util::scalar_channel("sensor", None, &[(0, CHANNEL_VALUE)]);
    channel.times[0] = DateTime::from_timestamp(TEST_EPOCH, 750_000_000).unwrap();
    let results = self::project_channel(channel, ROW_REQUEST).unwrap();
    assert_eq!(results.columns[0].label, "sensor");
    let QueryResultPage::Rows { rows, .. } = &results.page else {
        panic!("expected rows");
    };
    assert_eq!(
        rows[0].time,
        Some(QueryResultTime::UnixSeconds(TEST_EPOCH as f64 + 0.75))
    );
    assert_eq!(
        rows[0].values,
        [NumericObservation::Finite {
            value: CHANNEL_VALUE
        }]
    );
    let expected = Reply::QueryResults { results };
    let (mut writer, mut reader) = tokio::io::duplex(MAX_REPLY_BYTES + mem::size_of::<u32>());
    Frame::encode(&expected, MAX_REPLY_BYTES)
        .unwrap()
        .write(&mut writer)
        .await
        .unwrap();
    let decoded: Reply = framing::read(&mut reader, MAX_REPLY_BYTES).await.unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn a_one_row_page_rejects_oversized_column_metadata() {
    let label = "\"".repeat(MAX_QUERY_COLUMN_BYTES / 2);
    let channel = test_util::vector_channel(
        "sensor",
        None,
        &[&label, "y", "z"],
        &[(0, [CHANNEL_VALUE; 3])],
    );
    assert_eq!(
        self::project_channel(channel, ROW_REQUEST),
        Err(ErrorCode::QueryColumnsTooLarge)
    );
}

#[test]
fn a_wide_sample_page_stops_at_the_value_budget() {
    const COMPONENTS: usize = 64;
    let samples: Vec<_> = (0..40).map(|second| (second, CHANNEL_VALUE)).collect();
    let mut channel = test_util::scalar_channel("sensor", None, &samples);
    channel.components = (0..COMPONENTS)
        .map(|index| format!("axis{index}"))
        .collect();
    channel.values = vec![CHANNEL_VALUE; samples.len() * COMPONENTS];
    let results = self::project_channel(
        channel,
        GetQueryResults {
            limit: None,
            ..ROW_REQUEST
        },
    )
    .unwrap();
    assert_eq!(results.columns.len(), COMPONENTS);
    assert_eq!(results.columns[63].label, "axis63");
    let QueryResultPage::Rows {
        rows, next_start, ..
    } = results.page
    else {
        panic!("expected rows");
    };
    assert_eq!(rows.len(), 32);
    assert_eq!(rows[31].index, 31);
    assert_eq!(rows[31].values.len(), COMPONENTS);
    assert_eq!(next_start, Some(32));
}

#[test]
fn an_empty_match_page_has_no_continuation() {
    let channel = test_util::scalar_channel("sensor", None, &[]);
    let page = |start| GetQueryResults {
        page: QueryPageRequest::Matches { start },
        ..ROW_REQUEST
    };
    let results = self::project_channel(channel.clone(), page(None)).unwrap();
    assert_eq!(
        results.page,
        QueryResultPage::Matches {
            matches: Vec::new(),
            next_start: None,
        }
    );
    assert_eq!(
        self::project_channel(channel, page(Some(QueryMatchPosition::default()))),
        Err(ErrorCode::InvalidQueryCursor),
    );
}

fn project_channel(channel: Channel, request: GetQueryResults) -> Result<QueryResults, ErrorCode> {
    let state = LoadedState::with_channels(vec![channel]);
    let mut session = QuerySession::new();
    test_util::run_text(&mut session, &state, "@sensor");
    let results = session.retained_results(state.inputs().loaded_files)?;
    QueryResults::project(&results, request)
}

const CHANNEL_VALUE: f64 = 51.248178375505404;
const ROW_REQUEST: GetQueryResults = GetQueryResults {
    job: QueryJobId {
        session: Uuid::nil(),
        id: Uuid::nil(),
    },
    query: 0,
    page: QueryPageRequest::Rows {
        matched: QueryMatchPosition {
            track_ordinal: 0,
            match_ordinal: 0,
        },
        start: None,
    },
    limit: Some(NonZeroUsize::MIN),
};
