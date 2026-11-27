use std::io::{self, ErrorKind};
use std::sync::mpsc as startup;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use tokio::io::AsyncReadExt as _;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::endpoint::{BoundListener, Connection, Endpoint};
use crate::framing::{Frame, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
pub use crate::history::{
    GetHistoryOpenStatus, HistoryCursor, HistoryDates, HistoryListing, HistoryOpenId,
    HistoryOpenState, HistoryOpenStatus, HistoryRecording, HistoryRecordingId, HistoryRecordings,
    ListHistoryRecordings, OpenHistoryRecording,
};
pub use crate::inspection::{
    Inspection, InspectionRequest, MAX_POINTS_PER_PAGE, MAX_TRACKS_PER_PAGE, MapBounds,
    PointDetails, Position, RangeDetails, RecordingDetails, StoredTrackPage, StoredTrackRow,
    StoredTrackState, TrackDetails, TrackInspectionState, TrackPageState, TrackTreeState,
};
pub use crate::metrics::{
    ChannelComponent, ChannelDefinition, ChannelEntry, MetricDefinition, Metrics,
};
pub use crate::protocol::{
    ApplyProcessing, ErrorCode, GetRangeDetails, GetRecording, GetTrack, ListMetrics,
    ListRecordings, MAX_VISIBILITY_TRACKS, NumericObservation, OpenPoint, QueryText, RangeTarget,
    Recording, RecordingReference, Recordings, Reply, Request, SetTrackVisibility, ShowOnMap,
    ShowOnPlot, TimeRange, TrackReference, TrackVisibilityAction, TrackVisibilityCountError,
    TrackVisibilityResult, VisibilityTrackRefs,
};
pub use crate::query_job::{
    GetQueryStatus, QueryCounts, QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus,
};
pub use crate::query_results::{
    GetQueryResults, QueryMatch, QueryMatchPosition, QueryPageRequest, QueryResultColumn,
    QueryResultPage, QueryResultRow, QueryResultSource, QueryResultTime, QueryResults,
};
pub use crate::query_validation::{QueryPreparation, QueryValidation};
pub use crate::recording_control::{
    MAX_RECORDING_BATCH, ManageRecordings, RecordingBatch, RecordingBatchSizeError,
};
pub use crate::session_reply::{DeferredSessionReply, SessionReply};
pub use crate::session_state::{
    OpenPanel, PanelOpenState, SessionMapBounds, SessionMapLayer, SessionMapState, SessionPanel,
    SessionQueryPhase, SessionQueryState, SessionState, SessionTimeFilter,
};
pub use crate::session_view::{DisplayCategoryChange, SetSessionView, TimeFilterChange};
pub use crate::settings::{
    AnalysisSettingKey, ProcessingSettingKey, SetSetting, SettingApply, SettingInfo, SettingInput,
    SettingKey, SettingSelection, SettingUnit, SettingValue,
};
pub use crate::status_wait::{StatusWait, StatusWaitOutOfRange};
use crate::traffic_capture::ExchangeCapture;
pub use crate::traffic_capture::{TrafficCapture, TrafficDirection, TrafficFrame};

mod endpoint;
mod framing;
mod history;
mod inspection;
mod metrics;
mod protocol;
mod query_job;
mod query_results;
mod query_validation;
mod recording_control;
mod session_reply;
mod session_state;
mod session_view;
mod settings;
mod status_wait;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
mod traffic_capture;

#[derive(Debug)]
pub struct Server {
    session: Uuid,
    pending: mpsc::Receiver<Pending>,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
    traffic_capture: TrafficCapture,
}

impl Server {
    pub fn start(wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        Self::bind(Endpoint::current_user()?, wake)
    }

    fn bind(endpoint: Endpoint, wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let (pending_tx, pending) = mpsc::channel(1);
        let (stop_tx, stop_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = startup::sync_channel(1);
        let traffic_capture = TrafficCapture::new();
        let worker_capture = traffic_capture.clone();
        let worker = thread::Builder::new()
            .name("session-bridge".to_owned())
            .spawn(move || {
                let setup = (|| -> io::Result<_> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let listener = {
                        let _entered = runtime.enter();
                        endpoint.bind()?
                    };
                    Ok((runtime, listener))
                })();
                match setup {
                    Err(error) => drop(ready_tx.send(Err(error))),
                    Ok((runtime, listener)) => {
                        if ready_tx.send(Ok(())).is_ok() {
                            let stopped = runtime.block_on(async {
                                tokio::select! {
                                    biased;
                                    _ = stop_rx => None,
                                    result = self::accept_connections(listener, &pending_tx, &wake, &worker_capture) => result.err(),
                                }
                            });
                            drop(pending_tx);
                            if let Some(error) = stopped {
                                log::warn!("Local session bridge stopped ({error})");
                                wake();
                            }
                        }
                    }
                }
            })?;
        ready_rx.recv().map_err(io::Error::other)??;
        Ok(Self {
            session: Uuid::new_v4(),
            pending,
            stop: Some(stop_tx),
            worker: Some(worker),
            traffic_capture,
        })
    }

    /// Skip a queued request if its reply receiver has closed.
    pub fn dispatch(&mut self, execute: impl FnOnce(SessionRequest) -> Reply) {
        if let Some(exchange) = self.receive() {
            exchange.respond(execute);
        }
    }

    pub fn receive(&mut self) -> Option<SessionExchange> {
        let pending = self.pending.try_recv().ok()?;
        if !pending.reply.client_is_waiting() {
            return None;
        }
        Some(SessionExchange {
            request: SessionRequest {
                request: pending.request,
                session: self.session,
            },
            reply: pending.reply,
        })
    }

    pub fn traffic_capture(&self) -> TrafficCapture {
        self.traffic_capture.clone()
    }

    pub fn is_running(&self) -> bool {
        !self.pending.is_closed()
    }
}

pub struct SessionRequest {
    pub request: Request,
    pub session: Uuid,
}

pub struct SessionExchange {
    request: SessionRequest,
    reply: SessionReply,
}

impl SessionExchange {
    pub fn request(&self) -> &Request {
        &self.request.request
    }

    pub fn client_is_waiting(&self) -> bool {
        self.reply.client_is_waiting()
    }

    pub fn respond(self, execute: impl FnOnce(SessionRequest) -> Reply) {
        if let Some((request, reply)) = self.into_request() {
            reply.send(execute(request));
        }
    }

    pub fn into_request(self) -> Option<(SessionRequest, SessionReply)> {
        self.client_is_waiting()
            .then_some((self.request, self.reply))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.traffic_capture.set_enabled(false);
        self.traffic_capture.clear();
        drop(self.stop.take());
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            log::warn!("Local session bridge worker panicked");
        }
    }
}

/// Call from a [`tokio::runtime::Runtime`] with I/O and time enabled.
pub async fn request(request: &Request) -> io::Result<Reply> {
    self::request_at(&Endpoint::current_user()?, request).await
}

async fn request_at(endpoint: &Endpoint, request: &Request) -> io::Result<Reply> {
    let frame = Frame::encode(request, MAX_REQUEST_BYTES).map_err(|error| {
        io::Error::new(
            error.io_error_kind().unwrap_or(ErrorKind::InvalidData),
            error,
        )
    })?;
    tokio::time::timeout(EXCHANGE_TIMEOUT, async {
        let mut stream = endpoint.connect().await?;
        frame.write(&mut stream).await?;
        framing::read(&mut stream, MAX_REPLY_BYTES).await
    })
    .await
    .map_err(|error| io::Error::new(ErrorKind::TimedOut, error))?
}

async fn accept_connections(
    listener: BoundListener,
    pending: &mpsc::Sender<Pending>,
    wake: &impl Fn(),
    traffic_capture: &TrafficCapture,
) -> io::Result<()> {
    let accept = listener.accept();
    tokio::pin!(accept);
    let mut exchanges = FuturesUnordered::new();
    loop {
        tokio::select! {
            biased;
            Some(()) = exchanges.next() => {}
            accepted = &mut accept => {
                accept.set(listener.accept());
                let Some(connection) = accepted? else {
                    continue;
                };
                if exchanges.len() == MAX_ACTIVE_CONNECTIONS {
                    continue;
                }
                exchanges.push(self::serve_connection(
                    connection, pending, wake, traffic_capture.start_exchange(),
                ));
            }
        }
    }
}

async fn serve_connection(
    mut connection: Connection,
    pending: &mpsc::Sender<Pending>,
    wake: &impl Fn(),
    capture: Option<ExchangeCapture<'_>>,
) {
    let deadline = Instant::now() + EXCHANGE_TIMEOUT;
    let exchange = self::exchange(&mut connection.0, pending, wake, capture.as_ref(), deadline);
    if let Ok(Err(error)) = tokio::time::timeout_at(deadline.into(), exchange).await {
        match error.kind() {
            ErrorKind::BrokenPipe
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected
            | ErrorKind::UnexpectedEof => {}
            kind => log::warn!("Local session bridge exchange failed ({kind:?})"),
        }
    }
}

async fn exchange(
    stream: &mut interprocess::local_socket::tokio::Stream,
    pending: &mpsc::Sender<Pending>,
    wake: &impl Fn(),
    capture: Option<&ExchangeCapture<'_>>,
    deadline: Instant,
) -> io::Result<()> {
    let reply = match self::read_request(stream, capture).await {
        Ok(request) => {
            let (reply, response) = SessionReply::channel(deadline);
            match pending.try_send(Pending { request, reply }) {
                Ok(()) => {
                    wake();
                    tokio::select! {
                        incoming = stream.read_u8() => return incoming.map(|_byte| ()),
                        response = async { response.await.map_err(io::Error::other)?.resolve().await } => response?,
                    }
                }
                Err(mpsc::error::TrySendError::Full(_)) => Reply::Error {
                    code: ErrorCode::Busy,
                },
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Ok(());
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::InvalidData => Reply::Error {
            code: ErrorCode::InvalidRequest,
        },
        Err(error) if error.kind() == ErrorKind::FileTooLarge => Reply::Error {
            code: ErrorCode::RequestTooLarge,
        },
        Err(error) => return Err(error),
    };
    let frame = Frame::encode(&reply, MAX_REPLY_BYTES)
        .or_else(|error| {
            if error.io_error_kind() == Some(ErrorKind::FileTooLarge) {
                Frame::encode(
                    &Reply::Error {
                        code: ErrorCode::ReplyTooLarge,
                    },
                    MAX_REPLY_BYTES,
                )
            } else {
                Err(error)
            }
        })
        .map_err(io::Error::other)?;
    {
        let payload = frame.write(stream).await?;
        if let Some(capture) = capture {
            capture.record(TrafficDirection::Reply, payload);
        }
    }
    // The client closes the pipe after reading its reply.
    stream.read_u8().await.map(|_byte| ())
}

async fn read_request(
    stream: &mut interprocess::local_socket::tokio::Stream,
    capture: Option<&ExchangeCapture<'_>>,
) -> io::Result<Request> {
    let payload = framing::read_payload(stream, MAX_REQUEST_BYTES).await?;
    let request = framing::decode(&payload);
    if let Some(capture) = capture {
        capture.record(TrafficDirection::Request, payload);
    }
    request
}

struct Pending {
    request: Request,
    reply: SessionReply,
}

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_ACTIVE_CONNECTIONS: usize = 8;
/// Reserve half of the active connections for requests outside the query and History waits.
pub const MAX_DEFERRED_OPERATION_REPLIES: usize = MAX_ACTIVE_CONNECTIONS / 2;

#[cfg(test)]
mod tests;
