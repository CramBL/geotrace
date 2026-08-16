//! Layout of the geomagnetic index archive.
//!
//! One group per index, each holding the index's samples and its own day
//! index, all extensible along their one dimension:
//!
//! ```text
//! /kp/samples/{period_start,activity,activity_presence,status}
//! /kp/days/{day,offset,count,fetched_at,host}
//! /hp30/samples/{period_start,activity,activity_presence}
//! /hp30/days/{day,offset,count,fetched_at,host}
//! ```
//!
//! The day groups are [`gt_hdf5_archive::day_index`] columns. One entry turns
//! a day into one slice of the sample columns, a day's samples being
//! contiguous. Storing a day that is already archived appends its new samples
//! and repoints the day's entry: the samples it replaces stay in the columns
//! with nothing referring to them.
//!
//! Each column is a dataset of its own, shuffled and deflated.

use gt_hdf5_archive::ColumnFormat;
use gt_solar::GeomagneticIndex;
use gt_solar::activity::GeomagneticActivity;
use gt_solar::series::KpStatus;

/// Group holding the Kp columns.
pub const KP_GROUP: &str = "kp";

/// Group holding the Hp30 columns.
pub const HP30_GROUP: &str = "hp30";

/// Subgroup holding an index's per-period columns.
pub const SAMPLES_GROUP: &str = "samples";

/// Subgroup holding an index's day index.
pub const DAYS_GROUP: &str = "days";

/// Start of the period a sample covers, Unix seconds.
pub const SAMPLE_PERIOD_START: &str = "period_start";
/// The period's value on the Kp scale, read only where
/// [`SAMPLE_ACTIVITY_PRESENCE`] says the service published one.
pub const SAMPLE_ACTIVITY: &str = "activity";
/// Whether the service published a value for the period, coded by
/// [`StoredActivityPresence`].
pub const SAMPLE_ACTIVITY_PRESENCE: &str = "activity_presence";
/// The status of a Kp value, coded by [`StoredKpStatus`].
pub const SAMPLE_KP_STATUS: &str = "status";

/// Written in [`SAMPLE_ACTIVITY`] for a period the service published no value
/// for. Never read back: [`SAMPLE_ACTIVITY_PRESENCE`] is what says the value
/// is a gap.
pub const UNPUBLISHED_ACTIVITY_FILL: f64 = 0.0;

/// Attribute naming the archive's schema version.
pub const SCHEMA_VERSION_ATTR: &str = "schema_version";

/// Schema this build writes and can read.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

const DEFLATE_LEVEL: u8 = 6;

/// Chunking of the sample columns. One chunk holds about three weeks of Hp30
/// at 48 samples a day, or four months of Kp at 8.
pub const SAMPLE_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};

/// Chunking of the day index. One row per day, so a chunk holds several years.
pub const DAY_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};

const KP_SAMPLE_COLUMNS: [&str; 4] = [
    SAMPLE_PERIOD_START,
    SAMPLE_ACTIVITY,
    SAMPLE_ACTIVITY_PRESENCE,
    SAMPLE_KP_STATUS,
];

const HP30_SAMPLE_COLUMNS: [&str; 3] = [
    SAMPLE_PERIOD_START,
    SAMPLE_ACTIVITY,
    SAMPLE_ACTIVITY_PRESENCE,
];

/// Where one index's columns sit in the archive.
pub trait IndexArchiveLayout {
    /// Group holding everything stored for the index.
    fn archive_group(self) -> &'static str;

    /// Path of the group holding the index's sample columns.
    fn samples_group_path(self) -> String;

    /// Path of the group holding the index's day index.
    fn days_group_path(self) -> String;

    /// The index's sample columns, for checks that must cover all of them.
    fn sample_columns(self) -> &'static [&'static str];
}

impl IndexArchiveLayout for GeomagneticIndex {
    fn archive_group(self) -> &'static str {
        match self {
            Self::Kp => KP_GROUP,
            Self::Hp30 => HP30_GROUP,
        }
    }

    fn samples_group_path(self) -> String {
        format!("{}/{SAMPLES_GROUP}", self.archive_group())
    }

    fn days_group_path(self) -> String {
        format!("{}/{DAYS_GROUP}", self.archive_group())
    }

    fn sample_columns(self) -> &'static [&'static str] {
        match self {
            Self::Kp => &KP_SAMPLE_COLUMNS,
            Self::Hp30 => &HP30_SAMPLE_COLUMNS,
        }
    }
}

/// How [`KpStatus`] is written in the [`SAMPLE_KP_STATUS`] column.
///
/// Reordering [`KpStatus`]'s variants cannot change what an archived day
/// means: the codes here are fixed independently of that declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredKpStatus {
    Definitive,
    Nowcast,
}

impl StoredKpStatus {
    pub const fn code(self) -> u8 {
        match self {
            Self::Definitive => 0,
            Self::Nowcast => 1,
        }
    }

    /// The status `code` stands for, or [`None`] for a code the schema does
    /// not define.
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Definitive),
            1 => Some(Self::Nowcast),
            _ => None,
        }
    }
}

impl From<KpStatus> for StoredKpStatus {
    fn from(status: KpStatus) -> Self {
        match status {
            KpStatus::Definitive => Self::Definitive,
            KpStatus::Nowcast => Self::Nowcast,
        }
    }
}

impl From<StoredKpStatus> for KpStatus {
    fn from(status: StoredKpStatus) -> Self {
        match status {
            StoredKpStatus::Definitive => Self::Definitive,
            StoredKpStatus::Nowcast => Self::Nowcast,
        }
    }
}

/// Whether a sample's [`SAMPLE_ACTIVITY`] value is one the service published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredActivityPresence {
    /// The service published no value for the period, and the activity column
    /// holds [`UNPUBLISHED_ACTIVITY_FILL`].
    Unpublished,
    Published,
}

impl StoredActivityPresence {
    pub const fn code(self) -> u8 {
        match self {
            Self::Unpublished => 0,
            Self::Published => 1,
        }
    }

    /// What `code` stands for, or [`None`] for a code the schema does not
    /// define.
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unpublished),
            1 => Some(Self::Published),
            _ => None,
        }
    }
}

impl From<Option<GeomagneticActivity>> for StoredActivityPresence {
    fn from(activity: Option<GeomagneticActivity>) -> Self {
        match activity {
            Some(_) => Self::Published,
            None => Self::Unpublished,
        }
    }
}
