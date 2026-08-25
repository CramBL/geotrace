use chrono::NaiveDate;
use gt_pending_writes::WriteRefusal;

use super::day_fetch_queue::DayFetchQueue;

/// A finished day fetch that added nothing to the archive, held by each of
/// the four environment schedulers' message enums next to its own `Stored`
/// variant.
pub enum UnarchivedDay {
    /// The fetch, the parse or the insert returned an error.
    Failed { day: NaiveDate, detail: String },
    /// The day was downloaded, then discarded unarchived because
    /// [`gt_store::WritableArchive::write`] returned a [`WriteRefusal`].
    Refused {
        day: NaiveDate,
        refusal: WriteRefusal,
    },
}

impl UnarchivedDay {
    pub fn failed(day: NaiveDate, detail: String) -> Self {
        Self::Failed { day, detail }
    }

    pub fn refused(day: NaiveDate, refusal: WriteRefusal) -> Self {
        Self::Refused { day, refusal }
    }

    pub fn day(&self) -> NaiveDate {
        match *self {
            Self::Failed { day, .. } | Self::Refused { day, .. } => day,
        }
    }

    /// Log the outcome and, for [`Self::Failed`], push the detail onto `days`
    /// so the settings dialog lists the day.
    ///
    /// `dataset` fills the `No {dataset} archived for {day}` log line: a
    /// plural noun phrase as it reads mid-sentence, such as `TEC maps`.
    pub fn log_and_record_failure(self, dataset: &str, days: &mut DayFetchQueue) {
        match self {
            Self::Failed { day, detail } => {
                log::error!("No {dataset} archived for {day}: {detail}");
                days.report_failure(day, detail);
            }
            Self::Refused { day, refusal } => {
                log::debug!("No {dataset} archived for {day}: {refusal}");
            }
        }
    }
}
