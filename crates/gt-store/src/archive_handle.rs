//! The archive a session holds, and what it may do with it.

use std::ops::Deref;
use std::sync::Arc;

use gt_pending_writes::PendingWrites;

use crate::writable_archive::WritableArchive;

/// One day archive, opened writable for the instance that owns the data
/// directory and read-only for a session beside it.
///
/// A read-only session holds a type with no insert and no delete method, so a
/// write in one fails to compile. The only path to an owner session's mutators
/// is [`Self::writer`], which returns a [`WritableArchive`]: every write
/// through it is registered in [`PendingWrites`].
#[derive(Debug)]
pub struct ArchiveHandle<W, R>(Opened<W, R>);

#[derive(Debug)]
enum Opened<W, R> {
    Owner(Arc<W>),
    ReadOnly(Arc<R>),
}

impl<W, R> Clone for ArchiveHandle<W, R> {
    fn clone(&self) -> Self {
        Self(match &self.0 {
            Opened::Owner(archive) => Opened::Owner(Arc::clone(archive)),
            Opened::ReadOnly(archive) => Opened::ReadOnly(Arc::clone(archive)),
        })
    }
}

impl<W, R> ArchiveHandle<W, R> {
    pub(crate) fn owner(archive: W) -> Self {
        Self(Opened::Owner(Arc::new(archive)))
    }

    pub(crate) fn read_only(archive: R) -> Self {
        Self(Opened::ReadOnly(Arc::new(archive)))
    }
}

impl<W: Deref<Target = R>, R> ArchiveHandle<W, R> {
    /// The archive's read methods, which both kinds of session have.
    pub fn read(&self) -> &R {
        match &self.0 {
            Opened::Owner(archive) => archive,
            Opened::ReadOnly(archive) => archive,
        }
    }

    /// The archive to write to, registering every write in `pending_writes`,
    /// or [`None`] in a read-only session.
    pub fn writer(&self, pending_writes: &PendingWrites) -> Option<WritableArchive<W>> {
        match &self.0 {
            Opened::Owner(archive) => Some(WritableArchive::new(
                Arc::clone(archive),
                pending_writes.clone(),
            )),
            Opened::ReadOnly(_) => None,
        }
    }
}
