//! The date and time forms archives store, and reading them back.

use chrono::{DateTime, NaiveDate, Utc};

use crate::ArchiveError;

pub fn date_from_epoch_days(days: i32) -> Result<NaiveDate, ArchiveError> {
    NaiveDate::from_epoch_days(days)
        .ok_or_else(|| ArchiveError::Corrupt(format!("day {days} is not a date")))
}

pub fn timestamp_from_seconds(seconds: i64) -> Result<DateTime<Utc>, ArchiveError> {
    DateTime::from_timestamp(seconds, 0)
        .ok_or_else(|| ArchiveError::Corrupt(format!("{seconds} is not a Unix timestamp")))
}
