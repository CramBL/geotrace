//! The loaded logs of a session and each log's association state.

use std::fmt::Write as _;
use std::mem;
use std::ops::Deref;
use std::sync::Arc;

use chrono::Duration;
use gt_fmt::MIDDLE_DOT;
use gt_history_types::{LogContentHash, StoredLogFilterStack};
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_logfile::{EntryPlacement, ParsedLog, RecordingAssociationIndex};
use gt_types::{TimeRange, mercator};
use gt_ui_types::{
    LoadedLogId, LogMatch, LogMatchColor, LogMatchLayer, LogMatchSource, LogMatches,
};

use crate::anchor::RecordingKey;
use crate::association::AssociationCandidates;
use crate::attachment::{LogAttachmentRef, LogAttachmentState};
use crate::filter::{EntryMatches, FilterStack, LayerColorSlots};

#[derive(Debug)]
struct LogDocument {
    parsed: Arc<ParsedLog>,
    content_hash: LogContentHash,
    entry_time_range: Option<TimeRange>,
}

impl LogDocument {
    fn new(parsed: ParsedLog) -> Self {
        Self {
            content_hash: LogContentHash::of_log_bytes(parsed.text().as_bytes()),
            entry_time_range: parsed.time_range(),
            parsed: Arc::new(parsed),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogContextOrigin {
    DetachedAttachment,
    LooseImport,
    SavedAttachment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionSourceState {
    None,
    PendingInitialSelection,
    Recording(RecordingKey),
}

#[derive(Debug)]
enum LogContext {
    DetachedAttachment { source: Option<RecordingKey> },
    LooseImport { source: PositionSourceState },
    SavedAttachment(LogAttachmentState),
}

#[derive(Debug)]
pub struct LoadedLog {
    name: String,
    document: Arc<LogDocument>,
    context: LogContext,
    association: Association,
    filters: FilterStack,
    visible: bool,
}

/// What the log's anchor produced, under the window it was associated with.
#[derive(Debug)]
struct Association {
    window: Duration,

    /// One slot per entry of the log, in entry order. Empty while the anchor
    /// resolves to no loaded recording.
    entry_placements: Vec<Option<EntryPlacement>>,

    associated_entry_count: usize,

    /// The loaded recording the anchor last resolved to.
    recording: Option<LoadedFileId>,
}

impl LoadedLog {
    /// `filename` is `None` for log text that arrived without a name of its
    /// own (pasted text, or a drop carrying bytes only): such a log takes its
    /// name from the time of its first anchored entry, e.g. "pasted 14:02:11".
    pub fn new(filename: Option<String>, parsed: ParsedLog, association_window: Duration) -> Self {
        let name = filename.unwrap_or_else(|| name_from_first_anchored_entry(&parsed));
        let document = Arc::new(LogDocument::new(parsed));
        Self {
            name,
            filters: FilterStack::new(Arc::clone(&document.parsed)),
            document,
            context: LogContext::LooseImport {
                source: PositionSourceState::PendingInitialSelection,
            },
            association: Association {
                window: association_window,
                entry_placements: Vec::new(),
                associated_entry_count: 0,
                recording: None,
            },
            visible: true,
        }
    }

    pub fn context_origin(&self) -> LogContextOrigin {
        match self.context {
            LogContext::LooseImport { .. } => LogContextOrigin::LooseImport,
            LogContext::SavedAttachment(_) => LogContextOrigin::SavedAttachment,
            LogContext::DetachedAttachment { .. } => LogContextOrigin::DetachedAttachment,
        }
    }

    pub fn attachment(&self) -> Option<&LogAttachmentRef> {
        match &self.context {
            LogContext::SavedAttachment(state) => Some(&state.reference),
            LogContext::LooseImport { .. } | LogContext::DetachedAttachment { .. } => None,
        }
    }

    /// The filter-stack edits this log's attachment has yet to be written,
    /// and `None` while the database holds the stack the user is looking at.
    pub fn take_filter_stack_edits_to_store(
        &mut self,
    ) -> Option<(LogAttachmentRef, StoredLogFilterStack)> {
        let LogContext::SavedAttachment(state) = &mut self.context else {
            return None;
        };
        let filters = self.filters.to_stored_stack();
        if filters == state.stored_filters {
            return None;
        }
        state.stored_filters.clone_from(&filters);
        Some((state.reference.clone(), filters))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parsed(&self) -> &ParsedLog {
        &self.document.parsed
    }

    pub fn content_hash(&self) -> LogContentHash {
        self.document.content_hash
    }

    /// The filters over this log. Mutating them goes through
    /// [`LoadedLogs::filter_stack_mut_by_id`], which hands out the palette
    /// slots the layer chips share with every other log.
    pub fn filters(&self) -> &FilterStack {
        &self.filters
    }

    /// The one-line parse summary the viewer shows beside the log's name:
    /// detected format, entry count with the interpolated portion, boot count,
    /// how many entries took no position, and what a lossy decode cost.
    pub fn parse_summary_line(&self) -> String {
        let entries = self.document.parsed.entries().len();
        let interpolated = self.document.parsed.interpolated_entry_count();
        let boots = self.document.parsed.boot_sessions().len();
        let unassociated = self.unassociated_entry_count();
        let replaced_bytes = self.document.parsed.replaced_byte_count();

        let mut summary = format!(
            "{} {MIDDLE_DOT} {} {}",
            self.document.parsed.format().display_name(),
            gt_fmt::format_count(entries),
            gt_fmt::pluralize(entries, "entry", "entries"),
        );
        if interpolated > 0 {
            write!(
                summary,
                " ({} interpolated)",
                gt_fmt::format_count(interpolated)
            )
            .ok();
        }
        write!(
            summary,
            " {MIDDLE_DOT} {} {} {MIDDLE_DOT} {} unassociated",
            gt_fmt::format_count(boots),
            gt_fmt::pluralize(boots, "boot", "boots"),
            gt_fmt::format_count(unassociated),
        )
        .ok();
        if replaced_bytes > 0 {
            write!(
                summary,
                " {MIDDLE_DOT} {} {} replaced",
                gt_fmt::format_count(replaced_bytes),
                gt_fmt::pluralize(replaced_bytes, "byte", "bytes"),
            )
            .ok();
        }
        summary
    }

    /// First to last entry timestamp, `None` for a log with no entries.
    pub fn entry_time_range(&self) -> Option<TimeRange> {
        self.document.entry_time_range
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn position_source_state(&self) -> PositionSourceState {
        match &self.context {
            LogContext::LooseImport { source } => source.clone(),
            LogContext::DetachedAttachment { source } => source
                .clone()
                .map_or(PositionSourceState::None, PositionSourceState::Recording),
            LogContext::SavedAttachment(state) => PositionSourceState::Recording(
                RecordingKey::Stored(state.reference.recording.clone()),
            ),
        }
    }

    pub fn anchor_key(&self) -> Option<RecordingKey> {
        match self.position_source_state() {
            PositionSourceState::Recording(key) => Some(key),
            PositionSourceState::PendingInitialSelection | PositionSourceState::None => None,
        }
    }

    pub fn is_anchored_to(&self, recording_key: &RecordingKey) -> bool {
        self.anchor_key().as_ref() == Some(recording_key)
    }

    /// The loaded recording the anchor resolved to, `None` while the anchored
    /// recording is not loaded.
    pub fn associated_recording(&self) -> Option<LoadedFileId> {
        self.association.recording
    }

    pub fn association_window(&self) -> Duration {
        self.association.window
    }

    /// Where the entry at `entry_index` of [`ParsedLog::entries`] sits on the
    /// recording this log is anchored to, `None` for an entry with no fix
    /// inside the association window.
    pub fn entry_placement(&self, entry_index: usize) -> Option<EntryPlacement> {
        self.association
            .entry_placements
            .get(entry_index)
            .copied()
            .flatten()
    }

    pub fn associated_entry_count(&self) -> usize {
        self.association.associated_entry_count
    }

    pub fn unassociated_entry_count(&self) -> usize {
        self.document
            .parsed
            .entries()
            .len()
            .saturating_sub(self.association.associated_entry_count)
    }

    pub fn anchor_to(&mut self, recording_key: RecordingKey, recordings: &LoadedFilesView<'_>) {
        match &mut self.context {
            LogContext::LooseImport { source } => {
                *source = PositionSourceState::Recording(recording_key);
            }
            LogContext::DetachedAttachment { source } => {
                *source = Some(recording_key);
            }
            LogContext::SavedAttachment(_) => {}
        }
        self.reassociate(recordings);
    }

    /// Anchors the log to the loaded recording `chosen` identifies, and removes
    /// its anchor when `chosen` is `None`: what a choice among the loaded
    /// recordings does.
    pub fn anchor_to_loaded_recording(
        &mut self,
        chosen: Option<LoadedFileId>,
        recordings: &LoadedFilesView<'_>,
    ) {
        match chosen.and_then(|id| recordings.entry_for_id(id)) {
            Some(entry) => self.anchor_to(RecordingKey::of_loaded_recording(entry), recordings),
            None => self.remove_anchor(),
        }
    }

    pub fn remove_anchor(&mut self) {
        match &mut self.context {
            LogContext::LooseImport { source } => {
                *source = PositionSourceState::None;
                self.clear_entry_placements();
            }
            LogContext::DetachedAttachment { source } => {
                *source = None;
                self.clear_entry_placements();
            }
            LogContext::SavedAttachment(_) => {}
        }
    }

    /// Sets how far an entry may be from a fix to take its position, and
    /// associates the log again under the new window.
    pub fn set_association_window(&mut self, window: Duration, recordings: &LoadedFilesView<'_>) {
        self.association.window = window;
        self.reassociate(recordings);
    }

    /// Associates every entry against the recording the anchor resolves to,
    /// after the loaded recordings changed.
    ///
    /// An anchor that resolves to no loaded recording leaves the entries
    /// without a position and stays: no log ever re-anchors to another
    /// recording without being pointed at it.
    pub fn reassociate(&mut self, recordings: &LoadedFilesView<'_>) {
        let anchored = self
            .anchor_key()
            .and_then(|key| key.loaded_recording(recordings));
        let Some(recording) = anchored else {
            self.clear_entry_placements();
            return;
        };
        self.association.recording = Some(recording.id());
        let entry_placements = RecordingAssociationIndex::from_recording(recording)
            .associate_entries(self.document.parsed.entries(), self.association.window);
        self.association.associated_entry_count = entry_placements
            .iter()
            .filter(|placement| placement.is_some())
            .count();
        self.association.entry_placements = entry_placements;
    }

    /// The loaded recordings this log could associate against, best first.
    pub fn rank_association_candidates(
        &self,
        recordings: &LoadedFilesView<'_>,
    ) -> AssociationCandidates {
        match self.document.entry_time_range {
            Some(log_range) => AssociationCandidates::rank(log_range, recordings),
            None => AssociationCandidates::none(),
        }
    }

    fn restore_attachment(
        &mut self,
        attachment: LogAttachmentRef,
        stored_filters: StoredLogFilterStack,
        recordings: &LoadedFilesView<'_>,
    ) {
        self.filters =
            FilterStack::from_stored_stack(Arc::clone(&self.document.parsed), &stored_filters);
        self.record_attachment(attachment, stored_filters, recordings);
    }

    fn record_attachment(
        &mut self,
        attachment: LogAttachmentRef,
        stored_filters: StoredLogFilterStack,
        recordings: &LoadedFilesView<'_>,
    ) {
        self.context = LogContext::SavedAttachment(LogAttachmentState {
            reference: attachment,
            stored_filters,
        });
        self.reassociate(recordings);
    }

    fn forget_attachment(&mut self) {
        if let LogContext::SavedAttachment(state) = &self.context {
            self.context = LogContext::DetachedAttachment {
                source: Some(RecordingKey::Stored(state.reference.recording.clone())),
            };
        }
    }

    /// Where on the map the entries `matches` selected are, in file order,
    /// each under the fix it is attributed to. Every position comes from the
    /// recording this log is associated against: an entry with no fix inside
    /// the association window contributes nothing.
    fn matched_points(&self, matches: &EntryMatches) -> Vec<LogMatch> {
        matches
            .matched_entry_indices()
            .filter_map(|entry_index| {
                let placement = self.entry_placement(entry_index)?;
                let (latitude, longitude) = placement.position;
                Some(LogMatch {
                    merc: mercator::normalize(latitude, longitude),
                    entry_index,
                    fix: placement.fix,
                })
            })
            .collect()
    }

    fn clear_entry_placements(&mut self) {
        self.association.entry_placements = Vec::new();
        self.association.associated_entry_count = 0;
        self.association.recording = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogSaveOutcome {
    AlreadyLoaded(LoadedLogId),
    AlreadySaved,
    LogUnloaded,
    Saved(LoadedLogId),
}

/// Attachment transitions require collection methods.
pub struct LoadedLogEditor<'a> {
    log: &'a mut LoadedLog,
}

impl LoadedLogEditor<'_> {
    pub fn anchor_to(&mut self, key: RecordingKey, recordings: &LoadedFilesView<'_>) {
        self.log.anchor_to(key, recordings);
    }

    pub fn anchor_to_loaded_recording(
        &mut self,
        chosen: Option<LoadedFileId>,
        recordings: &LoadedFilesView<'_>,
    ) {
        self.log.anchor_to_loaded_recording(chosen, recordings);
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.log.set_visible(visible);
    }

    pub fn set_association_window(&mut self, window: Duration, recordings: &LoadedFilesView<'_>) {
        self.log.set_association_window(window, recordings);
    }
}

impl Deref for LoadedLogEditor<'_> {
    type Target = LoadedLog;

    fn deref(&self) -> &Self::Target {
        self.log
    }
}

/// One loaded log under the identity it was loaded with.
#[derive(Debug)]
struct StoredLog {
    id: LoadedLogId,
    log: LoadedLog,
}

impl StoredLog {
    /// What the map needs to read this log's hovered lines back: its identity,
    /// the parse its layers' entries index into, and what its tooltip
    /// identifies the log by.
    fn source(&self, display_name: Option<String>) -> LogMatchSource {
        LogMatchSource {
            id: self.id,
            parsed: Arc::clone(&self.log.document.parsed),
            display_name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogPushOutcome {
    AlreadyLoaded(LoadedLogId),

    NewlyLoaded(LoadedLogId),
}

impl LogPushOutcome {
    pub fn id(self) -> LoadedLogId {
        match self {
            Self::NewlyLoaded(id) | Self::AlreadyLoaded(id) => id,
        }
    }
}

#[derive(Debug, Default)]
pub struct LoadedLogs {
    logs: Vec<StoredLog>,

    /// The identity the next loaded log takes. Nothing that named an unloaded
    /// log ever resolves to the one that took its place in the list: an id is
    /// never handed out twice in a session.
    next_id: LoadedLogId,

    /// Shared by every log's layer chips: a colour means one filter across the
    /// session, whichever log added it.
    layer_color_slots: LayerColorSlots,

    map_matches: LogMatches,

    /// Raised by every path that can change what the map draws, including the
    /// ones handing out `&mut` to a log or its filters. Cleared by
    /// [`LoadedLogs::map_matches`], which rebuilds what it stands for.
    map_matches_stale: bool,

    /// The recording names used when the cached layers' display names were
    /// resolved. A name template change resolves other names, and the tooltips
    /// follow it.
    map_matches_recording_names: RecordingNames,
}

impl LoadedLogs {
    pub fn len(&self) -> usize {
        self.logs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.logs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LoadedLog> {
        self.logs.iter().map(|stored| &stored.log)
    }

    /// Every loaded log under the identity it was loaded with, in load order.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (LoadedLogId, &LoadedLog)> {
        self.logs.iter().map(|stored| (stored.id, &stored.log))
    }

    pub fn pending_initial_position_sources(&self) -> Vec<LoadedLogId> {
        let mut pending: Vec<_> = self
            .logs
            .iter()
            .filter(|stored| {
                matches!(
                    stored.log.context,
                    LogContext::LooseImport {
                        source: PositionSourceState::PendingInitialSelection
                    }
                )
            })
            .collect();
        pending.sort_by(|left, right| {
            (left.log.name(), left.log.parsed().text().as_bytes())
                .cmp(&(right.log.name(), right.log.parsed().text().as_bytes()))
        });
        pending.into_iter().map(|stored| stored.id).collect()
    }

    /// The log that loaded first, which is what the viewer falls back to when
    /// the log it was showing unloads.
    pub fn first_id(&self) -> Option<LoadedLogId> {
        self.logs.first().map(|stored| stored.id)
    }

    pub fn id_of_loose_import(&self, content_hash: LogContentHash) -> Option<LoadedLogId> {
        self.logs
            .iter()
            .find(|stored| {
                stored.log.context_origin() == LogContextOrigin::LooseImport
                    && stored.log.content_hash() == content_hash
            })
            .map(|stored| stored.id)
    }

    pub fn push(&mut self, mut log: LoadedLog) -> LogPushOutcome {
        let duplicate = match &log.context {
            LogContext::SavedAttachment(state) => self.id_of_attachment(&state.reference),
            LogContext::LooseImport { .. } => self.id_of_loose_import(log.content_hash()),
            LogContext::DetachedAttachment { .. } => None,
        };
        if let Some(loaded) = duplicate {
            return LogPushOutcome::AlreadyLoaded(loaded);
        }
        if let Some(document) =
            self.logs
                .iter()
                .map(|stored| &stored.log.document)
                .find(|document| {
                    document.content_hash == log.content_hash()
                        && document.parsed == log.document.parsed
                })
        {
            log.document = Arc::clone(document);
            log.filters
                .share_equal_parsed_log(Arc::clone(&document.parsed));
        }
        log.filters
            .take_layer_color_slots(&mut self.layer_color_slots);
        let id = self.next_id;
        self.logs.push(StoredLog { id, log });
        self.next_id = self.next_id.next();
        self.map_matches_stale = true;
        LogPushOutcome::NewlyLoaded(id)
    }

    pub fn get_by_id(&self, id: LoadedLogId) -> Option<&LoadedLog> {
        self.logs
            .iter()
            .find(|stored| stored.id == id)
            .map(|stored| &stored.log)
    }

    pub fn get_mut_by_id(&mut self, id: LoadedLogId) -> Option<LoadedLogEditor<'_>> {
        self.map_matches_stale = true;
        self.logs
            .iter_mut()
            .find(|stored| stored.id == id)
            .map(|stored| LoadedLogEditor {
                log: &mut stored.log,
            })
    }

    pub fn restore_attachment(
        &mut self,
        mut log: LoadedLog,
        attachment: LogAttachmentRef,
        stored_filters: StoredLogFilterStack,
        recordings: &LoadedFilesView<'_>,
    ) -> LogPushOutcome {
        if let Some(id) = self.id_of_attachment(&attachment) {
            return LogPushOutcome::AlreadyLoaded(id);
        }
        log.restore_attachment(attachment, stored_filters, recordings);
        self.push(log)
    }

    pub fn save_attachment(
        &mut self,
        id: LoadedLogId,
        attachment: LogAttachmentRef,
        stored_filters: StoredLogFilterStack,
        recordings: &LoadedFilesView<'_>,
    ) -> LogSaveOutcome {
        if let Some(existing) = self.id_of_attachment(&attachment) {
            return LogSaveOutcome::AlreadyLoaded(existing);
        }
        let Some(stored) = self.logs.iter_mut().find(|stored| stored.id == id) else {
            return LogSaveOutcome::LogUnloaded;
        };
        if stored.log.attachment().is_some() {
            return LogSaveOutcome::AlreadySaved;
        }
        stored
            .log
            .record_attachment(attachment, stored_filters, recordings);
        self.map_matches_stale = true;
        LogSaveOutcome::Saved(id)
    }

    /// Retains the removed attachment's context independently of loose imports.
    pub fn forget_attachment(&mut self, attachment: &LogAttachmentRef) {
        if let Some(stored) = self
            .logs
            .iter_mut()
            .find(|stored| stored.log.attachment() == Some(attachment))
        {
            stored.log.forget_attachment();
            self.map_matches_stale = true;
        }
    }

    pub fn id_of_attachment(&self, attachment: &LogAttachmentRef) -> Option<LoadedLogId> {
        self.logs
            .iter()
            .find(|stored| stored.log.attachment() == Some(attachment))
            .map(|stored| stored.id)
    }

    pub fn any_loaded_log_holds(&self, attachment: &LogAttachmentRef) -> bool {
        self.id_of_attachment(attachment).is_some()
    }

    /// Every loaded log anchored to one of `recording_keys`, in load order.
    pub fn anchored_to<'a>(
        &'a self,
        recording_keys: &'a [RecordingKey],
    ) -> impl Iterator<Item = (LoadedLogId, &'a LoadedLog)> {
        self.logs
            .iter()
            .filter(|stored| {
                recording_keys
                    .iter()
                    .any(|key| stored.log.is_anchored_to(key))
            })
            .map(|stored| (stored.id, &stored.log))
    }

    /// Unloads every log anchored to one of `recording_keys`, freeing the
    /// colour slots their layer chips held.
    pub fn unload_anchored_to(&mut self, recording_keys: &[RecordingKey]) -> Vec<LoadedLog> {
        let unloading: Vec<LoadedLogId> =
            self.anchored_to(recording_keys).map(|(id, _)| id).collect();
        unloading
            .into_iter()
            .filter_map(|id| self.remove_by_id(id))
            .collect()
    }

    /// Unloads the log `id` names, freeing the colour slots its layer chips
    /// held.
    pub fn remove_by_id(&mut self, id: LoadedLogId) -> Option<LoadedLog> {
        let index = self.logs.iter().position(|stored| stored.id == id)?;
        let removed = self.logs.remove(index);
        removed
            .log
            .filters
            .release_layer_color_slots(&mut self.layer_color_slots);
        self.map_matches_stale = true;
        Some(removed.log)
    }

    /// The filter stack of the log `id` names, with the palette its layer chips
    /// take their colours from.
    pub fn filter_stack_mut_by_id(
        &mut self,
        id: LoadedLogId,
    ) -> Option<(&mut FilterStack, &mut LayerColorSlots)> {
        self.map_matches_stale = true;
        let stored = self.logs.iter_mut().find(|stored| stored.id == id)?;
        Some((&mut stored.log.filters, &mut self.layer_color_slots))
    }

    pub fn layer_color_slots(&self) -> &LayerColorSlots {
        &self.layer_color_slots
    }

    /// Reads in every log's finished filter scans. The viewer calls this once a
    /// frame, before it reads what the filters matched.
    pub fn apply_finished_queries(&mut self) {
        for stored in &mut self.logs {
            self.map_matches_stale |= stored.log.filters.apply_finished_queries();
        }
    }

    /// What the shown logs' filters put on the map, rebuilt only after
    /// something changed what that is.
    ///
    /// The added filters draw first, in the order their colours were handed
    /// out, and the live filters over them: the filter being typed is what the
    /// user is doing right now.
    pub fn map_matches(
        &mut self,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
    ) -> &LogMatches {
        if mem::take(&mut self.map_matches_stale)
            || self.map_matches_recording_names != *recording_names
        {
            self.map_matches = self.build_map_matches(recordings, recording_names);
            self.map_matches_recording_names = recording_names.clone();
        }
        &self.map_matches
    }

    fn build_map_matches(
        &self,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
    ) -> LogMatches {
        let display_names = self.map_display_names(recordings, recording_names);
        let shown = || {
            self.logs
                .iter()
                .zip(&display_names)
                .filter(|(stored, _)| stored.log.visible)
        };
        let mut layers = Vec::new();
        for (stored, display_name) in shown() {
            for (slot, chip) in stored.log.filters.enabled_layer_chips() {
                layers.push(LogMatchLayer {
                    color: LogMatchColor::LayerSlot {
                        index: slot.index(),
                        shared: self.layer_color_slots.is_shared(slot),
                    },
                    log: stored.source(display_name.clone()),
                    matches: stored.log.matched_points(chip.matches()),
                });
            }
        }
        for (stored, display_name) in shown() {
            layers.push(LogMatchLayer {
                color: LogMatchColor::LiveFilter,
                log: stored.source(display_name.clone()),
                matches: stored
                    .log
                    .matched_points(stored.log.filters.live_filter_matches()),
            });
        }
        layers.retain(|layer| !layer.matches.is_empty());
        LogMatches::from_layers(layers)
    }

    /// What a hexagon's tooltip identifies each loaded log by, in load order:
    /// the log's name, with the recording it takes its positions from after a
    /// middle dot where another loaded log has the same name.
    ///
    /// A session of one log gets `None`: its hexagons can belong to no other
    /// log.
    fn map_display_names(
        &self,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
    ) -> Vec<Option<String>> {
        if self.logs.len() < 2 {
            return vec![None; self.logs.len()];
        }
        self.logs
            .iter()
            .map(|stored| {
                let name = stored.log.name();
                let shares_its_name = self
                    .logs
                    .iter()
                    .filter(|other| other.log.name() == name)
                    .count()
                    > 1;
                let recording = shares_its_name
                    .then(|| stored.log.associated_recording())
                    .flatten()
                    .and_then(|recording| {
                        recording_names.display_name_of_loaded_recording(recordings, recording)
                    });
                Some(match recording {
                    Some(recording) => format!("{name} {MIDDLE_DOT} {recording}"),
                    None => name.to_owned(),
                })
            })
            .collect()
    }

    /// The filter-stack edits the attached logs have yet to be written, one
    /// entry per log whose stack the database no longer holds.
    pub fn take_filter_stack_edits_to_store(
        &mut self,
    ) -> Vec<(LogAttachmentRef, StoredLogFilterStack)> {
        self.logs
            .iter_mut()
            .filter_map(|stored| stored.log.take_filter_stack_edits_to_store())
            .collect()
    }

    /// Associates every loaded log again, after the loaded recordings changed.
    pub fn reassociate_all(&mut self, recordings: &LoadedFilesView<'_>) {
        for stored in &mut self.logs {
            stored.log.reassociate(recordings);
        }
        self.map_matches_stale = true;
    }
}

fn name_from_first_anchored_entry(parsed: &ParsedLog) -> String {
    match parsed.first_anchored_timestamp() {
        Some(time) => format!(
            "{UNNAMED_LOG_NAME_PREFIX} {}",
            time.format(UNNAMED_LOG_NAME_TIME_FORMAT)
        ),
        None => UNNAMED_LOG_NAME_PREFIX.to_owned(),
    }
}

/// How a log that arrived without a filename is named, followed by the time of
/// its first anchored entry.
const UNNAMED_LOG_NAME_PREFIX: &str = "pasted";

const UNNAMED_LOG_NAME_TIME_FORMAT: &str = "%H:%M:%S";

#[cfg(test)]
mod tests;
