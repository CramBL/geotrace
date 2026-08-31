//! Attaching a log to a recording, and reading it back.
//!
//! An attachment has two halves. The history database keeps one attribute per
//! attachment, holding the log's name, its content hash and the filter stack
//! it was attached with. The log itself lives here: one zstd-compressed file
//! per attachment under [`Store::logs_path`](crate::Store::logs_path),
//! written once and checked against its attribute's content hash on every
//! read.

use std::{
    fs, io,
    path::{Path, PathBuf},
    string::FromUtf8Error,
};

use gt_history::{
    DatabaseRef, DbError, HistoryDatabase, LogAttachment, LogAttachmentEntry, LogAttachmentId,
    LogContentHash, ReadOnlyHistoryDatabase, StoredLogFilter, log_attachment,
};
use thiserror::Error;

/// zstd's default level.
const COMPRESSION_LEVEL: i32 = 3;

/// A log to store with a recording.
#[derive(Debug)]
pub struct LogToAttach<'a> {
    /// Name the log was loaded under.
    pub name: &'a str,

    /// The parse buffer: everything that was loaded, in one piece.
    pub text: &'a str,

    /// The filter stack to restore the log with.
    pub filters: Vec<StoredLogFilter>,
}

/// A log read back from an attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedLog {
    pub name: String,
    pub text: String,
    pub filters: Vec<StoredLogFilter>,
}

#[derive(Debug, Error)]
pub enum LogAttachmentError {
    #[error(transparent)]
    Database(#[from] DbError),

    /// The recording carries no attachment under this id.
    #[error("the recording has no log attachment {id}")]
    UnknownAttachment { id: LogAttachmentId },

    /// The attribute names a log the store no longer holds.
    #[error("the attached log at {} is missing", path.display())]
    MissingLog { id: LogAttachmentId, path: PathBuf },

    #[error("could not read or write the attached log at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// The file decompressed to a different log than the one attached.
    #[error(
        "the attached log {id} is not the log it was stored as (hash {found}, expected {expected})"
    )]
    ContentHashMismatch {
        id: LogAttachmentId,
        expected: LogContentHash,
        found: LogContentHash,
    },

    #[error("the attached log {id} is not UTF-8")]
    NotUtf8 {
        id: LogAttachmentId,
        #[source]
        source: FromUtf8Error,
    },
}

/// Reading the logs stored with the recordings of a history database.
///
/// Implemented for every [`ReadOnlyHistoryDatabase`], so a read-only session
/// reads the logs of the recordings it lists.
pub trait ReadOnlyLogAttachments: ReadOnlyHistoryDatabase {
    /// Read an attachment back, checking the log against the hash it was
    /// stored under.
    fn load_attached_log(
        &self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
    ) -> Result<AttachedLog, LogAttachmentError> {
        let attachment = self
            .log_attachments(db_ref)?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or(LogAttachmentError::UnknownAttachment { id })?
            .attachment;

        let path = id.file_path(&log_attachment::logs_directory_for_database(self.path()));
        let bytes = read_compressed_log(&path).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => LogAttachmentError::MissingLog {
                id,
                path: path.clone(),
            },
            _ => LogAttachmentError::Io {
                path: path.clone(),
                source,
            },
        })?;

        let found = LogContentHash::of_log_bytes(&bytes);
        if found != attachment.content_hash {
            return Err(LogAttachmentError::ContentHashMismatch {
                id,
                expected: attachment.content_hash,
                found,
            });
        }

        Ok(AttachedLog {
            name: attachment.name,
            text: String::from_utf8(bytes)
                .map_err(|source| LogAttachmentError::NotUtf8 { id, source })?,
            filters: attachment.filters,
        })
    }
}

impl<T: ReadOnlyHistoryDatabase + ?Sized> ReadOnlyLogAttachments for T {}

/// Storing and removing the logs of a history database's recordings.
///
/// Implemented for every [`HistoryDatabase`]. The database holds the
/// attributes, and these operations pair each one with its compressed log.
pub trait LogAttachments: HistoryDatabase {
    /// Store `log` with a recording and return the attachment it was stored
    /// as.
    ///
    /// A failure to write the attribute removes the log again: an attachment
    /// the database does not name is one nothing could delete later.
    fn attach_log(
        &mut self,
        db_ref: &DatabaseRef,
        log: &LogToAttach<'_>,
    ) -> Result<LogAttachmentEntry, LogAttachmentError> {
        let directory = log_attachment::logs_directory_for_database(self.path());
        let id = LogAttachmentId::new_random();
        let path = id.file_path(&directory);
        write_compressed_log(&path, log.text.as_bytes()).map_err(|source| {
            LogAttachmentError::Io {
                path: path.clone(),
                source,
            }
        })?;

        let attachment = LogAttachment::new(
            log.name.to_owned(),
            LogContentHash::of_log_bytes(log.text.as_bytes()),
            log.filters.clone(),
        );
        if let Err(err) = self.write_log_attachment_attribute(db_ref, id, &attachment) {
            log_attachment::delete_files(&directory, &[id]);
            return Err(err.into());
        }

        log::info!(
            "Attached the log {:?} to the recording {:?} as {id}",
            log.name,
            db_ref.group_name
        );
        Ok(LogAttachmentEntry { id, attachment })
    }

    /// Remove one attachment: its attribute, and the log stored with it.
    ///
    /// Removing an attachment the recording does not have succeeds.
    fn detach_log(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
    ) -> Result<(), LogAttachmentError> {
        let directory = log_attachment::logs_directory_for_database(self.path());
        self.delete_log_attachment_attribute(db_ref, id)?;
        log_attachment::delete_files(&directory, &[id]);
        log::info!(
            "Removed the log attachment {id} from the recording {:?}",
            db_ref.group_name
        );
        Ok(())
    }

    /// Store the filter stack of an attachment the user kept exploring.
    ///
    /// The stored log is untouched: the attribute this rewrites is what
    /// content-addresses it.
    fn set_attached_log_filters(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
        filters: Vec<StoredLogFilter>,
    ) -> Result<(), LogAttachmentError> {
        let stored = self
            .log_attachments(db_ref)?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or(LogAttachmentError::UnknownAttachment { id })?
            .attachment;

        let updated = LogAttachment::new(stored.name, stored.content_hash, filters);
        self.write_log_attachment_attribute(db_ref, id, &updated)?;
        Ok(())
    }
}

impl<T: HistoryDatabase + ?Sized> LogAttachments for T {}

/// Compress `bytes` into `path`, creating the logs directory on the first
/// attachment.
fn write_compressed_log(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    fs::write(path, zstd::encode_all(bytes, COMPRESSION_LEVEL)?)
}

fn read_compressed_log(path: &Path) -> io::Result<Vec<u8>> {
    zstd::decode_all(fs::read(path)?.as_slice())
}
