//! The recording history a session holds, and what it may do with it.

use crate::{ReadOnlyRecordings, Recordings};

/// The recording history database, opened writable for the instance that owns
/// the data directory and read-only for a session beside it.
///
/// [`Self::ReadOnly`] holds a type with no insert, no update and no delete
/// method, so a write in a read-only session fails to compile.
pub enum RecordingsHandle {
    Owner(Recordings),
    ReadOnly(ReadOnlyRecordings),
}

impl RecordingsHandle {
    /// The database's read methods, which both variants have.
    pub fn read(&self) -> &ReadOnlyRecordings {
        match self {
            Self::Owner(recordings) => recordings,
            Self::ReadOnly(recordings) => recordings,
        }
    }

    /// The database to write to, or [`None`] in a read-only session.
    pub fn writer(&mut self) -> Option<&mut Recordings> {
        match self {
            Self::Owner(recordings) => Some(recordings),
            Self::ReadOnly(_) => None,
        }
    }
}
