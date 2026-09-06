//! The UI state the history database keeps for one recording.
//!
//! A recording group may hold one [`UI_STATE_GROUP`] subgroup: a
//! [`UI_STATE_VERSION_ATTR`] attribute and one dataset per kind of UI state.
//! A later build adds a kind of UI state by adding a dataset: this build reads
//! the datasets it defines and, at its own version, keeps the rest. This build
//! leaves a group at a higher version as it stands, and the version rises when
//! a dataset already defined changes its meaning or its encoding.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use parking_lot::Mutex;

use crate::DatabaseRef;

/// DB-internal subgroup (under each recording group) holding that recording's
/// UI state. Prefixed like [`TRACKS_GROUP`](crate::TRACKS_GROUP), skipped when
/// reconstructing the GTD file, and deleted with the recording group.
pub const UI_STATE_GROUP: &str = "__geotrace_ui_state__";

/// The [`UI_STATE_GROUP`] attribute holding the version its layout was written
/// at.
pub const UI_STATE_VERSION_ATTR: &str = "ui_state_version";

/// The UI state layout this build writes.
///
/// This number and [`CURRENT_SCHEMA_VERSION`](crate::CURRENT_SCHEMA_VERSION)
/// move independently: UI state changes shape without a database schema
/// change, and a schema change leaves this at the value it had.
pub const CURRENT_UI_STATE_VERSION: i64 = 1;

/// The [`UI_STATE_GROUP`] dataset holding
/// [`RecordingUiState::hidden_track_numbers`] as `u64`.
pub const HIDDEN_TRACKS_DATASET: &str = "hidden_tracks";

/// The UI state stored with one recording.
///
/// Each field is one kind of UI state, and one dataset of
/// [`UI_STATE_GROUP`]. A write stores every field, so a caller reads the
/// recording's state, sets the fields it changes, and writes the result back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordingUiState {
    hidden_track_numbers: BTreeSet<u64>,
}

impl RecordingUiState {
    pub fn with_hidden_track_numbers(track_numbers: impl IntoIterator<Item = u64>) -> Self {
        Self {
            hidden_track_numbers: track_numbers.into_iter().collect(),
        }
    }

    /// The numbers of the tracks the user hid in this recording, ascending.
    pub fn hidden_track_numbers(&self) -> &BTreeSet<u64> {
        &self.hidden_track_numbers
    }
}

/// A recording's [`UI_STATE_VERSION_ATTR`] value against
/// [`CURRENT_UI_STATE_VERSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredUiStateVersion {
    /// [`CURRENT_UI_STATE_VERSION`]. A write replaces the datasets this build
    /// defines and keeps the rest of the group's datasets, which hold the
    /// kinds of UI state a later build added.
    Current,

    /// A version below [`CURRENT_UI_STATE_VERSION`]. A write replaces the
    /// whole group, which removes the datasets of the older layout.
    Older(i64),

    /// A version above [`CURRENT_UI_STATE_VERSION`]. This build leaves the
    /// group as it stands and reports the version through
    /// [`UiStateVersionReporter`].
    Newer(i64),
}

impl StoredUiStateVersion {
    /// A group with no [`UI_STATE_VERSION_ATTR`] counts as version 0.
    pub fn from_attribute_value(value: Option<i64>) -> Self {
        let version = value.unwrap_or(0);
        match version.cmp(&CURRENT_UI_STATE_VERSION) {
            Ordering::Equal => Self::Current,
            Ordering::Less => Self::Older(version),
            Ordering::Greater => Self::Newer(version),
        }
    }
}

/// A recording whose UI state a newer build wrote.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UiStateVersionTooNew {
    pub db_ref: DatabaseRef,
    pub found: i64,
}

/// Collects the recordings whose UI state this build left as it stands
/// because their version is above [`CURRENT_UI_STATE_VERSION`].
///
/// A caller passes one reporter through a run over several recordings and
/// reads the run's findings from it once at the end.
#[derive(Debug, Default)]
pub struct UiStateVersionReporter {
    findings: Mutex<Vec<UiStateVersionTooNew>>,
}

impl UiStateVersionReporter {
    pub fn report_too_new(&self, db_ref: &DatabaseRef, found: i64) {
        self.findings.lock().push(UiStateVersionTooNew {
            db_ref: db_ref.clone(),
            found,
        });
    }

    /// The recordings reported so far, sorted, each listed once.
    pub fn versions_too_new(&self) -> Vec<UiStateVersionTooNew> {
        let mut findings = self.findings.lock().clone();
        findings.sort();
        findings.dedup();
        findings
    }

    pub fn is_empty(&self) -> bool {
        self.findings.lock().is_empty()
    }
}
