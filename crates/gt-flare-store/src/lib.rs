//! The solar flare archive: fetched catalog days, accumulated on disk.
//!
//! A day already archived costs no request: one HDF5 file holds every day
//! ever fetched, queried per day. See [`schema`] for the layout.
//!
//! A day the catalog lists no flare for is archived with no events, which is
//! what keeps it from being requested again.

use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use gt_flare::SolarFlare;
use gt_flare::class::{FlareClass, FlareClassification};
use gt_hdf5_archive::day_index::{DayIndex, RowPlacement};
use gt_hdf5_archive::prune::{
    ArchiveLayout, DeclinedRecovery, InterruptedDelete, InterruptedDeleteRecovery,
    PruneProgressSink, RowLevel,
};
use gt_hdf5_archive::{
    ArchiveError, ArchiveFile, Column, OpenArchive, StoredPresence, attributes, dates,
};
use hdf5::Group;
use hdf5::types::VarLenUnicode;
use parking_lot::Mutex;

use crate::schema::StoredFlareClass;

pub mod schema;

/// The archive's name in messages about its columns.
const ARCHIVE_NAME: &str = "solar flare archive";

/// Name of the archive file, joined to the data directory by the caller.
pub const FILE_NAME: &str = "solar_flares.h5";

#[derive(Debug, thiserror::Error)]
pub enum FlareStoreError {
    #[error("archive error: {0}")]
    Backend(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("archive schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },

    #[error("archive is inconsistent: {0}")]
    Corrupt(String),

    #[error("another process has the archive open")]
    HeldByAnotherProcess,

    #[error(transparent)]
    DeclinedRecovery(#[from] DeclinedRecovery),
}

impl From<ArchiveError> for FlareStoreError {
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

impl From<hdf5::Error> for FlareStoreError {
    fn from(err: hdf5::Error) -> Self {
        ArchiveError::from(err).into()
    }
}

/// One day's entry in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedFlareDay {
    pub day: NaiveDate,
    /// Flares stored for the day, zero for a day the catalog lists none for.
    pub flares: u32,
    pub fetched_at: DateTime<Utc>,
    /// Host that served it. The API key is never part of it.
    pub host: String,
}

/// The solar flare archive.
#[derive(Debug)]
pub struct FlareStore {
    /// Every operation holds the lock for its whole sequence:
    /// [`Self::insert_or_replace_day`] reads a column length, resizes, appends
    /// and writes the day index last, and a caller reading between those steps
    /// sees events that no index entry names.
    archive: Mutex<ArchiveFile>,
    /// Held beside the lock: a caller reading the archive's path never waits
    /// for a delete rewriting it.
    path: PathBuf,
}

impl FlareStore {
    /// Open the archive at `path`, creating it if it does not exist.
    ///
    /// An archive created before archives recorded their free space in pages
    /// is rebuilt first, see [`ArchiveFile::migrate_file_space_if_needed`].
    ///
    /// Events left behind by an interrupted store are dropped here, and so
    /// are the days an interrupted [`Self::delete_days_before`] left in an
    /// unknown layout.
    pub fn open_or_create(path: &Path) -> Result<Self, FlareStoreError> {
        Self::open_or_create_with_recovery_choice(path, InterruptedDeleteRecovery::Recover)
    }

    /// Open the archive at `path` as [`Self::open_or_create`] does, recovering
    /// an interrupted delete only when `recovery` asks for it.
    ///
    /// A declined recovery leaves the file exactly as it was found and fails
    /// with [`FlareStoreError::DeclinedRecovery`], which is checked before anything
    /// else the open would write.
    pub fn open_or_create_with_recovery_choice(
        path: &Path,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<Self, FlareStoreError> {
        let mut archive = ArchiveFile::new(path);
        if archive.exists() {
            if recovery == InterruptedDeleteRecovery::Decline
                && let Some(interrupted) = Self::interrupted_delete_in(&mut archive)?
            {
                return Err(DeclinedRecovery(interrupted).into());
            }
            archive.migrate_file_space_if_needed()?;
            archive.validate_schema_version(
                schema::SCHEMA_VERSION_ATTR,
                schema::CURRENT_SCHEMA_VERSION,
            )?;
            Self::recover_interrupted_delete(&mut archive)?;
            Self::drop_unindexed_events(&mut archive)?;
        } else {
            Self::create(&mut archive)?;
        }
        Ok(Self {
            archive: Mutex::new(archive),
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What an interrupted delete left in the archive at `path`, or [`None`]
    /// when there is nothing to recover. An archive that does not exist yet
    /// has nothing to recover either.
    ///
    /// The file is opened read-only and nothing in it changes.
    pub fn interrupted_delete_at(
        path: &Path,
    ) -> Result<Option<InterruptedDelete>, FlareStoreError> {
        let mut archive = ArchiveFile::new(path);
        if !archive.exists() {
            return Ok(None);
        }
        Self::interrupted_delete_in(&mut archive)
    }

    fn interrupted_delete_in(
        archive: &mut ArchiveFile,
    ) -> Result<Option<InterruptedDelete>, FlareStoreError> {
        let file = archive.open_read_only()?;
        with_layout(&file, |layout| layout.interrupted_delete())
    }

    fn create(archive: &mut ArchiveFile) -> Result<(), FlareStoreError> {
        let file = archive.create()?;
        attributes::write_i64(
            &file,
            schema::SCHEMA_VERSION_ATTR,
            schema::CURRENT_SCHEMA_VERSION,
        )?;

        let events = file.create_group(schema::EVENTS_GROUP)?;
        Column::create_strings(&events, schema::EVENT_ID, schema::EVENT_FORMAT)?;
        Column::create::<i64>(&events, schema::EVENT_BEGIN, schema::EVENT_FORMAT)?;
        Column::create::<i64>(&events, schema::EVENT_PEAK, schema::EVENT_FORMAT)?;
        Column::create::<i64>(&events, schema::EVENT_END, schema::EVENT_FORMAT)?;
        Column::create::<u8>(&events, schema::EVENT_END_PRESENCE, schema::EVENT_FORMAT)?;
        Column::create::<u8>(&events, schema::EVENT_CLASS, schema::EVENT_FORMAT)?;
        Column::create::<f64>(&events, schema::EVENT_MAGNITUDE, schema::EVENT_FORMAT)?;
        Column::create_strings(&events, schema::EVENT_SOURCE_LOCATION, schema::EVENT_FORMAT)?;
        Column::create::<u8>(
            &events,
            schema::EVENT_SOURCE_LOCATION_PRESENCE,
            schema::EVENT_FORMAT,
        )?;
        Column::create::<u32>(&events, schema::EVENT_ACTIVE_REGION, schema::EVENT_FORMAT)?;
        Column::create::<u8>(
            &events,
            schema::EVENT_ACTIVE_REGION_PRESENCE,
            schema::EVENT_FORMAT,
        )?;

        let days = file.create_group(schema::DAYS_GROUP)?;
        DayIndex::create_columns(&days, schema::DAY_FORMAT)?;
        Ok(())
    }

    fn recover_interrupted_delete(archive: &mut ArchiveFile) -> Result<(), FlareStoreError> {
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| {
            layout.recover_interrupted_delete(ARCHIVE_NAME)
        })
    }

    fn drop_unindexed_events(archive: &mut ArchiveFile) -> Result<(), FlareStoreError> {
        let file = archive.open_read_write()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let events = file.group(schema::EVENTS_GROUP)?;
        DayIndex::new(&days).drop_unindexed_rows(&events, &schema::EVENT_COLUMNS, ARCHIVE_NAME)?;
        Ok(())
    }

    /// Remove every day before `cutoff`, reporting how many days went.
    ///
    /// The events the remaining days hold move down to close the gap. The
    /// file itself does not shrink here at all: most of what a flare holds is
    /// text, whose bytes libhdf5 never hands back. The space the rest freed is
    /// what the days stored after the delete are written into.
    pub fn delete_days_before(
        &self,
        cutoff: NaiveDate,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, FlareStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| layout.delete_days_before(cutoff, report))
    }

    /// Remove every archived day, reporting how many went.
    pub fn delete_all_days(&self, report: PruneProgressSink<'_>) -> Result<usize, FlareStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| layout.delete_all_days(report))
    }

    /// Every day archived, oldest first.
    pub fn archived_days(&self) -> Result<Vec<ArchivedFlareDay>, FlareStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        Ok(DayIndex::new(&days)
            .entries()?
            .into_iter()
            .map(|entry| ArchivedFlareDay {
                day: entry.day,
                flares: entry.rows,
                fetched_at: entry.fetched_at,
                host: entry.host,
            })
            .collect())
    }

    /// Whether `day` is archived.
    pub fn contains(&self, day: NaiveDate) -> Result<bool, FlareStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        Ok(DayIndex::new(&days).row_of(day)?.is_some())
    }

    /// Store `flares` as the events of `day`, served by `host`, replacing
    /// whatever was archived for that day.
    pub fn insert_or_replace_day(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        flares: &[SolarFlare],
    ) -> Result<(), FlareStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_write()?;
        let events = file.group(schema::EVENTS_GROUP)?;

        let offset = Column::new(&events, schema::EVENT_BEGIN).rows()?;
        append_events(&events, flares)?;

        let placement = RowPlacement {
            offset: u64::try_from(offset)
                .map_err(|err| FlareStoreError::Corrupt(format!("event offset {offset}: {err}")))?,
            rows: u32::try_from(flares.len()).map_err(|err| {
                FlareStoreError::Corrupt(format!("{day} has too many flares: {err}"))
            })?,
        };

        // The day index goes last: events an interrupted store leaves behind
        // stay unindexed, and the next open cuts them.
        let days = file.group(schema::DAYS_GROUP)?;
        DayIndex::new(&days).insert_or_replace(day, placement, fetched_at, host)?;
        Ok(())
    }

    /// The flares archived for `day`, or [`None`] if the day is not archived.
    pub fn flares(&self, day: NaiveDate) -> Result<Option<Vec<SolarFlare>>, FlareStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let Some(rows) = DayIndex::new(&days).extent_of(day)? else {
            return Ok(None);
        };
        let events = file.group(schema::EVENTS_GROUP)?;
        Ok(Some(read_events(&events, rows)?))
    }
}

fn append_events(group: &Group, flares: &[SolarFlare]) -> Result<(), FlareStoreError> {
    let ids = stored_strings(flares.iter().map(|flare| flare.id.as_str()))?;
    let begins: Vec<i64> = flares.iter().map(|flare| flare.begin.timestamp()).collect();
    let peaks: Vec<i64> = flares.iter().map(|flare| flare.peak.timestamp()).collect();
    let ends: Vec<i64> = flares
        .iter()
        .map(|flare| {
            flare
                .end
                .map_or(schema::ABSENT_TIME_FILL, |end| end.timestamp())
        })
        .collect();
    let end_presence: Vec<u8> = flares
        .iter()
        .map(|flare| StoredPresence::of(&flare.end).code())
        .collect();
    let classes: Vec<u8> = flares
        .iter()
        .map(|flare| StoredFlareClass::from(flare.classification.class()).code())
        .collect();
    let magnitudes: Vec<f64> = flares
        .iter()
        .map(|flare| flare.classification.magnitude())
        .collect();
    let locations = stored_strings(
        flares
            .iter()
            .map(|flare| flare.source_location.as_deref().unwrap_or_default()),
    )?;
    let location_presence: Vec<u8> = flares
        .iter()
        .map(|flare| StoredPresence::of(&flare.source_location).code())
        .collect();
    let regions: Vec<u32> = flares
        .iter()
        .map(|flare| {
            flare
                .active_region
                .unwrap_or(schema::ABSENT_ACTIVE_REGION_FILL)
        })
        .collect();
    let region_presence: Vec<u8> = flares
        .iter()
        .map(|flare| StoredPresence::of(&flare.active_region).code())
        .collect();

    Column::new(group, schema::EVENT_ID).append(&ids)?;
    Column::new(group, schema::EVENT_BEGIN).append(&begins)?;
    Column::new(group, schema::EVENT_PEAK).append(&peaks)?;
    Column::new(group, schema::EVENT_END).append(&ends)?;
    Column::new(group, schema::EVENT_END_PRESENCE).append(&end_presence)?;
    Column::new(group, schema::EVENT_CLASS).append(&classes)?;
    Column::new(group, schema::EVENT_MAGNITUDE).append(&magnitudes)?;
    Column::new(group, schema::EVENT_SOURCE_LOCATION).append(&locations)?;
    Column::new(group, schema::EVENT_SOURCE_LOCATION_PRESENCE).append(&location_presence)?;
    Column::new(group, schema::EVENT_ACTIVE_REGION).append(&regions)?;
    Ok(Column::new(group, schema::EVENT_ACTIVE_REGION_PRESENCE).append(&region_presence)?)
}

fn read_events(group: &Group, rows: Range<usize>) -> Result<Vec<SolarFlare>, FlareStoreError> {
    let ids: Vec<VarLenUnicode> = Column::new(group, schema::EVENT_ID).read_slice(rows.clone())?;
    let begins: Vec<i64> = Column::new(group, schema::EVENT_BEGIN).read_slice(rows.clone())?;
    let peaks: Vec<i64> = Column::new(group, schema::EVENT_PEAK).read_slice(rows.clone())?;
    let ends: Vec<i64> = Column::new(group, schema::EVENT_END).read_slice(rows.clone())?;
    let end_presence: Vec<u8> =
        Column::new(group, schema::EVENT_END_PRESENCE).read_slice(rows.clone())?;
    let classes: Vec<u8> = Column::new(group, schema::EVENT_CLASS).read_slice(rows.clone())?;
    let magnitudes: Vec<f64> =
        Column::new(group, schema::EVENT_MAGNITUDE).read_slice(rows.clone())?;
    let locations: Vec<VarLenUnicode> =
        Column::new(group, schema::EVENT_SOURCE_LOCATION).read_slice(rows.clone())?;
    let location_presence: Vec<u8> =
        Column::new(group, schema::EVENT_SOURCE_LOCATION_PRESENCE).read_slice(rows.clone())?;
    let regions: Vec<u32> =
        Column::new(group, schema::EVENT_ACTIVE_REGION).read_slice(rows.clone())?;
    let region_presence: Vec<u8> =
        Column::new(group, schema::EVENT_ACTIVE_REGION_PRESENCE).read_slice(rows)?;

    ids.iter()
        .enumerate()
        .map(|(row, id)| {
            let (
                Some(&begin),
                Some(&peak),
                Some(&end),
                Some(&end_presence),
                Some(&class),
                Some(&magnitude),
                Some(location),
                Some(&location_presence),
                Some(&region),
                Some(&region_presence),
            ) = (
                begins.get(row),
                peaks.get(row),
                ends.get(row),
                end_presence.get(row),
                classes.get(row),
                magnitudes.get(row),
                locations.get(row),
                location_presence.get(row),
                regions.get(row),
                region_presence.get(row),
            )
            else {
                return Err(FlareStoreError::Corrupt(format!(
                    "flare row {row} ({id}) is short"
                )));
            };
            Ok(SolarFlare {
                id: id.as_str().to_owned(),
                begin: dates::timestamp_from_seconds(begin)?,
                peak: dates::timestamp_from_seconds(peak)?,
                end: match stored_presence(end_presence, row, "end")? {
                    StoredPresence::Published => Some(dates::timestamp_from_seconds(end)?),
                    StoredPresence::Unpublished => None,
                },
                classification: stored_classification(class, magnitude, row)?,
                source_location: match stored_presence(location_presence, row, "source location")? {
                    StoredPresence::Published => Some(location.as_str().to_owned()),
                    StoredPresence::Unpublished => None,
                },
                active_region: match stored_presence(region_presence, row, "active region")? {
                    StoredPresence::Published => Some(region),
                    StoredPresence::Unpublished => None,
                },
            })
        })
        .collect()
}

/// Whether the column beside the presence column holds a value the catalog
/// published, naming the field in the error a code outside the schema
/// produces.
fn stored_presence(code: u8, row: usize, field: &str) -> Result<StoredPresence, FlareStoreError> {
    StoredPresence::from_code(code).ok_or_else(|| {
        FlareStoreError::Corrupt(format!("flare row {row} has {field} presence code {code}"))
    })
}

fn stored_classification(
    class: u8,
    magnitude: f64,
    row: usize,
) -> Result<FlareClassification, FlareStoreError> {
    let class = StoredFlareClass::from_code(class).ok_or_else(|| {
        FlareStoreError::Corrupt(format!("flare row {row} has class code {class}"))
    })?;
    FlareClassification::new(FlareClass::from(class), magnitude).ok_or_else(|| {
        FlareStoreError::Corrupt(format!(
            "flare row {row} has magnitude {magnitude}, which no class publishes"
        ))
    })
}

fn stored_strings<'a>(
    values: impl Iterator<Item = &'a str>,
) -> Result<Vec<VarLenUnicode>, FlareStoreError> {
    values
        .map(|value| {
            value.parse::<VarLenUnicode>().map_err(|err| {
                FlareStoreError::Corrupt(format!("{value:?} cannot be stored: {err}"))
            })
        })
        .collect()
}

/// Run `act` against the archive's layout: one day index over one group of
/// event columns.
fn with_layout<T>(
    file: &OpenArchive<'_>,
    act: impl FnOnce(&ArchiveLayout<'_>) -> Result<T, ArchiveError>,
) -> Result<T, FlareStoreError> {
    let events = file.group(schema::EVENTS_GROUP)?;
    let levels = [RowLevel {
        group: &events,
        columns: &schema::EVENT_COLUMNS,
        extent: None,
    }];
    Ok(act(&ArchiveLayout {
        parent: file,
        index_name: schema::DAYS_GROUP,
        day_columns: &[],
        levels: &levels,
    })?)
}
