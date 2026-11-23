use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{
    DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _,
};
use std::path::{Path, PathBuf};

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::StreamCommon as _;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName as _};

use super::{BoundListener, ENDPOINT_PREFIX};

#[derive(Clone, Debug)]
pub(crate) struct Endpoint {
    directory: PathBuf,
    user: u32,
}

impl Endpoint {
    pub(crate) fn current_user() -> io::Result<Self> {
        let user = rustix::process::geteuid().as_raw();
        let root = Path::new(TEMP_ROOT);
        let metadata = fs::metadata(root)?;
        if !metadata.is_dir()
            || metadata.uid() != ROOT_USER_ID
            || (metadata.mode() & NON_OWNER_WRITE_PERMISSIONS != 0
                && metadata.mode() & DIRECTORY_STICKY_BIT == 0)
        {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "Untrusted local socket directory",
            ));
        }
        Ok(Self {
            directory: root.join(format!("{ENDPOINT_PREFIX}-{user}")),
            user,
        })
    }

    fn check_directory(&self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(&self.directory)?;
        if metadata.is_dir()
            && metadata.uid() == self.user
            && metadata.mode() & NON_OWNER_PERMISSIONS == 0
        {
            Ok(())
        } else {
            Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "Local socket directory must be private",
            ))
        }
    }

    pub(crate) fn bind(&self) -> io::Result<BoundListener> {
        if let Err(error) = DirBuilder::new()
            .mode(PRIVATE_DIRECTORY_MODE)
            .create(&self.directory)
            && error.kind() != ErrorKind::AlreadyExists
        {
            return Err(error);
        }
        self.check_directory()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(PRIVATE_LOCK_MODE)
            .open(self.directory.join("session.lock"))?;
        lock.try_lock()?;
        let path = self.directory.join(SOCKET_FILENAME);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == self.user => {
                fs::remove_file(&path)?;
            }
            Ok(_) => return Err(ErrorKind::AddrInUse.into()),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let socket = ListenerOptions::new()
            .name(path.to_fs_name::<GenericFilePath>()?)
            .create_tokio()?;
        Ok(BoundListener {
            socket,
            user: self.user,
            _lock: lock,
        })
    }

    pub(crate) async fn connect(&self) -> io::Result<Stream> {
        self.check_directory()?;
        let path = self.directory.join(SOCKET_FILENAME);
        let stream = Stream::connect(path.to_fs_name::<GenericFilePath>()?).await?;
        self.verify_peer(&stream)?;
        Ok(stream)
    }

    pub(crate) fn verify_peer(&self, stream: &Stream) -> io::Result<()> {
        if self::peer_user_matches(stream, self.user)? {
            Ok(())
        } else {
            Err(ErrorKind::PermissionDenied.into())
        }
    }

    #[cfg(test)]
    pub(crate) fn in_directory(directory: PathBuf) -> Self {
        Self {
            directory,
            user: rustix::process::geteuid().as_raw(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_different_owner(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            user: self.user.wrapping_add(1),
        }
    }
}

pub(super) fn peer_user_matches(stream: &Stream, user: u32) -> io::Result<bool> {
    Ok(stream.peer_creds()?.euid() == Some(user))
}

const TEMP_ROOT: &str = "/tmp";
const SOCKET_FILENAME: &str = "session.sock";
const ROOT_USER_ID: u32 = 0;
const NON_OWNER_WRITE_PERMISSIONS: u32 = 0o022;
const NON_OWNER_PERMISSIONS: u32 = 0o077;
const DIRECTORY_STICKY_BIT: u32 = 0o1000;
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_LOCK_MODE: u32 = 0o600;
