//! The interference archive: published days, accumulated on disk.
//!
//! One HDF5 file holding every day ever fetched, appended to as days arrive
//! and queried per day. See [`schema`] for the layout.
//!
//! Days are immutable once stored: the host does not republish a settled day.

use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use gt_jam::dataset::JamDataset;
use gt_jam::wire::HexObservation;
use h3o::CellIndex;
use hdf5::filters::Filter;
use hdf5::types::VarLenUnicode;
use hdf5::{Dataset, Extents, Group, SimpleExtents};
use parking_lot::Mutex;

pub mod schema;

/// Name of the archive file, joined to the data directory by the caller.
pub const FILE_NAME: &str = "jamming.h5";

/// Serializes archive access from this process.
///
/// [`JamStore::insert_day`] reads a column length, resizes, then writes, and
/// a reader between those steps would see a day's rows without its index
/// entry. `gt-history` guards its own HDF5 file the same way.
static ARCHIVE_LOCK: Mutex<()> = Mutex::new(());

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
}

/// Map an HDF5 error, which carries no structure worth preserving.
fn backend(err: &hdf5::Error) -> JamStoreError {
    JamStoreError::Backend(err.to_string())
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

/// The interference archive.
///
/// Holds a path, not an open handle: each call opens the file for the access
/// it needs.
#[derive(Debug, Clone)]
pub struct JamStore {
    path: PathBuf,
}

impl JamStore {
    /// Open the archive at `path`, creating it if it does not exist.
    ///
    /// Rows left behind by an interrupted [`Self::insert_day`] are dropped
    /// here rather than leaking for the life of the file.
    pub fn open_or_create(path: &Path) -> Result<Self, JamStoreError> {
        let _guard = ARCHIVE_LOCK.lock();
        if path.exists() {
            Self::validate(path)?;
            Self::drop_unindexed_rows(path)?;
        } else {
            Self::create(path)?;
        }
        Ok(Self {
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn create(path: &Path) -> Result<(), JamStoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = hdf5::File::create(path).map_err(|err| backend(&err))?;
        write_attr(
            &file,
            schema::SCHEMA_VERSION_ATTR,
            schema::CURRENT_SCHEMA_VERSION,
        )?;
        write_attr(
            &file,
            schema::H3_RESOLUTION_ATTR,
            i64::from(u8::from(gt_jam::H3_RESOLUTION)),
        )?;

        let chunk = schema::OBSERVATION_CHUNK_ROWS;
        let observations = file
            .create_group(schema::OBSERVATIONS_GROUP)
            .map_err(|err| backend(&err))?;
        Column::create::<i32>(&observations, schema::OBS_DAY, chunk)?;
        Column::create::<u64>(&observations, schema::OBS_CELL, chunk)?;
        Column::create::<u32>(&observations, schema::OBS_GOOD, chunk)?;
        Column::create::<u32>(&observations, schema::OBS_BAD, chunk)?;

        let chunk = schema::DAY_CHUNK_ROWS;
        let days = file
            .create_group(schema::DAYS_GROUP)
            .map_err(|err| backend(&err))?;
        Column::create::<i32>(&days, schema::DAY_DAY, chunk)?;
        Column::create::<u64>(&days, schema::DAY_OFFSET, chunk)?;
        Column::create::<u32>(&days, schema::DAY_COUNT, chunk)?;
        Column::create::<i64>(&days, schema::DAY_FETCHED_AT, chunk)?;
        Column::create_strings(&days, schema::DAY_HOST, chunk)?;
        Ok(())
    }

    fn validate(path: &Path) -> Result<(), JamStoreError> {
        let file = hdf5::File::open(path).map_err(|err| backend(&err))?;
        let found = file
            .attr(schema::SCHEMA_VERSION_ATTR)
            .and_then(|attr| attr.read_scalar::<i64>())
            .unwrap_or(0);
        if found > schema::CURRENT_SCHEMA_VERSION {
            return Err(JamStoreError::SchemaTooNew {
                found,
                supported: schema::CURRENT_SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    /// Cut observation rows past the end of the day index.
    ///
    /// [`Self::insert_day`] appends observations before indexing them, so an
    /// interrupted insert leaves rows no day refers to. Columns *shorter*
    /// than the index means indexed rows are missing, which is not
    /// recoverable and is reported instead.
    fn drop_unindexed_rows(path: &Path) -> Result<(), JamStoreError> {
        let file = hdf5::File::open_rw(path).map_err(|err| backend(&err))?;
        let days = file
            .group(schema::DAYS_GROUP)
            .map_err(|err| backend(&err))?;
        let offsets: Vec<u64> = Column::new(&days, schema::DAY_OFFSET).read()?;
        let counts: Vec<u32> = Column::new(&days, schema::DAY_COUNT).read()?;

        let mut indexed: usize = 0;
        for (&offset, &count) in offsets.iter().zip(&counts) {
            let end = usize::try_from(offset + u64::from(count))
                .map_err(|err| JamStoreError::Corrupt(format!("day extent {offset}: {err}")))?;
            indexed = indexed.max(end);
        }

        let observations = file
            .group(schema::OBSERVATIONS_GROUP)
            .map_err(|err| backend(&err))?;
        for name in schema::OBSERVATION_COLUMNS {
            let column = Column::new(&observations, name);
            let rows = column.rows()?;
            if rows < indexed {
                return Err(JamStoreError::Corrupt(format!(
                    "{name} holds {rows} rows but the day index reaches {indexed}"
                )));
            }
            if rows > indexed {
                log::warn!(
                    "Dropping {} unindexed rows from interference archive column {name:?}",
                    rows - indexed
                );
                column.truncate(indexed)?;
            }
        }
        Ok(())
    }

    /// Every stored day, oldest first.
    pub fn days(&self) -> Result<Vec<StoredDay>, JamStoreError> {
        let _guard = ARCHIVE_LOCK.lock();
        let file = hdf5::File::open(&self.path).map_err(|err| backend(&err))?;
        let group = file
            .group(schema::DAYS_GROUP)
            .map_err(|err| backend(&err))?;

        let days: Vec<i32> = Column::new(&group, schema::DAY_DAY).read()?;
        let counts: Vec<u32> = Column::new(&group, schema::DAY_COUNT).read()?;
        let fetched: Vec<i64> = Column::new(&group, schema::DAY_FETCHED_AT).read()?;
        let hosts: Vec<VarLenUnicode> = Column::new(&group, schema::DAY_HOST).read()?;
        if counts.len() != days.len() || fetched.len() != days.len() || hosts.len() != days.len() {
            return Err(JamStoreError::Corrupt(format!(
                "day index columns disagree: {} days, {} counts, {} timestamps, {} hosts",
                days.len(),
                counts.len(),
                fetched.len(),
                hosts.len()
            )));
        }

        let mut stored: Vec<StoredDay> = Vec::with_capacity(days.len());
        for (index, &day) in days.iter().enumerate() {
            let (Some(&cells), Some(&fetched_at), Some(host)) =
                (counts.get(index), fetched.get(index), hosts.get(index))
            else {
                return Err(JamStoreError::Corrupt(format!("day row {index} is short")));
            };
            stored.push(StoredDay {
                day: date_from_epoch_days(day)?,
                cells,
                fetched_at: DateTime::from_timestamp(fetched_at, 0).unwrap_or_default(),
                host: host.as_str().to_owned(),
            });
        }
        stored.sort_by_key(|entry| entry.day);
        Ok(stored)
    }

    /// Whether `day` has already been stored.
    pub fn contains(&self, day: NaiveDate) -> Result<bool, JamStoreError> {
        let _guard = ARCHIVE_LOCK.lock();
        let file = hdf5::File::open(&self.path).map_err(|err| backend(&err))?;
        Ok(locate(&file, day)?.is_some())
    }

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
        let _guard = ARCHIVE_LOCK.lock();
        let file = hdf5::File::open_rw(&self.path).map_err(|err| backend(&err))?;
        if locate(&file, day)?.is_some() {
            return Err(JamStoreError::DayAlreadyStored { day });
        }
        let epoch_day = day.to_epoch_days();
        let count = u32::try_from(observations.len())
            .map_err(|err| JamStoreError::Corrupt(format!("{day} has too many rows: {err}")))?;

        let group = file
            .group(schema::OBSERVATIONS_GROUP)
            .map_err(|err| backend(&err))?;
        let offset = Column::new(&group, schema::OBS_CELL).rows()?;

        Column::new(&group, schema::OBS_DAY).append(&vec![epoch_day; observations.len()])?;
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

        // The index goes last, so an interrupted insert leaves rows no day
        // points at - which `drop_unindexed_rows` cuts on the next open.
        let offset = u64::try_from(offset)
            .map_err(|err| JamStoreError::Corrupt(format!("row offset {offset}: {err}")))?;
        let days = file
            .group(schema::DAYS_GROUP)
            .map_err(|err| backend(&err))?;
        Column::new(&days, schema::DAY_DAY).append(&[epoch_day])?;
        Column::new(&days, schema::DAY_OFFSET).append(&[offset])?;
        Column::new(&days, schema::DAY_COUNT).append(&[count])?;
        Column::new(&days, schema::DAY_FETCHED_AT).append(&[fetched_at.timestamp()])?;
        Column::new(&days, schema::DAY_HOST).append(&[host
            .parse::<VarLenUnicode>()
            .map_err(|err| JamStoreError::Corrupt(format!("host {host:?}: {err}")))?])?;
        Ok(())
    }

    /// The observations stored for `day`, or [`None`] if it is not stored.
    pub fn observations(
        &self,
        day: NaiveDate,
    ) -> Result<Option<Vec<HexObservation>>, JamStoreError> {
        let _guard = ARCHIVE_LOCK.lock();
        let file = hdf5::File::open(&self.path).map_err(|err| backend(&err))?;
        let Some(rows) = locate(&file, day)? else {
            return Ok(None);
        };
        let group = file
            .group(schema::OBSERVATIONS_GROUP)
            .map_err(|err| backend(&err))?;

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

/// Which observation rows belong to `day`.
fn locate(file: &hdf5::File, day: NaiveDate) -> Result<Option<Range<usize>>, JamStoreError> {
    let group = file
        .group(schema::DAYS_GROUP)
        .map_err(|err| backend(&err))?;
    let days: Vec<i32> = Column::new(&group, schema::DAY_DAY).read()?;
    let Some(index) = days
        .iter()
        .position(|&stored| stored == day.to_epoch_days())
    else {
        return Ok(None);
    };
    let offsets: Vec<u64> = Column::new(&group, schema::DAY_OFFSET).read()?;
    let counts: Vec<u32> = Column::new(&group, schema::DAY_COUNT).read()?;
    let (Some(&offset), Some(&count)) = (offsets.get(index), counts.get(index)) else {
        return Err(JamStoreError::Corrupt(format!(
            "{day} is indexed at row {index} but has no extent"
        )));
    };
    let start = usize::try_from(offset)
        .map_err(|err| JamStoreError::Corrupt(format!("{day} offset {offset}: {err}")))?;
    let count = usize::try_from(count)
        .map_err(|err| JamStoreError::Corrupt(format!("{day} count {count}: {err}")))?;
    Ok(Some(start..start + count))
}

fn date_from_epoch_days(days: i32) -> Result<NaiveDate, JamStoreError> {
    NaiveDate::from_epoch_days(days)
        .ok_or_else(|| JamStoreError::Corrupt(format!("day {days} is not a date")))
}

fn write_attr(file: &hdf5::File, name: &str, value: i64) -> Result<(), JamStoreError> {
    file.new_attr::<i64>()
        .create(name)
        .map_err(|err| backend(&err))?
        .write_scalar(&value)
        .map_err(|err| backend(&err))
}

/// One extensible column of the archive.
struct Column<'a> {
    group: &'a Group,
    name: &'a str,
}

impl<'a> Column<'a> {
    const fn new(group: &'a Group, name: &'a str) -> Self {
        Self { group, name }
    }

    /// An empty, extensible, shuffled and deflated column.
    fn create<T: hdf5::H5Type>(
        group: &Group,
        name: &str,
        chunk_rows: usize,
    ) -> Result<(), JamStoreError> {
        group
            .new_dataset::<T>()
            .shape(Extents::Simple(SimpleExtents::resizable([0])))
            .chunk([chunk_rows])
            .set_filters(&[Filter::Shuffle, Filter::Deflate(schema::DEFLATE_LEVEL)])
            .create(name)
            .map(|_| ())
            .map_err(|err| backend(&err))
    }

    /// Shuffle transposes fixed-width elements, which a variable-length
    /// string is not, so a string column is deflated only.
    fn create_strings(group: &Group, name: &str, chunk_rows: usize) -> Result<(), JamStoreError> {
        group
            .new_dataset::<VarLenUnicode>()
            .shape(Extents::Simple(SimpleExtents::resizable([0])))
            .chunk([chunk_rows])
            .set_filters(&[Filter::Deflate(schema::DEFLATE_LEVEL)])
            .create(name)
            .map(|_| ())
            .map_err(|err| backend(&err))
    }

    fn dataset(&self) -> Result<Dataset, JamStoreError> {
        self.group.dataset(self.name).map_err(|err| backend(&err))
    }

    fn rows(&self) -> Result<usize, JamStoreError> {
        self.dataset()?
            .shape()
            .first()
            .copied()
            .ok_or_else(|| JamStoreError::Corrupt(format!("{} has no dimensions", self.name)))
    }

    fn read<T: hdf5::H5Type + Clone>(&self) -> Result<Vec<T>, JamStoreError> {
        self.dataset()?
            .read_1d::<T>()
            .map(|array| array.to_vec())
            .map_err(|err| backend(&err))
    }

    /// Reads `rows`, refusing a range the column does not hold rather than
    /// letting HDF5 answer with fewer values than asked for.
    fn read_slice<T: hdf5::H5Type + Clone>(
        &self,
        rows: Range<usize>,
    ) -> Result<Vec<T>, JamStoreError> {
        let available = self.rows()?;
        if rows.end > available {
            return Err(JamStoreError::Corrupt(format!(
                "{} holds {available} rows, asked for {}..{}",
                self.name, rows.start, rows.end
            )));
        }
        self.dataset()?
            .read_slice_1d::<T, _>(rows)
            .map(|array| array.to_vec())
            .map_err(|err| backend(&err))
    }

    fn append(&self, values: &[impl hdf5::H5Type]) -> Result<(), JamStoreError> {
        let dataset = self.dataset()?;
        let start = self.rows()?;
        dataset
            .resize([start + values.len()])
            .map_err(|err| backend(&err))?;
        dataset
            .write_slice(values, start..start + values.len())
            .map_err(|err| backend(&err))
    }

    fn truncate(&self, rows: usize) -> Result<(), JamStoreError> {
        self.dataset()?.resize([rows]).map_err(|err| backend(&err))
    }
}
