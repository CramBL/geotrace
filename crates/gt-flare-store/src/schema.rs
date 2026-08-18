//! Layout of the solar flare archive.
//!
//! One group of event columns and one day index, all extensible along their
//! one dimension:
//!
//! ```text
//! /events/{id,begin,peak,end,end_presence,class,magnitude,
//!          source_location,source_location_presence,
//!          active_region,active_region_presence}
//! /days/{day,offset,count,fetched_at,host}
//! ```
//!
//! The day group is a [`gt_hdf5_archive::day_index`]. One entry turns a day
//! into one slice of the event columns, a day's events being contiguous.
//! Storing a day that is already archived appends its new events and repoints
//! the day's entry: the events it replaces stay in the columns with nothing
//! referring to them.
//!
//! Each column is a dataset of its own, shuffled and deflated.

use gt_flare::class::FlareClass;
use gt_hdf5_archive::ColumnFormat;

/// Group holding the per-event columns.
pub const EVENTS_GROUP: &str = "events";

/// Group holding the day index.
pub const DAYS_GROUP: &str = "days";

/// The catalog's own identifier for the event.
pub const EVENT_ID: &str = "id";
/// When the flare began, Unix seconds.
pub const EVENT_BEGIN: &str = "begin";
/// When the flare peaked, Unix seconds. The plot marks this time.
pub const EVENT_PEAK: &str = "peak";
/// When the flare ended, Unix seconds, read only where [`EVENT_END_PRESENCE`]
/// says the catalog published one.
pub const EVENT_END: &str = "end";
/// Whether the catalog published an end time, coded by
/// [`gt_hdf5_archive::StoredPresence`].
pub const EVENT_END_PRESENCE: &str = "end_presence";
/// The class letter, coded by [`StoredFlareClass`].
pub const EVENT_CLASS: &str = "class";
/// The magnitude within the class.
pub const EVENT_MAGNITUDE: &str = "magnitude";
/// Heliographic coordinates of the flaring region, read only where
/// [`EVENT_SOURCE_LOCATION_PRESENCE`] says the catalog published them.
pub const EVENT_SOURCE_LOCATION: &str = "source_location";
/// Whether the catalog published a source location, coded by
/// [`gt_hdf5_archive::StoredPresence`].
pub const EVENT_SOURCE_LOCATION_PRESENCE: &str = "source_location_presence";
/// NOAA number of the active region, read only where
/// [`EVENT_ACTIVE_REGION_PRESENCE`] says the catalog published one.
pub const EVENT_ACTIVE_REGION: &str = "active_region";
/// Whether the catalog published an active region, coded by
/// [`gt_hdf5_archive::StoredPresence`].
pub const EVENT_ACTIVE_REGION_PRESENCE: &str = "active_region_presence";

/// Written wherever the matching presence column says the catalog published
/// nothing. Never read back.
pub const ABSENT_TIME_FILL: i64 = 0;
/// Written for an absent active region, like [`ABSENT_TIME_FILL`].
pub const ABSENT_ACTIVE_REGION_FILL: u32 = 0;

/// Attribute naming the archive's schema version.
pub const SCHEMA_VERSION_ATTR: &str = "schema_version";

/// Schema this build writes and can read.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

const DEFLATE_LEVEL: u8 = 6;

/// Chunking of the event columns. One chunk holds a few months of solar
/// maximum, where the catalog lists a dozen flares a day.
pub const EVENT_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};

/// Chunking of the day index. One row per day, so a chunk holds several years.
pub const DAY_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};

/// Every event column, for checks that must cover all of them.
pub const EVENT_COLUMNS: [&str; 11] = [
    EVENT_ID,
    EVENT_BEGIN,
    EVENT_PEAK,
    EVENT_END,
    EVENT_END_PRESENCE,
    EVENT_CLASS,
    EVENT_MAGNITUDE,
    EVENT_SOURCE_LOCATION,
    EVENT_SOURCE_LOCATION_PRESENCE,
    EVENT_ACTIVE_REGION,
    EVENT_ACTIVE_REGION_PRESENCE,
];

/// How [`FlareClass`] is written in the [`EVENT_CLASS`] column.
///
/// Reordering [`FlareClass`]'s variants cannot change what an archived day
/// means: the codes here are fixed independently of that declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredFlareClass {
    A,
    B,
    C,
    M,
    X,
}

impl StoredFlareClass {
    pub const fn code(self) -> u8 {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
            Self::M => 3,
            Self::X => 4,
        }
    }

    /// The class `code` stands for, or [`None`] for a code the schema does
    /// not define.
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::A),
            1 => Some(Self::B),
            2 => Some(Self::C),
            3 => Some(Self::M),
            4 => Some(Self::X),
            _ => None,
        }
    }
}

impl From<FlareClass> for StoredFlareClass {
    fn from(class: FlareClass) -> Self {
        match class {
            FlareClass::A => Self::A,
            FlareClass::B => Self::B,
            FlareClass::C => Self::C,
            FlareClass::M => Self::M,
            FlareClass::X => Self::X,
        }
    }
}

impl From<StoredFlareClass> for FlareClass {
    fn from(class: StoredFlareClass) -> Self {
        match class {
            StoredFlareClass::A => Self::A,
            StoredFlareClass::B => Self::B,
            StoredFlareClass::C => Self::C,
            StoredFlareClass::M => Self::M,
            StoredFlareClass::X => Self::X,
        }
    }
}
