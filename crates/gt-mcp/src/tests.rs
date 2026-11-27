use std::io::{self, ErrorKind};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta};
use gt_query::{Diagnostic, Span};
use gt_session_bridge::{
    AnalysisSettingKey, ApplyProcessing, ErrorCode, GetHistoryOpenStatus, GetQueryResults,
    GetQueryStatus, GetRangeDetails, HistoryDates, HistoryOpenId, HistoryOpenState,
    HistoryOpenStatus, HistoryRecording, HistoryRecordings, ListHistoryRecordings, ListMetrics,
    ListRecordings, MAX_RECORDING_BATCH, MAX_VISIBILITY_TRACKS, ManageRecordings, Metrics,
    NumericObservation, OpenHistoryRecording, OpenPanel, PanelOpenState, PointDetails, QueryCounts,
    QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus, QueryMatchPosition,
    QueryPageRequest, QueryResultColumn, QueryResultPage, QueryResultRow, QueryResultSource,
    QueryResultTime, QueryResults, QueryText, QueryValidation, RangeDetails, RangeTarget,
    Recording, RecordingReference, Recordings, Reply, Request, SessionPanel, SessionState,
    SetSetting, SetTrackVisibility, SettingApply, SettingInfo, SettingInput, SettingKey,
    SettingSelection, SettingUnit, SettingValue, ShowOnMap, ShowOnPlot, StatusWait, TimeRange,
    TrackReference, TrackVisibilityAction, TrackVisibilityResult,
};
use gt_types::PointIdx;
use rmcp::ServiceExt as _;
use rmcp::model::ErrorCode as McpErrorCode;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{Bridge, COMPANION_NAME, Companion, START_DESKTOP, VERSION};

#[tokio::test]
async fn status_tools_advertise_and_forward_bounded_waits() {
    let (queued, mut requests) = mpsc::channel(1);
    let (mut client, worker) = Client::start(Bridge::Pending(PendingBridge(queued)));
    client.initialize().await;
    let listing = client
        .exchange(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await;
    let tools = listing
        .pointer("/result/tools")
        .unwrap()
        .as_array()
        .unwrap();
    let job = self::query_job_id();
    let wait_ms = StatusWait::try_from(2_000).unwrap();
    let cases = [
        (
            "get_query_status",
            json!({"job": job, "wait_ms": 2_000}),
            Request::GetQueryStatus(GetQueryStatus { job, wait_ms }),
            self::query_reply(QueryJobState::Stale),
        ),
        (
            "get_history_open_status",
            json!({"operation": HISTORY_OPEN, "wait_ms": 2_000}),
            Request::GetHistoryOpenStatus(GetHistoryOpenStatus {
                operation: HISTORY_OPEN,
                wait_ms,
            }),
            Reply::HistoryOpen {
                operation: HistoryOpenStatus {
                    operation: HISTORY_OPEN,
                    state: HistoryOpenState::AwaitingTrackSettings,
                },
            },
        ),
    ];
    for (id, (method, arguments, request, reply)) in (3..).zip(cases) {
        let schema = tools
            .iter()
            .find(|tool| tool.get("name") == Some(&json!(method)))
            .and_then(|tool| tool.get("inputSchema"))
            .unwrap();
        let wait = schema.pointer("/properties/wait_ms").unwrap();
        let limits = wait
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| schema.pointer(reference.trim_start_matches('#')))
            .unwrap_or(wait);
        assert_eq!(limits.get("type"), Some(&json!("integer")));
        assert_eq!(limits.get("minimum").and_then(Value::as_f64), Some(0.0));
        assert_eq!(limits.get("maximum").and_then(Value::as_f64), Some(2_000.0));
        assert!(
            !schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("wait_ms"))
        );
        client.send_tool(id, method, arguments).await;
        let exchange = self::next_bridge_exchange(&mut requests).await;
        assert_eq!(exchange.request, request);
        exchange.reply.send(reply).unwrap();
        assert_eq!(
            client.receive().await.pointer("/result/isError"),
            Some(&json!(false))
        );
        assert!(requests.try_recv().is_err());
    }
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn invalid_waits_and_cancellation_waits_do_not_reach_geotrace() {
    let (queued, mut requests) = mpsc::channel(1);
    let (mut client, worker) = Client::start(Bridge::Pending(PendingBridge(queued)));
    client.initialize().await;
    let cases = [
        (
            "get_query_status",
            json!({"job": self::query_job_id(), "wait_ms": 2_001}),
        ),
        (
            "get_history_open_status",
            json!({"operation": HISTORY_OPEN, "wait_ms": -1}),
        ),
        (
            "cancel_query",
            json!({"job": self::query_job_id(), "wait_ms": 0}),
        ),
    ];
    for (id, (method, arguments)) in (2..).zip(cases) {
        client.send_tool(id, method, arguments).await;
        assert_eq!(
            client.receive().await.pointer("/result/isError"),
            Some(&json!(true))
        );
        assert!(requests.try_recv().is_err());
    }
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn open_panel_validates_panel_arguments_before_requesting_the_desktop() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| match request {
        Request::OpenPanel(OpenPanel::History {}) => Ok(Reply::PanelOpen {
            panel: SessionPanel::History,
            state: PanelOpenState::Frontmost,
        }),
        Request::OpenPanel(OpenPanel::Settings {}) => Ok(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::TimedOut,
        }),
        Request::OpenPanel(OpenPanel::SkyTrails { track }) => {
            assert_eq!(*track, self::track_reference());
            Ok(Reply::from(ErrorCode::NoSkyTrailReports))
        }
        _ => panic!("unexpected desktop request"),
    }));
    client.initialize().await;
    let track = self::track_reference();
    client
        .send_tool(2, "open_panel", json!({"panel": "history"}))
        .await;
    let history = client.receive().await;
    assert_eq!(
        history.pointer("/result/structuredContent"),
        Some(&json!({"status": "panel_open", "panel": "history", "state": "frontmost"}))
    );
    client
        .send_tool(
            3,
            "open_panel",
            json!({"panel": "sky_trails", "track": track}),
        )
        .await;
    let no_reports = client.receive().await;
    assert_eq!(
        no_reports.pointer("/result/content/0/text"),
        Some(&json!(
            "The track has no satellite azimuth and elevation reports for Sky trails."
        ))
    );
    client
        .send_tool(4, "open_panel", json!({"panel": "settings"}))
        .await;
    let timed_out = client.receive().await;
    assert_eq!(timed_out.pointer("/result/isError"), Some(&json!(true)));
    assert_eq!(
        timed_out.pointer("/result/structuredContent"),
        Some(&json!({"status": "panel_open", "panel": "settings", "state": "timed_out"}))
    );
    for (id, arguments) in (5..).zip([
        json!({"panel": "history", "track": track}),
        json!({"panel": "sky_trails"}),
    ]) {
        client.send_tool(id, "open_panel", arguments).await;
        let result = client.receive().await;
        assert_eq!(result.pointer("/result/isError"), Some(&json!(true)));
    }
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn set_session_view_sends_the_time_bound_at_nanosecond_precision() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| {
        let Request::SetSessionView(change) = request else {
            panic!("expected a view update");
        };
        let mut state: SessionState =
            serde_json::from_value(self::session_state_fixture()).unwrap();
        state.time_filter = change.time_filter.unwrap().window().unwrap().into();
        Ok(Reply::SessionState { state })
    }));
    client.initialize().await;
    let end = "2026-02-01T14:00:00.123456789Z";
    client
        .send_tool(
            2,
            "set_session_view",
            json!({
                "time_filter": {"action": "set", "end": end}
            }),
        )
        .await;
    let result = client.receive().await;
    assert_eq!(result.pointer("/result/isError"), Some(&json!(false)));
    assert_eq!(
        result.pointer("/result/structuredContent/time_filter"),
        Some(&json!({"kind":"bounds", "start":null, "end":end}))
    );
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn set_track_visibility_checks_bounds_and_returns_the_actual_reply() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| {
        let Request::SetTrackVisibility(SetTrackVisibility { action, tracks }) = request else {
            panic!("expected track visibility request");
        };
        assert_eq!(*action, TrackVisibilityAction::Solo);
        assert_eq!(tracks.as_ref(), [self::track_reference()]);
        Ok(Reply::TrackVisibility(TrackVisibilityResult {
            action: *action,
            targeted_tracks: 1,
            changed_tracks: 2,
        }))
    }));
    client.initialize().await;
    for (id, count) in (2..).zip([0, MAX_VISIBILITY_TRACKS + 1, 1]) {
        let reply = client
            .exchange(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "set_track_visibility", "arguments": {
                    "action": "solo", "tracks": vec![self::track_reference(); count]
                }}
            }))
            .await;
        if count == 1 {
            assert_eq!(
                reply.pointer("/result/structuredContent"),
                Some(&json!({
                    "status": "track_visibility", "action": "solo", "targeted_tracks": 1, "changed_tracks": 2
                }))
            );
        } else {
            assert_eq!(reply.pointer("/result/isError"), Some(&json!(true)));
        }
    }
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn show_on_map_reports_disabled_query_highlights() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| {
        assert_eq!(
            *request,
            Request::ShowOnMap(ShowOnMap {
                target: RangeTarget::Track {
                    track: self::track_reference(),
                    start: None,
                    end: None,
                },
            })
        );
        Ok(Reply::from(ErrorCode::QueryHighlightsDisabled))
    }));
    client.initialize().await;
    let result = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "show_on_map", "arguments": {
                "target": {"kind": "track", "track": self::track_reference()}
            }}
        }))
        .await;
    assert_eq!(result.pointer("/result/isError"), Some(&json!(true)));
    assert_eq!(
        result.pointer("/result/content/0/text"),
        Some(&json!(
            "Enable Query highlights in the map's visibility controls before showing a range."
        ))
    );
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn the_query_language_resource_is_available_without_the_desktop() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|_| {
        panic!("resource lookup sent a desktop request")
    }));
    let initialized = client.initialize().await;
    assert!(
        initialized
            .pointer("/result/capabilities/resources")
            .is_some_and(Value::is_object)
    );
    let instructions = initialized
        .pointer("/result/instructions")
        .and_then(Value::as_str)
        .unwrap();
    let uri = "geotrace://query-language";
    assert!(instructions.contains(uri));
    assert_eq!(instructions.lines().count(), 1);

    let listed = client
        .exchange(json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}))
        .await;
    let resources = listed
        .pointer("/result/resources")
        .and_then(Value::as_array)
        .unwrap();
    let [reference] = resources.as_slice() else {
        panic!("expected one query language resource");
    };
    assert_eq!(reference.get("uri"), Some(&json!(uri)));
    assert_eq!(reference.get("mimeType"), Some(&json!("text/markdown")));
    assert!(reference.get("text").is_none());

    let read = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": uri}
        }))
        .await;
    assert_eq!(
        read.pointer("/result/contents"),
        Some(&json!([{
            "uri": uri,
            "mimeType": "text/markdown",
            "text": gt_query::query_language_reference(),
        }]))
    );

    let unknown = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 4, "method": "resources/read",
            "params": {"uri": "geotrace://unknown"}
        }))
        .await;
    assert_eq!(
        unknown.pointer("/error/code"),
        Some(&json!(McpErrorCode::RESOURCE_NOT_FOUND.0))
    );
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .expect("companion shutdown")
        .expect("companion task");
}

#[tokio::test]
async fn the_companion_lists_tools_and_returns_structured_results() {
    let (mut client, worker) = Client::start(Bridge::Fixture(self::inspection_fixture));
    let initialized = client.initialize().await;
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
            "apply_processing",
            "cancel_query",
            "check_query",
            "describe_setting",
            "get_history_open_status",
            "get_query_results",
            "get_query_status",
            "get_range_details",
            "get_recording",
            "get_session_state",
            "get_settings",
            "get_track",
            "list_history_recordings",
            "list_metrics",
            "list_recordings",
            "manage_recordings",
            "open_history_recording",
            "open_panel",
            "open_point",
            "open_query",
            "open_setting",
            "run_query",
            "set_session_view",
            "set_setting",
            "set_track_visibility",
            "show_on_map",
            "show_on_plot",
        ]
    );
    let state_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("get_session_state")))
        .unwrap();
    assert_eq!(
        state_tool.pointer("/annotations/readOnlyHint"),
        Some(&json!(true))
    );
    client.send_tool(36, "get_session_state", json!({})).await;
    assert_eq!(
        client.receive().await.pointer("/result/structuredContent"),
        Some(&self::session_state_fixture())
    );
    let view_schema = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("set_session_view")))
        .and_then(|tool| tool.get("inputSchema"))
        .unwrap();
    let categories = view_schema
        .pointer("/$defs/DisplayCategory/oneOf")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        view_schema.pointer("/properties/display_categories/maxItems"),
        Some(&json!(categories.len()))
    );
    let visibility_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("set_track_visibility")))
        .unwrap();
    let schema = visibility_tool
        .pointer("/inputSchema/$defs/VisibilityTrackRefs")
        .unwrap();
    assert_eq!(schema.get("minItems"), Some(&json!(1)));
    assert_eq!(schema.get("maxItems"), Some(&json!(MAX_VISIBILITY_TRACKS)));
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
    let recording_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("get_recording")))
        .unwrap();
    assert!(
        recording_tool
            .pointer("/inputSchema/properties/after")
            .is_some()
    );
    assert!(
        recording_tool
            .pointer("/inputSchema/properties/stored_track_start")
            .is_some()
    );
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
    let setting_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("set_setting")))
        .unwrap();
    assert_eq!(
        setting_tool.pointer("/annotations/destructiveHint"),
        Some(&json!(true))
    );
    assert_eq!(
        setting_tool.pointer("/annotations/idempotentHint"),
        Some(&json!(true))
    );
    let processing_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("apply_processing")))
        .unwrap();
    assert_eq!(
        processing_tool.pointer("/annotations/destructiveHint"),
        Some(&json!(true))
    );
    assert_eq!(
        processing_tool.pointer("/annotations/idempotentHint"),
        Some(&json!(false))
    );
    assert_eq!(
        processing_tool.pointer("/inputSchema/type"),
        Some(&json!("object"))
    );
    assert_eq!(
        processing_tool.pointer("/inputSchema/additionalProperties"),
        Some(&json!(false))
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
        points.pointer("/result/structuredContent"),
        Some(&json!({
            "points": [{
                "index": 0, "time": "1970-01-01T00:00:00.000000021Z", "gps_time": null,
                "system_time": null, "position": null,
                "eph_m": {"status": "finite", "value": 2.375},
                "velocity_mps": {"status": "missing"}
            }],
            "next_start": null
        }))
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
        Some(&json!({
            "job": self::query_job_id(), "status": "completed",
            "queries": [{"matches": 2, "tracks_with_matches": 1, "skipped": 0}]
        }))
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
            "job": job, "status": "completed",
            "queries": [{"matches": 2, "tracks_with_matches": 1, "skipped": 0}]
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
    self::assert_ui_tool_replies(&mut client).await;
    self::assert_setting_tools_report_typed_values(&mut client).await;
    self::assert_processing_requires_empty_arguments(&mut client).await;
    self::assert_history_listing_uses_the_date_filter(&mut client).await;
    let opened = client.exchange(json!({
        "jsonrpc": "2.0", "id": 31, "method": "tools/call",
        "params": {"name": "open_history_recording", "arguments": {"recording": Uuid::from_u128(6)}}
    })).await;
    assert_eq!(
        opened.pointer("/result/structuredContent/operation"),
        Some(&json!(HISTORY_OPEN))
    );
    assert_eq!(
        opened.pointer("/result/structuredContent/state/kind"),
        Some(&json!("ready"))
    );
    let ready = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 32, "method": "tools/call",
            "params": {"name": "get_history_open_status", "arguments": {"operation": HISTORY_OPEN}}
        }))
        .await;
    assert_eq!(
        ready.pointer("/result/structuredContent/state"),
        Some(&json!({
            "kind": "ready", "recording": self::recording_id(7), "reused": false
        }))
    );
    self::assert_query_reply_shapes(
        &mut client,
        checked.pointer("/result/structuredContent").unwrap(),
    )
    .await;
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn recording_control_uses_typed_actions_and_reports_their_outcomes() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| match request {
        Request::ManageRecordings(ManageRecordings::Open { paths }) => {
            assert_eq!(*paths, vec![PathBuf::from("/test.GTD")].try_into().unwrap());
            Ok(Reply::FilesQueued { queued_files: 1 })
        }
        Request::ManageRecordings(ManageRecordings::Unload { recordings }) => {
            assert_eq!(*recordings, vec![self::recording_id(7)].try_into().unwrap());
            Ok(Reply::RecordingsUnloaded {
                unloaded_recordings: 1,
            })
        }
        _ => panic!("unexpected request: {request:?}"),
    }));
    client.initialize().await;
    let listed = client
        .exchange(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await;
    let file_schema = listed
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|tool| tool.get("name") == Some(&json!("manage_recordings")))
        .and_then(|tool| tool.get("inputSchema"))
        .unwrap();
    assert_eq!(file_schema.get("type"), Some(&json!("object")));
    let batch_limits: Vec<_> = file_schema
        .get("$defs")
        .and_then(Value::as_object)
        .unwrap()
        .values()
        .filter_map(|definition| {
            Some((
                definition.get("minItems")?.as_u64()?,
                definition.get("maxItems")?.as_u64()?,
            ))
        })
        .collect();
    assert_eq!(batch_limits, [(1, MAX_RECORDING_BATCH as u64); 2]);
    for arguments in [
        json!({"action": "open", "paths": []}),
        json!({"action": "open", "paths": ["/test.GTD"], "dry_run": true}),
        json!({"action": "unload", "recordings": [self::recording_id(7)], "paths": ["/test.GTD"]}),
    ] {
        client.send_tool(2, "manage_recordings", arguments).await;
        assert_eq!(
            client.receive().await.pointer("/result/isError"),
            Some(&json!(true))
        );
    }
    for (arguments, expected) in [
        (
            json!({"action": "open", "paths": ["/test.GTD"]}),
            json!({"status": "files_queued", "queued_files": 1}),
        ),
        (
            json!({"action": "unload", "recordings": [self::recording_id(7)]}),
            json!({"status": "recordings_unloaded", "unloaded_recordings": 1}),
        ),
    ] {
        client.send_tool(3, "manage_recordings", arguments).await;
        assert_eq!(
            client.receive().await.pointer("/result/structuredContent"),
            Some(&expected)
        );
    }
    drop(client);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn mcp_cancellation_drops_only_the_targeted_bridge_request() {
    let (queued, mut requests) = mpsc::channel(1);
    let (mut client, worker) = Client::start(Bridge::Pending(PendingBridge(queued)));
    client.initialize().await;
    client
        .send_tool(2, "run_query", json!({"text": "points | draw"}))
        .await;
    let initial = self::next_bridge_exchange(&mut requests).await;
    assert!(matches!(initial.request, Request::RunQuery(_)));
    let mut cancelled = initial.reply;

    client
        .send_tool(3, "open_query", json!({"text": "points | draw"}))
        .await;
    let control = self::next_bridge_exchange(&mut requests).await;
    assert!(matches!(control.request, Request::OpenQuery(_)));
    client
        .send(json!({
            "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 2}
        }))
        .await;
    tokio::time::timeout(Duration::from_secs(2), cancelled.closed())
        .await
        .expect("MCP cancellation did not close the outstanding bridge response");
    control.reply.send(Reply::QueryDraftOpened).unwrap();
    let response = client.receive().await;
    assert_eq!(response.get("id"), Some(&json!(3)));
    assert_eq!(
        response.pointer("/result/structuredContent"),
        Some(&json!({"status": "query_draft_opened"}))
    );
    drop(client);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_pending_status_finishes_the_original_exchange() {
    let (queued, mut requests) = mpsc::channel(1);
    let (mut client, worker) = Client::start(Bridge::Pending(PendingBridge(queued)));
    client.initialize().await;
    for (id, name, arguments, reply, expected) in [
        (
            2,
            "run_query",
            json!({"text": "points | draw"}),
            self::query_reply(QueryJobState::Running {
                tracks_prepared: 2,
                track_total: 3,
            }),
            json!({
                "job": self::query_job_id(), "status": "running", "tracks_prepared": 2, "track_total": 3
            }),
        ),
        (
            3,
            "open_history_recording",
            json!({"recording": Uuid::from_u128(6)}),
            Reply::HistoryOpen {
                operation: HistoryOpenStatus {
                    operation: HISTORY_OPEN,
                    state: HistoryOpenState::Loading,
                },
            },
            json!({"operation": HISTORY_OPEN, "state": {"kind": "loading"}}),
        ),
    ] {
        client.send_tool(id, name, arguments).await;
        let initial = self::next_bridge_exchange(&mut requests).await;
        assert!(matches!(
            initial.request,
            Request::RunQuery(_) | Request::OpenHistoryRecording(_)
        ));
        initial.reply.send(reply).unwrap();
        let response = tokio::time::timeout(Duration::from_millis(50), client.receive())
            .await
            .expect("a pending status is a complete reply");
        assert_eq!(response.get("id"), Some(&json!(id)));
        assert_eq!(
            response.pointer("/result/structuredContent"),
            Some(&expected)
        );
        assert!(requests.try_recv().is_err());
    }
    drop(client);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_history_open_returns_the_desktop_settings_choice_without_waiting_for_it() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| match request {
        Request::OpenHistoryRecording(_) => Ok(Reply::HistoryOpen {
            operation: HistoryOpenStatus {
                operation: HISTORY_OPEN,
                state: HistoryOpenState::AwaitingTrackSettings,
            },
        }),
        _ => panic!("unexpected History request: {request:?}"),
    }));
    client.initialize().await;
    let submitted_at = tokio::time::Instant::now();
    client
        .send_tool(
            2,
            "open_history_recording",
            json!({"recording": Uuid::from_u128(6)}),
        )
        .await;
    let response = client.receive().await;
    assert_eq!(
        response.pointer("/result/structuredContent"),
        Some(&json!({
            "operation": HISTORY_OPEN, "state": {"kind": "awaiting_track_settings"}
        }))
    );
    assert_eq!(submitted_at.elapsed(), Duration::ZERO);
    drop(client);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn nonpending_start_replies_return_without_a_status_probe() {
    let (mut client, worker) = Client::start(Bridge::Fixture(|request| match request {
        Request::RunQuery(_) => Ok(self::query_reply(QueryJobState::Busy)),
        Request::OpenHistoryRecording(_) => Ok(Reply::HistoryOpen {
            operation: HistoryOpenStatus {
                operation: HISTORY_OPEN,
                state: HistoryOpenState::Failed {
                    code: ErrorCode::HistoryOpenFailed,
                },
            },
        }),
        _ => panic!("unexpected status probe: {request:?}"),
    }));
    client.initialize().await;
    let submitted_at = tokio::time::Instant::now();
    client
        .send_tool(2, "run_query", json!({"text": "points | draw"}))
        .await;
    assert_eq!(
        client.receive().await.pointer("/result/structuredContent"),
        Some(&json!({
            "job": self::query_job_id(), "status": "busy"
        }))
    );
    client
        .send_tool(
            3,
            "open_history_recording",
            json!({"recording": Uuid::from_u128(6)}),
        )
        .await;
    assert_eq!(
        client.receive().await.pointer("/result/structuredContent"),
        Some(&json!({
            "operation": HISTORY_OPEN, "state": {"kind": "failed", "code": "history_open_failed"}
        }))
    );
    assert_eq!(submitted_at.elapsed(), Duration::ZERO);
    drop(client);
    worker.await.unwrap();
}

async fn next_bridge_exchange(requests: &mut mpsc::Receiver<BridgeExchange>) -> BridgeExchange {
    tokio::time::timeout(Duration::from_secs(2), requests.recv())
        .await
        .unwrap()
        .unwrap()
}

fn query_reply(state: QueryJobState) -> Reply {
    Reply::QueryJob {
        job: QueryJobStatus {
            job: self::query_job_id(),
            state,
        },
    }
}

async fn assert_query_reply_shapes(client: &mut Client, expected_validation: &Value) {
    let mismatched = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 33, "method": "tools/call",
            "params": {"name": "check_query", "arguments": {"text": "points | draw"}}
        }))
        .await;
    assert_eq!(mismatched.pointer("/result/isError"), Some(&json!(true)));
    assert_eq!(mismatched.pointer("/result/structuredContent"), None);

    let invalid = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 34, "method": "tools/call",
            "params": {"name": "run_query", "arguments": {"text": "points | where speed > 30 km/h"}}
        }))
        .await;
    assert_eq!(invalid.pointer("/result/isError"), Some(&json!(false)));
    assert_eq!(
        invalid.pointer("/result/structuredContent"),
        Some(expected_validation)
    );
}

async fn assert_history_listing_uses_the_date_filter(client: &mut Client) {
    let result = client.exchange(json!({
        "jsonrpc": "2.0", "id": 30, "method": "tools/call",
        "params": {"name": "list_history_recordings", "arguments": {"dates": {"from": "2025-02-02"}, "limit": 1}}
    })).await;
    assert_eq!(result.pointer("/result/isError"), Some(&json!(false)));
    assert_eq!(
        result.pointer("/result/structuredContent"),
        Some(&json!({
            "recordings": [{
                "id": Uuid::from_u128(42), "identity": "recorder", "title": null,
                "metadata_truncated": false,
                "time_range": {
                    "start": "2025-02-02T00:00:00.000001Z",
                    "end": "2025-02-02T00:00:01.234567Z"
                },
                "nav_points": 2, "total_tracks": 1, "shelved_tracks": 0
            }],
            "next_cursor": null
        }))
    );
}

async fn assert_ui_tool_replies(client: &mut Client) {
    let track = self::track_reference();
    let commands = [
        (
            "open_point",
            json!({"track": track, "point": 1}),
            json!({"status": "map_point_opened", "track": track, "point": 1}),
        ),
        (
            "show_on_map",
            json!({"target": {"kind": "track", "track": track, "start": 1, "end": 2}}),
            json!({"status": "map_range_framed", "track": track, "start": 1, "end": 2}),
        ),
        (
            "show_on_map",
            json!({"target": {
                "kind": "query_match", "job": self::query_job_id(), "query": 1,
                "matched": {"track_ordinal": 2, "match_ordinal": 3}
            }}),
            json!({"status": "map_range_framed", "track": track, "start": 1, "end": 2}),
        ),
        (
            "show_on_plot",
            json!({"target": {"kind": "track", "track": track, "start": 1, "end": 2}, "metrics": ["velocity"]}),
            json!({
                "status": "plot_framed",
                "track": track,
                "time_range": { "start": "2026-05-29T18:48:26.125Z", "end": "2026-05-29T18:48:26.125Z" }
            }),
        ),
        (
            "open_query",
            json!({"text": INVALID_QUERY_DRAFT}),
            json!({"status": "query_draft_opened"}),
        ),
        (
            "open_setting",
            json!({"key": "analysis.slip_window_min"}),
            json!({"status": "setting_opened", "key": "analysis.slip_window_min"}),
        ),
    ];
    for (index, (name, arguments, expected)) in commands.into_iter().enumerate() {
        let presented = client
            .exchange(json!({
                "jsonrpc": "2.0", "id": 13 + index, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }))
            .await;
        assert_eq!(
            presented.pointer("/result/structuredContent"),
            Some(&expected)
        );
    }
}

async fn assert_setting_tools_report_typed_values(client: &mut Client) {
    let listed = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 19, "method": "tools/call",
            "params": {"name": "get_settings", "arguments": {}}
        }))
        .await;
    let setting = listed
        .pointer("/result/structuredContent/settings/0")
        .unwrap();
    assert_eq!(setting.get("kind"), Some(&json!("number")));
    assert_eq!(setting.get("value"), Some(&json!(10.0)));
    let described = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 20, "method": "tools/call",
            "params": {"name": "describe_setting", "arguments": {"key": "analysis.slip_window_min"}}
        }))
        .await;
    assert_eq!(
        described.pointer("/result/structuredContent"),
        Some(setting)
    );
    let changed = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 21, "method": "tools/call",
            "params": {
                "name": "set_setting",
                "arguments": {"key": "analysis.slip_window_min", "value": 12.5}
            }
        }))
        .await;
    let mut expected = setting.clone();
    expected["value"] = json!(12.5);
    assert_eq!(
        changed.pointer("/result/structuredContent"),
        Some(&expected)
    );
}

async fn assert_processing_requires_empty_arguments(client: &mut Client) {
    for (id, (field, value)) in (22..).zip([
        ("recording", json!(self::recording_id(7))),
        ("dry_run", json!(true)),
        ("extra", Value::Null),
    ]) {
        let reply = client
            .exchange(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "apply_processing", "arguments": {field: value}}
            }))
            .await;
        assert_eq!(
            reply.pointer("/result/isError"),
            Some(&json!(true)),
            "field {field} returned {reply}"
        );
        assert!(
            reply
                .pointer("/result/content/0/text")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains(field))
        );
    }
    assert_eq!(PROCESSING_DISPATCHES.load(Ordering::Relaxed), 0);

    for (id, params) in (25..).zip([
        json!({"name": "apply_processing", "arguments": {}}),
        json!({"name": "apply_processing"}),
    ]) {
        let reply = client
            .exchange(json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params}))
            .await;
        assert_eq!(
            reply.pointer("/result/structuredContent"),
            Some(&json!({"status": "processing_applied"}))
        );
    }
    assert_eq!(PROCESSING_DISPATCHES.load(Ordering::Relaxed), 2);
}

fn inspection_fixture(request: &Request) -> io::Result<Reply> {
    match request {
        Request::GetSessionState => serde_json::from_value(self::session_state_fixture())
            .map(|state| Reply::SessionState { state })
            .map_err(io::Error::other),
        Request::OpenHistoryRecording(_) | Request::GetHistoryOpenStatus(_) => {
            self::history_open_fixture(request)
        }
        Request::ApplyProcessing(ApplyProcessing {}) => {
            PROCESSING_DISPATCHES.fetch_add(1, Ordering::Relaxed);
            Ok(Reply::ProcessingApplied)
        }
        Request::GetSettings => Ok(Reply::Settings {
            settings: vec![self::setting_fixture(10.0)],
        }),
        Request::DescribeSetting(SettingSelection {
            key: SettingKey::Analysis(AnalysisSettingKey::SlipWindow),
        }) => Ok(Reply::Setting {
            setting: self::setting_fixture(10.0),
        }),
        Request::OpenPoint(point) if point.track == self::track_reference() && point.point == 1 => {
            Ok(Reply::MapPointOpened(*point))
        }
        Request::ShowOnMap(ShowOnMap {
            target:
                RangeTarget::Track {
                    track,
                    start: Some(1),
                    end: Some(2),
                },
        }) if *track == self::track_reference() => Ok(Reply::MapRangeFramed {
            track: *track,
            start: 1,
            end: 2,
        }),
        Request::OpenQuery(QueryText { text }) if text == INVALID_QUERY_DRAFT => {
            Ok(Reply::QueryDraftOpened)
        }
        Request::OpenSetting(SettingSelection {
            key: SettingKey::Analysis(AnalysisSettingKey::SlipWindow),
        }) => Ok(Reply::SettingOpened {
            key: SettingKey::Analysis(AnalysisSettingKey::SlipWindow),
        }),
        Request::SetSetting(SetSetting {
            key: SettingKey::Analysis(AnalysisSettingKey::SlipWindow),
            value: SettingInput::Number(12.5),
        }) => Ok(Reply::Setting {
            setting: self::setting_fixture(12.5),
        }),
        Request::ShowOnPlot(ShowOnPlot {
            target:
                RangeTarget::Track {
                    track,
                    start: Some(1),
                    end: Some(2),
                },
            metrics,
        }) if *track == self::track_reference() && metrics.as_slice() == ["velocity"] => {
            let time = "2026-05-29T18:48:26.125Z"
                .parse()
                .map_err(io::Error::other)?;
            Ok(Reply::PlotFramed {
                track: *track,
                time_range: TimeRange {
                    start: time,
                    end: time,
                },
            })
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
        }) if *job == self::query_job_id() => Ok(Reply::MapRangeFramed {
            track: self::track_reference(),
            start: 1,
            end: 2,
        }),
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
            && *matched == QueryMatchPosition::default() =>
        {
            Ok(Reply::QueryResults {
                results: QueryResults {
                    job: *job,
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
                        matched: *matched,
                        next_start: Some(4),
                        rows: vec![QueryResultRow {
                            index: 3,
                            time: Some(QueryResultTime::UnixSeconds(0.000000021)),
                            values: vec![NumericObservation::from(Some(f64::NAN))],
                        }],
                    },
                },
            })
        }
        Request::CheckQuery(QueryText { text }) | Request::RunQuery(QueryText { text })
            if text == "points | draw" =>
        {
            Ok(self::query_reply(QueryJobState::Completed {
                queries: vec![QueryCounts {
                    matches: 2,
                    tracks_with_matches: 1,
                    skipped: 0,
                }],
            }))
        }
        Request::GetQueryStatus(GetQueryStatus { job, .. }) if *job == self::query_job_id() => {
            Ok(self::query_reply(QueryJobState::Completed {
                queries: vec![QueryCounts {
                    matches: 2,
                    tracks_with_matches: 1,
                    skipped: 0,
                }],
            }))
        }
        Request::CancelQuery(QueryJobRequest { job }) if *job == self::query_job_id() => {
            Ok(Reply::QueryJob {
                job: QueryJobStatus {
                    job: *job,
                    state: QueryJobState::Cancelling,
                },
            })
        }
        Request::CheckQuery(QueryText { text }) | Request::RunQuery(QueryText { text })
            if text == "points | where speed > 30 km/h" =>
        {
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
        }) if *after == self::recording_id(6) => Ok(self::recording_page()),
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
        Request::ListHistoryRecordings(ListHistoryRecordings {
            cursor: None,
            dates:
                Some(HistoryDates {
                    from: Some(from),
                    through: None,
                }),
            limit: Some(NonZeroUsize::MIN),
        }) if Some(*from) == NaiveDate::from_ymd_opt(2025, 2, 2) => {
            let midnight = from.and_time(NaiveTime::MIN).and_utc();
            Ok(Reply::HistoryRecordings {
                page: HistoryRecordings {
                    recordings: vec![HistoryRecording {
                        id: serde_json::from_value(json!(Uuid::from_u128(42)))
                            .map_err(io::Error::other)?,
                        identity: "recorder".to_owned(),
                        title: None,
                        metadata_truncated: false,
                        time_range: Some(TimeRange {
                            start: midnight + TimeDelta::microseconds(1),
                            end: midnight + TimeDelta::microseconds(1_234_567),
                        }),
                        time_range_unrepresentable: false,
                        nav_points: 2,
                        total_tracks: 1,
                        shelved_tracks: 0,
                    }],
                    next_cursor: None,
                },
            })
        }
        Request::GetRangeDetails(GetRangeDetails {
            track,
            start: None,
            end: None,
            limit: None,
        }) if *track == self::track_reference() => Ok(Reply::Range {
            details: RangeDetails {
                points: vec![PointDetails {
                    index: PointIdx::new(0),
                    time: DateTime::UNIX_EPOCH + TimeDelta::nanoseconds(21),
                    gps_time: None,
                    system_time: None,
                    position: None,
                    eph_m: NumericObservation::Finite { value: 2.375 },
                    velocity_mps: NumericObservation::Missing,
                }],
                next_start: None,
            },
        }),
        _ => Err(ErrorKind::InvalidInput.into()),
    }
}

fn session_state_fixture() -> Value {
    json!({
        "recording_count": 0,
        "recordings": [],
        "next_after": null,
        "open_panels": ["plot", "settings"],
        "time_filter": {"kind": "all"},
        "map": {
            "base_layer": "open_street_map",
            "visible_categories": ["tracks", "query_highlights"],
            "bounds": null,
            "pinned_point": null
        },
        "query": null
    })
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
                metadata_truncated: false,
                time_range: None,
                time_range_unrepresentable: false,
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

fn history_open_fixture(request: &Request) -> io::Result<Reply> {
    match request {
        Request::OpenHistoryRecording(OpenHistoryRecording { recording })
            if serde_json::to_value(recording)
                .is_ok_and(|value| value == json!(Uuid::from_u128(6))) =>
        {
            Ok(Reply::HistoryOpen {
                operation: HistoryOpenStatus {
                    operation: HISTORY_OPEN,
                    state: HistoryOpenState::Ready {
                        recording: self::recording_id(7),
                        reused: false,
                    },
                },
            })
        }
        Request::GetHistoryOpenStatus(GetHistoryOpenStatus { operation, .. })
            if *operation == HISTORY_OPEN =>
        {
            Ok(Reply::HistoryOpen {
                operation: HistoryOpenStatus {
                    operation: *operation,
                    state: HistoryOpenState::Ready {
                        recording: self::recording_id(7),
                        reused: false,
                    },
                },
            })
        }
        _ => Err(ErrorKind::InvalidInput.into()),
    }
}

const HISTORY_OPEN: HistoryOpenId = HistoryOpenId {
    session: Uuid::from_u128(1),
    id: Uuid::from_u128(8),
};

fn track_reference() -> TrackReference {
    TrackReference {
        recording: self::recording_id(7),
        index: 0,
        revision: 2,
    }
}

fn setting_fixture(value: f64) -> SettingInfo {
    SettingInfo {
        key: SettingKey::Analysis(AnalysisSettingKey::SlipWindow),
        page: "Analysis".to_owned(),
        label: "Slip window".to_owned(),
        help: "Time window for calculating the slip rate.".to_owned(),
        apply: SettingApply::Live,
        value: SettingValue::Number {
            value: Some(value),
            minimum: 1.0,
            maximum: 120.0,
            unit: SettingUnit::Minutes,
        },
    }
}

struct Client {
    input: Lines<BufReader<ReadHalf<DuplexStream>>>,
    output: WriteHalf<DuplexStream>,
}

impl Client {
    fn start(bridge: Bridge) -> (Self, JoinHandle<()>) {
        let (server, stream) = tokio::io::duplex(8192);
        let worker = tokio::spawn(async move {
            Companion { bridge }
                .serve(server)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let (input, output) = tokio::io::split(stream);
        (
            Self {
                input: BufReader::new(input).lines(),
                output,
            },
            worker,
        )
    }

    async fn initialize(&mut self) -> Value {
        let result = self
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
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        result
    }

    async fn exchange(&mut self, message: Value) -> Value {
        self.send(message).await;
        self.receive().await
    }

    async fn receive(&mut self) -> Value {
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

    async fn send_tool(&mut self, id: u64, name: &str, arguments: Value) {
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }))
        .await;
    }
}

pub(super) struct PendingBridge(mpsc::Sender<BridgeExchange>);

struct BridgeExchange {
    request: Request,
    reply: oneshot::Sender<Reply>,
}

impl PendingBridge {
    pub(super) async fn request(&self, request: &Request) -> io::Result<Reply> {
        let (reply, response) = oneshot::channel();
        self.0
            .send(BridgeExchange {
                request: request.clone(),
                reply,
            })
            .await
            .map_err(io::Error::other)?;
        response.await.map_err(io::Error::other)
    }
}

const INVALID_QUERY_DRAFT: &str = "# Første\r\n\r\npoints | where speed > 10 km/h";
static PROCESSING_DISPATCHES: AtomicUsize = AtomicUsize::new(0);
