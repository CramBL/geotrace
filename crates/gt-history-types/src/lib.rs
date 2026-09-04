use std::ops::RangeInclusive;
use std::path::Path;

use thiserror::Error;

pub mod log_attachment;

pub use log_attachment::{
    LOG_ATTACHMENT_ATTR_PREFIX, LOGS_DIRECTORY, LogAttachment, LogAttachmentEntry, LogAttachmentId,
    LogContentHash, StoredLogFilter, StoredLogFilterMode, logs_directory_for_database,
};

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

/// Segmentation settings the stored tracks were produced with. `track_split_gap`
/// is stored in microseconds.
pub const ATTR_SEG_GAP_US: &str = "seg_track_split_gap_us";
pub const ATTR_SEG_DETECT_CLOCK: &str = "seg_detect_clock_discontinuities";
pub const ATTR_SEG_CLOCK_SIGMAS: &str = "seg_clock_discontinuity_sigmas";
/// The [`StoredTrackSplitRule`] the stored tracks were split by, written as the
/// rule's [`StoredTrackSplitRule::attribute_value`].
pub const ATTR_SEG_SPLIT_RULE: &str = "seg_track_split_rule";
/// The [`StoredFixPlacementRule`] the stored geometry was placed by, written as
/// the rule's [`StoredFixPlacementRule::attribute_value`].
pub const ATTR_SEG_PLACEMENT_RULE: &str = "seg_fix_placement_rule";

/// DB-internal subgroup (under each recording group) holding the stored track
/// ranges as parallel `start`/`end`/`hidden` datasets. The name is prefixed so
/// it cannot collide with a GTD data group, and is skipped when reconstructing
/// the original GTD file on load.
pub const TRACKS_GROUP: &str = "__geotrace_tracks__";
pub const TRACK_START_DATASET: &str = "start";
pub const TRACK_END_DATASET: &str = "end";
pub const TRACK_HIDDEN_DATASET: &str = "hidden";

/// DB-internal subgroup (under each recording group) holding the recording's
/// cached snap-to-road run as one opaque byte dataset. The bytes are the
/// app's own serialization (a versioned envelope). The history layer never
/// inspects them. Prefixed like [`TRACKS_GROUP`], skipped when
/// reconstructing the GTD file, and dropped automatically when the
/// recording group is deleted.
pub const SNAP_GROUP: &str = "__geotrace_snap__";
pub const SNAP_BLOB_DATASET: &str = "blob";

/// The GTD file-format root attribute carrying the format version, and the
/// value assumed for recordings stored before it was preserved.
pub const GTD_VERSION_ATTR: &str = "geotrace_version";
pub const GTD_VERSION_FALLBACK: &str = "1";

/// GTD root attributes carrying the recording's SDK metadata (title, device,
/// notes, travel mode). Written on the GTD root by `geotrace_sdk` and copied
/// verbatim onto each recording group, so the history listing can read them via
/// [`RecordingEntry`] without re-parsing the embedded GTD file. These are GTD
/// attributes, not DB bookkeeping - deliberately absent from
/// [`is_db_recording_attr`] so they are restored to the root on load.
pub const GTD_META_TITLE_ATTR: &str = "meta_title";
pub const GTD_META_DEVICE_ATTR: &str = "meta_device";
pub const GTD_META_NOTES_ATTR: &str = "meta_notes";
pub const GTD_META_TRAVEL_MODE_ATTR: &str = "meta_travel_mode";

/// GTD layout of the ad-hoc sensor channels, written by `geotrace_sdk` as
/// `channels/{name}/{time,value}` with the channel's metadata on the per-channel
/// group. The recording's GTD tree is copied verbatim into its database group,
/// so the History listing reads these back straight from storage - no GTD
/// re-parse, and recordings stored before the listing surfaced channels are
/// covered too.
pub const GTD_CHANNELS_GROUP: &str = "channels";
/// Sample timestamps of one channel: its row count is the sample count.
pub const GTD_CHANNEL_TIME_DATASET: &str = "time";
/// Per-channel unit label (`"g"`, `"deg"`), absent for a unitless channel.
pub const GTD_CHANNEL_UNIT_ATTR: &str = "unit";
/// Per-channel free-text description, absent when the producer set none.
pub const GTD_CHANNEL_DESCRIPTION_ATTR: &str = "description";
/// Component labels of a vector channel (`["x", "y", "z"]`), absent for a
/// scalar channel.
pub const GTD_CHANNEL_COMPONENTS_ATTR: &str = "components";

const IDENTITY_GROUP_PREFIX: &str = "identity-v1-";

/// Return the HDF5 child-group name used to store an identity.
///
/// Producer-supplied identities are opaque strings and may contain `/`, `.`, or
/// other characters with HDF5 path semantics, so they must never be used as raw
/// group names.
pub fn identity_group_name(identity: &str) -> String {
    let mut out = String::with_capacity(IDENTITY_GROUP_PREFIX.len() + identity.len() * 2);
    out.push_str(IDENTITY_GROUP_PREFIX);
    for byte in identity.as_bytes() {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").ok();
    }
    out
}

/// Decode an identity storage group name produced by [`identity_group_name`].
///
/// Returns `None` for legacy databases whose identity groups were stored under
/// the raw identity string.
pub fn identity_from_group_name(group_name: &str) -> Option<String> {
    let hex = group_name.strip_prefix(IDENTITY_GROUP_PREFIX)?;
    if !hex.len().is_multiple_of(2) {
        return None;
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks(2) {
        let hi = hex_nibble(*chunk.first()?)?;
        let lo = hex_nibble(*chunk.get(1)?)?;
        bytes.push((hi << 4) | lo);
    }
    String::from_utf8(bytes).ok()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        other => {
            let _ = other;
            None
        }
    }
}

/// Returns true for attribute keys that belong to the database's recording
/// metadata, and false for a GTD file-format root attribute.
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
            | ATTR_SEG_GAP_US
            | ATTR_SEG_DETECT_CLOCK
            | ATTR_SEG_CLOCK_SIGMAS
            | ATTR_SEG_SPLIT_RULE
            | ATTR_SEG_PLACEMENT_RULE
    ) || key.starts_with(LOG_ATTACHMENT_ATTR_PREFIX)
}

/// Returns true for recording child-group names that are GeoTrace history
/// bookkeeping (not part of the GTD file), so they are skipped when
/// reconstructing the original GTD file on load.
pub fn is_db_internal_group(name: &str) -> bool {
    name == TRACKS_GROUP || name == SNAP_GROUP
}

/// One stored track: a half-open index range `[start, end)` into the recording's
/// nav points, plus whether it is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackRange {
    pub start: u64,
    pub end: u64,
    pub hidden: bool,
}

/// A reference to a specific recording stored in the history database.
///
/// Identifies the HDF5 group at `by_identity/{identity}/{group_name}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRef {
    pub identity: String,
    pub group_name: String,
}

/// The rule that turned a recording's fixes into its stored track ranges.
///
/// The numbering is the on-disk one: a rule added later takes the next number,
/// and a build with no variant for a number reads [`Self::Unrecognized`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredTrackSplitRule {
    /// A forward timestamp gap reaching the split gap starts a new track.
    ForwardGapOnly,
    /// A timestamp step reaching the split gap in either direction starts a new
    /// track.
    StepInEitherDirection,
    /// A rule number this build has no variant for.
    Unrecognized(i64),
}

impl StoredTrackSplitRule {
    const FORWARD_GAP_ONLY_VALUE: i64 = 0;
    const STEP_IN_EITHER_DIRECTION_VALUE: i64 = 1;

    /// The rule an [`ATTR_SEG_SPLIT_RULE`] value names. An absent attribute is
    /// [`Self::ForwardGapOnly`]. Every build that wrote a recording without
    /// this attribute split its tracks by that rule.
    pub fn from_attribute_value(value: Option<i64>) -> Self {
        match value {
            None | Some(Self::FORWARD_GAP_ONLY_VALUE) => Self::ForwardGapOnly,
            Some(Self::STEP_IN_EITHER_DIRECTION_VALUE) => Self::StepInEitherDirection,
            Some(other) => Self::Unrecognized(other),
        }
    }

    pub fn attribute_value(self) -> i64 {
        match self {
            Self::ForwardGapOnly => Self::FORWARD_GAP_ONLY_VALUE,
            Self::StepInEitherDirection => Self::STEP_IN_EITHER_DIRECTION_VALUE,
            Self::Unrecognized(value) => value,
        }
    }
}

/// The rule that turned a recording's fixes into the positions its tracks are
/// drawn at.
///
/// The numbering is the on-disk one: a rule added later takes the next number,
/// and a build with no variant for a number reads [`Self::Unrecognized`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredFixPlacementRule {
    /// A fix with no heading was drawn between the fixes with a satellite in
    /// fix around it, on the great circle through them continued past either
    /// anchor for a fix stamped outside their time span.
    MissingHeading,
    /// A fix with no heading and no satellite in fix was drawn between the
    /// fixes with a satellite in fix around it, and at the nearer of them for
    /// a fix stamped outside their time span.
    MissingHeadingAndNothingInFix,
    /// A rule number this build has no variant for.
    Unrecognized(i64),
}

impl StoredFixPlacementRule {
    const MISSING_HEADING_VALUE: i64 = 0;
    const MISSING_HEADING_AND_NOTHING_IN_FIX_VALUE: i64 = 1;

    /// The rule an [`ATTR_SEG_PLACEMENT_RULE`] value names. An absent attribute
    /// is [`Self::MissingHeading`]. Every build that wrote a recording without
    /// this attribute placed its fixes by that rule.
    pub fn from_attribute_value(value: Option<i64>) -> Self {
        match value {
            None | Some(Self::MISSING_HEADING_VALUE) => Self::MissingHeading,
            Some(Self::MISSING_HEADING_AND_NOTHING_IN_FIX_VALUE) => {
                Self::MissingHeadingAndNothingInFix
            }
            Some(other) => Self::Unrecognized(other),
        }
    }

    pub fn attribute_value(self) -> i64 {
        match self {
            Self::MissingHeading => Self::MISSING_HEADING_VALUE,
            Self::MissingHeadingAndNothingInFix => Self::MISSING_HEADING_AND_NOTHING_IN_FIX_VALUE,
            Self::Unrecognized(value) => value,
        }
    }
}

/// History settings stored alongside a recording's track table.
///
/// These are primitive persistence fields, intentionally independent of
/// `gt-track-builder` runtime configuration types. `track_split_gap_us` and
/// `track_split_rule` are the track-layout settings that determine whether
/// stored track ranges can be reused safely, and `fix_placement_rule` is the
/// rule the positions those tracks are drawn at were placed by. The clock
/// marker fields are the generated-marker settings that this history schema
/// persisted when the tracks were written. Newer generated-marker settings are
/// supplied by the app's current processing config when the recording is
/// opened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredSegmentation {
    pub track_split_gap_us: i64,
    pub track_split_rule: StoredTrackSplitRule,
    pub fix_placement_rule: StoredFixPlacementRule,
    pub detect_clock_discontinuities: bool,
    pub clock_discontinuity_sigmas: f64,
}

/// Split track ranges into the parallel on-disk columns (`start`/`end`/`hidden`).
pub fn track_columns(tracks: &[TrackRange]) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let starts = tracks.iter().map(|t| t.start).collect();
    let ends = tracks.iter().map(|t| t.end).collect();
    let hidden = tracks.iter().map(|t| u64::from(t.hidden)).collect();
    (starts, ends, hidden)
}

/// Reconstruct track ranges from the on-disk columns, validating consistency.
///
/// Returns `None` when the columns are inconsistent (mismatched lengths or a
/// `start > end` range), so the caller can treat the table as absent and
/// recompute tracks from the original.
pub fn track_ranges_from_columns(
    starts: &[u64],
    ends: &[u64],
    hidden: &[u64],
) -> Option<Vec<TrackRange>> {
    if starts.len() != ends.len() || starts.len() != hidden.len() {
        return None;
    }
    let mut out = Vec::with_capacity(starts.len());
    for ((&start, &end), &h) in starts.iter().zip(ends).zip(hidden) {
        if start > end {
            return None;
        }
        out.push(TrackRange {
            start,
            end,
            hidden: h != 0,
        });
    }
    Some(out)
}

/// The time a recording's nav points cover, in microseconds since epoch UTC:
/// its earliest and its latest nav point time, whatever order it stores them
/// in. `gt_types::TimeRange` is the same span over a loaded recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NavPointTimeRange {
    start_us: i64,
    end_us: i64,
}

impl NavPointTimeRange {
    /// The range covering every time in `nav_point_times`, `None` for a
    /// recording with no nav point.
    pub fn covering(nav_point_times: &[i64]) -> Option<Self> {
        Some(Self {
            start_us: *nav_point_times.iter().min()?,
            end_us: *nav_point_times.iter().max()?,
        })
    }

    /// The range a recording's stored [`ATTR_START_US`] and [`ATTR_END_US`]
    /// attributes bound.
    ///
    /// `None` for a recording with no nav point (both attributes then hold 0).
    /// Also `None` when the start of `bounds` is past its end, which names no
    /// earliest and latest time.
    pub fn from_stored_attributes(
        nav_point_count: u64,
        bounds: RangeInclusive<i64>,
    ) -> Option<Self> {
        if nav_point_count == 0 || bounds.is_empty() {
            return None;
        }
        Some(Self {
            start_us: *bounds.start(),
            end_us: *bounds.end(),
        })
    }

    pub fn start_us(self) -> i64 {
        self.start_us
    }

    pub fn end_us(self) -> i64 {
        self.end_us
    }

    /// The time from the earliest to the latest nav point, never negative.
    pub fn duration_us(self) -> i64 {
        self.end_us.saturating_sub(self.start_us)
    }
}

/// Metadata for a recording - used for duplicate detection and indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecordingMeta {
    /// The time the recording's nav points cover, `None` for a recording with
    /// no nav point and for one whose stored bounds are inverted (see
    /// [`NavPointTimeRange::from_stored_attributes`]).
    pub time_range: Option<NavPointTimeRange>,
    pub nav_point_count: u64,
    pub sat_report_count: u64,
    pub marker_count: u64,
    pub event_marker_count: u64,
    /// Size of the original GTD bytes at import time.
    pub gtd_size_bytes: u64,
}

impl RecordingMeta {
    /// The recording's [`ATTR_START_US`] attribute: its earliest nav point
    /// time, and 0 for a recording with no nav point.
    pub fn stored_start_us(&self) -> i64 {
        self.time_range.map_or(0, NavPointTimeRange::start_us)
    }

    /// The recording's [`ATTR_END_US`] attribute: its latest nav point time,
    /// and 0 for a recording with no nav point.
    pub fn stored_end_us(&self) -> i64 {
        self.time_range.map_or(0, NavPointTimeRange::end_us)
    }

    pub fn matches(
        &self,
        start_us: i64,
        nav_point_count: u64,
        sat_report_count: u64,
        marker_count: u64,
        event_marker_count: u64,
    ) -> bool {
        self.stored_start_us() == start_us
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
            other.stored_start_us(),
            other.nav_point_count,
            other.sat_report_count,
            other.marker_count,
            other.event_marker_count,
        )
    }
}

/// What one of a recording's ad-hoc sensor channels holds, without its samples.
///
/// Read from the stored `channels/{name}` group (see [`GTD_CHANNELS_GROUP`]) so
/// the History listing can describe a recording's custom data without loading
/// it. The full series lives in `gt_types::Channel`. This is the listing's view
/// of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSummary {
    /// Channel name - its storage group name, and its primary key in the file.
    pub name: String,
    /// Unit label as the producer wrote it, or `None` for a unitless channel.
    pub unit: Option<String>,
    /// Producer-supplied description, or `None`.
    pub description: Option<String>,
    /// Component labels of a vector channel, empty for a scalar one.
    pub components: Vec<String>,
    /// Number of samples (rows of the channel's `time` dataset).
    pub sample_count: u64,
}

impl ChannelSummary {
    /// Both backends must return channels in the by-name order
    /// [`RecordingEntry::channels`] promises.
    pub fn sort_by_name(summaries: &mut [ChannelSummary]) {
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

/// One entry in the History window list.
pub struct RecordingEntry {
    pub db_ref: DatabaseRef,
    pub meta: RecordingMeta,
    /// Total number of stored tracks for this recording.
    pub total_tracks: usize,
    /// How many of those tracks are currently hidden.
    pub hidden_tracks: usize,
    /// Recording title, from the GTD [`GTD_META_TITLE_ATTR`] attribute.
    pub title: Option<String>,
    /// Producing device, from the GTD [`GTD_META_DEVICE_ATTR`] attribute.
    pub device: Option<String>,
    /// Free-text notes, from the GTD [`GTD_META_NOTES_ATTR`] attribute.
    pub notes: Option<String>,
    /// Declared travel mode wire value, from the GTD
    /// [`GTD_META_TRAVEL_MODE_ATTR`] attribute. Kept as the raw wire string
    /// because the DB stores attributes verbatim. Parse with
    /// `gt_types::TravelMode::from_wire` for display or matching.
    pub travel_mode: Option<String>,
    /// The recording's ad-hoc sensor channels, sorted by name. Empty for a
    /// recording that carries none.
    pub channels: Vec<ChannelSummary>,
    /// The logs stored with the recording, in the order
    /// [`LogAttachmentEntry::sort_by_name_then_id`] puts them. Empty for a
    /// recording that holds none, and for one whose attributes the listing
    /// could not read.
    pub log_attachments: Vec<LogAttachmentEntry>,
}

/// A recording read back from history: the reconstructed GTD bytes plus the
/// stored per-track ranges and the segmentation settings they were built with.
///
/// `tracks`/`segmentation` are empty/`None` for recordings stored before
/// per-track storage existed. The caller recomputes tracks from `bytes` then.
pub struct StoredRecording {
    pub bytes: Vec<u8>,
    pub tracks: Vec<TrackRange>,
    pub segmentation: Option<StoredSegmentation>,
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
    #[error(
        "track index {index} is past the end of the recording's {stored_track_count} stored tracks"
    )]
    TrackIndexOutOfRange {
        index: usize,
        stored_track_count: usize,
    },
    /// The database is marked as open for write - typically a stale flag left by
    /// an unclean shutdown. Recoverable via [`HistoryDatabase::clear_write_lock`]
    /// once the user confirms no other process is using it.
    #[error("database is marked as open for write (it may not have been closed cleanly)")]
    WriteLocked,
    /// Another process holds the database open. Unlike [`DbError::WriteLocked`]
    /// there is nothing to repair: the lock goes away when that process
    /// releases the file.
    #[error("the database is open in another process")]
    Busy,
}

impl PruneMode {
    pub fn select(&self, entries: &[RecordingEntry]) -> Vec<DatabaseRef> {
        match self {
            PruneMode::ByAge { max_age_secs } => {
                let now_us = chrono::Utc::now().timestamp_micros();
                let threshold_us = now_us - (*max_age_secs as i64) * 1_000_000;
                entries
                    .iter()
                    .filter(|e| {
                        e.meta
                            .time_range
                            .is_some_and(|range| range.end_us() < threshold_us)
                    })
                    .map(|e| e.db_ref.clone())
                    .collect()
            }
            PruneMode::ByTotalSize { max_bytes } => {
                let mut total: u64 = entries.iter().map(|e| e.meta.gtd_size_bytes).sum();
                let mut to_delete = Vec::new();
                // Remove from the end (oldest first): entries are sorted
                // descending by their stored start time.
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

/// The methods that read a history database and write nothing to it.
///
/// [`HistoryDatabase`] requires this trait, so these are callable on a
/// writable database as well.
pub trait ReadOnlyHistoryDatabase {
    fn path(&self) -> &Path;

    /// Read a recording back: reconstructed GTD bytes plus its stored tracks and
    /// segmentation settings.
    fn load(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError>;

    /// The stored snap run bytes for a recording, or `None` when it carries
    /// none (never snapped, or stored before snap persistence existed).
    fn snap_blob(&self, db_ref: &DatabaseRef) -> Result<Option<Vec<u8>>, DbError>;

    /// Every log attached to a recording, in the order
    /// [`LogAttachmentEntry::sort_by_name_then_id`] puts them.
    ///
    /// An attribute this build cannot decode is skipped with a warning. The
    /// recording's other attachments are still listed.
    fn log_attachments(&self, db_ref: &DatabaseRef) -> Result<Vec<LogAttachmentEntry>, DbError>;

    /// The recording's attachment holding this exact log, for warning about a
    /// duplicate before attaching one.
    fn log_attachment_with_content(
        &self,
        db_ref: &DatabaseRef,
        content_hash: LogContentHash,
    ) -> Result<Option<LogAttachmentEntry>, DbError> {
        Ok(self
            .log_attachments(db_ref)?
            .into_iter()
            .find(|entry| entry.attachment.content_hash == content_hash))
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError>;

    /// Whether a recording with the same content already exists (content-addressed
    /// across all identities).
    fn is_duplicate(&self, meta: &RecordingMeta) -> Result<bool, DbError>;

    /// Compute which recordings would be removed by a given prune mode.
    fn prune_candidates(&self, mode: &PruneMode) -> Result<Vec<DatabaseRef>, DbError> {
        let entries = self.list_recordings()?;
        Ok(mode.select(&entries))
    }
}

/// The methods that change a history database. Each backend's read-only type
/// implements [`ReadOnlyHistoryDatabase`] alone, so none of these can be called
/// on it.
pub trait HistoryDatabase: ReadOnlyHistoryDatabase {
    fn open_or_create(path: &Path) -> Result<Self, DbError>
    where
        Self: Sized;

    /// Forcibly clear a stale "open for write" lock left by an unclean shutdown
    /// so the database can be opened again.
    ///
    /// Only safe to call once the user has confirmed no other process has the
    /// file open. Clearing a lock that was never set must succeed and leave the
    /// database usable.
    fn clear_write_lock(path: &Path) -> Result<(), DbError>
    where
        Self: Sized;

    /// Store a recording: the original GTD `bytes`, the segmentation `settings`,
    /// and the resulting `tracks` (index ranges, none hidden initially).
    fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        bytes: &[u8],
    ) -> Result<DatabaseRef, DbError>;

    /// Replace a recording's stored tracks and segmentation settings (e.g. after
    /// recalculating from the original with new settings). Discards prior hidden
    /// marks - the supplied `tracks` carry the new hidden state.
    fn set_tracks(
        &mut self,
        db_ref: &DatabaseRef,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
    ) -> Result<(), DbError>;

    /// Mark or unmark individual tracks (by index) of a recording as hidden.
    fn set_tracks_hidden(
        &mut self,
        db_ref: &DatabaseRef,
        track_indices: &[usize],
        hidden: bool,
    ) -> Result<(), DbError>;

    /// Store the serialized snap run for a recording, replacing any prior
    /// one. The bytes are opaque to the history layer - the app owns the
    /// format (a versioned envelope), the database only keeps them with the
    /// recording so they prune with it.
    fn set_snap_blob(&mut self, db_ref: &DatabaseRef, blob: &[u8]) -> Result<(), DbError>;

    /// Write one attachment's attribute, replacing whatever was stored under
    /// its id.
    ///
    /// Fails when the recording is not in the database.
    fn write_log_attachment_attribute(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
        attachment: &LogAttachment,
    ) -> Result<(), DbError>;

    /// Remove one attachment's attribute, a no-op when the recording carries
    /// no such attachment. The compressed log is deleted alongside it by
    /// `gt_store`'s `LogAttachments::detach_log`.
    fn delete_log_attachment_attribute(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
    ) -> Result<(), DbError>;

    /// Delete whole recordings (used by pruning).
    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError>;

    /// Rename an identity, moving all its recordings under `new`.
    ///
    /// Renaming touches only the grouping label - content-addressed duplicate
    /// detection (see [`RecordingMeta::same_recording`]) is unaffected. If `new`
    /// already exists the recordings merge into it. A no-op when `old` is absent
    /// or equal to `new`. Callers must pass a non-empty `new`. Any loaded
    /// recordings under `old` keep a stale [`DatabaseRef`] until the caller
    /// refreshes them.
    fn rename_identity(&mut self, old: &str, new: &str) -> Result<(), DbError>;
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

/// Build a recording group name from the start timestamp and a caller-supplied
/// unique token.
///
/// The timestamp prefix (whole-second `%Y-%m-%dT%H:%M:%SZ`) keeps names readable
/// when the file is inspected directly, but is not unique on its own - two
/// recordings can start within the same second. `unique` (a UUID generated by the
/// backend) guarantees the name is collision-free. The backends do not rely on
/// scanning existing names. Pass a stable, already-unique string such as
/// `uuid::Uuid::new_v4().to_string()`.
pub fn make_group_name(start_us: i64, unique: &str) -> String {
    use chrono::{DateTime, Utc};
    let ts = DateTime::<Utc>::from_timestamp_micros(start_us)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    format!("{ts}_{unique}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> RecordingMeta {
        RecordingMeta {
            time_range: NavPointTimeRange::covering(&[1_000, 5_000]),
            nav_point_count: 100,
            sat_report_count: 20,
            marker_count: 3,
            event_marker_count: 1,
            gtd_size_bytes: 4_096,
        }
    }

    #[test]
    fn same_recording_ignores_size_and_end() {
        // A recording re-read from history counts as the same recording with a
        // different stored size or a recomputed end time: neither the latest
        // nav point time nor `gtd_size_bytes` is part of the content identity.
        let a = meta();
        let b = RecordingMeta {
            time_range: NavPointTimeRange::covering(&[1_000, 9_999]),
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
            RecordingMeta {
                time_range: NavPointTimeRange::covering(&[2, 5_000]),
                ..a
            },
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

    #[test]
    fn identity_group_names_round_trip_path_like_identity() {
        let identity = "/example.invalid/history/identity/with/slashes/";
        let group_name = identity_group_name(identity);
        assert!(!group_name.contains('/'));
        assert_eq!(
            identity_from_group_name(&group_name).as_deref(),
            Some(identity)
        );
    }

    #[test]
    fn legacy_identity_group_names_do_not_decode() {
        assert_eq!(identity_from_group_name("auto:recording.gtd"), None);
    }
}

#[cfg(test)]
mod track_column_properties {
    use proptest::prelude::*;

    use super::{TrackRange, track_columns, track_ranges_from_columns};

    /// Build valid (`start <= end`) ranges from arbitrary input.
    fn valid_ranges(raw: &[(u64, u64, bool)]) -> Vec<TrackRange> {
        raw.iter()
            .map(|&(a, b, hidden)| {
                let (start, end) = if a <= b { (a, b) } else { (b, a) };
                TrackRange { start, end, hidden }
            })
            .collect()
    }

    proptest! {
        /// The on-disk column split is lossless for any valid track table: the
        /// exact ranges (including hidden flags) come back.
        #[test]
        fn columns_round_trip(
            raw in proptest::collection::vec((any::<u64>(), any::<u64>(), any::<bool>()), 0..64),
        ) {
            let tracks = valid_ranges(&raw);
            let (starts, ends, hidden) = track_columns(&tracks);
            prop_assert_eq!(track_ranges_from_columns(&starts, &ends, &hidden), Some(tracks));
        }

        /// Any column with a `hidden` value other than 0/1 still decodes to a
        /// boolean (non-zero is hidden), so a stray on-disk value never corrupts
        /// the table.
        #[test]
        fn nonzero_hidden_decodes_as_true(
            rows in proptest::collection::vec((0u64..1_000, 0u64..1_000, any::<u64>()), 1..32),
        ) {
            let mut starts = Vec::with_capacity(rows.len());
            let mut ends = Vec::with_capacity(rows.len());
            let mut hidden = Vec::with_capacity(rows.len());
            for &(a, b, h) in &rows {
                let (s, e) = if a <= b { (a, b) } else { (b, a) };
                starts.push(s);
                ends.push(e);
                hidden.push(h);
            }
            let decoded = track_ranges_from_columns(&starts, &ends, &hidden)
                .expect("valid geometry decodes");
            for (range, &h) in decoded.iter().zip(&hidden) {
                prop_assert_eq!(range.hidden, h != 0);
            }
        }

        /// Mismatched column lengths are rejected, so a partially-written table is
        /// treated as absent.
        #[test]
        fn mismatched_lengths_reject(
            starts in proptest::collection::vec(any::<u64>(), 0..16),
            ends in proptest::collection::vec(any::<u64>(), 0..16),
            hidden in proptest::collection::vec(any::<u64>(), 0..16),
        ) {
            prop_assume!(!(starts.len() == ends.len() && starts.len() == hidden.len()));
            prop_assert_eq!(track_ranges_from_columns(&starts, &ends, &hidden), None);
        }

        /// A single inverted range (`start > end`) rejects the whole table.
        #[test]
        fn one_inverted_range_rejects_table(
            pairs in proptest::collection::vec((0u64..1_000, 0u64..1_000), 1..32),
            bad in any::<usize>(),
        ) {
            let n = pairs.len();
            let mut starts = Vec::with_capacity(n);
            let mut ends = Vec::with_capacity(n);
            for &(a, b) in &pairs {
                let (s, e) = if a <= b { (a, b) } else { (b, a) };
                starts.push(s);
                ends.push(e);
            }
            // Force range `i` to have start strictly greater than end.
            let i = bad % n;
            if starts[i] == 0 {
                starts[i] = 1;
                ends[i] = 0;
            } else {
                ends[i] = starts[i] - 1;
            }
            let hidden = vec![0u64; n];
            prop_assert_eq!(track_ranges_from_columns(&starts, &ends, &hidden), None);
        }
    }
}
