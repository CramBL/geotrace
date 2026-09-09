//! Opening a day-keyed archive, and inspecting one without writing to it.
//!
//! Every archive crate has two types: one with the read methods, which
//! implements [`ReadOnlyDayArchive`], and one that adds the mutators, which
//! implements [`WritableDayArchive`]. Both opens are provided here.

use std::path::Path;

use chrono::NaiveDate;

use crate::prune::{
    DeclinedRecovery, InterruptedDelete, InterruptedDeleteRecovery, PruneProgressSink,
};
use crate::{ArchiveError, ArchiveFile};

/// An [`ArchiveFile`] part-way through one of the opens in this module.
/// Nothing outside constructs one, so the methods taking it cannot run on an
/// archive whose schema version and interrupted delete were never checked.
pub struct ArchiveFileBeingOpened(ArchiveFile);

impl ArchiveFileBeingOpened {
    pub fn archive_file_mut(&mut self) -> &mut ArchiveFile {
        &mut self.0
    }

    pub fn into_archive_file(self) -> ArchiveFile {
        self.0
    }
}

/// A day archive with no method that writes to the file.
pub trait ReadOnlyDayArchive: Sized {
    type Error: From<ArchiveError> + From<DeclinedRecovery>;

    const SCHEMA_VERSION_ATTR: &'static str;
    const CURRENT_SCHEMA_VERSION: i64;

    /// The name of this archive's file inside the directory that the caller
    /// keeps its archives in.
    const FILE_NAME: &'static str;

    fn from_archive_file(archive: ArchiveFileBeingOpened) -> Self;

    /// What an interrupted delete left in `archive`, or [`None`] when there is
    /// nothing to recover. The file is opened read-only.
    fn interrupted_delete_in(
        archive: &mut ArchiveFile,
    ) -> Result<Option<InterruptedDelete>, Self::Error>;

    /// Open the archive at `path` without writing to it: it is not created
    /// where it is missing, not rebuilt, and neither an interrupted insert nor
    /// an interrupted delete in it is put right.
    ///
    /// An archive an interrupted delete left part-way through fails with
    /// [`DeclinedRecovery`]: its day index cannot be read as it stands, and
    /// putting it right is a write.
    fn open_existing_read_only(path: &Path) -> Result<Self, Self::Error> {
        let mut archive = ArchiveFile::new(path);
        archive.check_readable_without_writing(
            Self::interrupted_delete_in,
            Self::SCHEMA_VERSION_ATTR,
            Self::CURRENT_SCHEMA_VERSION,
        )?;
        Ok(Self::from_archive_file(ArchiveFileBeingOpened(archive)))
    }

    /// What an interrupted delete left in the archive at `path`, or [`None`]
    /// when there is nothing to recover. An archive that does not exist yet
    /// has nothing to recover either.
    ///
    /// The file is opened read-only and nothing in it changes.
    fn interrupted_delete_at(path: &Path) -> Result<Option<InterruptedDelete>, Self::Error> {
        let mut archive = ArchiveFile::new(path);
        if !archive.exists() {
            return Ok(None);
        }
        Self::interrupted_delete_in(&mut archive)
    }
}

/// A day archive with the mutators [`Self::ReadOnly`] does not have.
pub trait WritableDayArchive: Sized {
    type Error: From<ArchiveError> + From<DeclinedRecovery>;

    type ReadOnly: ReadOnlyDayArchive<Error = Self::Error>;

    fn from_archive_file(archive: ArchiveFileBeingOpened) -> Self;

    fn create_with_empty_columns(archive: &mut ArchiveFileBeingOpened) -> Result<(), Self::Error>;

    /// Bring the day index and the rows back into agreement after a delete
    /// that was interrupted, discarding the days it left behind.
    fn recover_interrupted_delete(archive: &mut ArchiveFileBeingOpened) -> Result<(), Self::Error>;

    /// Cut the rows an interrupted insert left behind, which no day index
    /// entry reaches.
    fn drop_unindexed_rows(archive: &mut ArchiveFileBeingOpened) -> Result<(), Self::Error>;

    /// Remove every archived day before `cutoff`, reporting how many went.
    ///
    /// The rows the remaining days hold move down to close the gap. The file
    /// itself rarely shrinks: the freed space is where later days are
    /// written.
    fn delete_days_before(
        &self,
        cutoff: NaiveDate,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, Self::Error>;

    /// Remove every archived day, reporting how many went.
    fn delete_all_days(&self, report: PruneProgressSink<'_>) -> Result<usize, Self::Error>;

    /// Open the archive at `path`, creating it if it does not exist.
    ///
    /// An archive created before archives recorded their free space in pages
    /// is rebuilt first, see [`ArchiveFile::migrate_file_space_if_needed`].
    ///
    /// Rows an interrupted insert left behind are dropped here, and so are the
    /// days an interrupted delete left in an unknown layout.
    fn open_or_create(path: &Path) -> Result<Self, Self::Error> {
        Self::open_or_create_with_recovery_choice(path, InterruptedDeleteRecovery::Recover)
    }

    /// Open the archive at `path` as [`Self::open_or_create`] does, recovering
    /// an interrupted delete only for [`InterruptedDeleteRecovery::Recover`].
    ///
    /// A declined recovery leaves the file exactly as it was found and fails
    /// with [`DeclinedRecovery`], which is checked before anything else the
    /// open would write.
    fn open_or_create_with_recovery_choice(
        path: &Path,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<Self, Self::Error> {
        let mut archive = ArchiveFileBeingOpened(ArchiveFile::new(path));
        if archive.0.exists() {
            if recovery == InterruptedDeleteRecovery::Decline
                && let Some(interrupted) =
                    <Self::ReadOnly as ReadOnlyDayArchive>::interrupted_delete_in(&mut archive.0)?
            {
                return Err(DeclinedRecovery(interrupted).into());
            }
            archive.0.migrate_file_space_if_needed()?;
            archive.0.validate_schema_version(
                <Self::ReadOnly as ReadOnlyDayArchive>::SCHEMA_VERSION_ATTR,
                <Self::ReadOnly as ReadOnlyDayArchive>::CURRENT_SCHEMA_VERSION,
            )?;
            Self::recover_interrupted_delete(&mut archive)?;
            Self::drop_unindexed_rows(&mut archive)?;
        } else {
            Self::create_with_empty_columns(&mut archive)?;
        }
        Ok(Self::from_archive_file(archive))
    }
}

/// The failures a caller opening every day archive acts on, whichever archive
/// reported one.
///
/// Each archive crate implements this for its own error type, through
/// [`impl_day_archive_error!`](crate::impl_day_archive_error).
pub trait DayArchiveError: std::error::Error {
    /// Another process has the file open. libhdf5 takes an OS lock for the
    /// duration of an open, readers included, so nothing here can read it
    /// until that process lets go.
    fn is_held_by_another_process(&self) -> bool;

    /// The interrupted delete an open declined to recover, which left the
    /// file untouched, or [`None`] for any other failure.
    fn interrupted_delete_left_unrecovered(&self) -> Option<InterruptedDelete>;

    /// The schema versions an open read out of a file that a newer build
    /// wrote, or [`None`] for any other failure.
    fn schema_too_new(&self) -> Option<SchemaVersions>;
}

/// The schema version an archive file states, beside the newest that this
/// build reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersions {
    pub found: i64,
    pub supported: i64,
}

/// Implements [`DayArchiveError`] for an archive error with a
/// `HeldByAnotherProcess`, a `SchemaTooNew { found, supported }` and a
/// `DeclinedRecovery` variant.
///
/// The macro expands in the crate that owns the error type, which the orphan
/// rule requires.
#[macro_export]
macro_rules! impl_day_archive_error {
    ($error:ty) => {
        impl $crate::DayArchiveError for $error {
            fn is_held_by_another_process(&self) -> bool {
                matches!(self, Self::HeldByAnotherProcess)
            }

            fn interrupted_delete_left_unrecovered(
                &self,
            ) -> Option<$crate::prune::InterruptedDelete> {
                match self {
                    Self::DeclinedRecovery($crate::prune::DeclinedRecovery(interrupted)) => {
                        Some(*interrupted)
                    }
                    _ => None,
                }
            }

            fn schema_too_new(&self) -> Option<$crate::SchemaVersions> {
                match *self {
                    Self::SchemaTooNew { found, supported } => {
                        Some($crate::SchemaVersions { found, supported })
                    }
                    _ => None,
                }
            }
        }
    };
}
