//! The archive file on disk, and the right to access it.
//!
//! An archive shared between threads is reached through a lock over the
//! [`ArchiveFile`] itself, since its IO methods take `&mut self`. The file is
//! opened per operation from the path the [`ArchiveFile`] holds.

use std::marker::PhantomData;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use hdf5::plist::file_create::FileSpaceStrategy;

use crate::{ArchiveError, attributes};

/// Smallest free block the file keeps track of. Every block is worth tracking:
/// a day's rows free whole pages, and the pages are what later days reuse.
const FREE_SPACE_THRESHOLD_BYTES: u64 = 1;

/// One archive file, named but not held open.
#[derive(Debug)]
pub struct ArchiveFile {
    path: PathBuf,
}

impl ArchiveFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Create the file, and the directory it sits in.
    ///
    /// The file records where its free space is, in pages: the space a delete
    /// frees is then what the days stored after it are written into. libhdf5
    /// otherwise forgets the free space when it closes the file, and every
    /// later day extends it. Measured over ten stored days of which five were
    /// deleted and five stored again: 729 KB, the size the file had before the
    /// delete, against 908 KB without the page record. Paged allocation costs
    /// about a fifth of the file in padding.
    pub fn create(&mut self) -> Result<OpenArchive<'_>, ArchiveError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut builder = hdf5::FileBuilder::new();
        builder.with_fcpl(|fcpl| {
            fcpl.file_space_strategy(FileSpaceStrategy::FreeSpaceManager {
                paged: true,
                persist: true,
                threshold: FREE_SPACE_THRESHOLD_BYTES,
            });
            fcpl
        });
        OpenArchive::of(builder.create(&self.path))
    }

    pub fn open_read_only(&mut self) -> Result<OpenArchive<'_>, ArchiveError> {
        OpenArchive::of(hdf5::File::open(&self.path))
    }

    pub fn open_read_write(&mut self) -> Result<OpenArchive<'_>, ArchiveError> {
        OpenArchive::of(hdf5::File::open_rw(&self.path))
    }

    /// Refuse an archive whose `attribute` names a schema newer than
    /// `supported`. An archive without the attribute reads as version 0.
    pub fn validate_schema_version(
        &mut self,
        attribute: &str,
        supported: i64,
    ) -> Result<(), ArchiveError> {
        let file = self.open_read_only()?;
        let found = attributes::read_i64(&file, attribute).unwrap_or_default();
        if found > supported {
            return Err(ArchiveError::SchemaTooNew { found, supported });
        }
        Ok(())
    }
}

/// An [`ArchiveFile`] open for one operation, closed when it is dropped.
///
/// Borrows its [`ArchiveFile`] exclusively for as long as it lives: no second
/// handle to the same archive can be opened while this one is in use.
pub struct OpenArchive<'a> {
    file: hdf5::File,
    access: PhantomData<&'a mut ArchiveFile>,
}

impl OpenArchive<'_> {
    fn of(opened: Result<hdf5::File, hdf5::Error>) -> Result<Self, ArchiveError> {
        Ok(Self {
            file: opened?,
            access: PhantomData,
        })
    }
}

impl Deref for OpenArchive<'_> {
    type Target = hdf5::File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}
