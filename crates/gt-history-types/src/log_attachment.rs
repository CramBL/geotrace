//! What the history database stores about a log attached to a recording.
//!
//! An attachment is one attribute on the recording group, keyed
//! [`LOG_ATTACHMENT_ATTR_PREFIX`] plus a UUID and holding the JSON of
//! [`LogAttachment`]. The log itself is one file under [`LOGS_DIRECTORY`],
//! written and read by `gt_store`. The attribute is what makes an attachment
//! exist, and it goes when the recording does.

use std::{
    fmt, fs, io,
    num::ParseIntError,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use xxhash_rust::xxh3;

use crate::DbError;

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
const LOG_ATTACHMENT_FORMAT_VERSION: u32 = 1;

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

/// What one chip of a stored filter stack does with the entries it matches,
/// mirroring `gt_log_view`'s chip modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum StoredLogFilterMode {
    /// An overlay drawing its matches on the map in the palette slot it held.
    Layer { color_slot: usize },

    /// A refinement of the table: it narrows the rows, with no palette slot.
    Refine,
}

/// One chip of a log's filter stack, as it is stored with an attachment: the
/// storage schema, independent of `gt_log_view`'s session model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredLogFilter {
    /// The filter as the user wrote it in the field.
    pub text: String,

    /// Whether the `.*` toggle was on while it was written.
    pub regex: bool,

    /// Whether the chip was drawing and narrowing when it was stored.
    pub enabled: bool,

    #[serde(flatten)]
    pub mode: StoredLogFilterMode,
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
    pub filters: Vec<StoredLogFilter>,
}

impl LogAttachment {
    pub fn new(name: String, content_hash: LogContentHash, filters: Vec<StoredLogFilter>) -> Self {
        Self {
            format_version: LOG_ATTACHMENT_FORMAT_VERSION,
            name,
            content_hash,
            filters,
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
            Ok(attachment) if attachment.format_version <= LOG_ATTACHMENT_FORMAT_VERSION => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::is_db_recording_attr;

    fn attachment() -> LogAttachment {
        LogAttachment::new(
            "navsyncd.log".to_owned(),
            LogContentHash::of_log_bytes(b"2026-01-01 14:02:11 navsyncd: gnss fix acquired\n"),
            vec![
                StoredLogFilter {
                    text: "gnss".to_owned(),
                    regex: false,
                    enabled: true,
                    mode: StoredLogFilterMode::Layer { color_slot: 2 },
                },
                StoredLogFilter {
                    text: "hal-powerd|navsyncd".to_owned(),
                    regex: true,
                    enabled: false,
                    mode: StoredLogFilterMode::Refine,
                },
            ],
        )
    }

    /// Every future build has to keep reading this exact form, which is why
    /// it is pinned in full.
    #[test]
    fn an_attachment_stores_its_name_hash_and_every_chip_of_its_stack() {
        let json = attachment().to_attribute_json().expect("encode");

        assert_eq!(
            json,
            r#"{"format_version":1,"name":"navsyncd.log","content_hash":"b3e7a3594637c2fbf4655e82bcf507d6","filters":[{"text":"gnss","regex":false,"enabled":true,"mode":"layer","color_slot":2},{"text":"hal-powerd|navsyncd","regex":true,"enabled":false,"mode":"refine"}]}"#
        );
        assert_eq!(
            LogAttachment::from_attribute_json(&json),
            Some(attachment())
        );
    }

    /// Neither a newer layout nor a corrupt value may fail the recording the
    /// attribute sits on.
    #[test]
    fn an_attachment_this_build_cannot_read_decodes_to_nothing() {
        let newer = r#"{"format_version":2,"name":"navsyncd.log","content_hash":"0","filters":[]}"#;
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
        assert!(is_db_recording_attr(&key));
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
}

#[cfg(test)]
mod attribute_json_properties {
    use proptest::prelude::*;

    use super::{LogAttachment, LogContentHash, StoredLogFilter, StoredLogFilterMode};

    fn filters() -> impl Strategy<Value = Vec<StoredLogFilter>> {
        proptest::collection::vec(
            (
                ".*",
                any::<bool>(),
                any::<bool>(),
                proptest::option::of(any::<usize>()),
            )
                .prop_map(|(text, regex, enabled, color_slot)| StoredLogFilter {
                    text,
                    regex,
                    enabled,
                    mode: match color_slot {
                        Some(color_slot) => StoredLogFilterMode::Layer { color_slot },
                        None => StoredLogFilterMode::Refine,
                    },
                }),
            0..8,
        )
    }

    proptest! {
        /// Whatever the user named a log and wrote into its filters, the
        /// attribute it is stored as decodes back to the same attachment.
        #[test]
        fn any_attachment_round_trips_through_its_attribute(
            name in ".*",
            log in proptest::collection::vec(any::<u8>(), 0..256),
            filters in filters(),
        ) {
            let attachment = LogAttachment::new(name, LogContentHash::of_log_bytes(&log), filters);
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
