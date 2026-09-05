//! What happens to a `.gtd` that arrives from outside the recording history:
//! dropped on the window, chosen in the file dialog, or named on the command
//! line.
//!
//! Every one of them is looked up in the history database on the history
//! worker's thread before it loads. The ones history already holds raise one
//! prompt for the whole batch, which offers the stored version or the file on
//! disk.

use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Label, RichText};
use gt_store::{
    DatabaseRef, ReadOnlyHistoryDatabase as _, ReadOnlyRecordings, RecordingEntry, RecordingMeta,
    TrackRange, TrackState,
};

use super::anchored_dialog::AnchoredDialogKind;
use super::storage::QueuedLoad;
use super::{App, loader, modals};

/// The 8-byte magic every HDF5 file begins with. A drop whose bytes start with
/// it is a recording, and every other drop is a log.
const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";

/// The name a drop that carried no file name is loaded under.
const UNNAMED_DROP_FILENAME: &str = "dropped.gtd";

/// Recordings the prompt lists one by one. The rest are counted.
const MOST_LISTED_RECORDINGS: usize = 10;

pub(in crate::app) const LEAVE_SHELVED_TRACKS_OUT_LABEL: &str = "Leave the shelved tracks out";

pub(in crate::app) const OPEN_THE_STORED_VERSION_LABEL: &str = "Open the stored version";

pub(in crate::app) const LOAD_FROM_DISK_LABEL: &str = "Load from disk";

pub(in crate::app) const NO_SHELVED_TRACK_HOVER: &str =
    "None of these recordings has a shelved track";

/// The title over the `count` recordings of one batch that history holds.
pub(in crate::app) fn recordings_already_in_history_title(count: usize) -> String {
    let recordings = gt_fmt::pluralize(count, "recording", "recordings");
    format!("{count} {recordings} already in history")
}

/// One `.gtd` on its way into the view from outside the history database.
pub struct RecordingFromDisk {
    pub filename: String,
    pub content: RecordingContent,
}

/// Where a [`RecordingFromDisk`]'s bytes come from. The file dialog, the
/// command line and a native drop each name a path. A web drop hands over the
/// bytes themselves.
pub enum RecordingContent {
    Path(PathBuf),
    Bytes(Arc<[u8]>),
}

impl RecordingFromDisk {
    /// The metadata the history lookup compares, read from the file on disk or
    /// from the bytes the drop carried.
    ///
    /// `None` where reading the bytes failed, and where they hold something
    /// other than a recording. The load job that follows reports the error.
    fn recording_meta(&self) -> Option<RecordingMeta> {
        let bytes: Cow<'_, [u8]> = match &self.content {
            RecordingContent::Path(path) => match fs::read(path) {
                Ok(bytes) => Cow::Owned(bytes),
                Err(e) => {
                    log::warn!(
                        "Could not read {} to look it up in the recording history: {e}",
                        path.display()
                    );
                    return None;
                }
            },
            RecordingContent::Bytes(bytes) => Cow::Borrowed(bytes),
        };
        match gt_store::extract_meta(&bytes) {
            Ok(meta) => Some(meta),
            Err(e) => {
                log::debug!(
                    "Could not read recording metadata from '{}': {e}",
                    self.filename
                );
                None
            }
        }
    }
}

/// One arriving recording that the history database already holds, with the
/// reference and the stored track table of what it matched.
pub struct RecordingAlreadyInHistory {
    pub from_disk: RecordingFromDisk,
    pub db_ref: DatabaseRef,
    pub stored_tracks: Vec<TrackRange>,
}

/// What the history lookup made of one batch of arriving recordings.
#[derive(Default)]
pub struct ScreenedRecordings {
    pub already_in_history: Vec<RecordingAlreadyInHistory>,
    /// The recordings that are new to the database, which load straight away.
    pub new_to_history: Vec<RecordingFromDisk>,
}

/// Look each arriving recording up in `db`, by the metadata
/// [`gt_store::extract_meta`] reads from its bytes.
///
/// Runs on the history worker's thread: it reads every one of the files.
pub fn screen_against_history(
    db: &ReadOnlyRecordings,
    recordings: Vec<RecordingFromDisk>,
) -> ScreenedRecordings {
    let entries = match db.list_recordings() {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!(
                "Could not list the recording history: {e}. Loading {} arriving recording(s) \
                 from disk.",
                recordings.len()
            );
            return ScreenedRecordings {
                already_in_history: Vec::new(),
                new_to_history: recordings,
            };
        }
    };
    let mut screened = ScreenedRecordings::default();
    for recording in recordings {
        match stored_recording_for(db, &entries, &recording) {
            Some((db_ref, stored_tracks)) => {
                log::info!(
                    "'{}' is already in history as {}/{}",
                    recording.filename,
                    db_ref.identity,
                    db_ref.group_name
                );
                screened.already_in_history.push(RecordingAlreadyInHistory {
                    from_disk: recording,
                    db_ref,
                    stored_tracks,
                });
            }
            None => screened.new_to_history.push(recording),
        }
    }
    screened
}

/// The database entry holding the same recording as `recording`, and that
/// entry's stored track table.
fn stored_recording_for(
    db: &ReadOnlyRecordings,
    entries: &[RecordingEntry],
    recording: &RecordingFromDisk,
) -> Option<(DatabaseRef, Vec<TrackRange>)> {
    let meta = recording.recording_meta()?;
    let entry = entries
        .iter()
        .find(|entry| entry.meta.same_recording(&meta))?;
    match db.stored_track_table(&entry.db_ref) {
        Ok(stored_tracks) => Some((entry.db_ref.clone(), stored_tracks)),
        Err(e) => {
            log::warn!(
                "Could not read the stored track table of {}/{}: {e}. Loading '{}' from disk.",
                entry.db_ref.identity,
                entry.db_ref.group_name,
                recording.filename
            );
            None
        }
    }
}

/// The recordings of one batch that history already holds, and what the user
/// has ticked about them.
pub struct RecordingsAlreadyInHistory {
    pub recordings: Vec<RecordingAlreadyInHistory>,
    /// Whether a load from disk leaves the tracks that are shelved in history
    /// out of the view. The stored version leaves them out either way.
    pub leave_shelved_tracks_out: bool,
}

impl RecordingsAlreadyInHistory {
    /// Over the recordings the history lookup found, with the shelved tracks
    /// left out to begin with, as the stored version leaves them out.
    fn over(recordings: Vec<RecordingAlreadyInHistory>) -> Self {
        Self {
            recordings,
            leave_shelved_tracks_out: true,
        }
    }

    fn shelved_track_count(&self) -> usize {
        self.recordings
            .iter()
            .flat_map(|recording| &recording.stored_tracks)
            .filter(|track| track.state == TrackState::Shelved)
            .count()
    }
}

/// What the user chose in the prompt over the recordings history already
/// holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlreadyInHistoryChoice {
    OpenTheStoredVersion,
    LoadFromDisk,
    Cancel,
}

impl App {
    /// Load the files that arrived together: one drop, one file-dialog choice,
    /// one command line, one paste, or the ones that waited for the databases
    /// to open.
    ///
    /// The recordings among them raise a single prompt where history already
    /// holds them: they are looked up in the history database first.
    pub(in crate::app) fn load_arriving_files(&mut self, arriving: Vec<QueuedLoad>) {
        if let Some(queued_loads) = self.storage_open.queued_loads_mut() {
            queued_loads.extend(arriving);
            return;
        }
        let mut recordings = Vec::new();
        for file in arriving {
            match file {
                QueuedLoad::Path(path) => {
                    let extension = path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if extension == "gtd" {
                        let filename = path
                            .file_name()
                            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                        recordings.push(RecordingFromDisk {
                            filename,
                            content: RecordingContent::Path(path),
                        });
                    } else {
                        self.loader.spawn_log_path(path);
                    }
                }
                // Bytes starting with the HDF5 magic are a recording,
                // everything else a log, lossily decoded where it is not UTF-8.
                QueuedLoad::Bytes { bytes, name } => {
                    if bytes.starts_with(HDF5_MAGIC) {
                        let filename = if name.is_empty() {
                            UNNAMED_DROP_FILENAME.to_owned()
                        } else {
                            name
                        };
                        recordings.push(RecordingFromDisk {
                            filename,
                            content: RecordingContent::Bytes(bytes),
                        });
                    } else {
                        // A log takes its name from its first entry when the
                        // drop carries no file name, as pasted text does.
                        self.loader
                            .spawn_log_bytes(bytes, (!name.is_empty()).then_some(name));
                    }
                }
                QueuedLoad::PastedText(text) => self.loader.spawn_pasted_log_text(text),
            }
        }
        if recordings.is_empty() {
            return;
        }
        // With no history database there is nothing to look them up in.
        if self.history.available() {
            self.recordings_awaiting_a_history_lookup += recordings.len();
            self.history.screen_recordings_from_disk(recordings);
        } else {
            for recording in recordings {
                self.spawn_recording_from_disk(recording, None);
            }
        }
    }

    /// Start the loads for the recordings history holds none of, and raise the
    /// prompt over the ones it holds.
    pub(in crate::app) fn load_screened_recordings(&mut self, screened: ScreenedRecordings) {
        let ScreenedRecordings {
            already_in_history,
            new_to_history,
        } = screened;
        self.recordings_awaiting_a_history_lookup = self
            .recordings_awaiting_a_history_lookup
            .saturating_sub(already_in_history.len() + new_to_history.len());
        for recording in new_to_history {
            self.spawn_recording_from_disk(recording, None);
        }
        if already_in_history.is_empty() {
            return;
        }
        self.pending_recordings_already_in_history =
            Some(RecordingsAlreadyInHistory::over(already_in_history));
    }

    /// Start the load of one recording that arrived from disk, under `open`
    /// where the history database holds it and the user chose what to do with
    /// its stored tracks.
    fn spawn_recording_from_disk(
        &mut self,
        recording: RecordingFromDisk,
        open: Option<loader::HistoryOpen>,
    ) {
        let RecordingFromDisk { filename, content } = recording;
        match content {
            RecordingContent::Path(path) => {
                self.loader
                    .spawn_gtd_path(path, self.processing_config, open);
            }
            RecordingContent::Bytes(bytes) => {
                self.loader
                    .spawn_gtd_bytes(bytes, filename, self.processing_config, open);
            }
        }
    }

    pub(super) fn show_recordings_already_in_history_prompt(&mut self, ui: &egui::Ui) {
        let Some(prompt) = self.pending_recordings_already_in_history.take() else {
            return;
        };
        let listed: Vec<&str> = prompt
            .recordings
            .iter()
            .take(MOST_LISTED_RECORDINGS)
            .map(|recording| recording.from_disk.filename.as_str())
            .collect();
        let unlisted = prompt.recordings.len().saturating_sub(listed.len());
        let has_a_shelved_track = prompt.shelved_track_count() > 0;
        let mut leave_shelved_tracks_out = prompt.leave_shelved_tracks_out;
        let choice = modals::anchored_confirmation_dialog(
            ui.ctx(),
            AnchoredDialogKind::RecordingsAlreadyInHistory,
            recordings_already_in_history_title(prompt.recordings.len()),
            AlreadyInHistoryChoice::Cancel,
            |ui, _regions| {
                for filename in listed {
                    ui.add(Label::new(filename).truncate());
                }
                if unlisted > 0 {
                    ui.label(RichText::new(format!("and {unlisted} more")).weak());
                }
                ui.add_space(4.0);
                ui.add(
                    Label::new(
                        "The stored version reproduces the tracks with the settings they were \
                         stored with. The file on disk is split into tracks again with the \
                         current settings.",
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.add_enabled_ui(has_a_shelved_track, |ui| {
                    ui.checkbox(
                        &mut leave_shelved_tracks_out,
                        LEAVE_SHELVED_TRACKS_OUT_LABEL,
                    )
                    .on_hover_text(
                        "Applies to the file on disk. The stored version leaves the shelved \
                         tracks out either way.",
                    )
                    .on_disabled_hover_text(NO_SHELVED_TRACK_HOVER);
                });
            },
            |ui| {
                let mut choice = None;
                if ui
                    .button(OPEN_THE_STORED_VERSION_LABEL)
                    .on_hover_text(
                        "Open what history holds, with the tracks it holds for the recording",
                    )
                    .clicked()
                {
                    choice = Some(AlreadyInHistoryChoice::OpenTheStoredVersion);
                }
                if ui
                    .button(LOAD_FROM_DISK_LABEL)
                    .on_hover_text("Read the file again and split it with the current settings")
                    .clicked()
                {
                    choice = Some(AlreadyInHistoryChoice::LoadFromDisk);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(AlreadyInHistoryChoice::Cancel);
                }
                choice
            },
        );

        match choice {
            Some(AlreadyInHistoryChoice::OpenTheStoredVersion) => {
                for recording in prompt.recordings {
                    self.history.open(recording.db_ref);
                }
            }
            Some(AlreadyInHistoryChoice::LoadFromDisk) => {
                for recording in prompt.recordings {
                    let RecordingAlreadyInHistory {
                        from_disk,
                        db_ref,
                        stored_tracks,
                    } = recording;
                    let open = if leave_shelved_tracks_out {
                        Some(loader::HistoryOpen::ApplyShelved {
                            db_ref,
                            stored_tracks,
                            applied_current_marker_settings: false,
                        })
                    } else {
                        None
                    };
                    self.spawn_recording_from_disk(from_disk, open);
                }
            }
            Some(AlreadyInHistoryChoice::Cancel) => {}
            // No choice yet: keep the prompt open for the next frame, with
            // the tickbox as the user left it.
            None => {
                self.pending_recordings_already_in_history = Some(RecordingsAlreadyInHistory {
                    leave_shelved_tracks_out,
                    ..prompt
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gt_store::{HistoryDatabase as _, Recordings};

    use crate::app::history_test_support::{SAMPLE_POINT_COUNT, sample_bytes, store_recording};

    use super::*;

    /// A database left unreadable behind the handle that opened it. Every
    /// recording of the batch loads from disk.
    #[test]
    fn a_database_that_cannot_be_listed_leaves_every_recording_new_to_history() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("history.h5");
        let bytes = sample_bytes();
        store_recording(
            &db_path,
            &bytes,
            &[TrackRange {
                start: 0,
                end: SAMPLE_POINT_COUNT,
                state: TrackState::Live,
            }],
        );
        let db = Recordings::open_or_create(&db_path).expect("reopen the database");
        std::fs::write(&db_path, b"not an HDF5 file").expect("overwrite the database");
        assert!(
            db.list_recordings().is_err(),
            "the overwritten database still lists its recordings"
        );

        let screened = screen_against_history(
            &db,
            vec![RecordingFromDisk {
                filename: "ride.gtd".to_owned(),
                content: RecordingContent::Bytes(bytes.into()),
            }],
        );

        assert!(screened.already_in_history.is_empty());
        assert_eq!(
            screened
                .new_to_history
                .iter()
                .map(|recording| recording.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["ride.gtd"]
        );
    }
}
