//! The recording a log is anchored to, and what identifies that recording.

use std::fmt;
use std::sync::Arc;

use gt_history_types::{DatabaseRef, StoredTrackSnapshot};
use gt_loaded_files::{
    FixPlacementFingerprint, IndexAlignedFiles, LoadedFileEntry, LoadedFileId, LoadedFilesView,
    RecordingLoadOrigin,
};
use gt_types::{FileSource, Generation};

/// Equality uses the loaded ID for a selected source and the database reference
/// for a saved attachment.
#[derive(Clone, Debug)]
pub enum RecordingKey {
    LoadedInHistory(LoadedHistoryAnchor),
    PendingHistory(DatabaseRef),
    Session(LoadedFileId),
    Stored(DatabaseRef),
}

impl RecordingKey {
    pub fn of_loaded_recording(entry: LoadedFileEntry<'_>) -> Self {
        match entry.history().db_ref() {
            Some(recording) => Self::LoadedInHistory(LoadedHistoryAnchor {
                loaded: entry.id(),
                recording: recording.clone(),
                observation: None,
                observed_generation: None,
            }),
            None => Self::Session(entry.id()),
        }
    }

    pub(crate) fn resolve<'a>(
        &mut self,
        recordings: &LoadedFilesView<'a>,
    ) -> RecordingLookup<LoadedFileEntry<'a>> {
        let lookup = match self {
            Self::LoadedInHistory(anchor) => anchor.lookup(recordings),
            Self::PendingHistory(recording) => {
                RecordingLookup::for_pending_history(recording, recordings)
            }
            Self::Session(id) => recordings
                .entry_for_id(*id)
                .map_or(RecordingLookup::Absent, RecordingLookup::Loaded),
            Self::Stored(recording) => RecordingLookup::for_attachment(recording, recordings),
        };
        let RecordingLookup::Loaded(entry) = lookup else {
            return lookup;
        };
        let generation = recordings.generation();
        if matches!(self, Self::LoadedInHistory(anchor) if anchor.loaded == entry.id()
            && anchor.observed_generation == Some(generation))
        {
            return lookup;
        }
        let mut resolved = Self::of_loaded_recording(entry);
        if let Self::LoadedInHistory(anchor) = &mut resolved {
            anchor.observation = StoredSourceObservation::of_loaded_recording(entry).map(Arc::new);
            anchor.observed_generation = Some(generation);
        }
        *self = resolved;
        lookup
    }

    pub fn database_ref(&self) -> Option<&DatabaseRef> {
        match self {
            Self::LoadedInHistory(anchor) => Some(&anchor.recording),
            Self::PendingHistory(db_ref) | Self::Stored(db_ref) => Some(db_ref),
            Self::Session(_) => None,
        }
    }

    fn loaded_file_id(&self) -> Option<LoadedFileId> {
        match self {
            Self::LoadedInHistory(anchor) => Some(anchor.loaded),
            Self::Session(id) => Some(*id),
            Self::PendingHistory(_) | Self::Stored(_) => None,
        }
    }
}

impl PartialEq for RecordingKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::PendingHistory(left), Self::PendingHistory(right))
            | (Self::Stored(left), Self::Stored(right)) => left == right,
            _ => self
                .loaded_file_id()
                .is_some_and(|id| other.loaded_file_id() == Some(id)),
        }
    }
}

impl Eq for RecordingKey {}

#[derive(Debug)]
pub(crate) enum RecordingLookup<R> {
    Absent,
    Loaded(R),
    SelectionRequired,
}

impl<'a> RecordingLookup<LoadedFileEntry<'a>> {
    pub(crate) fn for_attachment(
        recording: &DatabaseRef,
        recordings: &LoadedFilesView<'a>,
    ) -> Self {
        recordings
            .stored_recording_index(recording)
            .and_then(|file| recordings.entry_for(file))
            .map_or(Self::Absent, Self::Loaded)
    }

    fn for_pending_history(recording: &DatabaseRef, recordings: &LoadedFilesView<'a>) -> Self {
        Self::among_history_candidates(recording, recordings, |entry| {
            StoredSourceObservation::verified_contents(entry).is_some()
        })
    }

    fn among_history_candidates(
        recording: &DatabaseRef,
        recordings: &LoadedFilesView<'a>,
        is_match: impl Fn(LoadedFileEntry<'a>) -> bool,
    ) -> Self {
        let mut opened_from_history = false;
        let candidates = recordings
            .entries()
            .filter(|entry| entry.history().db_ref() == Some(recording))
            .inspect(|entry| {
                opened_from_history |=
                    entry.history().load_origin() == Some(RecordingLoadOrigin::History);
            })
            .filter(|entry| is_match(*entry));
        let lookup = Self::unique(candidates);
        if matches!(lookup, Self::Absent) && opened_from_history {
            Self::SelectionRequired
        } else {
            lookup
        }
    }

    fn unique(mut matches: impl Iterator<Item = LoadedFileEntry<'a>>) -> Self {
        let Some(entry) = matches.next() else {
            return Self::Absent;
        };
        if matches.next().is_some() {
            Self::SelectionRequired
        } else {
            Self::Loaded(entry)
        }
    }
}

#[derive(Clone)]
pub struct LoadedHistoryAnchor {
    loaded: LoadedFileId,
    recording: DatabaseRef,
    observation: Option<Arc<StoredSourceObservation>>,
    observed_generation: Option<Generation<IndexAlignedFiles>>,
}

impl LoadedHistoryAnchor {
    fn lookup<'a>(&self, recordings: &LoadedFilesView<'a>) -> RecordingLookup<LoadedFileEntry<'a>> {
        if let Some(entry) = recordings.entry_for_id(self.loaded) {
            return RecordingLookup::Loaded(entry);
        }
        let observation = self
            .observation
            .as_ref()
            .filter(|observation| observation.snapshot.is_current());
        RecordingLookup::among_history_candidates(&self.recording, recordings, |entry| {
            observation.is_some_and(|observation| observation.matches(entry))
        })
    }
}

impl fmt::Debug for LoadedHistoryAnchor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadedHistoryAnchor")
            .field("loaded", &self.loaded)
            .field("recording", &self.recording)
            .finish_non_exhaustive()
    }
}

struct StoredSourceObservation {
    snapshot: StoredTrackSnapshot,
    bytes: Arc<[u8]>,
    placements: FixPlacementFingerprint,
}

impl StoredSourceObservation {
    fn verified_contents(entry: LoadedFileEntry<'_>) -> Option<(&StoredTrackSnapshot, &Arc<[u8]>)> {
        let FileSource::GtdBytes(bytes) = &entry.file().source else {
            return None;
        };
        let snapshot = entry.history().track_snapshot()?;
        // Another process can reuse a legacy group address during a read-only session.
        if !snapshot.is_current() || !snapshot.recording().has_generated_group_id() {
            return None;
        }
        Some((snapshot, bytes))
    }

    fn of_loaded_recording(entry: LoadedFileEntry<'_>) -> Option<Self> {
        let (snapshot, bytes) = Self::verified_contents(entry)?;
        Some(Self {
            snapshot: snapshot.clone(),
            bytes: Arc::clone(bytes),
            placements: entry.fix_placement_fingerprint(),
        })
    }

    fn matches(&self, entry: LoadedFileEntry<'_>) -> bool {
        Self::verified_contents(entry).is_some_and(|(snapshot, bytes)| {
            snapshot.same_observation(&self.snapshot)
                && bytes == &self.bytes
                && self.placements == entry.fix_placement_fingerprint()
        })
    }
}
