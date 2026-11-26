use std::sync::{Arc, Weak};

use gt_loaded_files::{FileHistory, LoadedFileId, LoadedFilesView, RecordingData};
use gt_session_bridge::{
    ErrorCode, GetHistoryOpenStatus, HistoryListing, HistoryOpenId, HistoryOpenState,
    HistoryOpenStatus, HistoryRecordingId, ListHistoryRecordings, OpenHistoryRecording,
    RecordingReference, Reply,
};
use gt_store::{
    DbError, RecordingDebugTag, RecordingUiState, StoredLayoutRevision, StoredSegmentation,
    StoredTrackSnapshot,
};
use gt_track_builder::SegmentationConfig;
use gt_types::{FileSource, Generation};
use uuid::Uuid;

use super::App;
use super::association_batches::RecordingOperationOrigin;
use super::history_db::OpenedRecording;
use super::history_list::RecordingListing;
use super::history_open::ResegmentChoice;
use super::loader::{LoadCompletion, LoadedRecordingResult};

#[derive(Default)]
pub(super) struct SessionHistory {
    operation: Option<Operation>,
}

struct Operation {
    id: HistoryOpenId,
    selection: HistoryRecordingId,
    phase: Phase,
}

enum Phase {
    AwaitingTrackSettings(SelectedRead),
    Loading(Loading),
    Reading(Option<ReadyRecording>),
    Ready(ReadyRecording),
    Terminal(OpenStop),
}

pub(super) enum OpenStop {
    Cancelled,
    Expired,
    Failed(ErrorCode),
}

pub(super) enum HistoryLoadAdoption {
    Accept,
    ResetVisibility(StoredLayoutRevision),
    Skip,
}

struct Loading {
    read: SelectedRead,
    tracks: LoadTracks,
}

enum LoadTracks {
    Recalculate,
    Stored(Option<Inputs>),
}

enum DisplayStateUpdate {
    ApplySaved {
        observation: StoredLayoutRevision,
        state: Result<RecordingUiState, DbError>,
    },
    Clear(StoredLayoutRevision),
    Preserve,
}

struct SelectedRead {
    observation: StoredTrackSnapshot,
    visibility: Option<Result<RecordingUiState, DbError>>,
}

impl SelectedRead {
    fn from_opened(opened: &mut OpenedRecording) -> Self {
        Self {
            observation: opened.track_snapshot.clone(),
            visibility: opened.ui_state.take(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Inputs {
    debug_tag: Option<RecordingDebugTag>,
    stored: Option<StoredSegmentation>,
    processing: SegmentationConfig,
}

#[derive(Clone)]
struct ReadyRecording {
    file: LoadedFileId,
    bytes: Weak<[u8]>,
    observation: StoredTrackSnapshot,
    live_track_count: usize,
    generation: Generation<RecordingData>,
    inputs: Option<Inputs>,
    reused: bool,
}

impl ReadyRecording {
    fn remains_loaded(&self, files: LoadedFilesView<'_>) -> bool {
        let Some(entry) = files
            .entry_for_id(self.file)
            .filter(|_| self.observation.is_current_with_membership())
        else {
            return false;
        };
        matches!(&entry.file().source, FileSource::GtdBytes(bytes)
            if Weak::ptr_eq(&self.bytes, &Arc::downgrade(bytes)))
            && entry.file().tracks.len() == self.live_track_count
            && self::matching_stored_layout(&self.observation, entry.history()).is_ok()
    }

    fn can_reuse(
        &self,
        files: LoadedFilesView<'_>,
        snapshot: &StoredTrackSnapshot,
        inputs: Inputs,
    ) -> bool {
        self.inputs == Some(inputs)
            && self.generation == files.data_generation()
            && self.remains_loaded(files)
            && files.entry_for_id(self.file).is_some_and(|entry| {
                entry.history().track_snapshot().is_some_and(|previous| {
                    snapshot.is_current()
                        && previous.same_observation(snapshot)
                        && previous.rows() == snapshot.rows()
                })
            })
    }
}

impl Operation {
    fn pending(&self) -> bool {
        matches!(
            self.phase,
            Phase::Reading(_) | Phase::AwaitingTrackSettings(_) | Phase::Loading(_)
        )
    }

    fn status(&self, files: LoadedFilesView<'_>) -> HistoryOpenStatus {
        let state = match &self.phase {
            Phase::Reading(_) => HistoryOpenState::Reading,
            Phase::AwaitingTrackSettings(_) => HistoryOpenState::AwaitingTrackSettings,
            Phase::Loading(_) => HistoryOpenState::Loading,
            Phase::Ready(ready) if ready.remains_loaded(files) => HistoryOpenState::Ready {
                recording: RecordingReference {
                    session: self.id.session,
                    number: ready.file.into(),
                },
                reused: ready.reused,
            },
            Phase::Ready(_) | Phase::Terminal(OpenStop::Expired) => HistoryOpenState::Expired,
            Phase::Terminal(OpenStop::Cancelled) => HistoryOpenState::Cancelled,
            Phase::Terminal(OpenStop::Failed(code)) => HistoryOpenState::Failed { code: *code },
        };
        HistoryOpenStatus {
            operation: self.id,
            state,
        }
    }
}

impl SessionHistory {
    pub(super) fn is_pending(&self, id: HistoryOpenId) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.id == id && operation.pending())
    }

    pub(super) fn expire(&mut self) {
        if let Some(operation) = &mut self.operation {
            operation.phase = Phase::Terminal(OpenStop::Expired);
        }
    }

    pub(super) fn finish(&mut self, id: HistoryOpenId, state: OpenStop) {
        if let Some(operation) = self.operation_mut(id) {
            operation.phase = Phase::Terminal(state);
        }
    }

    pub(super) fn await_settings(&mut self, id: HistoryOpenId, opened: &mut OpenedRecording) {
        if let Some(operation) = self.operation_mut(id) {
            operation.phase = Phase::AwaitingTrackSettings(SelectedRead::from_opened(opened));
        }
    }

    pub(super) fn saved_hidden_track_count(&self, id: HistoryOpenId) -> Option<usize> {
        let operation = self.operation.as_ref().filter(|op| op.id == id)?;
        let Phase::AwaitingTrackSettings(read) = &operation.phase else {
            return None;
        };
        let state = read.visibility.as_ref()?.as_ref().ok()?;
        Some(state.hidden_track_numbers().len())
    }

    pub(super) fn apply_settings_choice(&mut self, id: HistoryOpenId, choice: ResegmentChoice) {
        if let Some(operation) = self.operation_mut(id)
            && let Phase::AwaitingTrackSettings(read) = &mut operation.phase
        {
            let (tracks, visibility) = match choice {
                ResegmentChoice::Cancel => {
                    operation.phase = Phase::Terminal(OpenStop::Cancelled);
                    return;
                }
                ResegmentChoice::RecalculateWithCurrentSettings => (LoadTracks::Recalculate, None),
                ResegmentChoice::UseStoredTracks => {
                    (LoadTracks::Stored(None), read.visibility.take())
                }
            };
            operation.phase = Phase::Loading(Loading {
                read: SelectedRead {
                    observation: read.observation.clone(),
                    visibility,
                },
                tracks,
            });
        }
    }

    pub(super) fn expire_stale_settings_decision(&mut self) -> Option<HistoryOpenId> {
        let operation = self.operation.as_mut()?;
        if matches!(&operation.phase, Phase::AwaitingTrackSettings(read) if !read.observation.is_current_with_membership())
        {
            operation.phase = Phase::Terminal(OpenStop::Expired);
            Some(operation.id)
        } else {
            None
        }
    }

    pub(super) fn begin_load(
        &mut self,
        id: HistoryOpenId,
        files: LoadedFilesView<'_>,
        opened: &mut OpenedRecording,
        processing: SegmentationConfig,
    ) -> bool {
        let Some(operation) = self.operation_mut(id) else {
            return false;
        };
        let inputs = Inputs {
            debug_tag: opened.stored.debug_tag,
            stored: opened.stored.segmentation,
            processing,
        };
        if opened.matches_reuse_payload
            && let Phase::Reading(Some(previous)) = &mut operation.phase
            && previous.can_reuse(files, &opened.track_snapshot, inputs)
        {
            previous.reused = true;
            operation.phase = Phase::Ready(previous.clone());
            return false;
        }
        operation.phase = Phase::Loading(Loading {
            read: SelectedRead::from_opened(opened),
            tracks: LoadTracks::Stored(Some(inputs)),
        });
        true
    }

    pub(super) fn loaded(
        &mut self,
        id: HistoryOpenId,
        files: LoadedFilesView<'_>,
        file: LoadedFileId,
    ) -> Option<StoredTrackSnapshot> {
        let operation = self.operation_mut(id)?;
        let Phase::Loading(loading) = &operation.phase else {
            return None;
        };
        let entry = files.entry_for_id(file)?;
        let FileSource::GtdBytes(bytes) = &entry.file().source else {
            operation.phase = Phase::Terminal(OpenStop::Expired);
            return None;
        };
        let observation = loading.read.observation.clone();
        let can_restore = entry
            .history()
            .track_snapshot()
            .is_some_and(|snapshot| snapshot.same_observation(&observation));
        operation.phase = Phase::Ready(ReadyRecording {
            file,
            bytes: Arc::downgrade(bytes),
            observation: observation.clone(),
            live_track_count: entry.file().tracks.len(),
            generation: files.data_generation(),
            inputs: match loading.tracks {
                LoadTracks::Recalculate => None,
                LoadTracks::Stored(inputs) => inputs,
            },
            reused: false,
        });
        can_restore.then_some(observation)
    }

    fn accept_recording(
        &mut self,
        id: HistoryOpenId,
        result: &LoadedRecordingResult,
    ) -> Option<DisplayStateUpdate> {
        let operation = self.operation_mut(id)?;
        let Phase::Loading(loading) = &mut operation.phase else {
            return None;
        };
        let observation = &loading.read.observation;
        match loading.tracks {
            LoadTracks::Stored(_) if observation.is_current_with_membership() => {
                let layout = match self::matching_stored_layout(observation, &result.history) {
                    Ok(layout) => layout,
                    Err(code) => {
                        operation.phase = Phase::Terminal(OpenStop::Failed(code));
                        return None;
                    }
                };
                if let Some(layout) = layout
                    && let Some(state) = loading.read.visibility.take()
                {
                    Some(DisplayStateUpdate::ApplySaved {
                        observation: layout.clone(),
                        state,
                    })
                } else {
                    Some(DisplayStateUpdate::Preserve)
                }
            }
            LoadTracks::Recalculate => {
                let successor = result.history.track_snapshot().filter(|snapshot| {
                    result.history_write_attempted
                        && snapshot.is_current_with_membership()
                        && snapshot.same_payload_revision(observation)
                });
                if let Some(successor) = successor {
                    loading.read.observation = successor.clone();
                    Some(DisplayStateUpdate::Clear(successor.layout().clone()))
                } else {
                    let reason = if observation.is_current_with_membership() {
                        OpenStop::Failed(ErrorCode::HistoryOpenFailed)
                    } else {
                        OpenStop::Expired
                    };
                    operation.phase = Phase::Terminal(reason);
                    None
                }
            }
            LoadTracks::Stored(_) => {
                operation.phase = Phase::Terminal(OpenStop::Expired);
                None
            }
        }
    }

    fn operation_mut(&mut self, id: HistoryOpenId) -> Option<&mut Operation> {
        self.operation
            .as_mut()
            .filter(|operation| operation.id == id)
    }
}

fn matching_stored_layout<'a>(
    expected: &StoredTrackSnapshot,
    loaded: &'a FileHistory,
) -> Result<Option<&'a StoredLayoutRevision>, ErrorCode> {
    let observed = loaded
        .track_snapshot()
        .filter(|actual| actual.same_observation(expected));
    match observed {
        Some(snapshot) => Ok(Some(snapshot.layout())),
        None if expected.rows().is_empty() => Ok(None),
        None => Err(ErrorCode::HistoryTrackLayoutUnavailable),
    }
}

impl App {
    pub(super) fn history_load_adoption(
        &mut self,
        completion: &LoadCompletion,
    ) -> HistoryLoadAdoption {
        let LoadCompletion::Recording { origin, result } = completion else {
            return HistoryLoadAdoption::Accept;
        };
        let Some(operation) = origin.session_history() else {
            return HistoryLoadAdoption::Accept;
        };
        if !self.session_history.is_pending(operation) {
            return HistoryLoadAdoption::Skip;
        }
        match result {
            Ok(result) => {
                let Some(display) = self.session_history.accept_recording(operation, result) else {
                    self.invalidate_history_listing();
                    return HistoryLoadAdoption::Skip;
                };
                match display {
                    DisplayStateUpdate::ApplySaved { observation, state }
                        if self
                            .shared
                            .borrow()
                            .tree
                            .hidden_track_numbers_for_layout(&observation)
                            .is_none() =>
                    {
                        self.apply_the_ui_state_stored_with_a_recording(observation, state);
                    }
                    DisplayStateUpdate::Clear(recording) => {
                        return HistoryLoadAdoption::ResetVisibility(recording);
                    }
                    DisplayStateUpdate::ApplySaved { .. } | DisplayStateUpdate::Preserve => {}
                }
                HistoryLoadAdoption::Accept
            }
            Err(failure) => {
                log::error!("Failed to open History recording: {}", failure.cause);
                self.session_history
                    .finish(operation, OpenStop::Failed(ErrorCode::HistoryOpenFailed));
                HistoryLoadAdoption::Skip
            }
        }
    }

    pub(super) fn list_history_recordings(
        &mut self,
        session: Uuid,
        request: ListHistoryRecordings,
    ) -> Result<Reply, ErrorCode> {
        self.session_history_listing(session)
            .and_then(|listing| listing.page(request))
            .map(|page| Reply::HistoryRecordings { page })
            .map_err(|error| {
                if request.cursor.is_some()
                    && matches!(
                        error,
                        ErrorCode::HistoryListingFailed | ErrorCode::HistoryNotReady
                    )
                    && self.storage_open.databases_pending().is_none()
                {
                    ErrorCode::InvalidHistoryCursor
                } else {
                    error
                }
            })
    }

    pub(super) fn open_session_history_recording(
        &mut self,
        session: Uuid,
        request: OpenHistoryRecording,
    ) -> Result<HistoryOpenStatus, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let ctx = self.ctx.clone();
        self.apply_finished_recording_work(&ctx);
        let files = self.shared.borrow();
        let previous = self.session_history.operation.as_ref();
        if let Some(operation) = previous.filter(|operation| operation.pending()) {
            return if operation.selection == request.recording && operation.id.session == session {
                Ok(operation.status(files.loaded_files.view()))
            } else {
                Err(ErrorCode::Busy)
            };
        }
        let reuse = previous
            .filter(|operation| {
                operation.selection == request.recording && operation.id.session == session
            })
            .and_then(|operation| match &operation.phase {
                Phase::Ready(ready) => Some(ready.clone()),
                _ => None,
            });
        drop(files);
        let selection = self
            .session_history_listing(session)?
            .select_for_open(request.recording)?;
        let id = HistoryOpenId {
            session,
            id: Uuid::new_v4(),
        };
        let reuse_bytes = reuse.as_ref().and_then(|ready| ready.bytes.upgrade());
        let phase = if self.history.open_checked_recording(
            RecordingOperationOrigin::SessionHistory(id),
            selection,
            reuse_bytes,
        ) {
            Phase::Reading(reuse)
        } else {
            Phase::Terminal(OpenStop::Failed(ErrorCode::HistoryUnavailable))
        };
        let operation = self.session_history.operation.insert(Operation {
            id,
            selection: request.recording,
            phase,
        });
        Ok(operation.status(self.shared.borrow().loaded_files.view()))
    }

    pub(super) fn history_open_status(
        &mut self,
        session: Uuid,
        request: GetHistoryOpenStatus,
    ) -> Result<HistoryOpenStatus, ErrorCode> {
        if request.operation.session != session {
            return Err(ErrorCode::StaleSession);
        }
        let ctx = self.ctx.clone();
        self.apply_finished_recording_work(&ctx);
        self.session_history
            .operation
            .as_ref()
            .filter(|operation| operation.id == request.operation)
            .map(|operation| operation.status(self.shared.borrow().loaded_files.view()))
            .ok_or(ErrorCode::HistoryOpenNotFound)
    }

    fn session_history_listing(&mut self, session: Uuid) -> Result<HistoryListing<'_>, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let ctx = self.ctx.clone();
        self.apply_finished_recording_work(&ctx);
        if self.storage_open.databases_pending().is_some() {
            return Err(ErrorCode::HistoryNotReady);
        }
        let database = self.history.path().ok_or(ErrorCode::HistoryUnavailable)?;
        self.history_listing.request_if_missing(&self.history);
        match self.history_listing.view() {
            RecordingListing::Ready { revision, entries } => Ok(HistoryListing {
                database,
                entries,
                revision,
                session,
            }),
            RecordingListing::Failed => Err(ErrorCode::HistoryListingFailed),
            RecordingListing::Pending => Err(ErrorCode::HistoryNotReady),
        }
    }
}
