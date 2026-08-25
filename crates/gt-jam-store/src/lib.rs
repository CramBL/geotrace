//! The interference archive: published days, accumulated on disk.
//!
//! One HDF5 file holding every day ever fetched, appended to as days arrive
//! and queried per day. See [`schema`] for the layout.
//!
//! Days are immutable once stored: the host does not republish a settled day.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use gt_hdf5_archive::day_index::{DayIndex, RowPlacement};
use gt_hdf5_archive::prune::{
    ArchiveLayout, DeclinedRecovery, InterruptedDelete, PruneProgressSink, RowLevel,
};
use gt_hdf5_archive::{
    ArchiveError, ArchiveFile, ArchiveFileBeingOpened, Column, OpenArchive, ReadOnlyDayArchive,
    WritableDayArchive, attributes,
};
use gt_jam::dataset::JamDataset;
use gt_jam::wire::HexObservation;
use h3o::CellIndex;
use parking_lot::Mutex;

pub mod schema;

/// Name of the archive file, joined to the data directory by the caller.
pub const FILE_NAME: &str = "jamming.h5";

/// The archive's name in messages about its columns.
const ARCHIVE_NAME: &str = "interference archive";

#[derive(Debug, thiserror::Error)]
pub enum JamStoreError {
    #[error("archive error: {0}")]
    Backend(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("archive schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },

    #[error("{day} is already stored")]
    DayAlreadyStored { day: NaiveDate },

    #[error("archive is inconsistent: {0}")]
    Corrupt(String),

    #[error("another process has the archive open")]
    HeldByAnotherProcess,

    #[error(transparent)]
    DeclinedRecovery(#[from] DeclinedRecovery),
}

impl From<ArchiveError> for JamStoreError {
    fn from(err: ArchiveError) -> Self {
        match err {
            ArchiveError::Backend(message) => Self::Backend(message),
            ArchiveError::Io(err) => Self::Io(err),
            ArchiveError::SchemaTooNew { found, supported } => {
                Self::SchemaTooNew { found, supported }
            }
            ArchiveError::Corrupt(message) => Self::Corrupt(message),
            ArchiveError::HeldByAnotherProcess => Self::HeldByAnotherProcess,
        }
    }
}

impl From<hdf5::Error> for JamStoreError {
    fn from(err: hdf5::Error) -> Self {
        ArchiveError::from(err).into()
    }
}

/// One day's entry in the archive index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDay {
    pub day: NaiveDate,
    /// Cells the day published.
    pub cells: u32,
    pub fetched_at: DateTime<Utc>,
    /// Host that served it.
    pub host: String,
}

/// The interference archive with no method that writes to the file, as
/// [`Self::open_existing_read_only`] opens it.
#[derive(Debug)]
pub struct ReadOnlyJamStore {
    /// Every operation holds the lock for its whole sequence:
    /// [`JamStore::insert_day`] reads a column length, resizes, appends, and
    /// writes the day index last, and a caller reading between those steps
    /// sees rows that no index entry names.
    archive: Mutex<ArchiveFile>,
    /// Held beside the lock: a caller reading the archive's path never waits
    /// for a delete rewriting it.
    path: PathBuf,
}

impl ReadOnlyDayArchive for ReadOnlyJamStore {
    type Error = JamStoreError;

    const SCHEMA_VERSION_ATTR: &'static str = schema::SCHEMA_VERSION_ATTR;
    const CURRENT_SCHEMA_VERSION: i64 = schema::CURRENT_SCHEMA_VERSION;

    fn from_archive_file(archive: ArchiveFileBeingOpened) -> Self {
        let archive = archive.into_archive_file();
        Self {
            path: archive.path().to_owned(),
            archive: Mutex::new(archive),
        }
    }

    fn interrupted_delete_in(
        archive: &mut ArchiveFile,
    ) -> Result<Option<InterruptedDelete>, JamStoreError> {
        let file = archive.open_read_only()?;
        with_layout(&file, |layout| layout.interrupted_delete())
    }
}

impl ReadOnlyJamStore {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every stored day, oldest first.
    pub fn days(&self) -> Result<Vec<StoredDay>, JamStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        Ok(DayIndex::new(&days)
            .entries()?
            .into_iter()
            .map(|entry| StoredDay {
                day: entry.day,
                cells: entry.rows,
                fetched_at: entry.fetched_at,
                host: entry.host,
            })
            .collect())
    }

    /// Whether `day` has already been stored.
    pub fn contains(&self, day: NaiveDate) -> Result<bool, JamStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        Ok(DayIndex::new(&days).row_of(day)?.is_some())
    }

    /// The observations stored for `day`, or [`None`] if it is not stored.
    pub fn observations(
        &self,
        day: NaiveDate,
    ) -> Result<Option<Vec<HexObservation>>, JamStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let Some(rows) = DayIndex::new(&days).extent_of(day)? else {
            return Ok(None);
        };
        let group = file.group(schema::OBSERVATIONS_GROUP)?;

        let cells: Vec<u64> = Column::new(&group, schema::OBS_CELL).read_slice(rows.clone())?;
        let good: Vec<u32> = Column::new(&group, schema::OBS_GOOD).read_slice(rows.clone())?;
        let bad: Vec<u32> = Column::new(&group, schema::OBS_BAD).read_slice(rows)?;

        let mut observations = Vec::with_capacity(cells.len());
        for (index, &raw) in cells.iter().enumerate() {
            let (Some(&good), Some(&bad)) = (good.get(index), bad.get(index)) else {
                return Err(JamStoreError::Corrupt(format!(
                    "{day} row {index} has no counts"
                )));
            };
            let cell = CellIndex::try_from(raw)
                .map_err(|err| JamStoreError::Corrupt(format!("{day} row {index}: {err}")))?;
            observations.push(HexObservation { cell, good, bad });
        }
        Ok(Some(observations))
    }

    /// The stored day, indexed for lookup and drawing.
    pub fn dataset(&self, day: NaiveDate) -> Result<Option<JamDataset>, JamStoreError> {
        Ok(self
            .observations(day)?
            .map(|observations| JamDataset::new(day, observations)))
    }
}

/// The interference archive, which reads through [`ReadOnlyJamStore`] and
/// adds [`Self::insert_day`] beside the deletes of [`WritableDayArchive`].
#[derive(Debug)]
pub struct JamStore {
    inner: ReadOnlyJamStore,
}

impl Deref for JamStore {
    type Target = ReadOnlyJamStore;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl WritableDayArchive for JamStore {
    type Error = JamStoreError;

    type ReadOnly = ReadOnlyJamStore;

    fn from_archive_file(archive: ArchiveFileBeingOpened) -> Self {
        Self {
            inner: ReadOnlyJamStore::from_archive_file(archive),
        }
    }

    fn create_with_empty_columns(
        archive: &mut ArchiveFileBeingOpened,
    ) -> Result<(), JamStoreError> {
        let file = archive.archive_file_mut().create()?;
        attributes::write_i64(
            &file,
            schema::SCHEMA_VERSION_ATTR,
            schema::CURRENT_SCHEMA_VERSION,
        )?;
        attributes::write_i64(
            &file,
            schema::H3_RESOLUTION_ATTR,
            i64::from(u8::from(gt_jam::H3_RESOLUTION)),
        )?;

        let observations = file.create_group(schema::OBSERVATIONS_GROUP)?;
        Column::create::<i32>(&observations, schema::OBS_DAY, schema::OBSERVATION_FORMAT)?;
        Column::create::<u64>(&observations, schema::OBS_CELL, schema::OBSERVATION_FORMAT)?;
        Column::create::<u32>(&observations, schema::OBS_GOOD, schema::OBSERVATION_FORMAT)?;
        Column::create::<u32>(&observations, schema::OBS_BAD, schema::OBSERVATION_FORMAT)?;

        let days = file.create_group(schema::DAYS_GROUP)?;
        DayIndex::create_columns(&days, schema::DAY_FORMAT)?;
        Ok(())
    }

    fn recover_interrupted_delete(
        archive: &mut ArchiveFileBeingOpened,
    ) -> Result<(), JamStoreError> {
        let file = archive.archive_file_mut().open_read_write()?;
        with_layout(&file, |layout| {
            layout.recover_interrupted_delete(ARCHIVE_NAME)
        })
    }

    fn drop_unindexed_rows(archive: &mut ArchiveFileBeingOpened) -> Result<(), JamStoreError> {
        let file = archive.archive_file_mut().open_read_write()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let observations = file.group(schema::OBSERVATIONS_GROUP)?;
        DayIndex::new(&days).drop_unindexed_rows(
            &observations,
            &schema::OBSERVATION_COLUMNS,
            ARCHIVE_NAME,
        )?;
        Ok(())
    }

    fn delete_days_before(
        &self,
        cutoff: NaiveDate,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, JamStoreError> {
        let mut archive = self.inner.archive.lock();
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| layout.delete_days_before(cutoff, report))
    }

    fn delete_all_days(&self, report: PruneProgressSink<'_>) -> Result<usize, JamStoreError> {
        let mut archive = self.inner.archive.lock();
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| layout.delete_all_days(report))
    }
}

impl JamStore {
    /// Append `observations` as `day`, served by `host`.
    ///
    /// Fails with [`JamStoreError::DayAlreadyStored`] if the day is present.
    pub fn insert_day(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        observations: &[HexObservation],
    ) -> Result<(), JamStoreError> {
        let mut archive = self.inner.archive.lock();
        let file = archive.open_read_write()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let index = DayIndex::new(&days);
        if index.row_of(day)?.is_some() {
            return Err(JamStoreError::DayAlreadyStored { day });
        }

        let group = file.group(schema::OBSERVATIONS_GROUP)?;
        let offset = Column::new(&group, schema::OBS_CELL).rows()?;

        Column::new(&group, schema::OBS_DAY)
            .append(&vec![day.to_epoch_days(); observations.len()])?;
        Column::new(&group, schema::OBS_CELL).append(
            &observations
                .iter()
                .map(|observation| u64::from(observation.cell))
                .collect::<Vec<u64>>(),
        )?;
        Column::new(&group, schema::OBS_GOOD).append(
            &observations
                .iter()
                .map(|observation| observation.good)
                .collect::<Vec<u32>>(),
        )?;
        Column::new(&group, schema::OBS_BAD).append(
            &observations
                .iter()
                .map(|observation| observation.bad)
                .collect::<Vec<u32>>(),
        )?;

        let placement = RowPlacement {
            offset: u64::try_from(offset)
                .map_err(|err| JamStoreError::Corrupt(format!("row offset {offset}: {err}")))?,
            rows: u32::try_from(observations.len())
                .map_err(|err| JamStoreError::Corrupt(format!("{day} has too many rows: {err}")))?,
        };

        // The day index goes last: rows an interrupted insert leaves behind
        // stay unindexed, and the next open cuts them.
        index.insert_or_replace(day, placement, fetched_at, host)?;
        Ok(())
    }
}

/// Run `act` against the archive's layout: one day index over one group of
/// observation columns.
fn with_layout<T>(
    file: &OpenArchive<'_>,
    act: impl FnOnce(&ArchiveLayout<'_>) -> Result<T, ArchiveError>,
) -> Result<T, JamStoreError> {
    let observations = file.group(schema::OBSERVATIONS_GROUP)?;
    let levels = [RowLevel {
        group: &observations,
        columns: &schema::OBSERVATION_COLUMNS,
        extent: None,
    }];
    Ok(act(&ArchiveLayout {
        parent: file,
        index_name: schema::DAYS_GROUP,
        day_columns: &[],
        levels: &levels,
    })?)
}
