use std::num::NonZeroUsize;
use std::ops::RangeInclusive;
use std::path::Path;

use chrono::{DateTime, NaiveDate, NaiveTime};
use gt_history_types::{
    DatabaseRef, HistoryEntries, HistoryOpenSelection, HistoryOpenSelectionError, RecordingEntry,
};
use serde::{Deserialize, Serialize};
use uuid::{Builder, Uuid};
use xxhash_rust::xxh3::Xxh3;

use crate::protocol::{ErrorCode, RecordingReference, TimeRange};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ListHistoryRecordings {
    pub cursor: Option<HistoryCursor>,
    pub dates: Option<HistoryDates>,
    pub limit: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HistoryDates {
    /// Inclusive lower UTC date for overlap with the recording's time range.
    pub from: Option<NaiveDate>,
    /// Inclusive upper UTC date for overlap with the recording's time range.
    pub through: Option<NaiveDate>,
}

impl HistoryDates {
    fn timestamps(self) -> Result<Option<RangeInclusive<i64>>, ErrorCode> {
        let midnight = |date: NaiveDate| date.and_time(NaiveTime::MIN).and_utc().timestamp_micros();
        match (self.from, self.through) {
            (None, None) => Ok(None),
            (Some(from), Some(through)) if from > through => {
                Err(ErrorCode::InvalidHistoryDateRange)
            }
            (from, through) => Ok(Some(
                from.map_or(i64::MIN, midnight)
                    ..=through.map_or(i64::MAX, |date| midnight(date) + UTC_DAY_MICROS - 1),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HistoryCursor {
    session: Uuid,
    revision: u64,
    index: usize,
    dates: HistoryDates,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct HistoryRecordingId(Uuid);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OpenHistoryRecording {
    pub recording: HistoryRecordingId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GetHistoryOpenStatus {
    pub operation: HistoryOpenId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HistoryOpenId {
    pub session: Uuid,
    pub id: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryOpenStatus {
    pub operation: HistoryOpenId,
    pub state: HistoryOpenState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[expect(variant_size_differences, reason = "The ready reply fits in 32 bytes")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryOpenState {
    AwaitingTrackSettings,
    Cancelled,
    Expired,
    Failed {
        code: ErrorCode,
    },
    Loading,
    Reading,
    Ready {
        recording: RecordingReference,
        reused: bool,
    },
}

impl HistoryRecordingId {
    fn of(database: &Path, recordings: &HistoryEntries, reference: &DatabaseRef) -> Self {
        let path_bytes = database.as_os_str().as_encoded_bytes();
        let mut hash = Xxh3::new();
        hash.update(&(path_bytes.len() as u64).to_le_bytes());
        hash.update(path_bytes);
        // An interrupted identity rename can leave a copy of a group under both identities.
        let include_identity = !reference.has_generated_group_id()
            || recordings.has_repeated_group(&reference.group_name);
        hash.update(&[u8::from(include_identity)]);
        if include_identity {
            hash.update(&(reference.identity.len() as u64).to_le_bytes());
            hash.update(reference.identity.as_bytes());
        }
        hash.update(reference.group_name.as_bytes());
        Self(Builder::from_custom_bytes(hash.digest128().to_be_bytes()).into_uuid())
    }
}

pub struct HistoryListing<'a> {
    pub database: &'a Path,
    pub entries: &'a HistoryEntries,
    pub revision: u64,
    pub session: Uuid,
}

impl HistoryListing<'_> {
    pub fn select_for_open(
        &self,
        id: HistoryRecordingId,
    ) -> Result<HistoryOpenSelection, ErrorCode> {
        let mut matches = self.entries.iter().enumerate().filter(|(_, entry)| {
            HistoryRecordingId::of(self.database, self.entries, &entry.db_ref) == id
        });
        let (entry_index, _) = matches.next().ok_or(ErrorCode::HistorySelectionExpired)?;
        if matches.next().is_some() {
            return Err(ErrorCode::HistorySelectionExpired);
        }
        self.entries
            .select_for_open(entry_index)
            .map_err(|error| match error {
                HistoryOpenSelectionError::AmbiguousGroup
                | HistoryOpenSelectionError::MissingEntry => ErrorCode::HistorySelectionExpired,
                HistoryOpenSelectionError::LegacyGroup => {
                    ErrorCode::HistorySelectionRequiresDesktop
                }
            })
    }

    pub fn page(&self, request: ListHistoryRecordings) -> Result<HistoryRecordings, ErrorCode> {
        let dates = request.dates.unwrap_or_default();
        let timestamps = dates.timestamps()?;
        let cursor = HistoryCursor {
            session: self.session,
            revision: self.revision,
            index: 0,
            dates,
        };
        let start = match request.cursor {
            Some(after)
                if after
                    == (HistoryCursor {
                        index: after.index,
                        ..cursor
                    })
                    && after.index > 0
                    && after.index < self.entries.len() =>
            {
                after.index
            }
            Some(_) => return Err(ErrorCode::InvalidHistoryCursor),
            None => 0,
        };
        let limit = request
            .limit
            .map_or(MAX_PER_PAGE, NonZeroUsize::get)
            .min(MAX_PER_PAGE);
        let mut matches = self
            .entries
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, entry)| {
                timestamps.as_ref().is_none_or(|filter| {
                    entry.meta.time_range.is_some_and(|span| {
                        span.start_us() <= *filter.end() && span.end_us() >= *filter.start()
                    })
                })
            });
        let recordings = matches
            .by_ref()
            .take(limit)
            .map(|(_, entry)| {
                let id = HistoryRecordingId::of(self.database, self.entries, &entry.db_ref);
                HistoryRecording::of(id, entry)
            })
            .collect();
        let next_cursor = matches
            .next()
            .map(|(index, _)| HistoryCursor { index, ..cursor });
        Ok(HistoryRecordings {
            recordings,
            next_cursor,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryRecordings {
    pub recordings: Vec<HistoryRecording>,
    pub next_cursor: Option<HistoryCursor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryRecording {
    pub id: HistoryRecordingId,
    pub identity: String,
    pub title: Option<String>,
    pub metadata_truncated: bool,
    pub time_range: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub time_range_unrepresentable: bool,
    pub nav_points: u64,
    pub total_tracks: usize,
    pub shelved_tracks: usize,
}

impl HistoryRecording {
    fn of(id: HistoryRecordingId, entry: &RecordingEntry) -> Self {
        let (identity, _) = gt_loaded_files::display_identity(&entry.db_ref.identity);
        let title = entry.title.as_deref();
        let time_range = entry.meta.time_range.and_then(|range| {
            let start = DateTime::from_timestamp_micros(range.start_us())?;
            let end = DateTime::from_timestamp_micros(range.end_us())?;
            TimeRange { start, end }.rfc3339()
        });
        Self {
            id,
            identity: self::metadata_preview(identity),
            title: title.map(self::metadata_preview),
            metadata_truncated: identity.len() > MAX_METADATA_BYTES
                || title.is_some_and(|title| title.len() > MAX_METADATA_BYTES),
            time_range,
            time_range_unrepresentable: entry.meta.time_range.is_some() && time_range.is_none(),
            nav_points: entry.meta.nav_point_count,
            total_tracks: entry.total_tracks,
            shelved_tracks: entry.shelved_tracks,
        }
    }
}

fn metadata_preview(text: &str) -> String {
    text.get(..text.floor_char_boundary(MAX_METADATA_BYTES))
        .unwrap_or_default()
        .to_owned()
}

const MAX_PER_PAGE: usize = 32;
const MAX_METADATA_BYTES: usize = 512;
const UTC_DAY_MICROS: i64 = 86_400_000_000;

#[cfg(test)]
mod tests;
