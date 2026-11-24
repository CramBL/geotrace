use std::io::{self, ErrorKind};
use std::num::NonZeroUsize;
use std::time::Duration;

use chrono::{DateTime, TimeDelta};
use gt_query::{Diagnostic, Span};
use gt_session_bridge::{
    GetQueryResults, GetRangeDetails, ListMetrics, ListRecordings, Metrics, OpenPoint,
    PointDetails, QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus, QueryMatchPosition,
    QueryPageRequest, QueryResultColumn, QueryResultPage, QueryResultRow, QueryResultSource,
    QueryResultTime, QueryResultValue, QueryResults, QueryText, QueryValidation, RangeDetails,
    RangeTarget, Recording, RecordingReference, Recordings, Reply, Request, ShowOnMap, ShowOnPlot,
    TrackReference,
};
use gt_types::PointIdx;
use rmcp::ServiceExt as _;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use uuid::Uuid;

use super::{Bridge, COMPANION_NAME, Companion, START_DESKTOP, VERSION};

#[tokio::test]
async fn the_companion_lists_tools_and_returns_structured_results() {
    let (server, stream) = tokio::io::duplex(8192);
    let worker = tokio::spawn(async move {
        Companion {
            bridge: Bridge::Fixture(self::inspection_fixture),
        }
        .serve(server)
        .await
        .unwrap()
        .waiting()
        .await
        .unwrap();
    });
    let mut client = Client::new(stream);
    let initialized = client
        .exchange(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}
            }
        }))
        .await;
    assert_eq!(
        initialized.pointer("/result/protocolVersion"),
        Some(&json!("2025-11-25"))
    );
    assert_eq!(
        initialized.pointer("/result/serverInfo/name"),
        Some(&json!(COMPANION_NAME))
    );
    assert_eq!(
        initialized.pointer("/result/serverInfo/version"),
        Some(&json!(VERSION))
    );
    client
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;

    let listed = client
        .exchange(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await;
    let tools = listed
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .unwrap();
    let mut names: Vec<_> = tools
        .iter()
        .filter_map(|tool| tool.get("name")?.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "cancel_query",
            "check_query",
            "get_query_results",
            "get_query_status",
            "get_range_details",
            "get_recording",
            "get_track",
            "list_metrics",
            "list_recordings",
            "open_point",
            "open_query",
            "run_query",
            "show_on_map",
            "show_on_plot",
        ]
    );
    let tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("list_recordings")))
        .unwrap();
    assert_eq!(
        tool.pointer("/annotations/readOnlyHint"),
        Some(&json!(true))
    );
    assert!(tool.pointer("/inputSchema/properties/after").is_some());
    assert!(tool.pointer("/inputSchema/properties/limit").is_some());
    let query_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("check_query")))
        .unwrap();
    assert_eq!(
        query_tool.pointer("/annotations/readOnlyHint"),
        Some(&json!(true))
    );
    assert_eq!(
        query_tool.pointer("/inputSchema/required"),
        Some(&json!(["text"]))
    );

    let absent = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "list_recordings"}
        }))
        .await;
    assert_eq!(absent.pointer("/result/isError"), Some(&json!(true)));
    assert_eq!(
        absent.pointer("/result/content/0/text"),
        Some(&json!(START_DESKTOP))
    );

    let cursor = self::recording_id(6);
    let recording = self::recording_id(7);
    let page = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "list_recordings", "arguments": {"after": cursor, "limit": 1}}
        }))
        .await;
    assert_eq!(page.pointer("/result/isError"), Some(&json!(false)));
    let expected = json!({
        "recordings": [{
            "id": recording, "display_name": "Survey", "filename": "survey.gtd",
            "title": null, "device": null, "time_range": null, "track_count": 2
        }],
        "next_after": recording
    });
    assert_eq!(page.pointer("/result/structuredContent"), Some(&expected));
    assert_eq!(
        page.pointer("/result/content/0/text"),
        Some(&json!(expected.to_string()))
    );
    let invalid = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "list_recordings", "arguments": {"after": cursor, "limit": 0}}
        }))
        .await;
    assert_eq!(invalid.pointer("/result/isError"), Some(&json!(true)));
    assert!(
        invalid
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("failed to deserialize parameters"))
    );
    let points = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": {"name": "get_range_details", "arguments": {"track": self::track_reference()}}
        }))
        .await;
    assert_eq!(points.pointer("/result/isError"), Some(&json!(false)));
    assert_eq!(
        points.pointer("/result/structuredContent/points/0/time"),
        Some(&json!("1970-01-01T00:00:00.000000021Z"))
    );
    let metrics = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": "list_metrics", "arguments": {"channel_start": 3, "channel_limit": 1}}
        }))
        .await;
    assert_eq!(metrics.pointer("/result/isError"), Some(&json!(false)));
    assert_eq!(
        metrics.pointer("/result/structuredContent"),
        Some(&json!({"definitions": [], "channels": [], "next_channel_start": 4}))
    );
    let checked = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": {"name": "check_query", "arguments": {"text": "points | where speed > 30 km/h"}}
        }))
        .await;
    assert_eq!(checked.pointer("/result/isError"), Some(&json!(false)));
    assert_eq!(
        checked.pointer("/result/structuredContent"),
        Some(&json!({
            "status": "invalid", "diagnostics": [{
                "message": "unknown metric `speed`", "help": "Use velocity", "span": {"start": 15, "end": 20}
            }]
        }))
    );
    let submitted = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": {"name": "run_query", "arguments": {"text": "points | draw"}}
        }))
        .await;
    let job = submitted.pointer("/result/structuredContent/job").unwrap();
    assert_eq!(
        submitted.pointer("/result/structuredContent"),
        Some(&json!({"job": self::query_job_id(), "status": "preparing"}))
    );
    let status = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": {"name": "get_query_status", "arguments": {"job": job}}
        }))
        .await;
    assert_eq!(
        status.pointer("/result/structuredContent"),
        Some(&json!({
            "job": job, "status": "running", "tracks_prepared": 2, "track_total": 3
        }))
    );
    let cancelled = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": {"name": "cancel_query", "arguments": {"job": job}}
        }))
        .await;
    assert_eq!(
        cancelled.pointer("/result/structuredContent"),
        Some(&json!({"job": job, "status": "cancelling"}))
    );
    let completed_job = QueryJobId {
        id: Uuid::from_u128(4),
        ..self::query_job_id()
    };
    let matched = QueryMatchPosition::default();
    let rows = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 12, "method": "tools/call",
            "params": {"name": "get_query_results", "arguments": {
                "job": completed_job, "query": 0, "limit": 1,
                "page": {"kind": "rows", "matched": matched, "start": 3}
            }}
        }))
        .await;
    assert_eq!(
        rows.pointer("/result/structuredContent"),
        Some(&json!({
            "job": completed_job, "query": 0, "source": {"kind": "channel", "channel": "accel"},
            "columns": [{"ordinal": 0, "label": "x", "unit": "g"}],
            "page": {"kind": "rows", "matched": matched, "next_start": 4, "rows": [{
                "index": 3, "time": 0.000000021, "values": [{"status": "non_finite"}]
            }]}
        }))
    );
    self::assert_ui_tools_queue_requests(&mut client).await;
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
}

async fn assert_ui_tools_queue_requests(client: &mut Client) {
    let track = self::track_reference();
    let commands = [
        (
            "open_point",
            json!({"track": track, "point": 1}),
            "map_presentation_queued",
        ),
        (
            "show_on_map",
            json!({"target": {"kind": "track", "track": track, "start": 1, "end": 2}}),
            "map_presentation_queued",
        ),
        (
            "show_on_map",
            json!({"target": {
                "kind": "query_match", "job": self::query_job_id(), "query": 1,
                "matched": {"track_ordinal": 2, "match_ordinal": 3}
            }}),
            "map_presentation_queued",
        ),
        (
            "show_on_plot",
            json!({"target": {"kind": "track", "track": track, "start": 1, "end": 2}, "metrics": ["velocity"]}),
            "plot_presentation_queued",
        ),
        (
            "open_query",
            json!({"text": INVALID_QUERY_DRAFT}),
            "query_draft_queued",
        ),
    ];
    for (index, (name, arguments, status)) in commands.into_iter().enumerate() {
        let presented = client
            .exchange(json!({
                "jsonrpc": "2.0", "id": 13 + index, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }))
            .await;
        assert_eq!(
            presented.pointer("/result/structuredContent"),
            Some(&json!({"status": status}))
        );
    }
}

fn inspection_fixture(request: Request) -> io::Result<Reply> {
    match request {
        Request::OpenPoint(OpenPoint { track, point: 1 })
        | Request::ShowOnMap(ShowOnMap {
            target:
                RangeTarget::Track {
                    track,
                    start: Some(1),
                    end: Some(2),
                },
        }) if track == self::track_reference() => Ok(Reply::MapPresentationQueued),
        Request::OpenQuery(QueryText { text }) if text == INVALID_QUERY_DRAFT => {
            Ok(Reply::QueryDraftQueued)
        }
        Request::ShowOnPlot(ShowOnPlot {
            target:
                RangeTarget::Track {
                    track,
                    start: Some(1),
                    end: Some(2),
                },
            metrics,
        }) if track == self::track_reference() && metrics == ["velocity"] => {
            Ok(Reply::PlotPresentationQueued)
        }
        Request::ShowOnMap(ShowOnMap {
            target:
                RangeTarget::QueryMatch {
                    job,
                    query: 1,
                    matched:
                        QueryMatchPosition {
                            track_ordinal: 2,
                            match_ordinal: 3,
                        },
                },
        }) if job == self::query_job_id() => Ok(Reply::MapPresentationQueued),
        Request::GetQueryResults(GetQueryResults {
            job,
            query: 0,
            limit: Some(NonZeroUsize::MIN),
            page:
                QueryPageRequest::Rows {
                    matched,
                    start: Some(3),
                },
        }) if job.session == self::query_job_id().session
            && job.id == Uuid::from_u128(4)
            && matched == QueryMatchPosition::default() =>
        {
            Ok(Reply::QueryResults {
                results: QueryResults {
                    job,
                    query: 0,
                    source: QueryResultSource::Channel {
                        channel: "accel".to_owned(),
                    },
                    columns: vec![QueryResultColumn {
                        ordinal: 0,
                        label: "x".to_owned(),
                        unit: Some("g".to_owned()),
                    }],
                    page: QueryResultPage::Rows {
                        matched,
                        next_start: Some(4),
                        rows: vec![QueryResultRow {
                            index: 3,
                            time: Some(QueryResultTime::UnixSeconds(0.000000021)),
                            values: vec![QueryResultValue::from(Some(f64::NAN))],
                        }],
                    },
                },
            })
        }
        Request::RunQuery(QueryText { text }) if text == "points | draw" => Ok(Reply::QueryJob {
            job: QueryJobStatus {
                job: self::query_job_id(),
                state: QueryJobState::Preparing,
            },
        }),
        Request::GetQueryStatus(QueryJobRequest { job }) if job == self::query_job_id() => {
            Ok(Reply::QueryJob {
                job: QueryJobStatus {
                    job,
                    state: QueryJobState::Running {
                        tracks_prepared: 2,
                        track_total: 3,
                    },
                },
            })
        }
        Request::CancelQuery(QueryJobRequest { job }) if job == self::query_job_id() => {
            Ok(Reply::QueryJob {
                job: QueryJobStatus {
                    job,
                    state: QueryJobState::Cancelling,
                },
            })
        }
        Request::CheckQuery(QueryText { text }) if text == "points | where speed > 30 km/h" => {
            Ok(Reply::QueryCheck {
                validation: QueryValidation::Invalid {
                    diagnostics: vec![Diagnostic {
                        message: "unknown metric `speed`".to_owned(),
                        help: Some("Use velocity".to_owned()),
                        span: Span::new(15, 20),
                    }],
                },
            })
        }
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }) => Err(ErrorKind::NotFound.into()),
        Request::ListRecordings(ListRecordings {
            after: Some(after),
            limit: Some(NonZeroUsize::MIN),
        }) if after == self::recording_id(6) => Ok(self::recording_page()),
        Request::ListMetrics(ListMetrics {
            channel_start: Some(3),
            channel_limit: Some(NonZeroUsize::MIN),
        }) => Ok(Reply::Metrics {
            page: Metrics {
                definitions: Vec::new(),
                channels: Vec::new(),
                next_channel_start: Some(4),
            },
        }),
        Request::GetRangeDetails(GetRangeDetails {
            track,
            start: None,
            end: None,
            limit: None,
        }) if track == self::track_reference() => Ok(Reply::Range {
            details: RangeDetails {
                points: vec![PointDetails {
                    index: PointIdx::new(0),
                    time: DateTime::UNIX_EPOCH + TimeDelta::nanoseconds(21),
                    gps_time: None,
                    system_time: None,
                    position: None,
                    eph_m: None,
                    velocity_mps: None,
                }],
                next_start: None,
            },
        }),
        _ => Err(ErrorKind::InvalidInput.into()),
    }
}

fn query_job_id() -> QueryJobId {
    QueryJobId {
        session: Uuid::from_u128(1),
        id: Uuid::from_u128(3),
    }
}

fn recording_page() -> Reply {
    Reply::Recordings {
        page: Recordings {
            recordings: vec![Recording {
                id: self::recording_id(7),
                display_name: "Survey".to_owned(),
                filename: "survey.gtd".to_owned(),
                title: None,
                device: None,
                time_range: None,
                track_count: 2,
            }],
            next_after: Some(self::recording_id(7)),
        },
    }
}

fn recording_id(number: u64) -> RecordingReference {
    RecordingReference {
        session: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        number,
    }
}

fn track_reference() -> TrackReference {
    TrackReference {
        recording: self::recording_id(7),
        index: 0,
        revision: 2,
    }
}

struct Client {
    input: Lines<BufReader<ReadHalf<DuplexStream>>>,
    output: WriteHalf<DuplexStream>,
}

impl Client {
    fn new(stream: DuplexStream) -> Self {
        let (input, output) = tokio::io::split(stream);
        Self {
            input: BufReader::new(input).lines(),
            output,
        }
    }

    async fn exchange(&mut self, message: Value) -> Value {
        self.send(message).await;
        let line = tokio::time::timeout(Duration::from_secs(2), self.input.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn send(&mut self, message: Value) {
        let line = format!("{message}\n");
        self.output.write_all(line.as_bytes()).await.unwrap();
    }
}

const INVALID_QUERY_DRAFT: &str = "# Første\r\n\r\npoints | where speed > 10 km/h";
