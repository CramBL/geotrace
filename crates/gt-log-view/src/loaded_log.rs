//! The loaded logs of a session and each log's association state.

use std::{fmt::Write as _, mem, sync::Arc};

use chrono::Duration;
use gt_fmt::MIDDLE_DOT;
use gt_history_types::{LogContentHash, StoredLogFilter};
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_logfile::{EntryPlacement, ParsedLog};
use gt_types::{TimeRange, mercator};
use gt_ui_types::{
    LoadedLogId, LogMatch, LogMatchColor, LogMatchLayer, LogMatchSource, LogMatches,
};

use crate::{
    anchor::{LogAnchor, RecordingKey},
    association::AssociationCandidates,
    attachment::{LogAttachmentRef, LogAttachmentState},
    filter::{EntryMatches, FilterStack, LayerColorSlots},
};

/// How a log that arrived without a filename is named, followed by the time of
/// its first anchored entry.
const UNNAMED_LOG_NAME_PREFIX: &str = "pasted";

const UNNAMED_LOG_NAME_TIME_FORMAT: &str = "%H:%M:%S";

/// One loaded log: the text it was parsed from, the recording it is anchored
/// to, the filters over it, and whether it draws on the map.
#[derive(Debug)]
pub struct LoadedLog {
    name: String,

    /// Shared with the workers scanning the log for its filters.
    parsed: Arc<ParsedLog>,

    /// Over the text the parse read, which is what tells this log apart from
    /// every other loaded one and from the attachments of a recording.
    content_hash: LogContentHash,

    entry_time_range: Option<TimeRange>,
    anchor: LogAnchor,
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
        let parsed = Arc::new(parsed);
        Self {
            name,
            content_hash: LogContentHash::of_log_bytes(parsed.text().as_bytes()),
            entry_time_range: parsed.time_range(),
            filters: FilterStack::new(Arc::clone(&parsed)),
            parsed,
            anchor: LogAnchor::None,
            association: Association {
                window: association_window,
                entry_placements: Vec::new(),
                associated_entry_count: 0,
                recording: None,
            },
            visible: true,
        }
    }

    /// Puts back the filter stack this log was stored with, and anchors it to
    /// the recording holding `attachment`.
    ///
    /// The restored layer chips take their palette slots when the log is
    /// loaded with [`LoadedLogs::push`].
    pub fn restore_attachment(
        &mut self,
        attachment: LogAttachmentRef,
        stored_filters: Vec<StoredLogFilter>,
        recordings: &LoadedFilesView<'_>,
    ) {
        self.filters = FilterStack::from_stored_filters(Arc::clone(&self.parsed), &stored_filters);
        self.record_attachment(attachment, stored_filters, recordings);
    }

    /// Notes that this log is now stored as `attachment`, holding
    /// `stored_filters`, and anchors it to the recording holding it.
    pub fn record_attachment(
        &mut self,
        attachment: LogAttachmentRef,
        stored_filters: Vec<StoredLogFilter>,
        recordings: &LoadedFilesView<'_>,
    ) {
        self.anchor = LogAnchor::Recording {
            key: RecordingKey::Stored(attachment.recording.clone()),
            attachment: Some(LogAttachmentState {
                reference: attachment,
                stored_filters,
            }),
        };
        self.reassociate(recordings);
    }

    /// Records `attachment` on this log. The log's text is assumed to already
    /// match what is stored under `attachment`.
    ///
    /// The log keeps the filter stack the user is reading it under.
    pub fn adopt_restored_attachment(
        &mut self,
        attachment: LogAttachmentRef,
        stored_filters: Vec<StoredLogFilter>,
        recordings: &LoadedFilesView<'_>,
    ) -> RestoredAttachmentAdoption {
        if self.attachment().is_some() {
            return RestoredAttachmentAdoption::AlreadyAttached;
        }
        if !self.is_anchored_to(&RecordingKey::Stored(attachment.recording.clone())) {
            return RestoredAttachmentAdoption::NotAnchoredToThatRecording;
        }
        self.record_attachment(attachment, stored_filters, recordings);
        RestoredAttachmentAdoption::Recorded
    }

    /// The attachment this log is stored as, `None` for one that lives only in
    /// this session.
    pub fn attachment(&self) -> Option<&LogAttachmentRef> {
        match &self.anchor {
            LogAnchor::None
            | LogAnchor::Recording {
                attachment: None, ..
            } => None,
            LogAnchor::Recording {
                attachment: Some(state),
                ..
            } => Some(&state.reference),
        }
    }

    /// Drops the attachment and leaves the log anchored to the recording that
    /// held it: what "Remove attachment" does once the database has removed it.
    pub fn forget_attachment(&mut self) {
        if let LogAnchor::Recording { attachment, .. } = &mut self.anchor {
            *attachment = None;
        }
    }

    /// The filter-stack edits this log's attachment has yet to be written,
    /// and `None` while the database holds the stack the user is looking at.
    pub fn take_filter_stack_edits_to_store(
        &mut self,
    ) -> Option<(LogAttachmentRef, Vec<StoredLogFilter>)> {
        let LogAnchor::Recording {
            attachment: Some(state),
            ..
        } = &mut self.anchor
        else {
            return None;
        };
        let filters = self.filters.to_stored_filters();
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
        &self.parsed
    }

    pub fn content_hash(&self) -> LogContentHash {
        self.content_hash
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
        let entries = self.parsed.entries().len();
        let interpolated = self.parsed.interpolated_entry_count();
        let boots = self.parsed.boot_sessions().len();
        let unassociated = self.unassociated_entry_count();
        let replaced_bytes = self.parsed.replaced_byte_count();

        let mut summary = format!(
            "{} {MIDDLE_DOT} {} {}",
            self.parsed.format().display_name(),
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
        self.entry_time_range
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// The recording this log is anchored to, `None` for a log that takes its
    /// positions from no recording.
    pub fn anchor_key(&self) -> Option<&RecordingKey> {
        match &self.anchor {
            LogAnchor::None => None,
            LogAnchor::Recording { key, .. } => Some(key),
        }
    }

    pub fn is_anchored_to(&self, recording_key: &RecordingKey) -> bool {
        self.anchor_key() == Some(recording_key)
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
        self.parsed
            .entries()
            .len()
            .saturating_sub(self.association.associated_entry_count)
    }

    /// Anchors the log to the recording `recording_key` identifies and
    /// associates every entry against it, keeping the attachment the log is
    /// stored as.
    pub fn anchor_to(&mut self, recording_key: RecordingKey, recordings: &LoadedFilesView<'_>) {
        match &mut self.anchor {
            LogAnchor::Recording { key, .. } => *key = recording_key,
            anchor @ LogAnchor::None => {
                *anchor = LogAnchor::Recording {
                    key: recording_key,
                    attachment: None,
                };
            }
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

    /// Takes the anchor off a log stored nowhere, leaving its entries without a
    /// position.
    ///
    /// An attached log keeps its anchor: a log stored with a recording takes
    /// its positions from one, until the attachment is removed.
    pub fn remove_anchor(&mut self) {
        if self.attachment().is_some() {
            log::warn!(
                "Kept the recording of the log {:?}: it is stored with a recording in history",
                self.name
            );
            return;
        }
        self.anchor = LogAnchor::None;
        self.clear_entry_placements();
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
        let anchored = match &self.anchor {
            LogAnchor::None => None,
            LogAnchor::Recording { key, .. } => key.loaded_recording(recordings),
        };
        let Some(recording) = anchored else {
            self.clear_entry_placements();
            return;
        };
        self.association.recording = Some(recording.id());
        let entry_placements = gt_logfile::associate_entries(
            self.parsed.entries(),
            &recording.addressed_fixes(),
            self.association.window,
        );
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
        match self.entry_time_range {
            Some(log_range) => AssociationCandidates::rank(log_range, recordings),
            None => AssociationCandidates::none(),
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

/// What [`LoadedLog::adopt_restored_attachment`] left the loaded log as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoredAttachmentAdoption {
    /// The log holds the attachment and the filter stack stored with it.
    Recorded,

    /// The log is anchored to another recording, or to none at all, and kept
    /// that anchor.
    NotAnchoredToThatRecording,

    /// The log already holds an attachment of its own, and kept it.
    AlreadyAttached,
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
            parsed: Arc::clone(&self.log.parsed),
            display_name,
        }
    }
}

/// What [`LoadedLogs::push`] did with the log it was handed. Both variants
/// name the log the session holds that content under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogPushOutcome {
    NewlyLoaded(LoadedLogId),

    /// A log with the same content hash was loaded already, and the pushed one
    /// was dropped.
    AlreadyLoaded(LoadedLogId),
}

impl LogPushOutcome {
    pub fn id(self) -> LoadedLogId {
        match self {
            Self::NewlyLoaded(id) | Self::AlreadyLoaded(id) => id,
        }
    }
}

/// Every log loaded in this session, in load order, one per content hash.
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

    /// The log that loaded first, which is what the viewer falls back to when
    /// the log it was showing unloads.
    pub fn first_id(&self) -> Option<LoadedLogId> {
        self.logs.first().map(|stored| stored.id)
    }

    /// The loaded log whose text hashes to `content_hash`, `None` while no
    /// loaded log holds that content.
    pub fn id_of_content(&self, content_hash: LogContentHash) -> Option<LoadedLogId> {
        self.logs
            .iter()
            .find(|stored| stored.log.content_hash == content_hash)
            .map(|stored| stored.id)
    }

    /// Loads `log` under a fresh identity, taking a colour slot for each layer
    /// chip it arrives with.
    ///
    /// Content already loaded is rejected: the outcome states which log the
    /// session holds it under, and `log` is dropped.
    pub fn push(&mut self, mut log: LoadedLog) -> LogPushOutcome {
        if let Some(loaded) = self.id_of_content(log.content_hash) {
            return LogPushOutcome::AlreadyLoaded(loaded);
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

    pub fn get_mut_by_id(&mut self, id: LoadedLogId) -> Option<&mut LoadedLog> {
        self.map_matches_stale = true;
        self.logs
            .iter_mut()
            .find(|stored| stored.id == id)
            .map(|stored| &mut stored.log)
    }

    pub fn any_loaded_log_holds(&self, attachment: &LogAttachmentRef) -> bool {
        self.logs
            .iter()
            .any(|stored| stored.log.attachment() == Some(attachment))
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
    ) -> Vec<(LogAttachmentRef, Vec<StoredLogFilter>)> {
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

#[cfg(test)]
mod tests;
