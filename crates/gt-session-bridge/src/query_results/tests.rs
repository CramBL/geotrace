use std::mem;
use std::num::NonZeroUsize;

use gt_loaded_files::LoadedFiles;
use gt_query::ChannelTimeline;
use gt_query_run::{ChannelResults, ChannelTrackResult, MatchValues, RunResults};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use uuid::Uuid;

use super::{
    GetQueryResults, MAX_QUERY_COLUMN_BYTES, QueryMatchPosition, QueryPageRequest, QueryResults,
};
use crate::framing::{self, Frame, MAX_REPLY_BYTES};
use crate::protocol::{ErrorCode, Reply};
use crate::query_job::QueryJobId;

#[tokio::test]
async fn a_framed_query_result_preserves_channel_time_and_value() {
    let files = LoadedFiles::new();
    let run = self::channel_run(Vec::new());
    let results = QueryResults::project(&run, files.view(), ROW_REQUEST).unwrap();
    assert_eq!(results.columns[0].label, "sensor");
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
    let files = LoadedFiles::new();
    let label = "\"".repeat(MAX_QUERY_COLUMN_BYTES / 2);
    let run = self::channel_run(vec![label]);
    assert_eq!(
        QueryResults::project(&run, files.view(), ROW_REQUEST),
        Err(ErrorCode::QueryColumnsTooLarge)
    );
}

fn channel_run(components: Vec<String>) -> RunResults {
    RunResults::Channel(ChannelResults {
        channel: "sensor".to_owned(),
        components,
        aggregate_columns: Vec::new(),
        summary: Default::default(),
        tracks: vec![ChannelTrackResult {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            unit: None,
            matches: vec![MatchValues {
                rows: 0..1,
                aggregates: Vec::new(),
            }],
            timeline: ChannelTimeline {
                times: vec![CHANNEL_VALUE],
                values: vec![CHANNEL_VALUE],
                columns: 1,
            },
        }],
        matches: Default::default(),
    })
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
