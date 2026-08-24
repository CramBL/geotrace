//! The archive a session holds, and what it may do with it.

use std::ops::Deref;
use std::sync::Arc;

/// One day archive, opened writable for the instance that owns the data
/// directory and read-only for a session beside it.
///
/// [`Self::ReadOnly`] holds a type with no insert and no delete method, so a
/// write in a read-only session fails to compile.
#[derive(Debug)]
pub enum ArchiveHandle<W, R> {
    Owner(Arc<W>),
    ReadOnly(Arc<R>),
}

impl<W, R> Clone for ArchiveHandle<W, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Owner(archive) => Self::Owner(Arc::clone(archive)),
            Self::ReadOnly(archive) => Self::ReadOnly(Arc::clone(archive)),
        }
    }
}

impl<W, R> ArchiveHandle<W, R> {
    pub(crate) fn owner(archive: W) -> Self {
        Self::Owner(Arc::new(archive))
    }

    pub(crate) fn read_only(archive: R) -> Self {
        Self::ReadOnly(Arc::new(archive))
    }
}

impl<W: Deref<Target = R>, R> ArchiveHandle<W, R> {
    /// The archive's read methods, which both variants have.
    pub fn read(&self) -> &R {
        match self {
            Self::Owner(archive) => archive,
            Self::ReadOnly(archive) => archive,
        }
    }

    /// The archive to write to, or [`None`] in a read-only session.
    pub fn writer(&self) -> Option<Arc<W>> {
        match self {
            Self::Owner(archive) => Some(Arc::clone(archive)),
            Self::ReadOnly(_) => None,
        }
    }
}
