//! Column storage for the day-keyed HDF5 archives.
//!
//! An archive holds one extensible dataset per field, and a day index whose
//! entries name a contiguous run of rows. [`ArchiveFile`] is the file itself,
//! [`Column`] the dataset access all archives share, [`day_index`] the row
//! bookkeeping. Which columns exist, and what they mean, belongs to each
//! archive.

use std::ops::Range;

use hdf5::filters::Filter;
use hdf5::types::VarLenUnicode;
use hdf5::{Dataset, Extents, Group, SimpleExtents};

mod archive_file;
pub mod attributes;
pub mod dates;
pub mod day_index;

pub use archive_file::{ArchiveFile, OpenArchive};

/// Why an archive access failed. Each archive converts this into its own
/// error type.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive error: {0}")]
    Backend(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("archive schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },

    #[error("archive is inconsistent: {0}")]
    Corrupt(String),
}

/// An HDF5 error arrives as its message: the hdf5 crate reports no structured
/// detail.
impl From<hdf5::Error> for ArchiveError {
    fn from(err: hdf5::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

/// How a column is chunked and compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnFormat {
    pub chunk_rows: usize,
    pub deflate_level: u8,
}

/// One extensible column of an archive.
pub struct Column<'a> {
    group: &'a Group,
    name: &'a str,
}

impl<'a> Column<'a> {
    pub const fn new(group: &'a Group, name: &'a str) -> Self {
        Self { group, name }
    }

    /// An empty, extensible, shuffled and deflated column.
    pub fn create<T: hdf5::H5Type>(
        group: &Group,
        name: &str,
        format: ColumnFormat,
    ) -> Result<(), ArchiveError> {
        group
            .new_dataset::<T>()
            .shape(Extents::Simple(SimpleExtents::resizable([0])))
            .chunk([format.chunk_rows])
            .set_filters(&[Filter::Shuffle, Filter::Deflate(format.deflate_level)])
            .create(name)
            .map(|_| ())
            .map_err(ArchiveError::from)
    }

    /// String columns are deflate-only: shuffle transposes fixed-width
    /// elements, which a variable-length string is not.
    pub fn create_strings(
        group: &Group,
        name: &str,
        format: ColumnFormat,
    ) -> Result<(), ArchiveError> {
        group
            .new_dataset::<VarLenUnicode>()
            .shape(Extents::Simple(SimpleExtents::resizable([0])))
            .chunk([format.chunk_rows])
            .set_filters(&[Filter::Deflate(format.deflate_level)])
            .create(name)
            .map(|_| ())
            .map_err(ArchiveError::from)
    }

    fn dataset(&self) -> Result<Dataset, ArchiveError> {
        Ok(self.group.dataset(self.name)?)
    }

    pub fn rows(&self) -> Result<usize, ArchiveError> {
        self.dataset()?
            .shape()
            .first()
            .copied()
            .ok_or_else(|| ArchiveError::Corrupt(format!("{} has no dimensions", self.name)))
    }

    pub fn read<T: hdf5::H5Type + Clone>(&self) -> Result<Vec<T>, ArchiveError> {
        Ok(self.dataset()?.read_1d::<T>().map(|array| array.to_vec())?)
    }

    /// Reads `rows`, refusing a range the column does not hold. HDF5 returns
    /// fewer values than requested for an out-of-range slice.
    pub fn read_slice<T: hdf5::H5Type + Clone>(
        &self,
        rows: Range<usize>,
    ) -> Result<Vec<T>, ArchiveError> {
        let available = self.rows()?;
        if rows.end > available {
            return Err(ArchiveError::Corrupt(format!(
                "{} holds {available} rows, requested {}..{}",
                self.name, rows.start, rows.end
            )));
        }
        Ok(self
            .dataset()?
            .read_slice_1d::<T, _>(rows)
            .map(|array| array.to_vec())?)
    }

    pub fn append(&self, values: &[impl hdf5::H5Type]) -> Result<(), ArchiveError> {
        let dataset = self.dataset()?;
        let start = self.rows()?;
        dataset.resize([start + values.len()])?;
        Ok(dataset.write_slice(values, start..start + values.len())?)
    }

    /// Overwrites one row, for an entry the archive stores again.
    pub fn write_row(&self, row: usize, value: impl hdf5::H5Type) -> Result<(), ArchiveError> {
        Ok(self.dataset()?.write_slice(&[value], row..row + 1)?)
    }

    pub fn truncate(&self, rows: usize) -> Result<(), ArchiveError> {
        Ok(self.dataset()?.resize([rows])?)
    }
}
