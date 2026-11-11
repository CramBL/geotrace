//! What the history database stores about a log attached to a recording.
//!
//! An attachment is one attribute on the recording group, keyed
//! [`LOG_ATTACHMENT_ATTR_PREFIX`] plus a UUID and holding the JSON of
//! [`LogAttachment`]. The log itself is one file under [`LOGS_DIRECTORY`],
//! written and read by `gt_store`. The attribute is what makes an attachment
//! exist, and it goes when the recording does.

use std::{
    borrow::Cow,
    collections::HashSet,
    fmt, fs, io,
    num::ParseIntError,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_json::value::RawValue;
use strum::VariantNames as _;
use thiserror::Error;
use uuid::Uuid;
use xxhash_rust::xxh3;

use crate::{DatabaseRef, DbError};

/// Where the database at `db_path` keeps its attached logs.
pub fn logs_directory_for_database(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOGS_DIRECTORY)
}

/// Identifies one attachment: its attribute on the recording, and its file
/// under the logs directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogAttachmentId(Uuid);

impl LogAttachmentId {
    pub fn new_random() -> Self {
        Self(Uuid::new_v4())
    }

    /// The id an attribute key names, or `None` for every other attribute.
    pub fn from_attr_key(key: &str) -> Option<Self> {
        let uuid = key.strip_prefix(LOG_ATTACHMENT_ATTR_PREFIX)?;
        Uuid::parse_str(uuid).ok().map(Self)
    }

    pub fn attr_key(self) -> String {
        format!("{LOG_ATTACHMENT_ATTR_PREFIX}{}", self.0)
    }

    /// Where this attachment's compressed log is stored, always directly
    /// inside `logs_directory`: a parsed UUID has no path separator.
    pub fn file_path(self, logs_directory: &Path) -> PathBuf {
        logs_directory.join(format!("{}{LOG_ATTACHMENT_FILE_SUFFIX}", self.0))
    }
}

impl fmt::Display for LogAttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A hash of a log's uncompressed bytes: what tells the recording's
/// attachments apart, and what a load checks the decompressed file against.
///
/// XXH3-128, a non-cryptographic hash. It catches a truncated or replaced
/// file, and the store it guards is local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct LogContentHash(u128);

impl LogContentHash {
    pub fn of_log_bytes(bytes: &[u8]) -> Self {
        Self(xxh3::xxh3_128(bytes))
    }
}

impl fmt::Display for LogContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl From<LogContentHash> for String {
    fn from(hash: LogContentHash) -> Self {
        hash.to_string()
    }
}

/// The stored hash was not 128 bits of hex.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("not a 128-bit hex content hash: {text:?}")]
pub struct InvalidLogContentHash {
    text: String,
    #[source]
    source: ParseIntError,
}

impl TryFrom<String> for LogContentHash {
    type Error = InvalidLogContentHash;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        match u128::from_str_radix(&text, 16) {
            Ok(hash) => Ok(Self(hash)),
            Err(source) => Err(InvalidLogContentHash { text, source }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredLogFilterEffects {
    Both {
        table_enabled: bool,
        map_enabled: bool,
        color_slot: usize,
    },
    Map {
        enabled: bool,
        color_slot: usize,
    },
    Table {
        enabled: bool,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum LegacyFilterMode {
    Layer { color_slot: usize },
    Refine,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyFilterKind {
    Layer,
    Refine,
}

impl LegacyFilterMode {
    fn effects(self, enabled: bool) -> StoredLogFilterEffects {
        match self {
            Self::Layer { color_slot } => StoredLogFilterEffects::Map {
                enabled,
                color_slot,
            },
            Self::Refine => StoredLogFilterEffects::Table { enabled },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoredLogFilter {
    pub condition: StoredLogFilterCondition,
    pub group_id: u64,
    pub effects: StoredLogFilterEffects,
}

impl<'de> Deserialize<'de> for StoredLogFilter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CurrentFields {
            condition: StoredLogFilterCondition,
            group_id: u64,
            effects: StoredLogFilterEffects,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LegacyFields {
            condition: Option<StoredLogFilterCondition>,
            text: Option<String>,
            regex: Option<bool>,
            group_id: u64,
            enabled: bool,
            mode: LegacyFilterKind,
            color_slot: Option<usize>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Fields {
            Current(CurrentFields),
            Legacy(LegacyFields),
        }
        match Fields::deserialize(deserializer)? {
            Fields::Current(fields) => Ok(Self {
                condition: fields.condition,
                group_id: fields.group_id,
                effects: fields.effects,
            }),
            Fields::Legacy(fields) => {
                let condition = match (fields.condition, fields.text, fields.regex) {
                    (Some(condition), None, None) => condition,
                    (None, Some(text), Some(regex)) => {
                        StoredLogFilterCondition::Message { text, regex }
                    }
                    _ => {
                        return Err(serde::de::Error::custom(
                            "invalid log filter condition fields",
                        ));
                    }
                };
                let effects = match (fields.mode, fields.color_slot) {
                    (LegacyFilterKind::Layer, Some(color_slot)) => StoredLogFilterEffects::Map {
                        enabled: fields.enabled,
                        color_slot,
                    },
                    (LegacyFilterKind::Refine, None) => StoredLogFilterEffects::Table {
                        enabled: fields.enabled,
                    },
                    _ => {
                        return Err(serde::de::Error::custom(
                            "invalid legacy log filter effect fields",
                        ));
                    }
                };
                Ok(Self {
                    condition,
                    group_id: fields.group_id,
                    effects,
                })
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredLogFilterCondition {
    Hostname { text: String },
    Level { value: StoredLogLevel },
    Message { text: String, regex: bool },
    Service { text: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredLogLevel {
    Debug,
    Error,
    Info,
    Warning,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredLogFilterOperator {
    #[default]
    All,
    Any,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredLogFilterGroup {
    pub id: u64,
    pub operator: StoredLogFilterOperator,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoredLogFilterStack {
    groups: Vec<StoredLogFilterGroup>,
    selected_group_id: u64,
    chips: Vec<StoredLogFilter>,
}

#[derive(Debug)]
pub struct StoredLogFilterStackParts {
    pub groups: Vec<StoredLogFilterGroup>,
    pub selected_group_id: u64,
    pub chips: Vec<StoredLogFilter>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum InvalidStoredLogFilterStack {
    #[error("duplicate log filter group identity {group_id}")]
    DuplicateGroup { group_id: u64 },
    #[error("a log filter stack requires at least one group")]
    EmptyGroups,
    #[error("log filter {chip_index} has unknown group identity {group_id}")]
    UnknownChipGroup { chip_index: usize, group_id: u64 },
    #[error("selected log filter group identity {group_id} is unknown")]
    UnknownSelectedGroup { group_id: u64 },
}

impl StoredLogFilterStack {
    /// Reassigns existing chip memberships to group 0.
    pub fn single_all_group(mut chips: Vec<StoredLogFilter>) -> Self {
        for chip in &mut chips {
            chip.group_id = 0;
        }
        Self {
            groups: vec![StoredLogFilterGroup {
                id: 0,
                operator: StoredLogFilterOperator::All,
            }],
            selected_group_id: 0,
            chips,
        }
    }

    pub fn try_from_parts(
        StoredLogFilterStackParts {
            groups,
            selected_group_id,
            chips,
        }: StoredLogFilterStackParts,
    ) -> Result<Self, InvalidStoredLogFilterStack> {
        if groups.is_empty() {
            return Err(InvalidStoredLogFilterStack::EmptyGroups);
        }
        let mut ids = HashSet::new();
        for group in &groups {
            if !ids.insert(group.id) {
                return Err(InvalidStoredLogFilterStack::DuplicateGroup { group_id: group.id });
            }
        }
        if !ids.contains(&selected_group_id) {
            return Err(InvalidStoredLogFilterStack::UnknownSelectedGroup {
                group_id: selected_group_id,
            });
        }
        for (chip_index, chip) in chips.iter().enumerate() {
            if !ids.contains(&chip.group_id) {
                return Err(InvalidStoredLogFilterStack::UnknownChipGroup {
                    chip_index,
                    group_id: chip.group_id,
                });
            }
        }
        Ok(Self {
            groups,
            selected_group_id,
            chips,
        })
    }

    pub fn groups(&self) -> &[StoredLogFilterGroup] {
        &self.groups
    }

    pub fn selected_group_id(&self) -> u64 {
        self.selected_group_id
    }

    pub fn chips(&self) -> &[StoredLogFilter] {
        &self.chips
    }
}

impl Default for StoredLogFilterStack {
    fn default() -> Self {
        Self::single_all_group(Vec::new())
    }
}

impl<'de> Deserialize<'de> for StoredLogFilterStack {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct GroupedStack {
            groups: Vec<StoredLogFilterGroup>,
            selected_group_id: u64,
            chips: Vec<StoredLogFilter>,
        }

        #[derive(Deserialize)]
        struct LegacyFilter {
            text: String,
            regex: bool,
            enabled: bool,
            #[serde(flatten)]
            mode: LegacyFilterMode,
        }

        impl LegacyFilter {
            fn migrate(self) -> StoredLogFilter {
                StoredLogFilter {
                    condition: StoredLogFilterCondition::Message {
                        text: self.text,
                        regex: self.regex,
                    },
                    group_id: 0,
                    effects: self.mode.effects(self.enabled),
                }
            }
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SingleGroupStack {
            operator: StoredLogFilterOperator,
            chips: Vec<LegacyFilter>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StoredStackSchema {
            Grouped(GroupedStack),
            Legacy(Vec<LegacyFilter>),
            SingleGroup(SingleGroupStack),
        }

        match StoredStackSchema::deserialize(deserializer)? {
            StoredStackSchema::Grouped(stack) => Self::try_from_parts(StoredLogFilterStackParts {
                groups: stack.groups,
                selected_group_id: stack.selected_group_id,
                chips: stack.chips,
            })
            .map_err(serde::de::Error::custom),
            StoredStackSchema::Legacy(chips) => Ok(Self::single_all_group(
                chips
                    .into_iter()
                    .map(LegacyFilter::migrate)
                    .collect::<Vec<_>>(),
            )),
            StoredStackSchema::SingleGroup(stack) => {
                let mut migrated = Self::single_all_group(
                    stack
                        .chips
                        .into_iter()
                        .map(LegacyFilter::migrate)
                        .collect::<Vec<_>>(),
                );
                if let Some(group) = migrated.groups.first_mut() {
                    group.operator = stack.operator;
                }
                Ok(migrated)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogAttachment {
    pub name: String,
    pub content_hash: LogContentHash,
    pub filters: StoredLogFilterStack,
    pub year_reference: Option<DateTime<Utc>>,
    pub year_resolution: StoredYearResolution,
}

impl LogAttachment {
    pub fn from_attribute_json(json: &str) -> Result<Self, LogAttachmentDecodeError> {
        let envelope = serde_json::from_str::<LogAttachmentEnvelope>(json).map_err(|source| {
            LogAttachmentDecodeError {
                name: None,
                reason: LogAttachmentDecodeErrorReason::MalformedMetadata(source),
            }
        })?;
        let name = envelope.name.as_str().map(str::to_owned);
        envelope
            .decode(json)
            .map_err(|reason| LogAttachmentDecodeError { name, reason })
    }

    pub fn new(name: String, content_hash: LogContentHash, filters: StoredLogFilterStack) -> Self {
        Self {
            name,
            content_hash,
            filters,
            year_reference: None,
            year_resolution: StoredYearResolution::Infer,
        }
    }

    pub fn to_attribute_json(&self) -> Result<String, DbError> {
        serde_json::to_string(self)
            .map_err(|err| DbError::Backend(format!("could not encode a log attachment: {err}")))
    }
}

impl<'de> Deserialize<'de> for LogAttachment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let json = Box::<RawValue>::deserialize(deserializer)?;
        Self::from_attribute_json(json.get()).map_err(serde::de::Error::custom)
    }
}

impl Serialize for LogAttachment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_struct(
            "LogAttachment",
            4 + usize::from(self.year_reference.is_some())
                + usize::from(self.year_resolution != StoredYearResolution::Infer),
        )?;
        fields.serialize_field("format_version", &LOG_ATTACHMENT_FORMAT_VERSION)?;
        fields.serialize_field("name", &self.name)?;
        fields.serialize_field("content_hash", &self.content_hash)?;
        fields.serialize_field("filters", &self.filters)?;
        if let Some(reference) = self.year_reference {
            fields.serialize_field("year_reference", &reference)?;
        }
        if self.year_resolution != StoredYearResolution::Infer {
            fields.serialize_field("year_resolution", &self.year_resolution)?;
        }
        fields.end()
    }
}

#[derive(Debug, Error)]
#[error("{reason}")]
pub struct LogAttachmentDecodeError {
    pub name: Option<String>,
    #[source]
    pub reason: LogAttachmentDecodeErrorReason,
}

#[derive(Debug, Error)]
pub enum LogAttachmentDecodeErrorReason {
    #[error("malformed log attachment metadata: {0}")]
    MalformedMetadata(#[source] serde_json::Error),
    #[error("malformed saved chronology policy: {0}")]
    MalformedYearResolution(#[source] serde_json::Error),
    #[error("unsupported log attachment format version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("unsupported saved chronology policy {kind:?}")]
    UnsupportedYearResolution { kind: String },
}

/// The variants of `gt_logfile::YearResolution`, as stored with a log attachment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, strum::VariantNames)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum StoredYearResolution {
    IgnoreSummaryBounds,
    #[default]
    Infer,
    PlaceLastEntryInYear {
        year: i32,
    },
}

impl<'de> Deserialize<'de> for StoredYearResolution {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde ignores extra fields on internally tagged unit variants.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Fields {
            IgnoreSummaryBounds {},
            Infer {},
            PlaceLastEntryInYear { year: i32 },
        }
        Ok(match Fields::deserialize(deserializer)? {
            Fields::IgnoreSummaryBounds {} => Self::IgnoreSummaryBounds,
            Fields::Infer {} => Self::Infer,
            Fields::PlaceLastEntryInYear { year } => Self::PlaceLastEntryInYear { year },
        })
    }
}

/// One of a recording's attachments, as listed from its attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogAttachmentEntry {
    pub id: LogAttachmentId,
    pub attachment: LogAttachment,
}

#[derive(Clone, Debug)]
pub struct LogAttachmentListingEntry {
    pub id: LogAttachmentId,
    pub attachment: Result<LogAttachment, Arc<LogAttachmentReadError>>,
}

impl LogAttachmentListingEntry {
    pub fn from_attribute_json(id: LogAttachmentId, json: &str) -> Self {
        Self {
            id,
            attachment: LogAttachment::from_attribute_json(json)
                .map_err(|source| Arc::new(LogAttachmentReadError::Decode(source))),
        }
    }

    pub fn sort_by_name_then_id(entries: &mut [Self]) {
        entries.sort_by(|left, right| {
            left.display_name()
                .cmp(&right.display_name())
                .then(left.id.cmp(&right.id))
        });
    }

    pub fn into_attachment(self, recording: &DatabaseRef) -> Result<LogAttachmentEntry, DbError> {
        match self.attachment {
            Ok(attachment) => Ok(LogAttachmentEntry {
                id: self.id,
                attachment,
            }),
            Err(source) => Err(DbError::UnreadableLogAttachment {
                recording: recording.clone(),
                id: self.id,
                source,
            }),
        }
    }

    pub fn display_name(&self) -> Cow<'_, str> {
        let name = match &self.attachment {
            Ok(attachment) => Some(attachment.name.as_str()),
            Err(source) => match source.as_ref() {
                LogAttachmentReadError::Decode(error) => error.name.as_deref(),
                LogAttachmentReadError::NonTextAttribute
                | LogAttachmentReadError::ReadAttribute(_) => None,
            },
        };
        name.map_or_else(
            || Cow::Owned(format!("Attachment {}", self.id)),
            Cow::Borrowed,
        )
    }
}

impl From<LogAttachmentEntry> for LogAttachmentListingEntry {
    fn from(entry: LogAttachmentEntry) -> Self {
        Self {
            id: entry.id,
            attachment: Ok(entry.attachment),
        }
    }
}

#[derive(Debug, Error)]
pub enum LogAttachmentReadError {
    #[error(transparent)]
    Decode(#[from] LogAttachmentDecodeError),
    #[error("attachment metadata is not text")]
    NonTextAttribute,
    #[error("could not read attachment metadata: {0}")]
    ReadAttribute(String),
}

/// Delete the stored logs of `ids`, carrying on past any that fail.
///
/// A file that is already gone is not a failure.
pub fn delete_files(logs_directory: &Path, ids: &[LogAttachmentId]) {
    for id in ids {
        let path = id.file_path(logs_directory);
        match fs::remove_file(&path) {
            Ok(()) => log::debug!("Deleted the attached log at {}", path.display()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => log::warn!(
                "The attached log at {} was already gone; removed its attachment anyway",
                path.display()
            ),
            Err(err) => log::error!(
                "Could not delete the attached log at {}: {err}",
                path.display()
            ),
        }
    }
}

#[derive(Deserialize)]
struct LogAttachmentEnvelope {
    format_version: u32,
    #[serde(default)]
    name: Value,
}

impl LogAttachmentEnvelope {
    fn decode(self, json: &str) -> Result<LogAttachment, LogAttachmentDecodeErrorReason> {
        if !(1..=LOG_ATTACHMENT_FORMAT_VERSION).contains(&self.format_version) {
            return Err(LogAttachmentDecodeErrorReason::UnsupportedVersion {
                version: self.format_version,
            });
        }
        #[derive(Deserialize)]
        struct Settings {
            name: String,
            content_hash: LogContentHash,
            filters: StoredLogFilterStack,
            year_reference: Option<DateTime<Utc>>,
            #[serde(default)]
            year_resolution: YearResolutionJson,
        }
        let settings: Settings = serde_json::from_str(json)
            .map_err(LogAttachmentDecodeErrorReason::MalformedMetadata)?;
        Ok(LogAttachment {
            name: settings.name,
            content_hash: settings.content_hash,
            filters: settings.filters,
            year_reference: settings.year_reference,
            year_resolution: settings.year_resolution.decode()?,
        })
    }
}

#[derive(Default)]
enum YearResolutionJson {
    #[default]
    Missing,
    Present(Box<RawValue>),
}

impl YearResolutionJson {
    fn decode(self) -> Result<StoredYearResolution, LogAttachmentDecodeErrorReason> {
        let Self::Present(json) = self else {
            return Ok(StoredYearResolution::Infer);
        };
        #[derive(Deserialize)]
        struct PolicyKind {
            kind: String,
        }
        let policy: PolicyKind = serde_json::from_str(json.get())
            .map_err(LogAttachmentDecodeErrorReason::MalformedYearResolution)?;
        if !StoredYearResolution::VARIANTS.contains(&policy.kind.as_str()) {
            return Err(LogAttachmentDecodeErrorReason::UnsupportedYearResolution {
                kind: policy.kind,
            });
        }
        serde_json::from_str(json.get())
            .map_err(LogAttachmentDecodeErrorReason::MalformedYearResolution)
    }
}

impl<'de> Deserialize<'de> for YearResolutionJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Box::<RawValue>::deserialize(deserializer).map(Self::Present)
    }
}

/// Start of the attribute key one attachment is stored under, followed by the
/// attachment's UUID. Registered in
/// [`is_db_recording_attr`](crate::is_db_recording_attr) as database
/// bookkeeping, which keeps it off the restored GTD root.
pub const LOG_ATTACHMENT_ATTR_PREFIX: &str = "log-attachment-";

/// Directory holding the attached logs, beside the database file.
pub const LOGS_DIRECTORY: &str = "logs";

const LOG_ATTACHMENT_FILE_SUFFIX: &str = ".zst";

/// Version of the attribute JSON layout, bumped only on a change older builds
/// cannot read. An attachment written in a newer version is ignored.
const LOG_ATTACHMENT_FORMAT_VERSION: u32 = 6;

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;
    use serde_json::Value;

    use super::*;

    fn attachment() -> LogAttachment {
        LogAttachment::new(
            "navsyncd.log".to_owned(),
            LogContentHash::of_log_bytes(b"2026-01-01 14:02:11 navsyncd: gnss fix acquired\n"),
            StoredLogFilterStack::single_all_group(vec![
                StoredLogFilter {
                    group_id: 0,
                    condition: StoredLogFilterCondition::Message {
                        text: "gnss".to_owned(),
                        regex: false,
                    },
                    effects: StoredLogFilterEffects::Map {
                        enabled: true,
                        color_slot: 2,
                    },
                },
                StoredLogFilter {
                    group_id: 0,
                    condition: StoredLogFilterCondition::Message {
                        text: "hal-powerd|navsyncd".to_owned(),
                        regex: true,
                    },
                    effects: StoredLogFilterEffects::Table { enabled: false },
                },
            ]),
        )
    }

    /// Every future build has to keep reading this exact form, which is why
    /// it is pinned in full.
    #[test]
    fn an_attachment_stores_its_name_hash_and_every_chip_of_its_stack() {
        let json = attachment().to_attribute_json().expect("encode");

        assert_eq!(
            json,
            r#"{"format_version":6,"name":"navsyncd.log","content_hash":"b3e7a3594637c2fbf4655e82bcf507d6","filters":{"groups":[{"id":0,"operator":"all"}],"selected_group_id":0,"chips":[{"condition":{"scope":"message","text":"gnss","regex":false},"group_id":0,"effects":{"kind":"map","enabled":true,"color_slot":2}},{"condition":{"scope":"message","text":"hal-powerd|navsyncd","regex":true},"group_id":0,"effects":{"kind":"table","enabled":false}}]}}"#
        );
        assert_eq!(
            LogAttachment::from_attribute_json(&json).expect("decode"),
            attachment()
        );
    }

    #[rstest]
    #[case::ignore_summary_bounds(
        StoredYearResolution::IgnoreSummaryBounds,
        r#""year_resolution":{"kind":"ignore_summary_bounds"}"#
    )]
    #[case::place_last_entry_in_year(
        StoredYearResolution::PlaceLastEntryInYear { year: 2024 },
        r#""year_resolution":{"kind":"place_last_entry_in_year","year":2024}"#
    )]
    fn an_attachment_stores_the_year_resolution_the_user_chose(
        #[case] year_resolution: StoredYearResolution,
        #[case] stored_field: &str,
    ) {
        let mut chosen = attachment();
        chosen.year_resolution = year_resolution;

        let json = chosen.to_attribute_json().expect("encode");

        assert!(json.contains(stored_field), "{json}");
        assert_eq!(
            LogAttachment::from_attribute_json(&json).expect("decode"),
            chosen
        );
    }

    #[rstest]
    #[case::infer(StoredYearResolution::Infer)]
    #[case::ignore_summary_bounds(StoredYearResolution::IgnoreSummaryBounds)]
    #[case::place_last_entry_in_year(StoredYearResolution::PlaceLastEntryInYear { year: 2024 })]
    fn new_writes_exceed_the_pre_policy_readers_version_gate(#[case] policy: StoredYearResolution) {
        #[derive(Deserialize)]
        struct PrePolicyAttachment {
            format_version: u32,
            name: String,
            content_hash: LogContentHash,
            filters: StoredLogFilterStack,
            year_reference: Option<DateTime<Utc>>,
        }
        let mut current = attachment();
        current.year_resolution = policy;
        let json = current.to_attribute_json().expect("encode");
        let old: PrePolicyAttachment = serde_json::from_str(&json).expect("old schema");
        assert!(old.format_version > 5);
        assert_eq!(old.name, current.name);
        assert_eq!(old.content_hash, current.content_hash);
        assert_eq!(old.filters, current.filters);
        assert_eq!(old.year_reference, current.year_reference);
    }

    #[rstest]
    #[case(1)]
    #[case(2)]
    #[case(3)]
    #[case(4)]
    #[case(5)]
    fn legacy_attachments_without_a_policy_restore_infer(#[case] version: u32) {
        let json = format!(
            r#"{{"format_version":{version},"name":"legacy.log","content_hash":"0","filters":[]}}"#
        );
        let restored = LogAttachment::from_attribute_json(&json).expect("legacy attachment");
        assert_eq!(restored.year_resolution, StoredYearResolution::Infer);
        assert_eq!(restored.year_reference, None);
    }

    #[rstest]
    #[case::ignore_summary_bounds(StoredYearResolution::IgnoreSummaryBounds)]
    #[case::place_last_entry_in_year(StoredYearResolution::PlaceLastEntryInYear { year: 2024 })]
    fn version_five_manual_policies_upgrade_without_changing_settings(
        #[case] policy: StoredYearResolution,
    ) {
        let mut expected = attachment();
        expected.year_resolution = policy;
        expected.year_reference = DateTime::from_timestamp(1_767_312_000, 123_456_789);
        let mut legacy = serde_json::to_value(&expected).expect("settings");
        legacy["format_version"] = serde_json::json!(5);
        assert_eq!(
            serde_json::from_value::<LogAttachment>(legacy.clone()).expect("Serde version five"),
            expected
        );
        let restored =
            LogAttachment::from_attribute_json(&legacy.to_string()).expect("version five");
        assert_eq!(restored, expected);
        let rewritten = restored.to_attribute_json().expect("upgrade");
        let wire: Value = serde_json::from_str(&rewritten).expect("JSON");
        assert_eq!(wire.get("format_version"), Some(&serde_json::json!(6)));
        assert_eq!(
            LogAttachment::from_attribute_json(&rewritten).expect("restore"),
            expected
        );
    }

    #[rstest]
    #[case(0)]
    #[case(7)]
    #[case(u32::MAX)]
    fn unsupported_versions_are_reported_before_decoding_settings(#[case] version: u32) {
        let json = format!(
            r#"{{"format_version":{version},"name":"future.log","year_resolution":{{"kind":"future_policy"}}}}"#
        );
        let error = LogAttachment::from_attribute_json(&json).expect_err("unsupported version");
        assert_eq!(error.name.as_deref(), Some("future.log"));
        assert!(
            matches!(error.reason, LogAttachmentDecodeErrorReason::UnsupportedVersion { version: found } if found == version)
        );
        serde_json::from_str::<LogAttachment>(&json).expect_err("Serde version gate");
    }

    #[rstest]
    #[case::null("null")]
    #[case::missing_kind("{}")]
    #[case::invalid_kind(r#"{"kind":2}"#)]
    #[case::missing_year(r#"{"kind":"place_last_entry_in_year"}"#)]
    #[case::invalid_year(r#"{"kind":"place_last_entry_in_year","year":"2024"}"#)]
    #[case::extra_field(r#"{"kind":"infer","year":2024}"#)]
    #[case::extra_ignore_field(r#"{"kind":"ignore_summary_bounds","year":2024}"#)]
    #[case::year_overflow(r#"{"kind":"place_last_entry_in_year","year":2147483648}"#)]
    #[case::array("[]")]
    #[case::duplicate_kind(r#"{"kind":"future_policy","kind":"infer"}"#)]
    #[case::duplicate_year(r#"{"kind":"place_last_entry_in_year","year":2024,"year":2025}"#)]
    fn malformed_policies_return_a_policy_error(#[case] policy: &str) {
        let json = format!(
            r#"{{"format_version":5,"name":"broken.log","content_hash":"0","filters":[],"year_resolution":{policy}}}"#
        );
        let error = LogAttachment::from_attribute_json(&json).expect_err("malformed policy");
        assert_eq!(error.name.as_deref(), Some("broken.log"));
        assert!(matches!(
            error.reason,
            LogAttachmentDecodeErrorReason::MalformedYearResolution(_)
        ));
        serde_json::from_str::<LogAttachment>(&json).expect_err("Serde policy check");
    }

    #[test]
    fn unknown_policies_return_an_unsupported_policy_error() {
        let json = r#"{"format_version":5,"name":"future.log","content_hash":"0","filters":[],"year_resolution":{"kind":"future_policy","anchor":2026}}"#;
        let error = LogAttachment::from_attribute_json(json).expect_err("unsupported policy");
        assert_eq!(error.name.as_deref(), Some("future.log"));
        assert!(
            matches!(error.reason, LogAttachmentDecodeErrorReason::UnsupportedYearResolution { kind } if kind == "future_policy")
        );
        serde_json::from_str::<LogAttachment>(json).expect_err("Serde policy check");
    }

    #[rstest]
    #[case::invalid_json("{", None)]
    #[case::invalid_version(r#"{"format_version":"six"}"#, None)]
    #[case::missing_hash(
        r#"{"format_version":6,"name":"broken.log","filters":[]}"#,
        Some("broken.log")
    )]
    fn malformed_metadata_returns_a_metadata_error(#[case] json: &str, #[case] name: Option<&str>) {
        let error = LogAttachment::from_attribute_json(json).expect_err("malformed metadata");
        assert_eq!(error.name.as_deref(), name);
        assert!(matches!(
            error.reason,
            LogAttachmentDecodeErrorReason::MalformedMetadata(_)
        ));
    }

    #[test]
    fn legacy_flat_filters_restore_as_all_and_upgrade_on_write() {
        let legacy = r#"{"format_version":1,"name":"navsyncd.log","content_hash":"b3e7a3594637c2fbf4655e82bcf507d6","filters":[{"text":"gnss","regex":false,"enabled":true,"mode":"layer","color_slot":2},{"text":"hal-powerd|navsyncd","regex":true,"enabled":false,"mode":"refine"}],"year_reference":"2026-01-02T00:00:00Z"}"#;
        let mut expected = attachment();
        expected.year_reference = DateTime::from_timestamp(1_767_312_000, 0);
        let restored = LogAttachment::from_attribute_json(legacy).expect("legacy attachment");
        assert_eq!(restored, expected);
        let rewritten = restored
            .to_attribute_json()
            .expect("encode the migrated attachment");
        let json: Value = serde_json::from_str(&rewritten).expect("encoded JSON");
        assert_eq!(json.get("format_version"), Some(&serde_json::json!(6)));
        assert_eq!(
            LogAttachment::from_attribute_json(&rewritten).expect("decode"),
            expected
        );
    }

    #[rstest]
    #[case::version_two(2, r#"{"operator":"any","chips":[{"text":"gnss","regex":false,"enabled":true,"mode":"refine"}]}"#)]
    #[case::version_three(3, r#"{"groups":[{"id":7,"operator":"any"}],"selected_group_id":7,"chips":[{"group_id":7,"text":"gnss","regex":false,"enabled":true,"mode":"refine"}]}"#)]
    fn pre_scope_stacks_restore_message_conditions(#[case] version: u32, #[case] filters: &str) {
        let json = format!(
            r#"{{"format_version":{version},"name":"log","content_hash":"0","filters":{filters}}}"#
        );
        let restored = LogAttachment::from_attribute_json(&json).unwrap();
        assert_eq!(
            restored.filters.chips().first().unwrap().condition,
            StoredLogFilterCondition::Message {
                text: "gnss".into(),
                regex: false
            }
        );
        assert_eq!(
            restored.filters.groups().first().unwrap().operator,
            StoredLogFilterOperator::Any
        );
        assert!(
            restored
                .to_attribute_json()
                .unwrap()
                .contains("\"format_version\":6")
        );
    }

    #[rstest]
    #[case::structured_regex(r#"{"scope":"service","text":"navsyncd","regex":true}"#)]
    #[case::invalid_level(r#"{"scope":"level","value":"fatal"}"#)]
    #[case::text_level(r#"{"scope":"level","text":"error"}"#)]
    fn invalid_stored_condition_combinations_are_rejected(#[case] condition: &str) {
        let json =
            format!(r#"{{"condition":{condition},"group_id":0,"enabled":true,"mode":"refine"}}"#);
        serde_json::from_str::<StoredLogFilter>(&json).expect_err("invalid condition");
    }

    #[test]
    fn an_empty_legacy_stack_restores_as_all() {
        let legacy = r#"{"format_version":1,"name":"empty.log","content_hash":"0","filters":[]}"#;
        let restored = LogAttachment::from_attribute_json(legacy).expect("legacy attachment");
        assert_eq!(restored.filters, StoredLogFilterStack::default());
    }

    #[rstest]
    #[case::empty_groups(r#"{"groups":[],"selected_group_id":0,"chips":[]}"#)]
    #[case::duplicate_groups(r#"{"groups":[{"id":0,"operator":"all"},{"id":0,"operator":"any"}],"selected_group_id":0,"chips":[]}"#)]
    #[case::unknown_selection(
        r#"{"groups":[{"id":0,"operator":"all"}],"selected_group_id":1,"chips":[]}"#
    )]
    #[case::unknown_membership(r#"{"groups":[{"id":0,"operator":"all"}],"selected_group_id":0,"chips":[{"group_id":1,"text":"a","regex":false,"enabled":true,"mode":"refine"}]}"#)]
    #[case::missing_membership(r#"{"groups":[{"id":0,"operator":"all"}],"selected_group_id":0,"chips":[{"text":"a","regex":false,"enabled":true,"mode":"refine"}]}"#)]
    fn invalid_grouped_filter_state_is_rejected(#[case] filters: &str) {
        let json = format!(
            r#"{{"format_version":3,"name":"bad.log","content_hash":"0","filters":{filters}}}"#
        );
        LogAttachment::from_attribute_json(&json).expect_err("invalid grouped filters");
    }

    #[rstest]
    #[case::empty(vec![], 0, None, InvalidStoredLogFilterStack::EmptyGroups)]
    #[case::duplicate(vec![3, 3], 3, None, InvalidStoredLogFilterStack::DuplicateGroup { group_id: 3 })]
    #[case::unknown_selection(vec![3], 9, None, InvalidStoredLogFilterStack::UnknownSelectedGroup { group_id: 9 })]
    #[case::unknown_membership(vec![3], 3, Some(9), InvalidStoredLogFilterStack::UnknownChipGroup { chip_index: 0, group_id: 9 })]
    fn invalid_stored_stack_parts_return_the_specific_construction_error(
        #[case] group_ids: Vec<u64>,
        #[case] selected_group_id: u64,
        #[case] chip_group_id: Option<u64>,
        #[case] expected: InvalidStoredLogFilterStack,
    ) {
        let groups = group_ids
            .into_iter()
            .map(|id| StoredLogFilterGroup {
                id,
                operator: StoredLogFilterOperator::All,
            })
            .collect();
        let chips = chip_group_id
            .into_iter()
            .map(|group_id| StoredLogFilter {
                condition: StoredLogFilterCondition::Message {
                    text: "gnss".to_owned(),
                    regex: false,
                },
                group_id,
                effects: StoredLogFilterEffects::Table { enabled: true },
            })
            .collect();
        assert_eq!(
            StoredLogFilterStack::try_from_parts(StoredLogFilterStackParts {
                groups,
                selected_group_id,
                chips,
            }),
            Err(expected)
        );
    }

    /// Changing the hash algorithm invalidates every stored attachment: the
    /// hash decides whether a decompressed file is still the log that was
    /// attached. This pins it.
    #[test]
    fn the_content_hash_of_a_log_is_the_same_in_every_build() {
        assert_eq!(
            LogContentHash::of_log_bytes(b"nav-devkit-mk2 boot").to_string(),
            "7b73cbe9f58aebbf0758787756abd0fd"
        );
        assert_ne!(
            LogContentHash::of_log_bytes(b"nav-devkit-mk2 boot"),
            LogContentHash::of_log_bytes(b"nav-devkit-mk2 boo")
        );
    }

    #[test]
    fn an_attachments_attribute_key_names_it_and_no_other_attribute_does() {
        let id = LogAttachmentId::new_random();
        let key = id.attr_key();

        assert_eq!(LogAttachmentId::from_attr_key(&key), Some(id));
        assert!(crate::is_db_recording_attr(&key));
        assert_eq!(LogAttachmentId::from_attr_key("meta_title"), None);
        assert_eq!(
            LogAttachmentId::from_attr_key("log-attachment-nav-devkit-mk2"),
            None,
            "an attribute whose key is not a UUID names no attachment"
        );
    }

    #[test]
    fn an_attachment_is_stored_beside_the_database_it_belongs_to() {
        let id = LogAttachmentId::new_random();
        let directory = logs_directory_for_database(Path::new("/store/geotrace.h5"));

        assert_eq!(directory, Path::new("/store/logs"));
        assert_eq!(
            id.file_path(&directory),
            Path::new("/store/logs").join(format!("{id}.zst"))
        );
    }

    fn filters() -> impl Strategy<Value = Vec<StoredLogFilter>> {
        proptest::collection::vec(
            (
                prop_oneof![
                    (".*", any::<bool>()).prop_map(|(text, regex)| {
                        StoredLogFilterCondition::Message { text, regex }
                    }),
                    ".*".prop_map(|text| StoredLogFilterCondition::Service { text }),
                    ".*".prop_map(|text| StoredLogFilterCondition::Hostname { text }),
                    prop::sample::select(vec![
                        StoredLogLevel::Debug,
                        StoredLogLevel::Info,
                        StoredLogLevel::Warning,
                        StoredLogLevel::Error
                    ])
                    .prop_map(|value| StoredLogFilterCondition::Level { value }),
                ],
                any::<bool>(),
                any::<bool>(),
                proptest::option::of(any::<usize>()),
                any::<bool>(),
            )
                .prop_map(|(condition, enabled, map_enabled, color_slot, both)| {
                    StoredLogFilter {
                        group_id: 0,
                        condition,
                        effects: match color_slot {
                            Some(color_slot) if !both => StoredLogFilterEffects::Map {
                                enabled,
                                color_slot,
                            },
                            Some(color_slot) => StoredLogFilterEffects::Both {
                                table_enabled: enabled,
                                map_enabled,
                                color_slot,
                            },
                            None => StoredLogFilterEffects::Table { enabled },
                        },
                    }
                }),
            0..8,
        )
    }

    #[rstest]
    #[case::missing_state(r#"{"kind":"table"}"#)]
    #[case::missing_slot(r#"{"kind":"map","enabled":true}"#)]
    #[case::missing_map_state(r#"{"kind":"both","table_enabled":true,"color_slot":0}"#)]
    #[case::empty(r#"{"kind":"none"}"#)]
    #[case::table_slot(r#"{"kind":"table","enabled":true,"color_slot":0}"#)]
    fn malformed_independent_effects_are_rejected(#[case] effects: &str) {
        let json = format!(
            r#"{{"condition":{{"scope":"level","value":"info"}},"group_id":0,"effects":{effects}}}"#
        );
        serde_json::from_str::<StoredLogFilter>(&json).expect_err("invalid effect fields");
    }

    #[test]
    fn mixed_legacy_and_independent_effect_fields_are_rejected() {
        let json = r#"{"condition":{"scope":"level","value":"info"},"group_id":0,"effects":{"kind":"table","enabled":true},"enabled":true,"mode":"refine"}"#;
        serde_json::from_str::<StoredLogFilter>(json).expect_err("conflicting effect encodings");
    }

    proptest! {
        /// Whatever the user named a log and wrote into its filters, the
        /// attribute it is stored as decodes back to the same attachment.
        #[test]
        fn any_attachment_round_trips_through_its_attribute(
            name in ".*",
            log in proptest::collection::vec(any::<u8>(), 0..256),
            filters in filters(),
            operators in proptest::collection::vec(any::<bool>(), 1..6),
            membership_offset in any::<usize>(),
            selected_index in any::<usize>(),
            reference_seconds in proptest::option::of(0i64..4_102_444_800),
            year_resolution in prop_oneof![
                Just(StoredYearResolution::IgnoreSummaryBounds),
                Just(StoredYearResolution::Infer),
                any::<i32>().prop_map(|year| StoredYearResolution::PlaceLastEntryInYear { year }),
            ],
        ) {
            let mut attachment = LogAttachment::new(name, LogContentHash::of_log_bytes(&log), StoredLogFilterStack::single_all_group(filters));
            let groups: Vec<_> = operators.iter().enumerate().map(|(index, any_operator)| StoredLogFilterGroup {
                id: index as u64 * 3 + 5,
                operator: if *any_operator { StoredLogFilterOperator::Any } else { StoredLogFilterOperator::All },
            }).collect();
            let selected_group_id = groups.get(selected_index % operators.len()).expect("selected group").id;
            let mut chips = attachment.filters.chips().to_vec();
            for (index, chip) in chips.iter_mut().enumerate() {
                chip.group_id = groups.get(index.wrapping_add(membership_offset) % operators.len()).expect("chip group").id;
            }
            attachment.filters = StoredLogFilterStack::try_from_parts(StoredLogFilterStackParts { groups, selected_group_id, chips }).expect("valid memberships");
            attachment.year_reference = reference_seconds.and_then(|seconds| DateTime::from_timestamp(seconds, 0));
            attachment.year_resolution = year_resolution;
            let json = attachment.to_attribute_json().expect("encode");
            prop_assert_eq!(LogAttachment::from_attribute_json(&json).expect("decode"), attachment);
        }

        #[test]
        fn any_attribute_value_decodes_or_returns_an_error(json in ".*") {
            let _decoded = LogAttachment::from_attribute_json(&json);
        }

        #[test]
        fn arbitrary_policy_fields_cannot_default_to_infer(
            kind in prop_oneof![Just("infer".to_owned()), Just("ignore_summary_bounds".to_owned()), Just("place_last_entry_in_year".to_owned()), ".*"],
            year in prop_oneof![any::<i64>(), any::<i32>().prop_map(i64::from)],
            version in 1u32..=6,
        ) {
            let mut wire = serde_json::to_value(attachment()).expect("settings");
            let policy = serde_json::json!({"kind": kind, "year": year});
            wire["format_version"] = serde_json::json!(version);
            wire["year_resolution"] = policy;
            match LogAttachment::from_attribute_json(&wire.to_string()) {
                Ok(restored) => {
                    prop_assert_eq!(restored.year_resolution, StoredYearResolution::PlaceLastEntryInYear { year: i32::try_from(year).expect("supported year") });
                    prop_assert_eq!(kind, "place_last_entry_in_year");
                }
                Err(error) => prop_assert!(matches!(error.reason, LogAttachmentDecodeErrorReason::MalformedYearResolution(_) | LogAttachmentDecodeErrorReason::UnsupportedYearResolution { .. }), "invalid policy"),
            }
        }

        /// The same for a stored content hash on its own, which is parsed
        /// from the same untrusted text.
        #[test]
        fn any_stored_content_hash_parses_or_is_rejected(text in "[0-9a-fA-FxX+-]{0,64}") {
            if let Ok(hash) = LogContentHash::try_from(text) {
                prop_assert_eq!(
                    LogContentHash::try_from(hash.to_string()),
                    Ok(hash),
                    "a hash that parsed must survive being written back out"
                );
            }
        }
    }
}
