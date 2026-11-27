use std::io::ErrorKind;
use std::mem;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime, Velocity};
use gt_filter::TimeWindow;
use gt_history_types::RecordingMeta;
use gt_loaded_files::{FileHistory, LoadedFiles, MAX_RECORDINGS_PER_PAGE};
use gt_loader::LoadedGtd;
use gt_query::{ConstructKind, Diagnostic, MetricSource, Span};
use gt_test_utils::fixtures;
use gt_types::{FileIdx, PointIdx, TimeRange, TrackIdx, TrackRef};
use gt_ui_types::DisplayCategory;
use proptest::strategy::Strategy as _;
use proptest::{arbitrary, collection, prop_assert_eq, prop_oneof, proptest};
use serde_json::{Value, json};
use strum::IntoEnumIterator as _;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::framing::{self, Frame, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use super::recording_control::{MAX_RECORDING_BATCH, ManageRecordings};
use super::test_util::TestEndpoint;
use super::traffic_capture::{MAX_CAPTURE_BYTES, MAX_CAPTURE_FRAMES};
use super::{
    ApplyProcessing, ChannelEntry, ErrorCode, GetRangeDetails, GetRecording, GetTrack, Inspection,
    InspectionRequest, ListMetrics, ListRecordings, MAX_POINTS_PER_PAGE, MAX_TRACKS_PER_PAGE,
    MAX_VISIBILITY_TRACKS, NumericObservation, OpenPoint, Pending, QueryJobId, QueryText,
    QueryValidation, Recording, RecordingReference, Recordings, Reply, Request, Server,
    SessionExchange, SessionMapBounds, SessionMapLayer, SessionMapState, SessionPanel,
    SessionQueryPhase, SessionQueryState, SessionRequest, SessionState, SetTrackVisibility,
    TrackReference, TrackVisibilityAction, TrafficCapture, TrafficDirection, TrafficFrame,
    VisibilityTrackRefs,
};

impl TestEndpoint {
    fn start(&self) -> (Server, mpsc::Receiver<()>) {
        let (wake, wakes) = mpsc::channel(1);
        let server = self
            .start_server(move || wake.try_send(()).unwrap())
            .unwrap();
        (server, wakes)
    }

    async fn send_recording_list(&self) -> impl AsyncRead + AsyncWrite + Unpin {
        let mut stream = self.endpoint.connect().await.unwrap();
        Frame::encode(&REQUEST, MAX_REQUEST_BYTES)
            .unwrap()
            .write(&mut stream)
            .await
            .unwrap();
        stream
    }
}

#[tokio::test]
async fn invalid_view_arguments_stop_at_the_bridge() {
    let endpoint = TestEndpoint::new().unwrap();
    let (_server, mut wakes) = endpoint.start();
    for request in [
        json!({"method":"set_session_view","base_layer":"terrain"}),
        json!({"method":"set_session_view","display_categories":[{"category":"terrain","visible":true}]}),
        json!({"method":"set_session_view","time_filter":{"action":"clear","end":null}}),
        json!({"method":"set_session_view","time_filter":{"action":"set","end":"2026-02-01T14:00:00+00:00"}}),
        json!({"method":"set_session_view","time_filter":{"action":"set","end":"2026-02-01T14:00:00.1234567891Z"}}),
    ] {
        let payload = serde_json::to_vec(&request).unwrap();
        let mut stream = endpoint.send_payload(&payload).await.unwrap();
        let reply: Reply = framing::read(&mut stream, MAX_REPLY_BYTES).await.unwrap();
        assert_eq!(reply, Reply::from(ErrorCode::InvalidRequest));
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    }
}

#[tokio::test]
async fn opening_a_panel_rejects_fields_from_other_variants_before_dispatch() {
    let endpoint = TestEndpoint::new().unwrap();
    let (_server, mut wakes) = endpoint.start();
    let track = TrackReference {
        recording: RecordingReference {
            session: Uuid::nil(),
            number: 0,
        },
        index: 0,
        revision: 0,
    };
    for request in [
        json!({"method": "open_panel", "panel": "plot", "track": track}),
        json!({"method": "open_panel", "panel": "sky_trails"}),
    ] {
        let payload = serde_json::to_vec(&request).unwrap();
        let mut stream = endpoint.send_payload(&payload).await.unwrap();
        let reply: Reply = framing::read(&mut stream, MAX_REPLY_BYTES).await.unwrap();
        assert_eq!(reply, Reply::from(ErrorCode::InvalidRequest));
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    }
}

#[tokio::test]
async fn track_visibility_requires_a_bounded_nonempty_array_before_dispatch() {
    let endpoint = TestEndpoint::new().unwrap();
    let (_server, mut wakes) = endpoint.start();
    let track = TrackReference {
        recording: RecordingReference {
            session: Uuid::nil(),
            number: 0,
        },
        index: 0,
        revision: 0,
    };
    for count in [0, MAX_VISIBILITY_TRACKS + 1] {
        let request = json!({"method": "set_track_visibility", "action": "hide", "tracks": vec![track; count]});
        let payload = serde_json::to_vec(&request).unwrap();
        let mut stream = endpoint.send_payload(&payload).await.unwrap();
        let reply = framing::read::<Reply>(&mut stream, MAX_REPLY_BYTES)
            .await
            .unwrap();
        assert_eq!(reply, Reply::from(ErrorCode::InvalidRequest));
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    }
}

#[tokio::test]
async fn recording_control_rejects_invalid_batches_before_dispatch() {
    let endpoint = TestEndpoint::new().unwrap();
    let (_server, mut wakes) = endpoint.start();
    let recording = RecordingReference {
        session: Uuid::nil(),
        number: 0,
    };
    for request in [
        json!({"method": "manage_recordings", "action": "open", "paths": []}),
        json!({"method": "manage_recordings", "action": "unload", "recordings": vec![recording; MAX_RECORDING_BATCH + 1]}),
        json!({"method": "manage_recordings", "action": "open", "paths": ["/test.GTD"], "recordings": [recording]}),
        json!({"method": "manage_recordings", "action": "unload", "recordings": [recording], "delete": true}),
    ] {
        let payload = serde_json::to_vec(&request).unwrap();
        let mut stream = endpoint.send_payload(&payload).await.unwrap();
        let reply: Reply = framing::read(&mut stream, MAX_REPLY_BYTES).await.unwrap();
        assert_eq!(reply, Reply::from(ErrorCode::InvalidRequest));
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    }

    let path = PathBuf::from(format!("/{}.log", "x".repeat(MAX_REQUEST_BYTES)));
    let request = Request::ManageRecordings(ManageRecordings::Open {
        paths: vec![path].try_into().unwrap(),
    });
    let error = super::request_at(&endpoint.endpoint, &request)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::FileTooLarge);
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn a_full_visibility_request_with_maximum_track_references_fits_the_request_frame() {
    let track = TrackReference {
        recording: RecordingReference {
            session: Uuid::from_u128(u128::MAX),
            number: u64::MAX,
        },
        index: usize::MAX,
        revision: u64::MAX,
    };
    let request = Request::SetTrackVisibility(SetTrackVisibility {
        action: TrackVisibilityAction::Hide,
        tracks: VisibilityTrackRefs::try_from(vec![track; MAX_VISIBILITY_TRACKS]).unwrap(),
    });
    let encoded = Frame::encode(&request, MAX_REQUEST_BYTES).unwrap();
    assert!(encoded.payload_len() <= MAX_REQUEST_BYTES);
}

#[tokio::test]
async fn traffic_capture_preserves_the_request_and_the_written_reply() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let capture = server.traffic_capture();
    capture.set_enabled(true);
    let raw_request = b" \r\n { \"text\": \"a\\n\\u00e9\\/\", \"method\": \"check_query\" } \n";
    let reply = self::query_diagnostic_reply("Line 1\n\"é/\\\"".to_owned());
    let mut stream = fixture.send_payload(raw_request).await.unwrap();
    self::wait_for_request(&mut wakes).await;
    server.dispatch(|request| {
        assert_eq!(
            request.request,
            Request::CheckQuery(QueryText {
                text: "a\né/".to_owned()
            })
        );
        reply.clone()
    });
    let raw_reply = framing::read_payload(&mut stream, MAX_REPLY_BYTES)
        .await
        .unwrap();
    assert_eq!(framing::decode::<Reply>(&raw_reply).unwrap(), reply);
    let frames = self::wait_for_captured_reply(&capture, 1).await;
    let [request, reply] = frames.as_slice() else {
        panic!("expected two captured frames");
    };
    assert_eq!(request.exchange_id, reply.exchange_id);
    assert_eq!(request.direction, TrafficDirection::Request);
    assert_eq!(request.payload(), raw_request);
    assert_eq!(reply.payload(), raw_reply);
}

#[tokio::test]
async fn traffic_capture_records_complete_bodies_and_written_errors() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let capture = server.traffic_capture();
    capture.set_enabled(true);

    let invalid = &[0xff, b'\n'];
    let mut invalid_stream = fixture.send_payload(invalid).await.unwrap();
    let invalid_reply = framing::read_payload(&mut invalid_stream, MAX_REPLY_BYTES)
        .await
        .unwrap();
    assert_eq!(
        framing::decode::<Reply>(&invalid_reply).unwrap(),
        ErrorCode::InvalidRequest.into()
    );
    drop(invalid_stream);

    let mut oversized = fixture.endpoint.connect().await.unwrap();
    oversized
        .write_u32(u32::try_from(MAX_REQUEST_BYTES + 1).unwrap())
        .await
        .unwrap();
    let oversized_reply = framing::read_payload(&mut oversized, MAX_REPLY_BYTES)
        .await
        .unwrap();
    assert_eq!(
        framing::decode::<Reply>(&oversized_reply).unwrap(),
        ErrorCode::RequestTooLarge.into()
    );
    drop(oversized);

    let mut incomplete = fixture.endpoint.connect().await.unwrap();
    incomplete.write_u32(2).await.unwrap();
    incomplete.write_u8(b'{').await.unwrap();
    drop(incomplete);
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));

    let mut stream = fixture.send_recording_list().await;
    self::wait_for_request(&mut wakes).await;
    let request_id = capture.snapshot().last().unwrap().exchange_id;
    server.dispatch(|_| self::query_diagnostic_reply("x".repeat(MAX_REPLY_BYTES)));
    let fallback_reply = framing::read_payload(&mut stream, MAX_REPLY_BYTES)
        .await
        .unwrap();
    assert_eq!(
        framing::decode::<Reply>(&fallback_reply).unwrap(),
        ErrorCode::ReplyTooLarge.into()
    );
    let frames = self::wait_for_captured_reply(&capture, request_id).await;
    let [
        invalid_request,
        invalid_response,
        oversized_response,
        request,
        fallback,
    ] = frames.as_slice()
    else {
        panic!("unexpected captured frame count: {}", frames.len());
    };
    assert_eq!(invalid_request.payload(), invalid);
    assert_eq!(invalid_response.payload(), invalid_reply);
    assert_eq!(oversized_response.payload(), oversized_reply);
    assert_eq!(request.direction, TrafficDirection::Request);
    assert_eq!(request.exchange_id, fallback.exchange_id);
    assert_eq!(fallback.payload(), fallback_reply);
}

#[tokio::test]
async fn traffic_capture_excludes_pending_replies_after_clear_or_disable() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let capture = server.traffic_capture();
    assert!(!capture.is_enabled());

    let mut before_enable = fixture.send_recording_list().await;
    self::wait_for_request(&mut wakes).await;
    capture.set_enabled(true);
    self::reply_with_empty_recording_list(&mut server);
    assert_eq!(
        framing::read::<Reply>(&mut before_enable, MAX_REPLY_BYTES)
            .await
            .unwrap(),
        EMPTY_RECORDINGS
    );
    drop(before_enable);

    let mut before_clear = fixture.send_recording_list().await;
    self::wait_for_request(&mut wakes).await;
    assert_eq!(capture.snapshot().len(), 1);
    capture.clear();
    assert!(capture.is_enabled());
    self::reply_with_empty_recording_list(&mut server);
    assert_eq!(
        framing::read::<Reply>(&mut before_clear, MAX_REPLY_BYTES)
            .await
            .unwrap(),
        EMPTY_RECORDINGS
    );
    drop(before_clear);

    let mut before_disable = fixture.send_recording_list().await;
    self::wait_for_request(&mut wakes).await;
    assert_eq!(capture.snapshot().len(), 1);
    let before_disable_id = capture.snapshot().last().unwrap().exchange_id;
    capture.set_enabled(false);
    assert_eq!(capture.snapshot().len(), 1);
    capture.set_enabled(true);
    self::reply_with_empty_recording_list(&mut server);
    assert_eq!(
        framing::read::<Reply>(&mut before_disable, MAX_REPLY_BYTES)
            .await
            .unwrap(),
        EMPTY_RECORDINGS
    );
    drop(before_disable);

    let mut after_enable = fixture.send_recording_list().await;
    self::dispatch_recording_list(&mut server, &mut wakes).await;
    assert_eq!(
        framing::read::<Reply>(&mut after_enable, MAX_REPLY_BYTES)
            .await
            .unwrap(),
        EMPTY_RECORDINGS
    );
    let frames = self::wait_for_captured_reply(&capture, before_disable_id + 1).await;
    assert_eq!(frames.len(), 3);
    assert_eq!(frames.first().unwrap().exchange_id, before_disable_id);
}

#[tokio::test]
async fn traffic_capture_evicts_oldest_frames_at_the_retention_limits() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let capture = server.traffic_capture();
    capture.set_enabled(true);

    let short_exchanges = MAX_CAPTURE_FRAMES / 2 + 1;
    for _ in 0..short_exchanges {
        let mut stream = fixture.send_recording_list().await;
        self::dispatch_recording_list(&mut server, &mut wakes).await;
        assert_eq!(
            framing::read::<Reply>(&mut stream, MAX_REPLY_BYTES)
                .await
                .unwrap(),
            EMPTY_RECORDINGS
        );
    }
    let short_exchanges = u64::try_from(short_exchanges).unwrap();
    let retained = self::wait_for_captured_reply(&capture, short_exchanges).await;
    assert_eq!(retained.len(), MAX_CAPTURE_FRAMES);
    assert_eq!(retained.first().unwrap().exchange_id, 2);

    capture.clear();
    let reply = self::query_diagnostic_reply("x".repeat(MAX_REPLY_BYTES / 2));
    let large_exchanges = MAX_CAPTURE_BYTES / (MAX_REPLY_BYTES / 2) + 1;
    for _ in 0..large_exchanges {
        let mut stream = fixture.send_recording_list().await;
        self::wait_for_request(&mut wakes).await;
        server.dispatch(|_| reply.clone());
        assert_eq!(
            framing::read::<Reply>(&mut stream, MAX_REPLY_BYTES)
                .await
                .unwrap(),
            reply
        );
    }
    let final_exchange = short_exchanges + u64::try_from(large_exchanges).unwrap();
    let retained = self::wait_for_captured_reply(&capture, final_exchange).await;
    assert!(retained.len() < large_exchanges * 2);
    assert!(
        retained
            .iter()
            .map(|frame| frame.payload().len())
            .sum::<usize>()
            <= MAX_CAPTURE_BYTES
    );
    assert!(retained.first().unwrap().exchange_id > short_exchanges + 1);
}

#[test]
fn traffic_capture_counts_capacity_for_short_payloads() {
    let capture = TrafficCapture::new();
    capture.set_enabled(true);
    let exchange = capture.start_exchange().unwrap();
    let mut request = Vec::with_capacity(MAX_CAPTURE_BYTES / 2 + 1);
    request.push(b'a');
    exchange.record(TrafficDirection::Request, request);
    let earlier = capture.snapshot();

    let mut reply = Vec::with_capacity(MAX_CAPTURE_BYTES / 2 + 1);
    reply.push(b'b');
    exchange.record(TrafficDirection::Reply, reply);
    let retained = capture.snapshot();
    let [frame] = retained.as_slice() else {
        panic!("expected only the last payload to fit");
    };
    assert_eq!(frame.payload(), b"b");
    capture.clear();
    assert!(capture.snapshot().is_empty());
    assert_eq!(earlier.first().unwrap().payload(), b"a");
    assert_eq!(frame.payload(), b"b");
}

fn query_diagnostic_reply(message: String) -> Reply {
    Reply::QueryCheck {
        validation: QueryValidation::Invalid {
            diagnostics: vec![Diagnostic {
                span: Span::new(0, 1),
                message,
                help: None,
            }],
        },
    }
}

async fn wait_for_captured_reply(capture: &TrafficCapture, exchange_id: u64) -> Vec<TrafficFrame> {
    tokio::time::timeout(BRIDGE_TEST_TIMEOUT, async {
        loop {
            let frames = capture.snapshot();
            if frames.last().is_some_and(|frame| {
                frame.exchange_id == exchange_id && frame.direction == TrafficDirection::Reply
            }) {
                return frames;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("The server did not capture its written reply")
}

#[tokio::test]
async fn a_client_waiting_to_close_does_not_block_another_request() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let mut first = fixture.send_recording_list().await;
    self::dispatch_recording_list(&mut server, &mut wakes).await;
    let reply: Reply = framing::read(&mut first, MAX_REPLY_BYTES).await.unwrap();
    assert_eq!(reply, EMPTY_RECORDINGS);

    let (reply, ()) = tokio::join!(
        super::request_at(&fixture.endpoint, &REQUEST),
        self::dispatch_recording_list(&mut server, &mut wakes)
    );
    assert_eq!(reply.unwrap(), EMPTY_RECORDINGS);
    drop(first);
}

#[tokio::test]
async fn a_full_app_queue_returns_busy_without_displacing_its_request() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let mut first = fixture.send_recording_list().await;
    self::wait_for_request(&mut wakes).await;

    let reply = super::request_at(&fixture.endpoint, &REQUEST)
        .await
        .unwrap();
    assert_eq!(reply, Reply::from(ErrorCode::Busy));
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    self::reply_with_empty_recording_list(&mut server);
    let reply: Reply = framing::read(&mut first, MAX_REPLY_BYTES).await.unwrap();
    assert_eq!(reply, EMPTY_RECORDINGS);
}

#[tokio::test]
async fn excess_connections_close_and_a_completed_exchange_releases_a_slot() {
    let fixture = TestEndpoint::new().unwrap();
    let (mut server, mut wakes) = fixture.start();
    let mut open = Vec::new();
    for _ in 0..super::MAX_ACTIVE_CONNECTIONS {
        let mut stream = fixture.send_recording_list().await;
        self::dispatch_recording_list(&mut server, &mut wakes).await;
        let reply: Reply = framing::read(&mut stream, MAX_REPLY_BYTES).await.unwrap();
        assert_eq!(reply, EMPTY_RECORDINGS);
        open.push(stream);
    }

    for _ in 0..2 {
        let overflow = async {
            let mut stream = fixture.endpoint.connect().await?;
            stream.read_u8().await
        };
        tokio::time::timeout(BRIDGE_TEST_TIMEOUT, overflow)
            .await
            .expect("An excess connection stayed open")
            .expect_err("The server replied on an excess connection");
    }
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    drop(open.pop().unwrap());

    let request_after_close = async {
        loop {
            if let Ok(reply) = super::request_at(&fixture.endpoint, &REQUEST).await {
                return reply;
            }
            tokio::task::yield_now().await;
        }
    };
    let (reply, ()) = tokio::time::timeout(BRIDGE_TEST_TIMEOUT, async {
        tokio::join!(
            request_after_close,
            self::dispatch_recording_list(&mut server, &mut wakes)
        )
    })
    .await
    .expect("No exchange slot became available after closing a connection");
    assert_eq!(reply, EMPTY_RECORDINGS);

    drop(server);
    for mut stream in open {
        tokio::time::timeout(BRIDGE_TEST_TIMEOUT, stream.read_u8())
            .await
            .expect("A connection stayed open after shutdown")
            .expect_err("The server replied after shutdown");
    }
}

async fn dispatch_recording_list(server: &mut Server, wakes: &mut mpsc::Receiver<()>) {
    self::wait_for_request(wakes).await;
    self::reply_with_empty_recording_list(server);
}

async fn wait_for_request(wakes: &mut mpsc::Receiver<()>) {
    assert_eq!(
        tokio::time::timeout(BRIDGE_TEST_TIMEOUT, wakes.recv())
            .await
            .expect("The server did not admit the request"),
        Some(())
    );
}

fn reply_with_empty_recording_list(server: &mut Server) {
    server.dispatch(|request| {
        assert_eq!(request.request, REQUEST);
        EMPTY_RECORDINGS
    });
}

#[tokio::test]
async fn local_request_dispatches_on_the_calling_thread() {
    let fixture = TestEndpoint::new().unwrap();
    let (wake, mut wakes) = mpsc::channel(1);
    let mut server =
        Server::bind(fixture.endpoint.clone(), move || wake.try_send(()).unwrap()).unwrap();
    let mut file = gt_test_utils::loaded_file_with_tracks(Vec::new());
    file.metadata.filename = "/private/path/receiver.gtd".to_owned();
    file.metadata.title = Some("Survey".to_owned());
    let mut files = LoadedFiles::new();
    files.push(file, FileHistory::None);
    let id = RecordingReference {
        session: server.session,
        number: files.view().get(0).unwrap().id().into(),
    };
    let caller = std::thread::current().id();
    let (reply, ()) = tokio::join!(super::request_at(&fixture.endpoint, &REQUEST), async {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), wakes.recv())
                .await
                .unwrap(),
            Some(())
        );
        server.dispatch(|request| {
            assert_eq!(std::thread::current().id(), caller);
            assert_eq!(request.session, id.session);
            let Request::ListRecordings(args) = request.request else {
                panic!("expected a recording list request");
            };
            Inspection::new(files.view(), "{title} ({filename})", request.session)
                .execute(InspectionRequest::ListRecordings(args))
                .unwrap_or_else(Reply::from)
        });
        server.dispatch(|_| panic!("unexpected request"));
    });
    assert_eq!(
        reply.unwrap(),
        Reply::Recordings {
            page: Recordings {
                recordings: vec![Recording {
                    id,
                    display_name: "Survey (receiver.gtd)".to_owned(),
                    filename: "receiver.gtd".to_owned(),
                    title: Some("Survey".to_owned()),
                    device: None,
                    metadata_truncated: false,
                    time_range: None,
                    time_range_unrepresentable: false,
                    track_count: 0,
                }],
                next_after: None
            }
        }
    );
}

#[test]
fn a_command_finishes_when_the_reply_receiver_closes_during_execution() {
    let (sender, pending) = mpsc::channel(1);
    let (reply, receiver) = oneshot::channel();
    sender
        .try_send(Pending {
            request: REQUEST,
            reply,
        })
        .unwrap_or_else(|error| panic!("Could not queue the request ({error})"));
    let mut server = Server {
        session: Uuid::nil(),
        pending,
        stop: None,
        worker: None,
        traffic_capture: TrafficCapture::new(),
    };
    let mut executed = None;
    server.dispatch(|request| {
        drop(receiver);
        executed = Some(request.request);
        EMPTY_RECORDINGS
    });
    assert_eq!(executed, Some(REQUEST));
}

#[tokio::test]
async fn dropping_the_client_exchange_skips_the_queued_command() {
    let fixture = TestEndpoint::new().unwrap();
    let listener = fixture.endpoint.bind().unwrap();
    let (sender, pending) = mpsc::channel(1);
    let mut server = Server {
        session: Uuid::nil(),
        pending,
        stop: None,
        worker: None,
        traffic_capture: TrafficCapture::new(),
    };
    let capture = server.traffic_capture();
    capture.set_enabled(true);
    let (wake, mut wakes) = mpsc::channel(1);
    let accept = async {
        let mut connection = listener.accept().await.unwrap().unwrap();
        let observed = capture.start_exchange();
        super::exchange(
            &mut connection.0,
            &sender,
            &|| wake.try_send(()).unwrap(),
            observed.as_ref(),
        )
        .await
    };
    let request = Request::OpenQuery(QueryText {
        text: "points | draw".to_owned(),
    });
    let abandon = async {
        tokio::select! {
            reply = super::request_at(&fixture.endpoint, &request) => {
                panic!("The exchange completed before dispatch ({reply:?})");
            }
            queued = wakes.recv() => assert_eq!(queued, Some(())),
        }
    };
    let (disconnected, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(accept, abandon)
    })
    .await
    .unwrap();
    assert!(disconnected.is_err());
    server.dispatch(|_| panic!("The bridge dispatched a disconnected request"));
    let frames = capture.snapshot();
    let [frame] = frames.as_slice() else {
        panic!("expected only the completed request");
    };
    assert_eq!(frame.direction, TrafficDirection::Request);
    assert_eq!(
        framing::decode::<Request>(frame.payload()).unwrap(),
        request
    );
}

#[test]
fn a_received_exchange_skips_execution_after_the_client_disconnects() {
    let (reply, receiver) = oneshot::channel();
    let exchange = SessionExchange {
        request: SessionRequest {
            request: REQUEST,
            session: Uuid::nil(),
        },
        reply,
    };
    drop(receiver);
    exchange.respond(|_| panic!("The bridge executed a disconnected request"));
}

#[test]
fn metric_definitions_are_listed_without_recordings() {
    let files = LoadedFiles::new();
    let view = Inspection::new(files.view(), "{filename}", Uuid::nil());
    let Ok(Reply::Metrics { page }) = view.execute(InspectionRequest::ListMetrics(ListMetrics {
        channel_start: None,
        channel_limit: None,
    })) else {
        panic!("expected metrics");
    };
    assert!(page.channels.is_empty());
    let names: Vec<_> = page.definitions.iter().map(|m| m.name.as_str()).collect();
    let expected: Vec<_> = gt_query::catalog()
        .iter()
        .filter(|item| item.kind == ConstructKind::Metric)
        .map(|item| item.name)
        .collect();
    assert_eq!(names, expected);
    let velocity = page
        .definitions
        .iter()
        .find(|m| m.name == "velocity")
        .unwrap();
    assert_eq!(velocity.summary, "ground speed");
    assert_eq!(velocity.quantity, "speed");
    assert_eq!(velocity.units, ["km/h", "m/s", "kn"]);
    assert_eq!(velocity.source, MetricSource::Recording);
    let tec = page.definitions.iter().find(|m| m.name == "tec").unwrap();
    assert!(tec.units.is_empty());
    assert_eq!(tec.source, MetricSource::EnvironmentArchive);
}

#[test]
fn metric_discovery_pages_channel_metadata_and_conflicts() {
    let mut first = gt_test_utils::loaded_track_with_points(Vec::new());
    let oversized_label = "x".repeat(MAX_REPLY_BYTES);
    first.channels.push(fixtures::vector_channel(
        "a_large",
        None,
        &[&oversized_label],
        vec![],
        vec![],
    ));
    first
        .channels
        .push(fixtures::scalar_channel("accel.x", None, vec![], vec![]));
    first.channels.push(fixtures::vector_channel(
        "accel",
        Some("g"),
        &["x", "y", "bad label", "x"],
        vec![],
        vec![],
    ));
    let mut second = first.clone();
    second.channels = vec![fixtures::vector_channel(
        "accel",
        Some("mg"),
        &["x", "y", "bad label", "x"],
        vec![],
        vec![],
    )];
    let mut incompatible = second.clone();
    incompatible.channels = vec![fixtures::scalar_channel(
        "accel",
        Some("deg"),
        vec![],
        vec![],
    )];
    let mut files = LoadedFiles::new();
    for tracks in [vec![first, second], vec![incompatible]] {
        files.push(
            gt_test_utils::loaded_file_with_tracks(tracks),
            FileHistory::None,
        );
    }
    let view = Inspection::new(files.view(), "{filename}", Uuid::nil());
    let mut args = ListMetrics {
        channel_start: None,
        channel_limit: Some(NonZeroUsize::MIN),
    };
    let Ok(Reply::Metrics { page }) = view.execute(InspectionRequest::ListMetrics(args)) else {
        panic!("expected first channel page");
    };
    assert_eq!(
        page.channels,
        [ChannelEntry::MetadataOmitted {
            name: Some("a_large".to_owned()),
        }]
    );
    args.channel_start = page.next_channel_start;
    assert_eq!(args.channel_start, Some(1));
    let Ok(Reply::Metrics { page }) = view.execute(InspectionRequest::ListMetrics(args)) else {
        panic!("expected next channel page");
    };
    let [ChannelEntry::Defined(accel)] = page.channels.as_slice() else {
        panic!("expected acceleration channel");
    };
    assert_eq!(accel.name, "accel");
    assert_eq!(accel.reference.as_deref(), Some("@accel"));
    assert_eq!(accel.unit.as_deref(), Some("g"));
    let components: Vec<_> = accel
        .components
        .iter()
        .map(|part| (part.label.as_str(), part.reference.as_deref()))
        .collect();
    assert_eq!(
        components,
        [
            ("x", Some("@accel.x")),
            ("y", Some("@accel.y")),
            ("bad label", None),
            ("x", None)
        ]
    );
    assert_eq!(
        accel.conflicts,
        [
            "units g and deg",
            "components [\"x\", \"y\", \"bad label\", \"x\"] and []"
        ]
    );
    assert_eq!(accel.conflict_count, 2);
    args.channel_start = page.next_channel_start;
    assert_eq!(args.channel_start, Some(2));
    let Ok(Reply::Metrics { page }) = view.execute(InspectionRequest::ListMetrics(args)) else {
        panic!("expected last channel page");
    };
    let [ChannelEntry::Defined(dotted_name)] = page.channels.as_slice() else {
        panic!("expected channel with a dotted name");
    };
    assert_eq!(dotted_name.name, "accel.x");
    assert_eq!(dotted_name.reference, None);
    assert_eq!(page.next_channel_start, None);
}

#[tokio::test]
async fn a_recording_with_oversized_sdk_metadata_fits_in_a_single_entry_page() {
    let text = format!("Grøn{JSON_CONTROL_CHARACTER}\"\\🛰").repeat(MAX_REPLY_BYTES / 8);
    let mut files = LoadedFiles::new();
    let LoadedGtd { file, identity } = self::load_recording_with_metadata(&text);
    assert_eq!(file.metadata.title.as_deref(), Some(text.as_str()));
    assert_eq!(file.metadata.device.as_deref(), Some(text.as_str()));
    assert_eq!(identity, text);
    let file_history = FileHistory::recording(identity, RecordingMeta::default(), None);
    files.push(file, file_history);
    let mut ordinary = gt_test_utils::loaded_file_with_tracks(Vec::new());
    ordinary.metadata.filename = "morning.gtd".to_owned();
    files.push(ordinary, FileHistory::None);

    let view = Inspection::new(files.view(), IDENTITY_NAME_TEMPLATE, Uuid::nil());
    let first = ListRecordings {
        after: None,
        limit: Some(NonZeroUsize::MIN),
    };
    let page = self::framed_recording_page(&view, first).await;
    let [recording] = page.recordings.as_slice() else {
        panic!("Expected one recording");
    };
    let recording_id = recording.id;
    assert!(recording.metadata_truncated);
    assert!(recording.display_name.starts_with("Grøn"));
    assert!(text.starts_with(&recording.display_name));
    assert!(recording.filename.starts_with("receiver-é"));
    assert_eq!(
        recording.title.as_deref(),
        Some(recording.display_name.as_str())
    );
    assert_eq!(recording.device, recording.title);
    assert_eq!(recording.track_count, 2);
    assert_eq!(page.next_after, Some(recording_id));

    let second = ListRecordings {
        after: page.next_after,
        ..first
    };
    let page = self::framed_recording_page(&view, second).await;
    let [last] = page.recordings.as_slice() else {
        panic!("Expected one more recording");
    };
    assert_ne!(last.id, recording_id);
    assert_eq!(last.display_name, "morning.gtd");
    assert!(!last.metadata_truncated);
    assert_eq!(page.next_after, None);

    let first = GetRecording {
        recording: recording_id,
        after: None,
        stored_track_start: None,
        limit: Some(NonZeroUsize::MIN),
    };
    let Reply::Recording { details } =
        self::framed_inspection_reply(&view, InspectionRequest::GetRecording(first)).await
    else {
        panic!("Expected recording details");
    };
    assert_eq!(details.recording, *recording);
    let [track] = details.tracks.as_slice() else {
        panic!("Expected one track");
    };
    assert_eq!(track.recording, recording_id);
    assert_eq!(track.index, 0);
    assert_eq!(details.next_after, Some(*track));
    let second = GetRecording {
        after: details.next_after,
        ..first
    };
    let Reply::Recording { details } =
        self::framed_inspection_reply(&view, InspectionRequest::GetRecording(second)).await
    else {
        panic!("Expected the next track page");
    };
    assert_eq!(details.recording, *recording);
    assert_eq!(details.tracks.first().unwrap().index, 1);
    assert_eq!(details.next_after, None);
}

#[tokio::test]
async fn display_names_use_the_original_fields_before_the_reply_budget() {
    let padded_title = format!("{}Summit", " ".repeat(600));
    let source_identity = "Ridge".repeat(200);
    for (raw_title, template, expected_name, expected_truncation) in [
        (None, "{filename}", "ridge.gtd", false),
        (None, "{identity:4}", "Ridg…", false),
        (Some(padded_title.as_str()), "{title}", "Summit", true),
    ] {
        let mut file = gt_test_utils::loaded_file_with_tracks(Vec::new());
        file.metadata.filename = "ridge.gtd".to_owned();
        file.metadata.title = raw_title.map(str::to_owned);
        let mut files = LoadedFiles::new();
        files.push(
            file,
            FileHistory::recording(source_identity.clone(), RecordingMeta::default(), None),
        );
        let view = Inspection::new(files.view(), template, Uuid::nil());
        let page = self::framed_recording_page(
            &view,
            ListRecordings {
                after: None,
                limit: None,
            },
        )
        .await;
        let [recording] = page.recordings.as_slice() else {
            panic!("Expected one recording");
        };
        assert_eq!(recording.display_name, expected_name);
        assert_eq!(recording.metadata_truncated, expected_truncation);
    }
}

#[tokio::test]
async fn the_bridge_flags_a_truncated_rendered_name() {
    let mut file = gt_test_utils::loaded_file_with_tracks(Vec::new());
    file.metadata.filename = "morning.gtd".to_owned();
    let mut files = LoadedFiles::new();
    files.push(file, FileHistory::None);
    let template = "{filename}".repeat(1024);
    let view = Inspection::new(files.view(), &template, Uuid::nil());
    let page = self::framed_recording_page(
        &view,
        ListRecordings {
            after: None,
            limit: None,
        },
    )
    .await;
    let [recording] = page.recordings.as_slice() else {
        panic!("Expected one recording");
    };
    assert!(recording.metadata_truncated);
    assert_eq!(recording.filename, "morning.gtd");
    assert!(recording.display_name.starts_with("morning.gtd"));
    assert!(recording.display_name.len() < "morning.gtd".len() * 1024);
}

#[tokio::test]
async fn a_full_recording_page_with_escaped_metadata_fits_the_frame() {
    let metadata = format!("{JSON_CONTROL_CHARACTER}\"é📍").repeat(300);
    let mut file = gt_test_utils::segmented_recording(1);
    file.metadata.filename = format!("C:\\private\\receiver-{metadata}");
    file.metadata.title = Some(metadata.clone());
    file.metadata.device = Some(metadata);
    let identity = format!("auto:{}", file.metadata.filename);
    let mut files = LoadedFiles::new();
    for _ in 0..=MAX_RECORDINGS_PER_PAGE {
        files.push(
            file.clone(),
            FileHistory::recording(identity.clone(), RecordingMeta::default(), None),
        );
    }
    let view = Inspection::new(files.view(), IDENTITY_NAME_TEMPLATE, Uuid::nil());
    let page = self::framed_recording_page(
        &view,
        ListRecordings {
            after: None,
            limit: None,
        },
    )
    .await;
    assert_eq!(page.recordings.len(), MAX_RECORDINGS_PER_PAGE);
    assert_eq!(
        page.next_after,
        page.recordings.last().map(|recording| recording.id)
    );
    assert!(page.recordings.iter().all(|recording| {
        recording.metadata_truncated
            && recording.display_name == recording.filename
            && recording.display_name.starts_with("receiver-")
    }));
}

#[tokio::test]
async fn a_full_session_preview_with_escaped_metadata_fits_the_reply_frame() {
    let metadata = format!("{JSON_CONTROL_CHARACTER}\"é📍").repeat(300);
    let LoadedGtd { mut file, identity } = self::load_recording_with_metadata(&metadata);
    file.metadata.filename = format!("C:\\private\\receiver-{metadata}");
    let start = DateTime::UNIX_EPOCH - TimeDelta::nanoseconds(1);
    let extended_year = Utc.with_ymd_and_hms(10_000, 1, 1, 0, 0, 0).unwrap();
    file.metadata.time_range = Some(TimeRange::new(start, extended_year));
    let mut files = LoadedFiles::new();
    for _ in 0..6 {
        files.push(
            file.clone(),
            FileHistory::recording(identity.clone(), RecordingMeta::default(), None),
        );
    }
    let session = Uuid::from_u128(u128::MAX);
    let preview = SessionState::recording_preview(files.view(), session, IDENTITY_NAME_TEMPLATE);
    assert_eq!(preview.recordings.len(), 5);
    assert!(
        preview
            .recordings
            .iter()
            .all(|recording| recording.metadata_truncated)
    );
    let continuation = preview.next_after.unwrap();
    assert_eq!(continuation, preview.recordings.last().unwrap().id);
    let next = self::framed_recording_page(
        &Inspection::new(files.view(), IDENTITY_NAME_TEMPLATE, session),
        ListRecordings {
            after: Some(continuation),
            limit: None,
        },
    )
    .await;
    assert_eq!(next.recordings.len(), 1);
    assert!(next.recordings[0].id.number > continuation.number);
    assert_eq!(next.next_after, None);

    let reply = Reply::SessionState {
        state: SessionState {
            recording_count: files.len(),
            recording_preview: preview,
            open_panels: SessionPanel::iter().collect(),
            time_filter: TimeWindow::from_bounds(Some(start), Some(extended_year)).into(),
            map: SessionMapState {
                base_layer: SessionMapLayer::OpenStreetMap,
                bounds: Some(SessionMapBounds {
                    south_deg: -70.12345678901234,
                    north_deg: 71.12345678901234,
                    west_deg: -179.87654321098765,
                    east_deg: 180.12345678901235,
                }),
                visible_categories: DisplayCategory::iter().collect(),
                pinned_point: Some(OpenPoint {
                    track: TrackReference {
                        recording: continuation,
                        index: usize::MAX,
                        revision: u64::MAX,
                    },
                    point: usize::MAX,
                }),
            },
            query: Some(SessionQueryState {
                phase: SessionQueryPhase::Cancelling,
                job: Some(QueryJobId {
                    session,
                    id: Uuid::from_u128(u128::MAX),
                }),
            }),
        },
    };
    let mut bytes = Vec::new();
    Frame::encode(&reply, MAX_REPLY_BYTES)
        .unwrap()
        .write(&mut bytes)
        .await
        .unwrap();
    let decoded: Reply = framing::read(&mut bytes.as_slice(), MAX_REPLY_BYTES)
        .await
        .unwrap();
    assert_eq!(decoded, reply);
    let wire = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        wire.pointer("/state/time_filter"),
        Some(&json!({
            "kind": "bounds", "start": "1969-12-31T23:59:59.999999999Z", "end": null,
            "end_unrepresentable": true
        }))
    );
    assert_eq!(
        wire.pointer("/state/recordings/0/time_range"),
        Some(&Value::Null)
    );
    assert_eq!(
        wire.pointer("/state/recordings/0/time_range_unrepresentable"),
        Some(&json!(true))
    );
}

async fn framed_recording_page(view: &Inspection<'_>, request: ListRecordings) -> Recordings {
    match self::framed_inspection_reply(view, InspectionRequest::ListRecordings(request)).await {
        Reply::Recordings { page } => page,
        reply => panic!("Expected a recording page, received {reply:?}"),
    }
}

async fn framed_inspection_reply(view: &Inspection<'_>, request: InspectionRequest) -> Reply {
    let reply = view.execute(request).unwrap();
    let mut bytes = Vec::new();
    Frame::encode(&reply, MAX_REPLY_BYTES)
        .unwrap()
        .write(&mut bytes)
        .await
        .unwrap();
    framing::read(&mut bytes.as_slice(), MAX_REPLY_BYTES)
        .await
        .unwrap()
}

fn load_recording_with_metadata(text: &str) -> LoadedGtd {
    let mut recorder = NavFileBuilder::new()
        .with_title(text)
        .unwrap()
        .with_device(text)
        .unwrap()
        .with_identity(text)
        .unwrap()
        .open();
    for offset in [TimeDelta::zero(), TimeDelta::hours(1)] {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(DateTime::UNIX_EPOCH + offset))
                .lat(Angle::degrees(55.7))
                .lon(Angle::degrees(12.6))
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    let name_fragment = format!("é{JSON_CONTROL_CHARACTER}\"📍").repeat(1024);
    gt_loader::load_gtd_bytes_with_progress(
        &bytes,
        format!("/private/recordings/receiver-{name_fragment}.gtd"),
        |_, _| {},
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn inspection_pages_tracks_and_rejects_old_session_and_track_references() {
    let mut files = LoadedFiles::new();
    files.push(
        gt_test_utils::segmented_recording(MAX_TRACKS_PER_PAGE + 1),
        FileHistory::None,
    );
    let id = RecordingReference {
        session: Uuid::nil(),
        number: files.view().get(0).unwrap().id().into(),
    };
    let mut args = GetRecording {
        recording: id,
        after: None,
        stored_track_start: None,
        limit: Some(NonZeroUsize::MAX),
    };
    let view = Inspection::new(files.view(), "{filename}", id.session);
    let Ok(Reply::Recording { details }) = view.execute(InspectionRequest::GetRecording(args))
    else {
        panic!("expected recording");
    };
    assert_eq!(details.recording.id, id);
    assert_eq!(details.tracks.len(), MAX_TRACKS_PER_PAGE);
    let track = *details.tracks.first().unwrap();
    let get_track = GetTrack { track };
    let Ok(Reply::Track {
        details: track_details,
    }) = view.execute(InspectionRequest::GetTrack(get_track))
    else {
        panic!("expected track");
    };
    assert_eq!(track_details.track_number, 1);
    assert_eq!(track_details.duration_seconds, 600.0);
    assert_eq!(track_details.distance_m, Some(1000.0));
    assert_eq!(track_details.point_count, 2);
    assert_eq!(track_details.fix_loss_count, Some(1));
    assert!(track_details.map_bounds.is_some());
    args.after = details.next_after;
    let Ok(Reply::Recording { details: last }) =
        view.execute(InspectionRequest::GetRecording(args))
    else {
        panic!("expected next page");
    };
    assert_eq!(last.tracks.first().unwrap().index, MAX_TRACKS_PER_PAGE);
    assert_eq!(last.next_after, None);

    let restarted = Inspection::new(files.view(), "{filename}", Uuid::from_u128(1));
    assert_eq!(
        restarted.execute(InspectionRequest::GetRecording(args)),
        Err(ErrorCode::StaleSession)
    );
    let cursor = InspectionRequest::ListRecordings(ListRecordings {
        after: Some(id),
        limit: None,
    });
    assert_eq!(restarted.execute(cursor), Err(ErrorCode::StaleSession));
    files.replace_file(0, gt_test_utils::segmented_recording(1), FileHistory::None);
    let updated = Inspection::new(files.view(), "{filename}", id.session);
    assert_eq!(
        updated.execute(InspectionRequest::GetTrack(get_track)),
        Err(ErrorCode::StaleTrack)
    );
}

#[test]
fn query_check_uses_loaded_channels_and_requires_a_standalone_channel_source() {
    let mut track = gt_test_utils::loaded_track_with_points(Vec::new());
    track
        .channels
        .push(fixtures::scalar_channel("accel", Some("g"), vec![], vec![]));
    let mut files = LoadedFiles::new();
    files.push(
        gt_test_utils::loaded_file_with_tracks(vec![track]),
        FileHistory::None,
    );
    let view = Inspection::new(files.view(), "{filename}", Uuid::nil());
    let check = |text: &str| {
        view.execute(InspectionRequest::CheckQuery(QueryText {
            text: text.to_owned(),
        }))
        .unwrap()
    };
    let channel = "@accel | where @accel > 1 g";
    assert_eq!(
        check(channel),
        Reply::QueryCheck {
            validation: QueryValidation::Valid
        }
    );
    assert_eq!(
        check(
            "points | where velocity > 1 km/h\n\n# På et nyt afsnit\n\npoints | where eph < 20 m"
        ),
        Reply::QueryCheck {
            validation: QueryValidation::Valid
        }
    );
    let text = format!("points | where velocity > 1 km/h\n\n# På et nyt afsnit\n\n{channel}");
    let Reply::QueryCheck {
        validation: QueryValidation::Invalid { diagnostics },
    } = check(&text)
    else {
        panic!("expected a mixed-channel diagnostic");
    };
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic");
    };
    let start = text.find("@accel").unwrap();
    assert_eq!(diagnostic.span, Span::new(start, start + "@accel".len()));
    assert_eq!(diagnostic.message, gt_query_run::MIXED_CHANNEL_MESSAGE);
}

#[test]
fn query_check_reports_byte_spans_and_hints_in_submitted_text() {
    let files = LoadedFiles::new();
    let view = Inspection::new(files.view(), "{filename}", Uuid::nil());
    let check = |text: &str| {
        view.execute(InspectionRequest::CheckQuery(QueryText {
            text: text.to_owned(),
        }))
        .unwrap()
    };
    assert_eq!(
        check(" # Første\r\n\r\n# Sidste"),
        Reply::QueryCheck {
            validation: QueryValidation::Empty
        }
    );
    let text = "# Første\r\n\r\npoints | where speed > 30 km/h\r\n\r\npoints | where 🧭\r\n\r\npoints | where";
    let Reply::QueryCheck {
        validation: QueryValidation::Invalid { diagnostics },
    } = check(text)
    else {
        panic!("expected query diagnostics");
    };
    let [unknown, symbol, missing] = diagnostics.as_slice() else {
        panic!("expected three diagnostics");
    };
    assert_eq!(unknown.message, "unknown metric `speed`");
    assert_eq!(
        unknown.help.as_deref(),
        Some("metrics are lowercase snake_case, e.g. velocity, heading, sats_fix")
    );
    assert_eq!(
        text.get(unknown.span.start..unknown.span.end),
        Some("speed")
    );
    assert_eq!(text.get(symbol.span.start..symbol.span.end), Some("🧭"));
    assert_eq!(missing.span, Span::new(text.len(), text.len()));
}

#[tokio::test]
async fn oversized_query_requests_fail_before_connecting() {
    let fixture = TestEndpoint::new().unwrap();
    let request = Request::CheckQuery(QueryText {
        text: "\n".repeat(MAX_REQUEST_BYTES / 2),
    });
    let error = super::request_at(&fixture.endpoint, &request)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::FileTooLarge);
}

#[tokio::test]
async fn query_diagnostics_exceeding_the_frame_limit_return_an_error() {
    let fixture = TestEndpoint::new().unwrap();
    let (wake, mut wakes) = mpsc::channel(1);
    let mut server =
        Server::bind(fixture.endpoint.clone(), move || wake.try_send(()).unwrap()).unwrap();
    let oversized_label = "x".repeat(MAX_REPLY_BYTES);
    let mut track = gt_test_utils::loaded_track_with_points(Vec::new());
    track.channels.push(fixtures::vector_channel(
        "accel",
        Some("g"),
        &[&oversized_label],
        vec![],
        vec![],
    ));
    let mut files = LoadedFiles::new();
    files.push(
        gt_test_utils::loaded_file_with_tracks(vec![track]),
        FileHistory::None,
    );
    let request = Request::CheckQuery(QueryText {
        text: "@accel | where @accel.x > 1 g".to_owned(),
    });
    let (reply, ()) = tokio::join!(super::request_at(&fixture.endpoint, &request), async {
        tokio::time::timeout(Duration::from_secs(1), wakes.recv())
            .await
            .unwrap()
            .unwrap();
        server.dispatch(|request| {
            let Request::CheckQuery(args) = request.request else {
                panic!("expected a query check request");
            };
            Inspection::new(files.view(), "{filename}", request.session)
                .execute(InspectionRequest::CheckQuery(args))
                .unwrap_or_else(Reply::from)
        });
    });
    assert_eq!(reply.unwrap(), ErrorCode::ReplyTooLarge.into());
}

#[test]
fn point_pages_keep_recorded_order_and_missing_values() {
    let start = DateTime::UNIX_EPOCH + TimeDelta::nanoseconds(739);
    let mut points =
        gt_test_utils::fixtures::nav_points_without_a_valid_position(MAX_POINTS_PER_PAGE + 1);
    let mut earlier = gt_test_utils::fixtures::nav_points_from(start, 1, 1);
    points.append(&mut earlier);
    let mut files = LoadedFiles::new();
    files.push(
        gt_test_utils::loaded_file_with_tracks(vec![gt_test_utils::loaded_track_with_points(
            points,
        )]),
        FileHistory::None,
    );
    let session = Uuid::nil();
    let view = Inspection::new(files.view(), "{filename}", session);
    let recording = RecordingReference {
        session,
        number: files.view().get(0).unwrap().id().into(),
    };
    let Ok(Reply::Recording { details }) =
        view.execute(InspectionRequest::GetRecording(GetRecording {
            recording,
            after: None,
            stored_track_start: None,
            limit: None,
        }))
    else {
        panic!("expected recording");
    };
    let track = *details.tracks.first().unwrap();
    let mut args = GetRangeDetails {
        track,
        start: Some(1),
        end: None,
        limit: Some(NonZeroUsize::MAX),
    };
    let Ok(Reply::Range { details: page }) = view.execute(InspectionRequest::GetRangeDetails(args))
    else {
        panic!("expected point page");
    };
    assert_eq!(page.points.len(), MAX_POINTS_PER_PAGE);
    let previous = page.points.last().unwrap();
    assert_eq!(previous.position, None);
    assert_eq!(previous.eph_m, NumericObservation::Missing);
    assert_eq!(previous.system_time, None);
    assert_eq!(
        page.next_start,
        Some(PointIdx::new(MAX_POINTS_PER_PAGE + 1))
    );

    args.start = page.next_start.map(PointIdx::as_usize);
    let Ok(Reply::Range { details: page }) = view.execute(InspectionRequest::GetRangeDetails(args))
    else {
        panic!("expected last point");
    };
    assert_eq!(page.points.first().unwrap().time, start);
    assert!(previous.time > start);
    assert_eq!(page.next_start, None);
    args.end = args.start;
    let Ok(Reply::Range { details: empty }) =
        view.execute(InspectionRequest::GetRangeDetails(args))
    else {
        panic!("expected empty interval");
    };
    assert!(empty.points.is_empty());
    args.end = Some(0);
    assert_eq!(
        view.execute(InspectionRequest::GetRangeDetails(args)),
        Err(ErrorCode::InvalidRange)
    );
    args.start = Some(0);
    args.end = Some(MAX_POINTS_PER_PAGE + 3);
    args.limit = Some(NonZeroUsize::MIN);
    assert_eq!(
        view.execute(InspectionRequest::GetRangeDetails(args)),
        Err(ErrorCode::InvalidRange)
    );
}

#[tokio::test]
async fn framed_point_measurements_distinguish_missing_finite_and_infinite_values() {
    let finite_eph = f64::from(2.4_f32);
    let finite_speed = 51.248178375505404;
    let mut recorder = NavFileBuilder::new().open();
    for (seconds, eph, speed) in [
        (0, None, None),
        (1, Some(f64::INFINITY), Some(f64::NEG_INFINITY)),
        (2, Some(f64::NEG_INFINITY), Some(f64::INFINITY)),
        (3, Some(finite_eph), Some(finite_speed)),
    ] {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(
                    DateTime::UNIX_EPOCH + TimeDelta::seconds(seconds),
                ))
                .lat(Angle::degrees(55.7))
                .lon(Angle::degrees(12.6))
                .maybe_eph_m(eph)
                .maybe_speed(speed.map(Velocity::meter_per_second))
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    let mut files = LoadedFiles::new();
    files.push(
        gt_loader::load_bytes(&bytes, "measurements.gtd".to_owned()).unwrap(),
        FileHistory::None,
    );
    let track = files
        .view()
        .session_track_ref(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)))
        .unwrap();
    let session = Uuid::nil();
    let view = Inspection::new(files.view(), "{filename}", session);
    let reply = self::framed_inspection_reply(
        &view,
        InspectionRequest::GetRangeDetails(GetRangeDetails {
            track: TrackReference::new(session, track),
            start: None,
            end: None,
            limit: None,
        }),
    )
    .await;
    let json = serde_json::to_value(reply).unwrap();
    let observations: Vec<_> = json
        .pointer("/details/points")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|point| {
            (
                point.get("eph_m").unwrap(),
                point.get("velocity_mps").unwrap(),
            )
        })
        .collect();
    let missing = json!({"status": "missing"});
    let non_finite = json!({"status": "non_finite"});
    assert_eq!(
        observations,
        [
            (&missing, &missing),
            (&non_finite, &non_finite),
            (&non_finite, &non_finite),
            (
                &json!({"status": "finite", "value": finite_eph}),
                &json!({"status": "finite", "value": finite_speed}),
            ),
        ]
    );
}

#[tokio::test]
async fn client_rejects_a_peer_with_a_different_owner() {
    let fixture = TestEndpoint::new().unwrap();
    let _server = Server::bind(fixture.endpoint.clone(), || {}).unwrap();
    let stream = fixture.endpoint.connect().await.unwrap();
    let other = fixture.endpoint.with_different_owner();
    assert_eq!(
        other.verify_peer(&stream).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn bad_frames_receive_errors_before_dispatch() {
    let fixture = TestEndpoint::new().unwrap();
    let (wake, mut wakes) = mpsc::channel(1);
    let _server =
        Server::bind(fixture.endpoint.clone(), move || wake.try_send(()).unwrap()).unwrap();
    for (length, body, code) in [
        (1, b"!".as_slice(), ErrorCode::InvalidRequest),
        (
            u32::try_from(MAX_REQUEST_BYTES + 1).unwrap(),
            &[],
            ErrorCode::RequestTooLarge,
        ),
    ] {
        let mut stream = fixture.endpoint.connect().await.unwrap();
        stream.write_u32(length).await.unwrap();
        stream.write_all(body).await.unwrap();
        let reply: Reply = framing::read(&mut stream, MAX_REPLY_BYTES).await.unwrap();
        assert_eq!(reply, Reply::Error { code });
    }
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
}

#[tokio::test]
async fn apply_processing_rejects_extra_wire_fields_before_dispatch() {
    let fixture = TestEndpoint::new().unwrap();
    let (wake, mut wakes) = mpsc::channel(1);
    let _server =
        Server::bind(fixture.endpoint.clone(), move || wake.try_send(()).unwrap()).unwrap();
    for (field, value) in [
        (
            "recording",
            serde_json::json!({"session": Uuid::nil(), "number": 1}),
        ),
        ("dry_run", serde_json::json!(true)),
        ("extra", Value::Null),
    ] {
        let mut request = serde_json::json!({"method": "apply_processing"});
        request[field] = value;
        let mut stream = fixture.endpoint.connect().await.unwrap();
        Frame::encode(&request, MAX_REQUEST_BYTES)
            .unwrap()
            .write(&mut stream)
            .await
            .unwrap();
        let reply = tokio::select! {
            reply = framing::read::<Reply>(&mut stream, MAX_REPLY_BYTES) => reply.unwrap(),
            _ = wakes.recv() => panic!("Unexpected field {field} reached dispatch"),
        };
        assert_eq!(
            reply,
            Reply::from(ErrorCode::InvalidRequest),
            "field {field}"
        );
    }
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn apply_processing_keeps_the_existing_wire_command() {
    let command = r#"{"method":"apply_processing"}"#;
    let request = Request::ApplyProcessing(ApplyProcessing {});
    assert_eq!(serde_json::to_string(&request).unwrap(), command);
    assert_eq!(serde_json::from_str::<Request>(command).unwrap(), request);
}

#[tokio::test]
async fn shutdown_disconnects_pending_client_and_releases_endpoint() {
    let fixture = TestEndpoint::new().unwrap();
    let (wake, mut wakes) = mpsc::channel(1);
    let server =
        Server::bind(fixture.endpoint.clone(), move || wake.try_send(()).unwrap()).unwrap();
    Server::bind(fixture.endpoint.clone(), || {}).unwrap_err();
    let mut stream = fixture.endpoint.connect().await.unwrap();
    Frame::encode(&REQUEST, MAX_REQUEST_BYTES)
        .unwrap()
        .write(&mut stream)
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), wakes.recv())
            .await
            .unwrap(),
        Some(())
    );
    drop(server);
    let after_shutdown = tokio::time::timeout(Duration::from_secs(1), stream.read_u8())
        .await
        .unwrap();
    after_shutdown.unwrap_err();
    assert_eq!(
        super::request_at(&fixture.endpoint, &REQUEST)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    drop(Server::bind(fixture.endpoint.clone(), || {}).unwrap());
}

proptest! {
    #[test]
    fn frame_decoder_checks_declared_size_before_decoding_body(
        length in 0u8..=FRAME_TEST_LIMIT + 1,
        bytes in prop_oneof![
            collection::vec(arbitrary::any::<u8>(), 0..=usize::from(FRAME_TEST_LIMIT) + 1),
            arbitrary::any::<i64>().prop_map(|value| value.to_string().into_bytes()),
        ],
    ) {
        let packet = u32::from(length).to_be_bytes().into_iter().chain(bytes.iter().copied()).collect::<Vec<_>>();
        let mut stream = packet.as_slice();
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let result = runtime.block_on(framing::read::<i64>(&mut stream, usize::from(FRAME_TEST_LIMIT)));
        let expected = if length > FRAME_TEST_LIMIT {
            Err(ErrorKind::FileTooLarge)
        } else if let Some(payload) = bytes.get(..usize::from(length)) {
            serde_json::from_slice(payload).map_err(|_error| ErrorKind::InvalidData)
        } else {
            Err(ErrorKind::UnexpectedEof)
        };
        prop_assert_eq!(result.map_err(|error| error.kind()), expected);
    }
}

#[tokio::test]
async fn frame_decoder_accepts_a_payload_at_the_limit() {
    let body = "a".repeat(MAX_REQUEST_BYTES - 2);
    let (mut writer, mut reader) = tokio::io::duplex(MAX_REQUEST_BYTES + mem::size_of::<u32>());
    Frame::encode(&body, MAX_REQUEST_BYTES)
        .unwrap()
        .write(&mut writer)
        .await
        .unwrap();
    let decoded: String = framing::read(&mut reader, MAX_REQUEST_BYTES).await.unwrap();
    assert_eq!(decoded, body);
}

const REQUEST: Request = Request::ListRecordings(ListRecordings {
    after: None,
    limit: Some(NonZeroUsize::MIN),
});

const EMPTY_RECORDINGS: Reply = Reply::Recordings {
    page: Recordings {
        recordings: Vec::new(),
        next_after: None,
    },
};

const BRIDGE_TEST_TIMEOUT: Duration = Duration::from_secs(1);
const FRAME_TEST_LIMIT: u8 = 32;
const IDENTITY_NAME_TEMPLATE: &str = "{identity}";
const JSON_CONTROL_CHARACTER: char = '\x01';
