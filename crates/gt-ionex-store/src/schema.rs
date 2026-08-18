//! Layout of the TEC map archive.
//!
//! Three groups, all extensible along their one dimension:
//!
//! ```text
//! /tec/days/{day,offset,count,fetched_at,host}
//! /tec/days/{product,interval_seconds,shell_height_km,
//!            latitude_first_degrees,latitude_last_degrees,latitude_step_degrees,
//!            longitude_first_degrees,longitude_last_degrees,longitude_step_degrees}
//! /tec/maps/{epoch,value_offset,value_count}
//! /tec/values/{tecu,tecu_presence}
//! ```
//!
//! The day group holds a [`gt_hdf5_archive::day_index`] whose entry turns a
//! day into one slice of the map columns, plus the day's own columns beside
//! it: one row each, at the same row the day index holds the day at. Each map
//! row in turn names one slice of the value columns, which hold the grid's
//! nodes band by band, northernmost first.
//!
//! Storing a day that is already archived appends its new rows and repoints
//! the day's entry: the rows it replaces stay in the columns with nothing
//! referring to them.
//!
//! Each column is a dataset of its own, shuffled and deflated.

use gt_hdf5_archive::ColumnFormat;
use gt_ionex::IonexProduct;

/// Group holding everything stored for TEC maps.
pub const TEC_GROUP: &str = "tec";

/// Subgroup holding the day index and the per-day columns beside it.
pub const DAYS_GROUP: &str = "tec/days";

/// Subgroup holding the per-map columns.
pub const MAPS_GROUP: &str = "tec/maps";

/// Subgroup holding the per-node columns.
pub const VALUES_GROUP: &str = "tec/values";

/// Which product the day was archived from, coded by [`StoredProduct`].
pub const DAY_PRODUCT: &str = "product";
/// Seconds between maps, from the file's `INTERVAL` record.
pub const DAY_INTERVAL_SECONDS: &str = "interval_seconds";
/// Height of the shell the maps model the ionosphere as.
pub const DAY_SHELL_HEIGHT_KM: &str = "shell_height_km";
/// Northernmost latitude of the grid.
pub const DAY_LATITUDE_FIRST_DEGREES: &str = "latitude_first_degrees";
/// Southernmost latitude of the grid.
pub const DAY_LATITUDE_LAST_DEGREES: &str = "latitude_last_degrees";
/// Latitude step, negative on the descending axis published products declare.
pub const DAY_LATITUDE_STEP_DEGREES: &str = "latitude_step_degrees";
/// First longitude of the grid.
pub const DAY_LONGITUDE_FIRST_DEGREES: &str = "longitude_first_degrees";
/// Last longitude of the grid.
pub const DAY_LONGITUDE_LAST_DEGREES: &str = "longitude_last_degrees";
/// Longitude step.
pub const DAY_LONGITUDE_STEP_DEGREES: &str = "longitude_step_degrees";

/// Epoch of one map, Unix seconds.
pub const MAP_EPOCH: &str = "epoch";
/// First row of the map in the value columns.
pub const MAP_VALUE_OFFSET: &str = "value_offset";
/// How many value rows the map holds, the grid's node count.
pub const MAP_VALUE_COUNT: &str = "value_count";

/// The node's value in TEC units, the file's exponent already applied. Read
/// only where [`VALUE_PRESENCE`] says the producer published one.
pub const VALUE_TECU: &str = "tecu";
/// Whether the producer published a value for the node, coded by
/// [`gt_hdf5_archive::StoredPresence`].
pub const VALUE_PRESENCE: &str = "tecu_presence";

/// Written in [`VALUE_TECU`] for a node the producer published no value for.
/// Never read back: [`VALUE_PRESENCE`] is what says the node is a gap.
pub const UNPUBLISHED_TECU_FILL: f64 = 0.0;

/// Attribute naming the archive's schema version.
pub const SCHEMA_VERSION_ATTR: &str = "schema_version";

/// Schema this build writes and can read.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

const DEFLATE_LEVEL: u8 = 6;

/// Chunking of the value columns. A final day is 13 maps of 71 by 73 nodes
/// and a rapid day 25 of 89 by 181, so one chunk holds about three final maps
/// or one rapid map.
pub const VALUE_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 16_384,
    deflate_level: DEFLATE_LEVEL,
};

/// Chunking of the map columns. 13 rows for a final day and 25 for a rapid
/// one, so a chunk holds about 11 weeks of final days or 6 weeks of rapid
/// ones.
pub const MAP_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};

/// Chunking of the day columns. One row per day, so a chunk holds several
/// years.
pub const DAY_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};

/// The per-day columns beside the day index, which must all stay as long as it
/// is.
pub const DAY_COLUMNS: [&str; 9] = [
    DAY_PRODUCT,
    DAY_INTERVAL_SECONDS,
    DAY_SHELL_HEIGHT_KM,
    DAY_LATITUDE_FIRST_DEGREES,
    DAY_LATITUDE_LAST_DEGREES,
    DAY_LATITUDE_STEP_DEGREES,
    DAY_LONGITUDE_FIRST_DEGREES,
    DAY_LONGITUDE_LAST_DEGREES,
    DAY_LONGITUDE_STEP_DEGREES,
];

/// The per-map columns, for checks that must cover all of them.
pub const MAP_COLUMNS: [&str; 3] = [MAP_EPOCH, MAP_VALUE_OFFSET, MAP_VALUE_COUNT];

/// The per-node columns, for checks that must cover all of them.
pub const VALUE_COLUMNS: [&str; 2] = [VALUE_TECU, VALUE_PRESENCE];

/// How [`IonexProduct`] is written in the [`DAY_PRODUCT`] column.
///
/// Reordering [`IonexProduct`]'s variants cannot change what an archived day
/// means: the codes here are fixed independently of that declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredProduct {
    Final,
    Rapid,
}

impl StoredProduct {
    pub const fn code(self) -> u8 {
        match self {
            Self::Final => 0,
            Self::Rapid => 1,
        }
    }

    /// The product `code` stands for, or [`None`] for a code the schema does
    /// not define.
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Final),
            1 => Some(Self::Rapid),
            _ => None,
        }
    }
}

impl From<IonexProduct> for StoredProduct {
    fn from(product: IonexProduct) -> Self {
        match product {
            IonexProduct::Final => Self::Final,
            IonexProduct::Rapid => Self::Rapid,
        }
    }
}

impl From<StoredProduct> for IonexProduct {
    fn from(product: StoredProduct) -> Self {
        match product {
            StoredProduct::Final => Self::Final,
            StoredProduct::Rapid => Self::Rapid,
        }
    }
}
