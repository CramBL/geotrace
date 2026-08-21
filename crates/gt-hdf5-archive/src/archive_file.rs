//! The archive file on disk, and the right to access it.
//!
//! An archive shared between threads is reached through a lock over the
//! [`ArchiveFile`] itself, since its IO methods take `&mut self`. The file is
//! opened per operation from the path the [`ArchiveFile`] holds.

use std::fs;
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use hdf5::plist::file_create::FileSpaceStrategy;
use hdf5::{Group, LocationType};

use crate::{ArchiveError, attributes};

/// Smallest free block the file keeps track of. Every block is worth tracking:
/// a day's rows free whole pages, and the pages are what later days reuse.
const FREE_SPACE_THRESHOLD_BYTES: u64 = 1;

/// Appended to an archive's path for the file a rebuild writes.
const REBUILD_SUFFIX: &str = ".rebuilding";

/// What [`ArchiveFile::migrate_file_space_if_needed`] found the archive
/// created with, and did about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSpaceMigration {
    /// The archive already records its free space in pages.
    NotNeeded,
    /// The archive was rebuilt into a file that does.
    Rebuilt,
}

/// One archive file, named but not held open.
#[derive(Debug)]
pub struct ArchiveFile {
    path: PathBuf,
}

impl ArchiveFile {
    /// How an archive records where its free space is. libhdf5 fixes this
    /// when it creates the file, so an archive from before [`Self::create`]
    /// set it keeps what libhdf5 defaults to until
    /// [`Self::migrate_file_space_if_needed`] rebuilds it.
    pub const FILE_SPACE_STRATEGY: FileSpaceStrategy = FileSpaceStrategy::FreeSpaceManager {
        paged: true,
        persist: true,
        threshold: FREE_SPACE_THRESHOLD_BYTES,
    };

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
        Ok(OpenArchive::of(create_file(&self.path)?))
    }

    pub fn open_read_only(&mut self) -> Result<OpenArchive<'_>, ArchiveError> {
        Ok(OpenArchive::of(hdf5::File::open(&self.path)?))
    }

    pub fn open_read_write(&mut self) -> Result<OpenArchive<'_>, ArchiveError> {
        Ok(OpenArchive::of(hdf5::File::open_rw(&self.path)?))
    }

    /// Rebuild an archive whose file space strategy is not the one
    /// [`Self::create`] sets: a delete on it frees space no day stored
    /// afterwards is written into.
    ///
    /// The rebuild fills a file beside the archive and renames it over the
    /// archive, which either happens whole or not at all. An interrupted
    /// rebuild therefore leaves the archive as it was, next to a file the
    /// following call removes before rebuilding again.
    pub fn migrate_file_space_if_needed(&mut self) -> Result<FileSpaceMigration, ArchiveError> {
        let rebuilding = self.rebuilding_path();
        if rebuilding.exists() {
            fs::remove_file(&rebuilding)?;
        }
        if self.file_space_strategy()? == Self::FILE_SPACE_STRATEGY {
            return Ok(FileSpaceMigration::NotNeeded);
        }

        let bytes = fs::metadata(&self.path)?.len();
        log::info!(
            "Rebuilding the archive {:?} of {bytes} bytes: it was created without the paged free \
             space record, and a delete on it frees no space later days are written into",
            self.path
        );
        self.copy_into_new_archive(&rebuilding)?;
        fs::rename(&rebuilding, &self.path)?;
        Ok(FileSpaceMigration::Rebuilt)
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

    /// Path of the file a rebuild fills, which is left behind when the
    /// rebuild is interrupted. It sits beside the archive, where a rename
    /// onto it stays within one filesystem.
    pub fn rebuilding_path(&self) -> PathBuf {
        let mut path = self.path.clone().into_os_string();
        path.push(REBUILD_SUFFIX);
        PathBuf::from(path)
    }

    /// The strategy the file was created with, which is
    /// [`Self::FILE_SPACE_STRATEGY`] once it has been migrated.
    pub fn file_space_strategy(&mut self) -> Result<FileSpaceStrategy, ArchiveError> {
        let file = self.open_read_only()?;
        Ok(file.fcpl()?.file_space_strategy())
    }

    /// Write everything the archive holds into a new file at `target`,
    /// created the way [`Self::create`] creates one.
    fn copy_into_new_archive(&self, target: &Path) -> Result<(), ArchiveError> {
        let source = hdf5::File::open(&self.path)?;
        let rebuilt = create_file(target)?;
        for name in source.member_names()? {
            copy_object(CopiedFrom(&source), CopiedInto(&rebuilt), &name)?;
        }
        copy_i64_attributes(CopiedFrom(&source), CopiedInto(&rebuilt))?;
        Ok(rebuilt.flush()?)
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
    const fn of(file: hdf5::File) -> Self {
        Self {
            file,
            access: PhantomData,
        }
    }
}

impl Deref for OpenArchive<'_> {
    type Target = hdf5::File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

fn create_file(path: &Path) -> Result<hdf5::File, ArchiveError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut builder = hdf5::FileBuilder::new();
    builder.with_fcpl(|fcpl| {
        fcpl.file_space_strategy(ArchiveFile::FILE_SPACE_STRATEGY);
        fcpl
    });
    Ok(builder.create(path)?)
}

/// The archive a rebuild reads.
#[derive(Clone, Copy)]
struct CopiedFrom<'a>(&'a Group);

/// The archive a rebuild fills.
#[derive(Clone, Copy)]
struct CopiedInto<'a>(&'a Group);

/// Copy one member of an archive into another under the same name, with
/// everything below it: its rows, chunking, filters and attributes.
fn copy_object(
    CopiedFrom(source): CopiedFrom<'_>,
    CopiedInto(target): CopiedInto<'_>,
    name: &str,
) -> Result<(), ArchiveError> {
    match source.loc_type_by_name(name)? {
        LocationType::Group => source.group(name)?.copy_to(target, name)?,
        LocationType::Dataset => source.dataset(name)?.copy_to(target, name)?,
        other => {
            return Err(ArchiveError::Corrupt(format!(
                "{name} is a {other:?}, which an archive does not hold"
            )));
        }
    }
    Ok(())
}

/// Copy the attributes an archive holds itself, which the copy of its members
/// does not reach. An archive writes whole numbers there, and an attribute of
/// any other type is refused.
fn copy_i64_attributes(
    CopiedFrom(source): CopiedFrom<'_>,
    CopiedInto(target): CopiedInto<'_>,
) -> Result<(), ArchiveError> {
    for name in source.attr_names()? {
        let value = attributes::read_i64(source, &name).ok_or_else(|| {
            ArchiveError::Corrupt(format!(
                "attribute {name} holds a type an archive cannot store"
            ))
        })?;
        attributes::write_i64(target, &name, value)?;
    }
    Ok(())
}
