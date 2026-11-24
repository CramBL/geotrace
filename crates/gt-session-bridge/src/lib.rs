use std::io::{self, ErrorKind};
use std::sync::mpsc as startup;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gt_loaded_files::LoadedFilesView;
use tokio::io::AsyncReadExt as _;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::endpoint::{BoundListener, Endpoint};
use crate::framing::{Frame, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use crate::inspection::Inspection;
pub use crate::inspection::{
    MAX_POINTS_PER_PAGE, MAX_TRACKS_PER_PAGE, MapBounds, PointDetails, Position, RangeDetails,
    RecordingDetails, TrackDetails,
};
pub use crate::metrics::{
    ChannelComponent, ChannelDefinition, ChannelEntry, MetricDefinition, Metrics,
};
pub use crate::protocol::{
    CheckQuery, ErrorCode, GetRangeDetails, GetRecording, GetTrack, ListMetrics, ListRecordings,
    OpenPoint, RangeTarget, Recording, RecordingReference, Recordings, Reply, Request, ShowOnMap,
    ShowOnPlot, TimeRange, TrackReference,
};
pub use crate::query_job::{
    QueryCounts, QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus,
};
pub use crate::query_results::{
    GetQueryResults, QueryMatch, QueryMatchExtent, QueryMatchPosition, QueryMatchRange,
    QueryPageRequest, QueryResultColumn, QueryResultPage, QueryResultRow, QueryResultSource,
    QueryResultTime, QueryResultValue, QueryResults,
};
pub use crate::query_validation::{QueryPreparation, QueryValidation};

mod endpoint;
mod framing;
mod inspection;
mod metrics;
mod protocol;
mod query_job;
mod query_results;
mod query_validation;

#[derive(Debug)]
pub struct Server {
    session: Uuid,
    pending: mpsc::Receiver<Pending>,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        Self::bind(Endpoint::current_user()?, wake)
    }

    fn bind(endpoint: Endpoint, wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let (pending_tx, pending) = mpsc::channel(1);
        let (stop_tx, stop_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = startup::sync_channel(1);
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
                            runtime.block_on(async move {
                                tokio::select! {
                                    biased;
                                    _ = stop_rx => {}
                                    result = self::accept_requests(listener, &pending_tx, wake) => {
                                        if let Err(error) = result {
                                        log::warn!("Local session bridge stopped ({error})");
                                        }
                                    }
                                }
                            });
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
        })
    }

    /// Return true after a successful reply channel send.
    pub fn dispatch(&mut self, execute: impl FnOnce(SessionRequest) -> Reply) -> bool {
        if let Ok(pending) = self.pending.try_recv()
            && !pending.reply.is_closed()
        {
            let request = SessionRequest {
                request: pending.request,
                session: self.session,
            };
            return pending.reply.send(execute(request)).is_ok();
        }
        false
    }
}

pub struct SessionRequest {
    pub request: Request,
    pub session: Uuid,
}

impl SessionRequest {
    pub fn inspect(self, files: LoadedFilesView<'_>, name_template: &str) -> Reply {
        Inspection {
            files,
            name_template,
            session: self.session,
        }
        .execute(self.request)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            log::warn!("Local session bridge worker panicked");
        }
    }
}

/// Call from a [`tokio::runtime::Runtime`] with I/O and time enabled.
pub async fn request(request: Request) -> io::Result<Reply> {
    self::request_at(&Endpoint::current_user()?, request).await
}

async fn request_at(endpoint: &Endpoint, request: Request) -> io::Result<Reply> {
    let frame = Frame::encode(&request, MAX_REQUEST_BYTES).map_err(|error| {
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

async fn accept_requests(
    listener: BoundListener,
    pending: &mpsc::Sender<Pending>,
    wake: impl Fn(),
) -> io::Result<()> {
    loop {
        if let Some(mut connection) = listener.accept().await? {
            match tokio::time::timeout(
                EXCHANGE_TIMEOUT,
                self::exchange(&mut connection.0, pending, &wake),
            )
            .await
            {
                Ok(Ok(())) | Err(_) => {}
                Ok(Err(error)) => match error.kind() {
                    ErrorKind::BrokenPipe
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionReset
                    | ErrorKind::NotConnected
                    | ErrorKind::UnexpectedEof => {}
                    kind => log::warn!("Local session bridge exchange failed ({kind:?})"),
                },
            }
        }
    }
}

async fn exchange(
    stream: &mut interprocess::local_socket::tokio::Stream,
    pending: &mpsc::Sender<Pending>,
    wake: &impl Fn(),
) -> io::Result<()> {
    let reply = match framing::read(stream, MAX_REQUEST_BYTES).await {
        Ok(request) => {
            let (reply, response) = oneshot::channel();
            match pending.try_send(Pending { request, reply }) {
                Ok(()) => {
                    wake();
                    tokio::select! {
                        incoming = stream.read_u8() => return incoming.map(|_byte| ()),
                        response = response => response.map_err(io::Error::other)?,
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
    frame.write(stream).await?;
    // The client closes the pipe after reading its reply.
    stream.read_u8().await.map(|_byte| ())
}

struct Pending {
    request: Request,
    reply: oneshot::Sender<Reply>,
}

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(3);

#[cfg(test)]
mod tests;
