use crate::DatabaseRef;
use std::path::Path;
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: i64 = 0;
pub const SCHEMA_VERSION_ATTR: &str = "schema_version";

pub const ATTR_IDENTITY: &str = "identity";
pub const ATTR_START_US: &str = "start_us";
pub const ATTR_END_US: &str = "end_us";
pub const ATTR_NAV_POINT_COUNT: &str = "nav_point_count";
pub const ATTR_SAT_REPORT_COUNT: &str = "sat_report_count";
pub const ATTR_MARKER_COUNT: &str = "marker_count";
pub const ATTR_EVENT_MARKER_COUNT: &str = "event_marker_count";
pub const ATTR_GTD_SIZE_BYTES: &str = "gtd_size_bytes";
/// Soft-delete marker: a recording with this attribute set to `1` is hidden from
/// the normal history listing and is a candidate for "delete hidden data".
/// Stored as `u64` (`1` = hidden) so both backends can read/write it with the
/// integer attribute handling they already have.
pub const ATTR_HIDDEN: &str = "hidden";

/// Returns true for attribute keys that belong to the database's recording
/// metadata (as opposed to GTD file-format root attributes).
pub fn is_db_recording_attr(key: &str) -> bool {
    matches!(
        key,
        ATTR_IDENTITY
            | ATTR_START_US
            | ATTR_END_US
            | ATTR_NAV_POINT_COUNT
            | ATTR_SAT_REPORT_COUNT
            | ATTR_MARKER_COUNT
            | ATTR_EVENT_MARKER_COUNT
            | ATTR_GTD_SIZE_BYTES
            | ATTR_HIDDEN
    )
}

/// Metadata for a recording - used for duplicate detection and indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecordingMeta {
    /// First nav-point timestamp in microseconds since epoch UTC.
    pub start_us: i64,
    /// Last nav-point timestamp in microseconds since epoch UTC.
    pub end_us: i64,
    pub nav_point_count: u64,
    pub sat_report_count: u64,
    pub marker_count: u64,
    pub event_marker_count: u64,
    /// Size of the original GTD bytes at import time.
    pub gtd_size_bytes: u64,
}

impl RecordingMeta {
    /// Total number of data points across all data types.
    pub fn total_count(&self) -> u64 {
        self.nav_point_count + self.sat_report_count + self.marker_count + self.event_marker_count
    }

    pub fn matches(
        &self,
        start_us: i64,
        nav_point_count: u64,
        sat_report_count: u64,
        marker_count: u64,
        event_marker_count: u64,
    ) -> bool {
        self.start_us == start_us
            && self.nav_point_count == nav_point_count
            && self.sat_report_count == sat_report_count
            && self.marker_count == marker_count
            && self.event_marker_count == event_marker_count
    }

    /// Whether `other` describes the same recording as `self`.
    ///
    /// Uses the same content-identity fields as the database's duplicate
    /// detection (`matches`), so two recordings are "the same" exactly when the
    /// history database would deduplicate them - independent of which identity
    /// they were filed under.
    pub fn same_recording(&self, other: &RecordingMeta) -> bool {
        self.matches(
            other.start_us,
            other.nav_point_count,
            other.sat_report_count,
            other.marker_count,
            other.event_marker_count,
        )
    }
}

/// One entry in the History window list.
pub struct RecordingEntry {
    pub db_ref: DatabaseRef,
    pub meta: RecordingMeta,
    /// Whether this recording has been soft-deleted (see [`ATTR_HIDDEN`]).
    pub hidden: bool,
}

/// Criteria for selecting recordings to prune.
#[derive(Debug, Clone, Copy)]
pub enum PruneMode {
    /// Remove recordings whose last nav-point is older than `now - max_age`.
    ByAge { max_age_secs: u64 },
    /// Remove the oldest recordings (by start timestamp) until total
    /// `gtd_size_bytes` across all remaining recordings is ≤ `max_bytes`.
    ByTotalSize { max_bytes: u64 },
    /// Keep at most `keep` recordings per identity (by start timestamp descending).
    ByCount { keep: usize },
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Backend error: {0}")]
    Backend(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("no platform data directory available")]
    NoDataDir,
}

impl PruneMode {
    pub fn select(&self, entries: &[RecordingEntry]) -> Vec<DatabaseRef> {
        match self {
            PruneMode::ByAge { max_age_secs } => {
                let now_us = chrono::Utc::now().timestamp_micros();
                let threshold_us = now_us - (*max_age_secs as i64) * 1_000_000;
                entries
                    .iter()
                    .filter(|e| e.meta.end_us < threshold_us)
                    .map(|e| e.db_ref.clone())
                    .collect()
            }
            PruneMode::ByTotalSize { max_bytes } => {
                let mut total: u64 = entries.iter().map(|e| e.meta.gtd_size_bytes).sum();
                let mut to_delete = Vec::new();
                // entries are sorted descending by start_us; remove from the end (oldest first)
                for entry in entries.iter().rev() {
                    if total <= *max_bytes {
                        break;
                    }
                    total = total.saturating_sub(entry.meta.gtd_size_bytes);
                    to_delete.push(entry.db_ref.clone());
                }
                to_delete
            }
            PruneMode::ByCount { keep } => {
                let mut seen: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                let mut to_delete = Vec::new();
                for entry in entries {
                    let count = seen.entry(entry.db_ref.identity.as_str()).or_insert(0);
                    *count += 1;
                    if *count > *keep {
                        to_delete.push(entry.db_ref.clone());
                    }
                }
                to_delete
            }
        }
    }
}

pub trait HistoryDatabase {
    fn open_or_create(path: &Path) -> Result<Self, DbError>
    where
        Self: Sized;
    fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        bytes: &[u8],
    ) -> Result<DatabaseRef, DbError>;
    fn delete(&mut self, db_ref: &DatabaseRef) -> Result<(), DbError>;
    /// Mark or unmark the given recordings as hidden (soft-delete) in place,
    /// without removing their data. Hidden recordings are excluded from the
    /// normal listing but can later be permanently removed with [`Self::delete_batch`].
    /// Missing recordings are skipped silently.
    fn set_hidden(&mut self, refs: &[DatabaseRef], hidden: bool) -> Result<(), DbError>;
    fn load_bytes(&self, db_ref: &DatabaseRef) -> Result<Vec<u8>, DbError>;
    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError>;
    fn is_duplicate(&self, identity: &str, meta: &RecordingMeta) -> Result<bool, DbError>;
    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError>;
    fn path(&self) -> &Path;

    /// Compute which recordings would be removed by a given prune mode.
    fn prune_candidates(&self, mode: &PruneMode) -> Result<Vec<DatabaseRef>, DbError> {
        let entries = self.list_recordings()?;
        Ok(mode.select(&entries))
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

    fn meta() -> RecordingMeta {
        RecordingMeta {
            start_us: 1_000,
            end_us: 5_000,
            nav_point_count: 100,
            sat_report_count: 20,
            marker_count: 3,
            event_marker_count: 1,
            gtd_size_bytes: 4_096,
        }
    }

    #[test]
    fn same_recording_ignores_size_and_end() {
        // end_us and gtd_size_bytes are not part of the content identity, so a
        // recording re-read from history (which can differ in stored size or a
        // recomputed end) still counts as the same recording.
        let a = meta();
        let b = RecordingMeta {
            end_us: 9_999,
            gtd_size_bytes: 1,
            ..a
        };
        assert!(a.same_recording(&b));
        assert!(b.same_recording(&a));
    }

    #[test]
    fn same_recording_distinguishes_content() {
        let a = meta();
        for b in [
            RecordingMeta { start_us: 2, ..a },
            RecordingMeta {
                nav_point_count: 101,
                ..a
            },
            RecordingMeta {
                sat_report_count: 0,
                ..a
            },
            RecordingMeta {
                marker_count: 0,
                ..a
            },
            RecordingMeta {
                event_marker_count: 0,
                ..a
            },
        ] {
            assert!(!a.same_recording(&b));
        }
    }
}
