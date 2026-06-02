use std::path::{Path, PathBuf};

use hdf5_pure::{AttrValue, FileBuilder};
use thiserror::Error;

mod copy;

pub use gt_types::DatabaseRef;

const CURRENT_SCHEMA_VERSION: i64 = 0;
const SCHEMA_VERSION_ATTR: &str = "schema_version";

/// A handle to the open history database file.
///
/// The database is backed by a single HDF5 file (`geotrace.h5`) in the
/// platform data directory.  All mutation is a read-modify-write cycle:
/// the existing file is read into memory, a new file is constructed with
/// the changes applied, and the result is written back atomically.
///
/// # HDF5 feature status (hdf5-pure 0.6)
///
/// | Feature | Status |
/// |---|---|
/// | Root / group attributes | ✅ supported |
/// | Shuffle + deflate compression | ✅ supported |
/// | Scale-offset filter | ✅ supported |
/// | Fletcher32 checksum | ✅ supported |
/// | SWMR (concurrent read + write) | ❌ not supported |
/// | Free-space management (space reclaim on delete) | ❌ not supported |
/// | zstd codec | ❌ not supported |
///
/// Workarounds: write exclusion via a lock file instead of SWMR; deleted
/// recordings leave space until the database is compacted (future work).
/// See `docs/storage-roadmap.md` for details.
pub struct Database {
    path: PathBuf,
}

/// Metadata for a recording — used for duplicate detection and indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingMeta {
    /// First nav-point timestamp in microseconds since epoch UTC.
    pub start_us: i64,
    pub nav_point_count: u64,
    pub sat_report_count: u64,
    pub marker_count: u64,
    pub event_marker_count: u64,
}

impl RecordingMeta {
    /// Extract recording metadata from raw NVD file bytes.
    ///
    /// The bytes must be a valid NVD/HDF5 file.  Returns an error if the
    /// file is corrupt or does not contain nav-point data.
    pub fn from_nvd_bytes(bytes: &[u8]) -> Result<Self, DbError> {
        let file = hdf5_pure::File::from_bytes(bytes.to_vec())?;

        let nav_grp = file.group("nav_points")?;
        let nav_shape = nav_grp.dataset("time")?.shape()?;
        let nav_point_count = nav_shape.first().copied().unwrap_or(0);

        let start_us = if nav_point_count > 0 {
            nav_grp
                .dataset("time")?
                .read_i64()?
                .first()
                .copied()
                .unwrap_or(0)
        } else {
            0
        };

        let sat_report_count = file
            .group("sat_reports")
            .ok()
            .and_then(|g| g.dataset("nav_point_idx").ok())
            .and_then(|ds| ds.shape().ok())
            .and_then(|s| s.first().copied())
            .unwrap_or(0);

        let marker_count = file
            .group("markers")
            .ok()
            .and_then(|g| g.dataset("time").ok())
            .and_then(|ds| ds.shape().ok())
            .and_then(|s| s.first().copied())
            .unwrap_or(0);

        let event_marker_count = file
            .group("event_markers")
            .ok()
            .and_then(|g| g.dataset("sys_time_us").ok())
            .and_then(|ds| ds.shape().ok())
            .and_then(|s| s.first().copied())
            .unwrap_or(0);

        Ok(RecordingMeta {
            start_us,
            nav_point_count,
            sat_report_count,
            marker_count,
            event_marker_count,
        })
    }

    /// Total number of data points across all data types.
    pub fn total_count(&self) -> u64 {
        self.nav_point_count + self.sat_report_count + self.marker_count + self.event_marker_count
    }

    fn matches_attrs(&self, attrs: &std::collections::HashMap<String, AttrValue>) -> bool {
        let start_matches =
            matches!(attrs.get("start_us"), Some(AttrValue::I64(v)) if *v == self.start_us);
        let nav_matches = matches!(attrs.get("nav_point_count"), Some(AttrValue::U64(v)) if *v == self.nav_point_count);
        let sat_matches = matches!(attrs.get("sat_report_count"), Some(AttrValue::U64(v)) if *v == self.sat_report_count);
        let mrk_matches =
            matches!(attrs.get("marker_count"), Some(AttrValue::U64(v)) if *v == self.marker_count);
        let em_matches = matches!(attrs.get("event_marker_count"), Some(AttrValue::U64(v)) if *v == self.event_marker_count);
        start_matches && nav_matches && sat_matches && mrk_matches && em_matches
    }
}

/// Errors produced by the database layer.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("HDF5 error: {0}")]
    Hdf5(#[from] hdf5_pure::error::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "database schema version {found} is newer than supported {supported}; open the app with a newer version"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("no platform data directory available")]
    NoDataDir,
}

impl Database {
    /// Open the database at `path`, creating it (and any missing parent
    /// directories) if it does not yet exist.
    ///
    /// On creation the file is initialised with:
    /// - `schema_version = 0` as a root attribute
    /// - an empty `by_identity/` group
    /// - an empty `meta/` group
    ///
    /// On open the `schema_version` attribute is read and validated; an error
    /// is returned if the file was written by a newer version of the app.
    pub fn open_or_create(path: &Path) -> Result<Self, DbError> {
        if path.exists() {
            Self::validate_existing(path)?;
        } else {
            Self::create_new(path)?;
        }
        Ok(Self {
            path: path.to_owned(),
        })
    }

    /// Returns the platform-specific default path for the database file:
    /// `<data_dir>/geotrace/geotrace.h5`.
    pub fn default_path() -> Result<PathBuf, DbError> {
        dirs::data_dir()
            .map(|d| d.join("geotrace").join("geotrace.h5"))
            .ok_or(DbError::NoDataDir)
    }

    /// The filesystem path of this database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check whether a recording with the given `identity` and `meta` already
    /// exists in the database.
    pub fn is_duplicate(&self, identity: &str, meta: &RecordingMeta) -> Result<bool, DbError> {
        let file = hdf5_pure::File::open(&self.path)?;
        let root = file.root();

        let Ok(by_id) = root.group("by_identity") else {
            return Ok(false);
        };
        let Ok(id_grp) = by_id.group(identity) else {
            return Ok(false);
        };

        for rec_name in id_grp.groups()? {
            if let Ok(rec_grp) = id_grp.group(&rec_name)
                && let Ok(attrs) = rec_grp.attrs()
                && meta.matches_attrs(&attrs)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Insert a recording into the database using a read-modify-write cycle.
    ///
    /// If the recording already exists (same `identity` and `meta`), returns the
    /// `DatabaseRef` of the existing entry without inserting a duplicate.
    ///
    /// The `nvd_bytes` must be the complete serialised NVD/HDF5 file; the data
    /// groups are re-encoded as native HDF5 datasets with deflate compression.
    pub fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        nvd_bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        copy::insert_recording(&self.path, identity, meta, nvd_bytes).map(|rec_name| DatabaseRef {
            identity: identity.to_owned(),
            group_name: rec_name,
        })
    }

    fn create_new(path: &Path) -> Result<(), DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut fb = FileBuilder::new();
        fb.set_attr(SCHEMA_VERSION_ATTR, AttrValue::I64(CURRENT_SCHEMA_VERSION));

        let by_identity = fb.create_group("by_identity");
        fb.add_group(by_identity.finish());

        let meta = fb.create_group("meta");
        fb.add_group(meta.finish());

        fb.write(path)?;

        log::info!("Created history database at {}", path.display());
        Ok(())
    }

    fn validate_existing(path: &Path) -> Result<(), DbError> {
        let file = hdf5_pure::File::open(path)?;
        let root = file.root();
        let attrs = root.attrs()?;

        let schema_version = match attrs.get(SCHEMA_VERSION_ATTR) {
            Some(AttrValue::I64(v)) => *v,
            _ => 0,
        };

        if schema_version > CURRENT_SCHEMA_VERSION {
            return Err(DbError::SchemaTooNew {
                found: schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }

        log::debug!(
            "Opened history database at {} (schema_version={schema_version})",
            path.display()
        );
        Ok(())
    }
}

/// Format a count as a human-readable short string: `230`, `1.3k`, `100k`, `1m`.
pub fn format_count_suffix(n: u64) -> String {
    if n < 1_000 {
        return format!("{n}");
    }
    if n < 1_000_000 {
        let tenths = (n * 10 + 500) / 1_000;
        return if tenths.is_multiple_of(10) {
            format!("{}k", tenths / 10)
        } else {
            format!("{}.{}k", tenths / 10, tenths % 10)
        };
    }
    let tenths = (n * 10 + 500_000) / 1_000_000;
    if tenths.is_multiple_of(10) {
        format!("{}m", tenths / 10)
    } else {
        format!("{}.{}m", tenths / 10, tenths % 10)
    }
}

/// Generate a recording group name from the start timestamp.
///
/// Returns the bare RFC 3339 timestamp when no collision exists in
/// `existing_names`.  Appends `_<total_count>` (human-readable) on collision.
pub fn make_group_name(start_us: i64, total_count: u64, existing_names: &[String]) -> String {
    use chrono::{DateTime, Utc};
    let ts = DateTime::<Utc>::from_timestamp_micros(start_us)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    if !existing_names.iter().any(|n| n == &ts) {
        return ts;
    }
    format!("{}_{}", ts, format_count_suffix(total_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_suffix_small() {
        assert_eq!(format_count_suffix(0), "0");
        assert_eq!(format_count_suffix(230), "230");
        assert_eq!(format_count_suffix(999), "999");
    }

    #[test]
    fn format_suffix_kilo() {
        assert_eq!(format_count_suffix(1_000), "1k");
        assert_eq!(format_count_suffix(1_300), "1.3k");
        assert_eq!(format_count_suffix(100_000), "100k");
    }

    #[test]
    fn format_suffix_mega() {
        assert_eq!(format_count_suffix(1_000_000), "1m");
        assert_eq!(format_count_suffix(1_300_000), "1.3m");
    }

    #[test]
    fn group_name_bare_when_no_collision() {
        let existing: Vec<String> = Vec::new();
        let name = make_group_name(0, 100, &existing);
        assert_eq!(name, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn group_name_suffix_on_collision() {
        let existing = vec!["1970-01-01T00:00:00Z".to_owned()];
        let name = make_group_name(0, 1300, &existing);
        assert_eq!(name, "1970-01-01T00:00:00Z_1.3k");
    }
}
