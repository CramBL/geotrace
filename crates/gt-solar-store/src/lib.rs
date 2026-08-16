//! The geomagnetic index archive: fetched index days, accumulated on disk.
//!
//! A day already archived costs no request: one HDF5 file holds every Kp and
//! Hp30 day ever fetched, queried per day. See [`schema`] for the layout.
//!
//! An archived day can be stored again: GFZ publishes Kp nowcast values and
//! replaces them with definitive ones once every station has reported.

use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use gt_hdf5_archive::day_index::{DayIndex, RowPlacement};
use gt_hdf5_archive::{ArchiveError, ArchiveFile, Column, attributes, dates};
use gt_solar::GeomagneticIndex;
use gt_solar::activity::GeomagneticActivity;
use gt_solar::series::{Hp30Sample, Hp30Series, IndexSample, IndexSeries, KpSample, KpSeries};
use hdf5::Group;
use parking_lot::Mutex;
use strum::IntoEnumIterator as _;

use crate::schema::{IndexArchiveLayout as _, StoredActivityPresence, StoredKpStatus};

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

/// The geomagnetic index archive.
#[derive(Debug)]
pub struct SolarStore {
    /// Every operation holds the lock for its whole sequence:
    /// [`Self::insert_or_replace_kp_day`] reads a column length, resizes,
    /// appends, and writes the day index last, and a caller reading between
    /// those steps sees samples that no index entry names.
    archive: Mutex<ArchiveFile>,
}

impl SolarStore {
    /// Open the archive at `path`, creating it if it does not exist.
    ///
    /// Samples left behind by an interrupted store are dropped here.
    pub fn open_or_create(path: &Path) -> Result<Self, SolarStoreError> {
        let mut archive = ArchiveFile::new(path);
        if archive.exists() {
            archive.validate_schema_version(
                schema::SCHEMA_VERSION_ATTR,
                schema::CURRENT_SCHEMA_VERSION,
            )?;
            Self::drop_unindexed_samples(&mut archive)?;
        } else {
            Self::create(&mut archive)?;
        }
        Ok(Self {
            archive: Mutex::new(archive),
        })
    }

    pub fn path(&self) -> PathBuf {
        self.archive.lock().path().to_owned()
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

    fn drop_unindexed_samples(archive: &mut ArchiveFile) -> Result<(), SolarStoreError> {
        let file = archive.open_read_write()?;
        for index in GeomagneticIndex::iter() {
            let days = file.group(&index.days_group_path())?;
            let samples = file.group(&index.samples_group_path())?;
            DayIndex::new(&days).drop_unindexed_rows(
                &samples,
                index.sample_columns(),
                &format!("geomagnetic archive {index}"),
            )?;
        }
        Ok(())
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

    /// Store `series` as the Kp of `day`, served by `host`, replacing whatever
    /// was archived for that day.
    ///
    /// The day is the key the series is read back under. A window requested
    /// midnight to midnight answers with the following day's first period too,
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

    /// The Kp archived for `day`, or [`None`] if the day is not archived.
    pub fn kp_series(&self, day: NaiveDate) -> Result<Option<KpSeries>, SolarStoreError> {
        self.day_series(day)
    }

    /// The Hp30 archived for `day`, or [`None`] if the day is not archived.
    pub fn hp30_series(&self, day: NaiveDate) -> Result<Option<Hp30Series>, SolarStoreError> {
        self.day_series(day)
    }

    fn insert_or_replace_day<S: ArchivedSample>(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        samples: &[S],
    ) -> Result<(), SolarStoreError> {
        let index = S::INDEX;
        let mut archive = self.archive.lock();
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

/// One index's sample type, and the columns the archive writes it to.
trait ArchivedSample: IndexSample + Sized {
    const INDEX: GeomagneticIndex;

    fn append_samples(group: &Group, samples: &[Self]) -> Result<(), SolarStoreError>;

    fn read_samples(group: &Group, rows: Range<usize>) -> Result<Vec<Self>, SolarStoreError>;
}

impl ArchivedSample for KpSample {
    const INDEX: GeomagneticIndex = GeomagneticIndex::Kp;

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
    const INDEX: GeomagneticIndex = GeomagneticIndex::Hp30;

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
        .map(|sample| StoredActivityPresence::from(sample.activity()).code())
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
            let presence = StoredActivityPresence::from_code(code).ok_or_else(|| {
                SolarStoreError::Corrupt(format!(
                    "{index} sample {position} has activity presence code {code}"
                ))
            })?;
            let activity = match presence {
                StoredActivityPresence::Unpublished => None,
                StoredActivityPresence::Published => Some(
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
