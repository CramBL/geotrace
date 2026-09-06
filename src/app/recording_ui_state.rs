//! The UI state the history database keeps with a recording: the tracks of it
//! the user hid.
//!
//! The tree holds a recording's hidden tracks before its tracks reach the
//! view: opening a recording from history reads its UI state in the same
//! request. A recording the user opens from a file on disk is read in a
//! request of its own, which runs once its load completes. The write goes back
//! per recording whose hidden tracks the user changed.

use std::collections::BTreeSet;
use std::fs;
use std::mem;
use std::path::Path;

use gt_store::{DatabaseRef, DbError, RecordingUiState};

use super::App;

/// What the user is told once a session where a newer version of GeoTrace
/// wrote UI state this version does not read.
pub(in crate::app) const NEWER_VERSION_STORED_DISPLAY_SETTINGS: &str = "A newer version of GeoTrace saved display settings with some recordings. Update GeoTrace to \
     use them.";

/// One `ui.hidden_tracks` entry of the settings file. An earlier version of
/// GeoTrace kept the hidden tracks there, before the history database held
/// them.
#[derive(Debug, serde::Deserialize)]
pub(in crate::app) struct HiddenTracksInTheSettingsFile {
    #[serde(flatten)]
    db_ref: DatabaseRef,
    track_numbers: Vec<u64>,
}

/// The settings file read for its `ui.hidden_tracks` key alone. Every other
/// key of the file, and a missing `ui` table, parse to the defaults here.
#[derive(Default, serde::Deserialize)]
struct SettingsFileHiddenTracks {
    #[serde(default)]
    ui: UiSectionHiddenTracks,
}

#[derive(Default, serde::Deserialize)]
struct UiSectionHiddenTracks {
    #[serde(default)]
    hidden_tracks: Vec<HiddenTracksInTheSettingsFile>,
}

/// The `ui.hidden_tracks` entries of the settings file at `path`, and none for
/// a file this build cannot read or one without the key.
pub(in crate::app) fn hidden_tracks_in_the_settings_file_at(
    path: &Path,
) -> Vec<HiddenTracksInTheSettingsFile> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    match toml::from_str::<SettingsFileHiddenTracks>(&text) {
        Ok(settings) => settings.ui.hidden_tracks,
        Err(error) => {
            log::debug!("Could not read `ui.hidden_tracks` from the settings file: {error}");
            Vec::new()
        }
    }
}

impl App {
    /// Apply the UI state the history database holds for `db_ref` to the tree.
    pub(in crate::app) fn apply_the_ui_state_stored_with_a_recording(
        &self,
        db_ref: &DatabaseRef,
        ui_state: Result<RecordingUiState, DbError>,
    ) {
        let ui_state = match ui_state {
            Ok(ui_state) => ui_state,
            Err(error) => {
                log::warn!("Reading the display settings of a recording failed: {error}");
                return;
            }
        };
        let hidden_track_numbers: BTreeSet<usize> = ui_state
            .hidden_track_numbers()
            .iter()
            .filter_map(|number| usize::try_from(*number).ok())
            .collect();
        self.shared
            .borrow_mut()
            .tree
            .set_hidden_tracks_of_recording(db_ref.clone(), hidden_track_numbers);
    }

    /// Read the UI state of a recording whose tracks reached the view without
    /// it, which is one the user loaded from a file on disk.
    pub(in crate::app) fn read_the_ui_state_stored_with_a_loaded_recording(
        &self,
        db_ref: &DatabaseRef,
    ) {
        if self
            .shared
            .borrow()
            .tree
            .hidden_track_numbers(db_ref)
            .is_some()
        {
            return;
        }
        self.history.load_recording_ui_state(db_ref.clone());
    }

    /// Store the hidden tracks of every recording the user changed them for
    /// since this last ran.
    pub(in crate::app) fn store_the_hidden_tracks_the_user_changed(&self) {
        let changed = self.shared.borrow_mut().tree.take_hidden_tracks_to_store();
        for (db_ref, track_numbers) in changed {
            self.history.store_recording_ui_state(
                db_ref,
                RecordingUiState::with_hidden_track_numbers(
                    track_numbers
                        .into_iter()
                        .filter_map(|number| u64::try_from(number).ok()),
                ),
            );
        }
    }

    /// Store the hidden tracks the settings file lists with the recordings
    /// they belong to, once a database is open to write them to.
    ///
    /// The key goes with the next settings write: the settings file this
    /// version writes lists none. A read-only session writes neither file, and
    /// leaves both to the next session that has write access.
    pub(in crate::app) fn store_the_hidden_tracks_the_settings_file_lists(&mut self) {
        if self.hidden_tracks_in_the_settings_file.is_empty() {
            return;
        }
        if !self.history.available() {
            log::warn!(
                "The settings file lists the hidden tracks of {} recordings, and this session has \
                 no recording history to store them with",
                self.hidden_tracks_in_the_settings_file.len()
            );
            return;
        }
        for entry in mem::take(&mut self.hidden_tracks_in_the_settings_file) {
            self.history.store_recording_ui_state(
                entry.db_ref,
                RecordingUiState::with_hidden_track_numbers(entry.track_numbers),
            );
        }
    }

    /// Tell the user, once a session, that a newer version of GeoTrace wrote
    /// UI state this version does not read.
    pub(in crate::app) fn warn_once_that_a_newer_version_stored_ui_state(&mut self) {
        if self.warned_that_a_newer_version_stored_ui_state {
            return;
        }
        let too_new = self.history.ui_state_versions().versions_too_new();
        if too_new.is_empty() {
            return;
        }
        self.warned_that_a_newer_version_stored_ui_state = true;
        let recordings: Vec<String> = too_new
            .iter()
            .map(|found| {
                format!(
                    "{}/{} at UI state version {}",
                    found.db_ref.identity, found.db_ref.group_name, found.found
                )
            })
            .collect();
        log::warn!(
            "A newer version of GeoTrace wrote the UI state of these recordings, which this \
             version leaves as it stands: {}",
            recordings.join(", ")
        );
        self.toasts.warning(NEWER_VERSION_STORED_DISPLAY_SETTINGS);
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn the_settings_file_lists_the_hidden_tracks_of_a_recording() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[ui]\n\
             visible_section_fraction = 0.25\n\n\
             [[ui.hidden_tracks]]\n\
             identity = \"dev\"\n\
             group_name = \"2026-01-01T00:00:00Z_ride\"\n\
             track_numbers = [2, 4]\n",
        )
        .expect("write the settings file");

        let listed = hidden_tracks_in_the_settings_file_at(&path);

        let [entry] = listed.as_slice() else {
            panic!("expected one entry, got {}", listed.len());
        };
        assert_eq!(entry.db_ref.identity, "dev");
        assert_eq!(entry.db_ref.group_name, "2026-01-01T00:00:00Z_ride");
        assert_eq!(entry.track_numbers, [2, 4]);
    }

    #[rstest]
    #[case::no_settings_file(None)]
    #[case::no_hidden_tracks_key(Some("[ui]\ntheme = \"system\"\n"))]
    #[case::an_entry_without_a_recording(Some("[[ui.hidden_tracks]]\ntrack_numbers = [2]\n"))]
    #[case::a_track_number_that_is_not_a_number(Some(
        "[[ui.hidden_tracks]]\nidentity = \"dev\"\ngroup_name = \"ride\"\ntrack_numbers = \"two\"\n"
    ))]
    fn a_settings_file_this_version_reads_no_hidden_tracks_from_lists_none(
        #[case] contents: Option<&str>,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        if let Some(contents) = contents {
            fs::write(&path, contents).expect("write the settings file");
        }

        assert!(hidden_tracks_in_the_settings_file_at(&path).is_empty());
    }
}
