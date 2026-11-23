use std::io::{self, ErrorKind};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName as _};
use interprocess::os::windows::local_socket::ListenerOptionsExt as _;
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use widestring::U16CString;
use win_custody::{ObjectSecurityDescriptor, OwnedSid};
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;

use super::{BoundListener, ENDPOINT_PREFIX};

#[derive(Clone, Debug)]
pub(crate) struct Endpoint {
    name: String,
    sid: String,
}

impl Endpoint {
    pub(crate) fn current_user() -> io::Result<Self> {
        let sid = win_custody::current_user_sid_string()?;
        Ok(Self {
            name: format!("{ENDPOINT_PREFIX}-{sid}"),
            sid,
        })
    }

    pub(crate) fn bind(&self) -> io::Result<BoundListener> {
        let sid = &self.sid;
        let descriptor = U16CString::from_str(format!("O:{sid}D:P(A;;GRGW;;;{sid})"))
            .map_err(io::Error::other)?;
        let socket = ListenerOptions::new()
            .name(self.name.as_str().to_ns_name::<GenericNamespaced>()?)
            .security_descriptor(SecurityDescriptor::deserialize(&descriptor)?)
            .create_tokio()?;
        Ok(BoundListener { socket })
    }

    pub(crate) async fn connect(&self) -> io::Result<NamedPipeClient> {
        let path = format!(r"\\.\pipe\{}", self.name);
        let stream = loop {
            match ClientOptions::new()
                .security_qos_flags(SECURITY_IDENTIFICATION)
                .open(&path)
            {
                Ok(stream) => break stream,
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY.cast_signed()) => {
                    tokio::time::sleep(PIPE_BUSY_RETRY_INTERVAL).await;
                }
                Err(error) => return Err(error),
            }
        };
        self.verify_peer(&stream)?;
        Ok(stream)
    }

    pub(crate) fn verify_peer(&self, stream: &NamedPipeClient) -> io::Result<()> {
        let descriptor = ObjectSecurityDescriptor::query_kernel_object(stream)?;
        let owner = descriptor.owner().map_err(io::Error::other)?;
        if !owner.matches(&OwnedSid::from_string(&self.sid)?) {
            return Err(ErrorKind::PermissionDenied.into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_endpoint() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut endpoint = Self::current_user()?;
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        endpoint.name = format!("{}-test-{}-{next}", endpoint.name, std::process::id());
        Ok(endpoint)
    }

    #[cfg(test)]
    pub(crate) fn with_different_owner(&self) -> Self {
        Self {
            name: self.name.clone(),
            sid: "S-1-0-0".to_owned(),
        }
    }
}

const PIPE_BUSY_RETRY_INTERVAL: Duration = Duration::from_millis(20);
