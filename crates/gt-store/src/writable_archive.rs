//! The archive the instance that owns the data directory writes to.

use std::sync::Arc;

use gt_pending_writes::{PendingWriteGuard, PendingWrites, WriteRefusal, WriteRegistration};

/// An archive opened writable, together with the registry its writes are
/// registered in.
///
/// [`Self::write`] is the only way to an archive method that changes the file,
/// and it registers the write before calling one: there is no accessor for the
/// archive, so no write runs unregistered.
#[derive(Debug)]
pub struct WritableArchive<W> {
    archive: Arc<W>,
    pending_writes: PendingWrites,
}

impl<W> Clone for WritableArchive<W> {
    fn clone(&self) -> Self {
        Self {
            archive: Arc::clone(&self.archive),
            pending_writes: self.pending_writes.clone(),
        }
    }
}

impl<W> WritableArchive<W> {
    /// Wraps `archive`, registering every write to it in `pending_writes`.
    pub fn new(archive: Arc<W>, pending_writes: PendingWrites) -> Self {
        Self {
            archive,
            pending_writes,
        }
    }

    /// Register `registration` and run `write` on the archive while it stays
    /// registered, or report why [`PendingWrites`] turned the write away.
    pub fn write<T>(
        &self,
        registration: WriteRegistration,
        write: impl FnOnce(&W) -> T,
    ) -> Result<T, WriteRefusal> {
        self.write_reporting_progress(registration, |archive, _| write(archive))
    }

    /// The same as [`Self::write`], handing `write` the guard the shutdown
    /// window reads progress and stage from.
    pub fn write_reporting_progress<T>(
        &self,
        registration: WriteRegistration,
        write: impl FnOnce(&W, &PendingWriteGuard) -> T,
    ) -> Result<T, WriteRefusal> {
        let guard = self
            .pending_writes
            .try_begin(registration.label, registration.kind)?;
        Ok(write(&self.archive, &guard))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use gt_pending_writes::{WriteAccess, WriteKind};
    use rstest::rstest;

    use super::*;

    /// Stands in for one of the day archives, which the wrapper never calls
    /// into itself.
    struct TestArchive;

    fn registration() -> WriteRegistration {
        WriteRegistration {
            label: "Archiving aircraft interference for 2026-07-20".to_owned(),
            kind: WriteKind::ArchiveDayInsert {
                archive: "aircraft interference",
            },
        }
    }

    fn writable(pending_writes: &PendingWrites) -> WritableArchive<TestArchive> {
        WritableArchive::new(Arc::new(TestArchive), pending_writes.clone())
    }

    #[test]
    fn a_write_is_registered_for_as_long_as_it_runs() {
        let pending_writes = PendingWrites::default();
        let archive = writable(&pending_writes);

        let running = archive
            .write(registration(), |_archive| pending_writes.snapshot().running)
            .expect("the registry takes the write");

        assert_eq!(
            running.iter().map(|write| &write.label).collect::<Vec<_>>(),
            ["Archiving aircraft interference for 2026-07-20"]
        );
        assert!(pending_writes.is_idle());
    }

    #[test]
    fn a_write_reports_its_progress_through_the_guard_it_registered_with() {
        let pending_writes = PendingWrites::default();
        let archive = writable(&pending_writes);

        let progress = archive
            .write_reporting_progress(registration(), |_archive, write| {
                write.set_progress(0.5);
                pending_writes.snapshot().running.first()?.progress
            })
            .expect("the registry takes the write");

        assert_eq!(progress, Some(0.5));
    }

    #[rstest]
    #[case(WriteRefusal::ReadOnlySession)]
    #[case(WriteRefusal::ShuttingDown)]
    fn a_refused_write_never_reaches_the_archive(#[case] refusal: WriteRefusal) {
        let pending_writes = match refusal {
            WriteRefusal::ReadOnlySession => PendingWrites::new(WriteAccess::ReadOnly),
            WriteRefusal::ShuttingDown => {
                let owner = PendingWrites::new(WriteAccess::Owner);
                owner.begin_shutdown();
                owner
            }
        };
        let archive = writable(&pending_writes);
        let reached = AtomicBool::new(false);

        let outcome = archive.write(registration(), |_archive| {
            reached.store(true, Ordering::Relaxed);
        });

        assert_eq!(outcome, Err(refusal));
        assert!(!reached.load(Ordering::Relaxed));
    }
}
