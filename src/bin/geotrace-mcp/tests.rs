use std::io::{self, ErrorKind};
use std::num::NonZeroUsize;
use std::time::Duration;

use chrono::{DateTime, TimeDelta};
use gt_session_bridge::{
    GetRangeDetails, ListMetrics, ListRecordings, Metrics, PointDetails, RangeDetails, Recording,
    RecordingReference, Recordings, Reply, Request, TrackReference,
};
use gt_types::PointIdx;
use rmcp::ServiceExt as _;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};

use super::{Bridge, COMPANION_NAME, Companion, START_DESKTOP, VERSION};

#[tokio::test]
async fn the_companion_lists_inspection_tools_and_returns_structured_pages() {
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
            "get_range_details",
            "get_recording",
            "get_track",
            "list_metrics",
            "list_recordings"
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
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
}

fn inspection_fixture(request: Request) -> io::Result<Reply> {
    match request {
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
