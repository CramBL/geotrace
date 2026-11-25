use std::io::ErrorKind;
use std::mem;
use std::num::NonZeroUsize;
use std::time::Duration;

use chrono::{DateTime, TimeDelta};
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_query::{ConstructKind, MetricSource, Span};
use gt_test_utils::fixtures;
use gt_types::PointIdx;
use proptest::strategy::Strategy as _;
use proptest::{arbitrary, collection, prop_assert_eq, prop_oneof, proptest};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::endpoint::Endpoint;
use super::framing::{self, Frame, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use super::{
    ApplyProcessing, ChannelEntry, ErrorCode, GetRangeDetails, GetRecording, GetTrack, Inspection,
    InspectionRequest, ListMetrics, ListRecordings, MAX_POINTS_PER_PAGE, MAX_TRACKS_PER_PAGE,
    Pending, QueryText, QueryValidation, Recording, RecordingReference, Recordings, Reply, Request,
    Server,
};

struct TestEndpoint {
    endpoint: Endpoint,
    #[cfg(unix)]
    _directory: tempfile::TempDir,
}

impl TestEndpoint {
    fn new() -> Self {
        #[cfg(unix)]
        {
            let directory = tempfile::tempdir().unwrap();
            Self {
                endpoint: Endpoint::in_directory(directory.path().join("endpoint")),
                _directory: directory,
            }
        }
        #[cfg(windows)]
        Self {
            endpoint: Endpoint::test_endpoint().unwrap(),
        }
    }
}

#[tokio::test]
async fn local_request_dispatches_on_the_calling_thread() {
    let fixture = TestEndpoint::new();
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
    let (reply, ()) = tokio::join!(super::request_at(&fixture.endpoint, REQUEST), async {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), wakes.recv())
                .await
                .unwrap(),
            Some(())
        );
        let effect = server.dispatch(|request| {
            let Request::ListRecordings(args) = request.request else {
                panic!("expected a recording list request");
            };
            let reply = Inspection::new(files.view(), "{title} ({filename})", request.session)
                .execute(InspectionRequest::ListRecordings(args))
                .unwrap_or_else(Reply::from);
            (reply, request.session)
        });
        assert_eq!(effect, Some(id.session));
        assert!(
            server
                .dispatch::<()>(|_| panic!("unexpected request"))
                .is_none()
        );
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
                    time_range: None,
                    track_count: 0,
                }],
                next_after: None
            }
        }
    );
}

#[test]
fn dispatcher_discards_the_effect_when_the_reply_receiver_closes_during_preparation() {
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
    };
    let effect = server.dispatch(|request| {
        drop(receiver);
        assert_eq!(request.request, REQUEST);
        (Reply::from(ErrorCode::InvalidRequest), request.session)
    });
    assert_eq!(effect, None);
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
    let fixture = TestEndpoint::new();
    let request = Request::CheckQuery(QueryText {
        text: "\n".repeat(MAX_REQUEST_BYTES / 2),
    });
    let error = super::request_at(&fixture.endpoint, request)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::FileTooLarge);
}

#[tokio::test]
async fn query_diagnostics_exceeding_the_frame_limit_return_an_error() {
    let fixture = TestEndpoint::new();
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
    let (reply, ()) = tokio::join!(super::request_at(&fixture.endpoint, request), async {
        tokio::time::timeout(Duration::from_secs(1), wakes.recv())
            .await
            .unwrap()
            .unwrap();
        let effect = server.dispatch(|request| {
            let Request::CheckQuery(args) = request.request else {
                panic!("expected a query check request");
            };
            let reply = Inspection::new(files.view(), "{filename}", request.session)
                .execute(InspectionRequest::CheckQuery(args))
                .unwrap_or_else(Reply::from);
            (reply, ())
        });
        assert_eq!(effect, Some(()));
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
    assert_eq!(previous.eph_m, None);
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
async fn client_rejects_a_peer_with_a_different_owner() {
    let fixture = TestEndpoint::new();
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
    let fixture = TestEndpoint::new();
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
    let fixture = TestEndpoint::new();
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
    let fixture = TestEndpoint::new();
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
        super::request_at(&fixture.endpoint, REQUEST)
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
const FRAME_TEST_LIMIT: u8 = 32;
