use std::io::{self, ErrorKind, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

pub(crate) async fn read<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
    limit: usize,
) -> io::Result<T> {
    let length = usize::try_from(stream.read_u32().await?)
        .map_err(|error| io::Error::new(ErrorKind::FileTooLarge, error))?;
    if length > limit {
        return Err(ErrorKind::FileTooLarge.into());
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
}

pub(crate) struct Frame {
    payload: Vec<u8>,
    limit: usize,
}

impl Frame {
    pub(crate) fn encode(value: &impl Serialize, limit: usize) -> Result<Self, serde_json::Error> {
        let mut frame = Self {
            payload: Vec::new(),
            limit,
        };
        serde_json::to_writer(&mut frame, value)?;
        Ok(frame)
    }

    pub(crate) async fn write(self, stream: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        let length = u32::try_from(self.payload.len())
            .map_err(|error| io::Error::new(ErrorKind::FileTooLarge, error))?;
        stream.write_u32(length).await?;
        stream.write_all(&self.payload).await
    }

    pub(crate) fn payload_len(&self) -> usize {
        self.payload.len()
    }
}

impl Write for Frame {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.payload.len() {
            return Err(ErrorKind::FileTooLarge.into());
        }
        self.payload.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) const MAX_REQUEST_BYTES: usize = 4 * 1024;
pub(crate) const MAX_REPLY_BYTES: usize = 256 * 1024;
