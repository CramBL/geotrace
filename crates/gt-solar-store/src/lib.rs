//! The geomagnetic index archive: fetched index days, accumulated on disk.
//!
//! A day already archived costs no request: one HDF5 file holds every Kp and
//! Hp30 day ever fetched, queried per day. See [`schema`] for the layout.
//!
//! An archived day can be stored again: GFZ publishes Kp nowcast values and
//! replaces them with definitive ones once every station has reported.

use std::collections::BTreeSet;
use std::ops::{Deref, Range};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use gt_hdf5_archive::day_index::{self, DayIndex, RowPlacement};
use gt_hdf5_archive::prune::{
    ArchiveLayout, DeclinedRecovery, InterruptedDelete, InterruptedDeleteRecovery, PruneProgress,
    PruneProgressSink, RowLevel,
};
use gt_hdf5_archive::{
    ArchiveError, ArchiveFile, Column, OpenArchive, StoredPresence, attributes, dates,
};
use gt_solar::GeomagneticIndex;
use gt_solar::activity::GeomagneticActivity;
use gt_solar::series::{Hp30Sample, Hp30Series, IndexSample, IndexSeries, KpSample, KpSeries};
use hdf5::Group;
use parking_lot::Mutex;
use strum::IntoEnumIterator as _;

use crate::schema::{IndexArchiveLayout as _, StoredKpStatus};

pub mod schema;

/// Name of the archive file, joined to the data directory by the caller.
pub const FILE_NAME: &str = "geomagnetic.h5";

#[derive(Debug, thiserror::Error)]
pub enum SolarStoreError {
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

impl From<ArchiveError> for SolarStoreError {
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

impl From<hdf5::Error> for SolarStoreError {
    fn from(err: hdf5::Error) -> Self {
        ArchiveError::from(err).into()
    }
}

/// One index's entry for one day in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedIndexDay {
    pub day: NaiveDate,
    /// Samples stored for the day, periods without a value included.
    pub samples: u32,
    pub fetched_at: DateTime<Utc>,
    /// Host that served it.
    pub host: String,
}

/// The geomagnetic index archive with no method that writes to the file, as
/// [`Self::open_existing_read_only`] opens it.
#[derive(Debug)]
pub struct ReadOnlySolarStore {
    /// Every operation holds the lock for its whole sequence:
    /// [`SolarStore::insert_or_replace_kp_day`] reads a column length,
    /// resizes, appends, and writes the day index last, and a caller reading
    /// between those steps sees samples that no index entry names.
    archive: Mutex<ArchiveFile>,
    /// Held beside the lock: a caller reading the archive's path never waits
    /// for a delete rewriting it.
    path: PathBuf,
}

impl ReadOnlySolarStore {
    /// Open the archive at `path` without writing to it: it is not created
    /// where it is missing, not rebuilt, and neither an interrupted insert nor
    /// an interrupted delete in it is put right.
    ///
    /// An archive an interrupted delete left part-way through fails with
    /// [`SolarStoreError::DeclinedRecovery`]: its day index cannot be read as it
    /// stands, and putting it right is a write.
    pub fn open_existing_read_only(path: &Path) -> Result<Self, SolarStoreError> {
        let mut archive = ArchiveFile::new(path);
        archive.check_readable_without_writing(
            interrupted_delete_in,
            schema::SCHEMA_VERSION_ATTR,
            schema::CURRENT_SCHEMA_VERSION,
        )?;
        Ok(Self {
            archive: Mutex::new(archive),
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every day archived for `index`, oldest first.
    pub fn archived_days(
        &self,
        index: GeomagneticIndex,
    ) -> Result<Vec<ArchivedIndexDay>, SolarStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(&index.days_group_path())?;
        Ok(DayIndex::new(&days)
            .entries()?
            .into_iter()
            .map(|entry| ArchivedIndexDay {
                day: entry.day,
                samples: entry.rows,
                fetched_at: entry.fetched_at,
                host: entry.host,
            })
            .collect())
    }

    /// Whether `day` is archived for `index`.
    pub fn contains(
        &self,
        index: GeomagneticIndex,
        day: NaiveDate,
    ) -> Result<bool, SolarStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(&index.days_group_path())?;
        Ok(DayIndex::new(&days).row_of(day)?.is_some())
    }

    /// The Kp archived for `day`, or [`None`] if the day is not archived.
    pub fn kp_series(&self, day: NaiveDate) -> Result<Option<KpSeries>, SolarStoreError> {
        self.day_series(day)
    }

    /// The Hp30 archived for `day`, or [`None`] if the day is not archived.
    pub fn hp30_series(&self, day: NaiveDate) -> Result<Option<Hp30Series>, SolarStoreError> {
        self.day_series(day)
    }

    fn day_series<S: ArchivedSample>(
        &self,
        day: NaiveDate,
    ) -> Result<Option<IndexSeries<S>>, SolarStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(&S::INDEX.days_group_path())?;
        let Some(rows) = DayIndex::new(&days).extent_of(day)? else {
            return Ok(None);
        };
        let group = file.group(&S::INDEX.samples_group_path())?;
        Ok(Some(IndexSeries {
            samples: S::read_samples(&group, rows)?,
        }))
    }
}

/// The geomagnetic index archive, which reads through [`ReadOnlySolarStore`]
/// and adds [`Self::insert_or_replace_kp_day`],
/// [`Self::insert_or_replace_hp30_day`], [`Self::delete_days_before`] and
/// [`Self::delete_all_days`].
#[derive(Debug)]
pub struct SolarStore {
    inner: ReadOnlySolarStore,
}

impl Deref for SolarStore {
    type Target = ReadOnlySolarStore;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl SolarStore {
    /// Open the archive at `path`, creating it if it does not exist.
    ///
    /// An archive created before archives recorded their free space in pages
    /// is rebuilt first, see [`ArchiveFile::migrate_file_space_if_needed`].
    ///
    /// Samples left behind by an interrupted store are dropped here, and so
    /// are the days an interrupted [`Self::delete_days_before`] left in an
    /// unknown layout.
    pub fn open_or_create(path: &Path) -> Result<Self, SolarStoreError> {
        Self::open_or_create_with_recovery_choice(path, InterruptedDeleteRecovery::Recover)
    }

    /// Open the archive at `path` as [`Self::open_or_create`] does, recovering
    /// an interrupted delete only when `recovery` asks for it.
    ///
    /// One interrupted index declines the whole archive: the two indices share
    /// the file, and a delete runs through both. A declined recovery leaves the
    /// file exactly as it was found and fails with
    /// [`SolarStoreError::DeclinedRecovery`], which is checked before anything
    /// else the open would write.
    pub fn open_or_create_with_recovery_choice(
        path: &Path,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<Self, SolarStoreError> {
        let mut archive = ArchiveFile::new(path);
        if archive.exists() {
            if recovery == InterruptedDeleteRecovery::Decline
                && let Some(interrupted) = interrupted_delete_in(&mut archive)?
            {
                return Err(DeclinedRecovery(interrupted).into());
            }
            archive.migrate_file_space_if_needed()?;
            archive.validate_schema_version(
                schema::SCHEMA_VERSION_ATTR,
                schema::CURRENT_SCHEMA_VERSION,
            )?;
            Self::recover_interrupted_delete(&mut archive)?;
            Self::drop_unindexed_samples(&mut archive)?;
        } else {
            Self::create(&mut archive)?;
        }
        Ok(Self {
            inner: ReadOnlySolarStore {
                archive: Mutex::new(archive),
                path: path.to_owned(),
            },
        })
    }

    /// What an interrupted delete left in the archive at `path`, or [`None`]
    /// when there is nothing to recover. An archive that does not exist yet
    /// has nothing to recover either.
    ///
    /// The count covers the days of the interrupted indices alone, a day both
    /// of them hold counting once: a delete runs through the Kp index and the
    /// Hp30 index in turn, and recovery discards the interrupted ones whole
    /// while a settled index keeps its days.
    ///
    /// The file is opened read-only and nothing in it changes.
    pub fn interrupted_delete_at(
        path: &Path,
    ) -> Result<Option<InterruptedDelete>, SolarStoreError> {
        let mut archive = ArchiveFile::new(path);
        if !archive.exists() {
            return Ok(None);
        }
        interrupted_delete_in(&mut archive)
    }

    fn create(archive: &mut ArchiveFile) -> Result<(), SolarStoreError> {
        let file = archive.create()?;
        attributes::write_i64(
            &file,
            schema::SCHEMA_VERSION_ATTR,
            schema::CURRENT_SCHEMA_VERSION,
        )?;

        for index in GeomagneticIndex::iter() {
            let samples = file.create_group(&index.samples_group_path())?;
            Column::create::<i64>(&samples, schema::SAMPLE_PERIOD_START, schema::SAMPLE_FORMAT)?;
            Column::create::<f64>(&samples, schema::SAMPLE_ACTIVITY, schema::SAMPLE_FORMAT)?;
            Column::create::<u8>(
                &samples,
                schema::SAMPLE_ACTIVITY_PRESENCE,
                schema::SAMPLE_FORMAT,
            )?;
            if index.publishes_status() {
                Column::create::<u8>(&samples, schema::SAMPLE_KP_STATUS, schema::SAMPLE_FORMAT)?;
            }

            let days = file.create_group(&index.days_group_path())?;
            DayIndex::create_columns(&days, schema::DAY_FORMAT)?;
        }
        Ok(())
    }

    fn recover_interrupted_delete(archive: &mut ArchiveFile) -> Result<(), SolarStoreError> {
        let file = archive.open_read_write()?;
        for index in GeomagneticIndex::iter() {
            with_layout(&file, index, |layout| {
                layout.recover_interrupted_delete(&archive_name(index))
            })?;
        }
        Ok(())
    }

    fn drop_unindexed_samples(archive: &mut ArchiveFile) -> Result<(), SolarStoreError> {
        let file = archive.open_read_write()?;
        for index in GeomagneticIndex::iter() {
            let days = file.group(&index.days_group_path())?;
            let samples = file.group(&index.samples_group_path())?;
            DayIndex::new(&days).drop_unindexed_rows(
                &samples,
                index.sample_columns(),
                &archive_name(index),
            )?;
        }
        Ok(())
    }

    /// Remove every day before `cutoff` from both indices, reporting how many
    /// days went. A day either index held counts once.
    ///
    /// The samples the remaining days hold move down to close the gap. The
    /// file itself rarely shrinks: the space is what the days stored after the
    /// delete are written into.
    pub fn delete_days_before(
        &self,
        cutoff: NaiveDate,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, SolarStoreError> {
        self.delete_from_both_indices(DeletedDays::Before(cutoff), report)
    }

    /// Remove every archived day from both indices, reporting how many went.
    /// A day either index held counts once.
    pub fn delete_all_days(&self, report: PruneProgressSink<'_>) -> Result<usize, SolarStoreError> {
        self.delete_from_both_indices(DeletedDays::Every, report)
    }

    /// Delete `deleted` from each index in turn, counting the columns of both
    /// as one run: the second index carries on where the first stopped rather
    /// than starting the count again.
    fn delete_from_both_indices(
        &self,
        deleted: DeletedDays,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, SolarStoreError> {
        let mut archive = self.inner.archive.lock();
        let file = archive.open_read_write()?;
        let mut columns_of_index: Vec<(GeomagneticIndex, usize)> = Vec::new();
        for index in GeomagneticIndex::iter() {
            let columns =
                with_layout(
                    &file,
                    index,
                    |layout| Ok(layout.columns_a_delete_rewrites()),
                )?;
            columns_of_index.push((index, columns));
        }
        let columns_total: usize = columns_of_index.iter().map(|(_, columns)| columns).sum();

        let mut columns_before = 0;
        let mut removed: BTreeSet<NaiveDate> = BTreeSet::new();
        for (index, columns) in columns_of_index {
            let days = file.group(&index.days_group_path())?;
            removed.extend(
                DayIndex::new(&days)
                    .entries()?
                    .into_iter()
                    .map(|entry| entry.day)
                    .filter(|day| deleted.covers(*day)),
            );
            drop(days);
            let forwarded = |progress: PruneProgress| {
                if let Some(report) = report {
                    report(PruneProgress {
                        columns_rewritten: columns_before + progress.columns_rewritten,
                        columns_total,
                    });
                }
            };
            with_layout(&file, index, |layout| {
                deleted.delete_from(layout, Some(&forwarded))
            })?;
            columns_before += columns;
        }
        Ok(removed.len())
    }

    /// Store `series` as the Kp of `day`, served by `host`, replacing whatever
    /// was archived for that day.
    ///
    /// The day is the key the series is read back under. A window requested
    /// midnight to midnight also returns the following day's first period,
    /// and the archive stores whichever samples it is given.
    pub fn insert_or_replace_kp_day(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        series: &KpSeries,
    ) -> Result<(), SolarStoreError> {
        self.insert_or_replace_day(day, host, fetched_at, &series.samples)
    }

    /// Store `series` as the Hp30 of `day`, served by `host`, replacing
    /// whatever was archived for that day.
    pub fn insert_or_replace_hp30_day(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        series: &Hp30Series,
    ) -> Result<(), SolarStoreError> {
        self.insert_or_replace_day(day, host, fetched_at, &series.samples)
    }

    fn insert_or_replace_day<S: ArchivedSample>(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        samples: &[S],
    ) -> Result<(), SolarStoreError> {
        let index = S::INDEX;
        let mut archive = self.inner.archive.lock();
        let file = archive.open_read_write()?;
        let group = file.group(&index.samples_group_path())?;

        let offset = Column::new(&group, schema::SAMPLE_PERIOD_START).rows()?;
        S::append_samples(&group, samples)?;

        let placement = RowPlacement {
            offset: u64::try_from(offset).map_err(|err| {
                SolarStoreError::Corrupt(format!("{index} sample offset {offset}: {err}"))
            })?,
            rows: u32::try_from(samples.len()).map_err(|err| {
                SolarStoreError::Corrupt(format!("{index} {day} has too many samples: {err}"))
            })?,
        };

        // The day index goes last: samples an interrupted store leaves
        // behind stay unindexed, and the next open cuts them.
        let days = file.group(&index.days_group_path())?;
        DayIndex::new(&days).insert_or_replace(day, placement, fetched_at, host)?;
        Ok(())
    }
}

fn interrupted_delete_in(
    archive: &mut ArchiveFile,
) -> Result<Option<InterruptedDelete>, SolarStoreError> {
    let file = archive.open_read_only()?;
    let mut interrupted = false;
    let mut discarded_epoch_days: BTreeSet<i32> = BTreeSet::new();
    for index in GeomagneticIndex::iter() {
        if with_layout(&file, index, |layout| layout.interrupted_delete())?.is_none() {
            continue;
        }
        interrupted = true;
        let days = file.group(&index.days_group_path())?;
        discarded_epoch_days.extend(Column::new(&days, day_index::DAY).read::<i32>()?);
    }
    Ok(interrupted.then_some(InterruptedDelete {
        archived_days: discarded_epoch_days.len(),
    }))
}

/// One index's sample type, and the columns the archive writes it to.
trait ArchivedSample: IndexSample + Sized {
    fn append_samples(group: &Group, samples: &[Self]) -> Result<(), SolarStoreError>;

    fn read_samples(group: &Group, rows: Range<usize>) -> Result<Vec<Self>, SolarStoreError>;
}

impl ArchivedSample for KpSample {
    fn append_samples(group: &Group, samples: &[Self]) -> Result<(), SolarStoreError> {
        append_period_columns(group, samples)?;
        let statuses: Vec<u8> = samples
            .iter()
            .map(|sample| StoredKpStatus::from(sample.status).code())
            .collect();
        Ok(Column::new(group, schema::SAMPLE_KP_STATUS).append(&statuses)?)
    }

    fn read_samples(group: &Group, rows: Range<usize>) -> Result<Vec<Self>, SolarStoreError> {
        let codes: Vec<u8> =
            Column::new(group, schema::SAMPLE_KP_STATUS).read_slice(rows.clone())?;
        read_period_columns(Self::INDEX, group, rows)?
            .into_iter()
            .enumerate()
            .map(|(position, period)| {
                let Some(&code) = codes.get(position) else {
                    return Err(SolarStoreError::Corrupt(format!(
                        "Kp sample {position} has no status"
                    )));
                };
                let status = StoredKpStatus::from_code(code).ok_or_else(|| {
                    SolarStoreError::Corrupt(format!("Kp sample {position} has status code {code}"))
                })?;
                Ok(Self {
                    period_start: period.start,
                    activity: period.activity,
                    status: status.into(),
                })
            })
            .collect()
    }
}

impl ArchivedSample for Hp30Sample {
    fn append_samples(group: &Group, samples: &[Self]) -> Result<(), SolarStoreError> {
        append_period_columns(group, samples)
    }

    fn read_samples(group: &Group, rows: Range<usize>) -> Result<Vec<Self>, SolarStoreError> {
        Ok(read_period_columns(Self::INDEX, group, rows)?
            .into_iter()
            .map(|period| Self {
                period_start: period.start,
                activity: period.activity,
            })
            .collect())
    }
}

/// One period as the columns every index shares hold it.
#[derive(Debug, Clone, Copy)]
struct StoredPeriod {
    start: DateTime<Utc>,
    activity: Option<GeomagneticActivity>,
}

fn append_period_columns<S: IndexSample>(
    group: &Group,
    samples: &[S],
) -> Result<(), SolarStoreError> {
    let starts: Vec<i64> = samples
        .iter()
        .map(|sample| sample.period_start().timestamp())
        .collect();
    let activities: Vec<f64> = samples
        .iter()
        .map(|sample| {
            sample.activity().map_or(
                schema::UNPUBLISHED_ACTIVITY_FILL,
                GeomagneticActivity::value,
            )
        })
        .collect();
    let presence: Vec<u8> = samples
        .iter()
        .map(|sample| StoredPresence::of(&sample.activity()).code())
        .collect();

    Column::new(group, schema::SAMPLE_PERIOD_START).append(&starts)?;
    Column::new(group, schema::SAMPLE_ACTIVITY).append(&activities)?;
    Ok(Column::new(group, schema::SAMPLE_ACTIVITY_PRESENCE).append(&presence)?)
}

fn read_period_columns(
    index: GeomagneticIndex,
    group: &Group,
    rows: Range<usize>,
) -> Result<Vec<StoredPeriod>, SolarStoreError> {
    let starts: Vec<i64> =
        Column::new(group, schema::SAMPLE_PERIOD_START).read_slice(rows.clone())?;
    let activities: Vec<f64> =
        Column::new(group, schema::SAMPLE_ACTIVITY).read_slice(rows.clone())?;
    let presence: Vec<u8> =
        Column::new(group, schema::SAMPLE_ACTIVITY_PRESENCE).read_slice(rows)?;

    starts
        .iter()
        .enumerate()
        .map(|(position, &start)| {
            let (Some(&value), Some(&code)) = (activities.get(position), presence.get(position))
            else {
                return Err(SolarStoreError::Corrupt(format!(
                    "{index} sample {position} has no value"
                )));
            };
            let presence = StoredPresence::from_code(code).ok_or_else(|| {
                SolarStoreError::Corrupt(format!(
                    "{index} sample {position} has activity presence code {code}"
                ))
            })?;
            let activity = match presence {
                StoredPresence::Unpublished => None,
                StoredPresence::Published => Some(
                    GeomagneticActivity::from_published_value(index, value).ok_or_else(|| {
                        SolarStoreError::Corrupt(format!(
                            "{index} sample {position} is {value}, outside the range {index} is published in"
                        ))
                    })?,
                ),
            };
            Ok(StoredPeriod {
                start: dates::timestamp_from_seconds(start)?,
                activity,
            })
        })
        .collect()
}

/// The days one delete removes from an index.
#[derive(Debug, Clone, Copy)]
enum DeletedDays {
    Before(NaiveDate),
    Every,
}

impl DeletedDays {
    fn covers(self, day: NaiveDate) -> bool {
        match self {
            Self::Before(cutoff) => day < cutoff,
            Self::Every => true,
        }
    }

    fn delete_from(
        self,
        layout: &ArchiveLayout<'_>,
        report: PruneProgressSink<'_>,
    ) -> Result<usize, ArchiveError> {
        match self {
            Self::Before(cutoff) => layout.delete_days_before(cutoff, report),
            Self::Every => layout.delete_all_days(report),
        }
    }
}

/// The index's archive, as messages about its columns name it.
fn archive_name(index: GeomagneticIndex) -> String {
    format!("geomagnetic archive {index}")
}

/// Run `act` against one index's layout: its day index over its sample
/// columns.
fn with_layout<T>(
    file: &OpenArchive<'_>,
    index: GeomagneticIndex,
    act: impl FnOnce(&ArchiveLayout<'_>) -> Result<T, ArchiveError>,
) -> Result<T, SolarStoreError> {
    let samples = file.group(&index.samples_group_path())?;
    let levels = [RowLevel {
        group: &samples,
        columns: index.sample_columns(),
        extent: None,
    }];
    Ok(act(&ArchiveLayout {
        parent: file,
        index_name: &index.days_group_path(),
        day_columns: &[],
        levels: &levels,
    })?)
}
