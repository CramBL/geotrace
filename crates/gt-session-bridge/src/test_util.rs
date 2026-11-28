use std::io::{self, ErrorKind};
use std::time::Instant;

#[cfg(unix)]
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::oneshot;

use crate::endpoint::Endpoint;
use crate::framing::{self, MAX_REPLY_BYTES};
use crate::session_reply::{DeferredResponse, ExchangeReply};
use crate::{EXCHANGE_TIMEOUT, Reply, Server, SessionExchange, SessionReply, SessionRequest};

pub struct TestSessionReply {
    exchange: oneshot::Receiver<ExchangeReply>,
    deferred: Option<DeferredResponse>,
}

impl TestSessionReply {
    pub fn channel(request: SessionRequest) -> (SessionExchange, Self) {
        let (reply, exchange) = SessionReply::channel(Instant::now() + EXCHANGE_TIMEOUT);
        (
            SessionExchange { request, reply },
            Self {
                exchange,
                deferred: None,
            },
        )
    }

    pub fn try_recv(&mut self) -> io::Result<Option<Reply>> {
        if self.deferred.is_none() {
            match self.exchange.try_recv() {
                Ok(ExchangeReply::Immediate(reply)) => return Ok(Some(reply)),
                Ok(ExchangeReply::Deferred(response)) => self.deferred = Some(response),
                Err(oneshot::error::TryRecvError::Empty) => return Ok(None),
                Err(error) => return Err(io::Error::other(error)),
            }
        }
        let result = self
            .deferred
            .as_mut()
            .and_then(DeferredResponse::ready_reply);
        if result.is_some() {
            self.deferred = None;
        }
        Ok(result)
    }
}

/// An isolated socket or pipe for local server integration tests.
#[derive(Debug)]
pub struct TestEndpoint {
    pub(crate) endpoint: Endpoint,
    #[cfg(unix)]
    _directory: TempDir,
}

impl TestEndpoint {
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let directory = tempfile::tempdir()?;
            Ok(Self {
                endpoint: Endpoint::in_directory(directory.path().join("endpoint")),
                _directory: directory,
            })
        }
        #[cfg(windows)]
        Ok(Self {
            endpoint: Endpoint::test_endpoint()?,
        })
    }

    pub fn start_server(&self, wake: impl Fn() + Send + 'static) -> io::Result<Server> {
        Server::bind(self.endpoint.clone(), wake)
    }

    /// Blocks on the exchange from a test client thread with its own runtime.
    pub fn exchange_payload(&self, payload: &[u8]) -> io::Result<Vec<u8>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            tokio::time::timeout(EXCHANGE_TIMEOUT, async {
                let mut stream = self.send_payload(payload).await?;
                framing::read_payload(&mut stream, MAX_REPLY_BYTES).await
            })
            .await
            .map_err(|error| io::Error::new(ErrorKind::TimedOut, error))?
        })
    }

    pub(crate) async fn send_payload(
        &self,
        payload: &[u8],
    ) -> io::Result<impl AsyncRead + AsyncWrite + Unpin> {
        let length = u32::try_from(payload.len()).map_err(io::Error::other)?;
        let mut stream = self.endpoint.connect().await?;
        stream.write_u32(length).await?;
        stream.write_all(payload).await?;
        Ok(stream)
    }
}
