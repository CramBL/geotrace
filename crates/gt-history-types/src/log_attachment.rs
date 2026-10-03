//! What the history database stores about a log attached to a recording.
//!
//! An attachment is one attribute on the recording group, keyed
//! [`LOG_ATTACHMENT_ATTR_PREFIX`] plus a UUID and holding the JSON of
//! [`LogAttachment`]. The log itself is one file under [`LOGS_DIRECTORY`],
//! written and read by `gt_store`. The attribute is what makes an attachment
//! exist, and it goes when the recording does.

use std::{
    collections::HashSet,
    fmt, fs, io,
    num::ParseIntError,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;
use xxhash_rust::xxh3;

use crate::DbError;

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

/// What a recording's attribute says about one attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogAttachment {
    format_version: u32,

    /// Name the log was loaded under, shown wherever the attachment is listed.
    pub name: String,

    /// Hash of the log the attachment's file holds.
    pub content_hash: LogContentHash,

    /// The filter stack the log was attached with, restored with it.
    pub filters: StoredLogFilterStack,

    /// The original reference for timestamps without a year. Absent in older attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_reference: Option<DateTime<Utc>>,
}

impl LogAttachment {
    pub fn new(name: String, content_hash: LogContentHash, filters: StoredLogFilterStack) -> Self {
        Self {
            format_version: LOG_ATTACHMENT_FORMAT_VERSION,
            name,
            content_hash,
            filters,
            year_reference: None,
        }
    }

    /// The attribute value stored on the recording group.
    pub fn to_attribute_json(&self) -> Result<String, DbError> {
        serde_json::to_string(self)
            .map_err(|err| DbError::Backend(format!("could not encode a log attachment: {err}")))
    }

    /// Decode an attribute value. One this build cannot read warns and
    /// decodes to `None`, leaving the recording and its other attachments
    /// readable.
    pub fn from_attribute_json(json: &str) -> Option<Self> {
        match serde_json::from_str::<Self>(json) {
            Ok(mut attachment) if attachment.format_version <= LOG_ATTACHMENT_FORMAT_VERSION => {
                attachment.format_version = LOG_ATTACHMENT_FORMAT_VERSION;
                Some(attachment)
            }
            Ok(attachment) => {
                log::warn!(
                    "Ignoring a log attachment written in format version {} (this build reads up to {LOG_ATTACHMENT_FORMAT_VERSION})",
                    attachment.format_version
                );
                None
            }
            Err(err) => {
                log::warn!("Ignoring an undecodable log attachment: {err}");
                None
            }
        }
    }
}

/// One of a recording's attachments, as listed from its attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogAttachmentEntry {
    pub id: LogAttachmentId,
    pub attachment: LogAttachment,
}

impl LogAttachmentEntry {
    /// Puts a recording's attachments in the order every list of them shows:
    /// by name, and by id for two attachments stored under one name.
    pub fn sort_by_name_then_id(entries: &mut [Self]) {
        entries.sort_by(|left, right| {
            left.attachment
                .name
                .cmp(&right.attachment.name)
                .then(left.id.cmp(&right.id))
        });
    }
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
const LOG_ATTACHMENT_FORMAT_VERSION: u32 = 5;

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
            r#"{"format_version":5,"name":"navsyncd.log","content_hash":"b3e7a3594637c2fbf4655e82bcf507d6","filters":{"groups":[{"id":0,"operator":"all"}],"selected_group_id":0,"chips":[{"condition":{"scope":"message","text":"gnss","regex":false},"group_id":0,"effects":{"kind":"map","enabled":true,"color_slot":2}},{"condition":{"scope":"message","text":"hal-powerd|navsyncd","regex":true},"group_id":0,"effects":{"kind":"table","enabled":false}}]}}"#
        );
        assert_eq!(
            LogAttachment::from_attribute_json(&json),
            Some(attachment())
        );
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
        assert_eq!(json.get("format_version"), Some(&serde_json::json!(5)));
        assert_eq!(
            LogAttachment::from_attribute_json(&rewritten),
            Some(expected)
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
                .contains("\"format_version\":5")
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
        assert_eq!(LogAttachment::from_attribute_json(&json), None);
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

    /// Neither a newer layout nor a corrupt value may fail the recording the
    /// attribute sits on.
    #[test]
    fn an_attachment_this_build_cannot_read_decodes_to_nothing() {
        let newer = r#"{"format_version":6,"name":"navsyncd.log","content_hash":"0","filters":[]}"#;
        assert_eq!(LogAttachment::from_attribute_json(newer), None);
        assert_eq!(LogAttachment::from_attribute_json("{"), None);
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
            let json = attachment.to_attribute_json().expect("encode");
            prop_assert_eq!(LogAttachment::from_attribute_json(&json), Some(attachment));
        }

        /// The attribute is read back from a file the app does not control,
        /// so any text at all has to decode to an attachment or to nothing.
        #[test]
        fn any_attribute_value_decodes_or_is_ignored(json in ".*") {
            LogAttachment::from_attribute_json(&json);
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
