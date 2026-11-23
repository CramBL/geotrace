use std::io;

use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;

#[cfg(unix)]
pub(crate) use self::unix::Endpoint;
#[cfg(windows)]
pub(crate) use self::windows::Endpoint;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub(crate) struct BoundListener {
    socket: Listener,
    #[cfg(unix)]
    user: u32,
    #[cfg(unix)]
    _lock: std::fs::File,
}

impl BoundListener {
    pub(crate) async fn accept(&self) -> io::Result<Option<Connection>> {
        let stream = self.socket.accept().await?;
        #[cfg(unix)]
        match unix::peer_user_matches(&stream, self.user) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(error) => {
                log::warn!("Local session bridge could not check peer credentials ({error})");
                return Ok(None);
            }
        }
        Ok(Some(Connection(stream)))
    }
}

pub(crate) struct Connection(pub(crate) Stream);

#[cfg(windows)]
impl Drop for Connection {
    fn drop(&mut self) {
        let Stream::NamedPipe(pipe) = &self.0;
        // The peer closes after reading its reply. On cancellation, drop any pending bytes.
        pipe.inner().assume_flushed();
    }
}

const ENDPOINT_PREFIX: &str = "geotrace-session-v1";
