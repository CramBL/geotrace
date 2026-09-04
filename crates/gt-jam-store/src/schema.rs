//! Layout of the interference archive.
//!
//! Two column groups, both extensible along their one dimension:
//!
//! ```text
//! /observations/{day,cell,good,bad}   one row per published cell
//! /days/{day,offset,count,fetched_at,host}   one row per ingested day
//! ```
//!
//! `/days` holds [`gt_hdf5_archive::day_index`] columns. One entry turns a day
//! into one slice of `/observations`, a day's rows being contiguous.
//! Columns are separate datasets because it compresses far better: 53 KiB as
//! shuffled columns against 161 KiB with the same rows interleaved, from
//! 891 KiB raw. A stored day costs about 81 KiB of file, the rest being
//! HDF5's per-chunk framing and metadata.

use gt_hdf5_archive::ColumnFormat;

/// Group holding the per-cell columns.
pub const OBSERVATIONS_GROUP: &str = "observations";

/// Group holding the per-day index.
pub const DAYS_GROUP: &str = "days";

/// Days since the Unix epoch, per observation row.
pub const OBS_DAY: &str = "day";
/// H3 cell index, per observation row.
pub const OBS_CELL: &str = "cell";
/// Aircraft reporting good navigation accuracy.
pub const OBS_GOOD: &str = "good";
/// Aircraft reporting low navigation accuracy.
pub const OBS_BAD: &str = "bad";

/// The observation columns, for checks that must cover all of them.
pub const OBSERVATION_COLUMNS: [&str; 4] = [OBS_DAY, OBS_CELL, OBS_GOOD, OBS_BAD];

/// Attribute holding the archive's schema version.
pub const SCHEMA_VERSION_ATTR: &str = "schema_version";

/// Attribute holding the H3 resolution the observations are addressed at.
pub const H3_RESOLUTION_ATTR: &str = "h3_resolution";

/// Schema this build writes and can read.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Deflate level for the column data. On the captured day, level 9 saves
/// 0.5 % over level 6 (53 439 against 53 699 bytes).
const DEFLATE_LEVEL: u8 = 6;

/// Chunking of the observation columns. A published day is about 44 500 rows,
/// so reading one touches three chunks. Measured across 8 192 to 65 536 rows
/// the stored size varies under 1 %, so this is chosen to read the least per
/// day, not to compress best.
pub const OBSERVATION_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 16_384,
    deflate_level: DEFLATE_LEVEL,
};

/// Chunking of the day index. One row per day, so a chunk holds several years.
pub const DAY_FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 1_024,
    deflate_level: DEFLATE_LEVEL,
};
