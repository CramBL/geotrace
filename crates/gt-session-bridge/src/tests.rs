use std::io::ErrorKind;
use std::mem;
use std::num::NonZeroUsize;
use std::time::Duration;

use gt_loaded_files::{FileHistory, LoadedFiles};
use proptest::strategy::Strategy as _;
use proptest::{arbitrary, collection, prop_assert_eq, prop_oneof, proptest};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc;

use super::endpoint::Endpoint;
use super::framing::{self, Frame, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use super::{ErrorCode, Recording, Recordings, Reply, Request, Server};

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
    let id = u64::from(files.view().get(0).unwrap().id());
    let (reply, ()) = tokio::join!(super::request_at(&fixture.endpoint, REQUEST), async {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), wakes.recv())
                .await
                .unwrap(),
            Some(())
        );
        server.dispatch(files.view(), "{title} ({filename})");
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
    assert_eq!(wakes.try_recv(), Err(mpsc::error::TryRecvError::Empty));
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

const REQUEST: Request = Request::ListRecordings {
    after: None,
    limit: NonZeroUsize::MIN,
};
const FRAME_TEST_LIMIT: u8 = 32;
