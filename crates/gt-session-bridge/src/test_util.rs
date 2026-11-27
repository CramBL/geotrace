use std::io::{self, ErrorKind};

#[cfg(unix)]
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::oneshot;

use crate::endpoint::Endpoint;
use crate::framing::{self, MAX_REPLY_BYTES};
use crate::{EXCHANGE_TIMEOUT, Reply, Server, SessionExchange, SessionRequest};

pub struct TestSessionReply(oneshot::Receiver<Reply>);

impl TestSessionReply {
    pub fn channel(request: SessionRequest) -> (SessionExchange, Self) {
        let (reply, receiver) = oneshot::channel();
        (SessionExchange { request, reply }, Self(receiver))
    }

    pub fn try_recv(&mut self) -> io::Result<Option<Reply>> {
        match self.0.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(oneshot::error::TryRecvError::Empty) => Ok(None),
            Err(error) => Err(io::Error::other(error)),
        }
    }
}

/// An isolated socket or pipe for local server integration tests.
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
