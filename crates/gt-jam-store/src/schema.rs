//! Layout of the interference archive.
//!
//! Two column groups, both extensible along their one dimension:
//!
//! ```text
//! /observations/{day,cell,good,bad}   one row per published cell
//! /days/{day,offset,count,fetched_at,host}   one row per ingested day
//! ```
//!
//! A day's rows are contiguous, so `/days` turns a day into one slice of
//! `/observations`. Columns are separate datasets because it compresses far
//! better: 53 KiB as shuffled columns against 161 KiB with the same rows
//! interleaved, from 891 KiB raw. A stored day costs about 81 KiB of file,
//! the rest being HDF5's per-chunk framing and metadata.

/// Group holding the per-cell columns.
pub const OBSERVATIONS_GROUP: &str = "observations";

/// Group holding the per-day index.
pub const DAYS_GROUP: &str = "days";

/// Days since the Unix epoch, per observation row.
pub const OBS_DAY: &str = "day";
/// H3 cell index, per observation row.
pub const OBS_CELL: &str = "cell";
/// Aircraft reporting good navigation integrity.
pub const OBS_GOOD: &str = "good";
/// Aircraft reporting low navigation integrity.
pub const OBS_BAD: &str = "bad";

/// The observation columns, for checks that must cover all of them.
pub const OBSERVATION_COLUMNS: [&str; 4] = [OBS_DAY, OBS_CELL, OBS_GOOD, OBS_BAD];

/// Days since the Unix epoch, per ingested day.
pub const DAY_DAY: &str = "day";
/// First row of the day in the observation columns.
pub const DAY_OFFSET: &str = "offset";
/// How many observation rows the day owns.
pub const DAY_COUNT: &str = "count";
/// When the day was fetched, Unix seconds.
pub const DAY_FETCHED_AT: &str = "fetched_at";
/// Host that served the day.
pub const DAY_HOST: &str = "host";

/// Attribute naming the archive's schema version.
pub const SCHEMA_VERSION_ATTR: &str = "schema_version";

/// Attribute naming the H3 resolution the observations are addressed at.
pub const H3_RESOLUTION_ATTR: &str = "h3_resolution";

/// Schema this build writes and can read.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Rows per chunk in the observation columns. A published day is about
/// 44 500 rows, so reading one touches three chunks. Measured across 8 192
/// to 65 536 rows the stored size varies under 1 %, so this is chosen to
/// read the least per day rather than to compress best.
pub const OBSERVATION_CHUNK_ROWS: usize = 16_384;

/// Rows per chunk in the day index. One row per day, so a chunk holds
/// several years.
pub const DAY_CHUNK_ROWS: usize = 1_024;

/// Deflate level for the column data. On the captured day, level 9 saves
/// 0.5 % over level 6 (53 439 against 53 699 bytes).
pub const DEFLATE_LEVEL: u8 = 6;
