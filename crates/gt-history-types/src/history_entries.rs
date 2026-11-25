use std::collections::HashSet;
use std::ops::Deref;

use thiserror::Error;

use crate::{DatabaseRef, RecordingEntry, RecordingMeta};

pub struct HistoryEntries {
    entries: Vec<RecordingEntry>,
    repeated_groups: HashSet<String>,
}

impl HistoryEntries {
    pub fn new(entries: Vec<RecordingEntry>) -> Self {
        let mut seen_groups = HashSet::with_capacity(entries.len());
        let mut repeated_groups = HashSet::new();
        for entry in &entries {
            let name = &entry.db_ref.group_name;
            if !seen_groups.insert(name) && !repeated_groups.contains(name) {
                repeated_groups.insert(name.clone());
            }
        }
        Self {
            entries,
            repeated_groups,
        }
    }

    pub fn into_entries(self) -> Vec<RecordingEntry> {
        self.entries
    }

    pub fn has_repeated_group(&self, group_name: &str) -> bool {
        self.repeated_groups.contains(group_name)
    }

    pub fn select_for_open(
        &self,
        entry_index: usize,
    ) -> Result<HistoryOpenSelection, HistoryOpenSelectionError> {
        let entry = self
            .entries
            .get(entry_index)
            .ok_or(HistoryOpenSelectionError::MissingEntry)?;
        if self.has_repeated_group(&entry.db_ref.group_name) {
            return Err(HistoryOpenSelectionError::AmbiguousGroup);
        }
        if !entry.db_ref.has_generated_group_id() {
            return Err(HistoryOpenSelectionError::LegacyGroup);
        }
        Ok(HistoryOpenSelection {
            recording: entry.db_ref.clone(),
            meta: entry.meta,
        })
    }
}

impl Deref for HistoryEntries {
    type Target = [RecordingEntry];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

pub struct HistoryOpenSelection {
    recording: DatabaseRef,
    meta: RecordingMeta,
}

impl HistoryOpenSelection {
    pub fn recording(&self) -> &DatabaseRef {
        &self.recording
    }

    pub fn matches_read_metadata(&self, meta: Option<RecordingMeta>) -> bool {
        meta == Some(self.meta)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum HistoryOpenSelectionError {
    #[error("History lists the same recording group more than once")]
    AmbiguousGroup,
    #[error("History group lacks a generated ID")]
    LegacyGroup,
    #[error("History listing has no entry at the selected index")]
    MissingEntry,
}
