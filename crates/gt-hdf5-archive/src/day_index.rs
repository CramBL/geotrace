//! The day index an archive keys its rows by.
//!
//! One row per stored day: which rows of the data columns the day holds, when
//! it was fetched, and which host served it. One entry turns a day into one
//! slice of every data column, a day's rows being contiguous.

use std::ops::Range;

use chrono::{DateTime, NaiveDate, Utc};
use hdf5::Group;
use hdf5::types::VarLenUnicode;

use crate::{ArchiveError, Column, ColumnFormat, dates};

/// Days since the Unix epoch, per stored day.
pub const DAY: &str = "day";
/// First row of the day in the data columns.
pub const OFFSET: &str = "offset";
/// How many rows the day holds.
pub const COUNT: &str = "count";
/// When the day was fetched, Unix seconds.
pub const FETCHED_AT: &str = "fetched_at";
/// Host that served the day.
pub const HOST: &str = "host";

/// One stored day, as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayEntry {
    pub day: NaiveDate,
    pub rows: u32,
    pub fetched_at: DateTime<Utc>,
    pub host: String,
}

/// Where a day's rows sit in the data columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowPlacement {
    pub offset: u64,
    pub rows: u32,
}

/// The day index of one archive, or of one series within it.
pub struct DayIndex<'a> {
    group: &'a Group,
}

impl<'a> DayIndex<'a> {
    pub const fn new(group: &'a Group) -> Self {
        Self { group }
    }

    pub fn create_columns(group: &Group, format: ColumnFormat) -> Result<(), ArchiveError> {
        Column::create::<i32>(group, DAY, format)?;
        Column::create::<u64>(group, OFFSET, format)?;
        Column::create::<u32>(group, COUNT, format)?;
        Column::create::<i64>(group, FETCHED_AT, format)?;
        Column::create_strings(group, HOST, format)
    }

    /// Every stored day, oldest first.
    pub fn entries(&self) -> Result<Vec<DayEntry>, ArchiveError> {
        let days: Vec<i32> = Column::new(self.group, DAY).read()?;
        let counts: Vec<u32> = Column::new(self.group, COUNT).read()?;
        let fetched: Vec<i64> = Column::new(self.group, FETCHED_AT).read()?;
        let hosts: Vec<VarLenUnicode> = Column::new(self.group, HOST).read()?;
        if counts.len() != days.len() || fetched.len() != days.len() || hosts.len() != days.len() {
            return Err(ArchiveError::Corrupt(format!(
                "day index columns disagree: {} days, {} counts, {} timestamps, {} hosts",
                days.len(),
                counts.len(),
                fetched.len(),
                hosts.len()
            )));
        }

        let mut entries: Vec<DayEntry> = Vec::with_capacity(days.len());
        for (row, &day) in days.iter().enumerate() {
            let (Some(&rows), Some(&fetched_at), Some(host)) =
                (counts.get(row), fetched.get(row), hosts.get(row))
            else {
                return Err(ArchiveError::Corrupt(format!("day row {row} is short")));
            };
            entries.push(DayEntry {
                day: dates::date_from_epoch_days(day)?,
                rows,
                fetched_at: dates::timestamp_from_seconds(fetched_at)?,
                host: host.as_str().to_owned(),
            });
        }
        entries.sort_by_key(|entry| entry.day);
        Ok(entries)
    }

    /// Which row of the index holds `day`.
    pub fn row_of(&self, day: NaiveDate) -> Result<Option<usize>, ArchiveError> {
        let stored: Vec<i32> = Column::new(self.group, DAY).read()?;
        Ok(stored
            .iter()
            .position(|&indexed| indexed == day.to_epoch_days()))
    }

    /// Which rows of the data columns belong to `day`.
    pub fn extent_of(&self, day: NaiveDate) -> Result<Option<Range<usize>>, ArchiveError> {
        let Some(row) = self.row_of(day)? else {
            return Ok(None);
        };
        let offsets: Vec<u64> = Column::new(self.group, OFFSET).read()?;
        let counts: Vec<u32> = Column::new(self.group, COUNT).read()?;
        let (Some(&offset), Some(&count)) = (offsets.get(row), counts.get(row)) else {
            return Err(ArchiveError::Corrupt(format!(
                "{day} is indexed at row {row} but has no extent"
            )));
        };
        let start = usize::try_from(offset)
            .map_err(|err| ArchiveError::Corrupt(format!("{day} offset {offset}: {err}")))?;
        let count = usize::try_from(count)
            .map_err(|err| ArchiveError::Corrupt(format!("{day} count {count}: {err}")))?;
        Ok(Some(start..start + count))
    }

    /// Point `day` at `placement`, adding the day to the index if it is not
    /// there yet.
    ///
    /// Rows a replaced entry pointed at stay in the data columns with nothing
    /// referring to them.
    pub fn insert_or_replace(
        &self,
        day: NaiveDate,
        placement: RowPlacement,
        fetched_at: DateTime<Utc>,
        host: &str,
    ) -> Result<(), ArchiveError> {
        let stored_host = host
            .parse::<VarLenUnicode>()
            .map_err(|err| ArchiveError::Corrupt(format!("host {host:?}: {err}")))?;
        match self.row_of(day)? {
            Some(row) => {
                Column::new(self.group, OFFSET).write_row(row, placement.offset)?;
                Column::new(self.group, COUNT).write_row(row, placement.rows)?;
                Column::new(self.group, FETCHED_AT).write_row(row, fetched_at.timestamp())?;
                Column::new(self.group, HOST).write_row(row, stored_host)
            }
            None => {
                Column::new(self.group, DAY).append(&[day.to_epoch_days()])?;
                Column::new(self.group, OFFSET).append(&[placement.offset])?;
                Column::new(self.group, COUNT).append(&[placement.rows])?;
                Column::new(self.group, FETCHED_AT).append(&[fetched_at.timestamp()])?;
                Column::new(self.group, HOST).append(&[stored_host])
            }
        }
    }

    /// Cut rows past the end of the index from each of `columns`.
    ///
    /// An interrupted store leaves rows no day refers to: data columns are
    /// appended before the day that owns them is indexed. Columns *shorter*
    /// than the index means indexed rows are missing, which is not recoverable
    /// and is reported instead.
    pub fn drop_unindexed_rows(
        &self,
        data: &Group,
        columns: &[&str],
        archive_name: &str,
    ) -> Result<(), ArchiveError> {
        let indexed = self.rows_reached()?;
        for &name in columns {
            let column = Column::new(data, name);
            let rows = column.rows()?;
            if rows < indexed {
                return Err(ArchiveError::Corrupt(format!(
                    "{archive_name} column {name} holds {rows} rows but the day index reaches {indexed}"
                )));
            }
            if rows > indexed {
                log::warn!(
                    "Dropping {} unindexed rows from {archive_name} column {name:?}",
                    rows - indexed
                );
                column.truncate(indexed)?;
            }
        }
        Ok(())
    }

    /// How far into the data columns the index reaches.
    fn rows_reached(&self) -> Result<usize, ArchiveError> {
        let offsets: Vec<u64> = Column::new(self.group, OFFSET).read()?;
        let counts: Vec<u32> = Column::new(self.group, COUNT).read()?;
        let mut reached: usize = 0;
        for (&offset, &count) in offsets.iter().zip(&counts) {
            let end = usize::try_from(offset + u64::from(count))
                .map_err(|err| ArchiveError::Corrupt(format!("day extent {offset}: {err}")))?;
            reached = reached.max(end);
        }
        Ok(reached)
    }
}
