//! The loaded logs of a session and each log's association state.

use std::{fmt::Write as _, mem, sync::Arc};

use chrono::Duration;
use gt_fmt::MIDDLE_DOT;
use gt_history_types::{LogContentHash, StoredLogFilter};
use gt_loaded_files::{LoadedFileId, LoadedFilesView};
use gt_logfile::ParsedLog;
use gt_types::{Latitude, Longitude, TimeRange, mercator};
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
    /// names no loaded recording.
    entry_positions: Vec<Option<(Latitude, Longitude)>>,

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
                entry_positions: Vec::new(),
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

    /// The position of the entry at `entry_index` of [`ParsedLog::entries`],
    /// `None` for an entry with no fix inside the association window.
    pub fn entry_position(&self, entry_index: usize) -> Option<(Latitude, Longitude)> {
        self.association
            .entry_positions
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

    /// Anchors the log to the recording `recording_key` names and associates
    /// every entry against it, keeping the attachment the log is stored as.
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

    /// Anchors the log to the loaded recording `chosen` names, and removes its
    /// anchor when `chosen` is `None`: what a choice among the loaded
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
        self.clear_entry_positions();
    }

    /// Sets how far an entry may be from a fix to take its position, and
    /// associates the log again under the new window.
    pub fn set_association_window(&mut self, window: Duration, recordings: &LoadedFilesView<'_>) {
        self.association.window = window;
        self.reassociate(recordings);
    }

    /// Associates every entry against the recording the anchor names, after the
    /// loaded recordings changed.
    ///
    /// An anchor naming no loaded recording leaves the entries without a
    /// position and stays: no log ever re-anchors to another recording without
    /// being pointed at it.
    pub fn reassociate(&mut self, recordings: &LoadedFilesView<'_>) {
        let anchored = match &self.anchor {
            LogAnchor::None => None,
            LogAnchor::Recording { key, .. } => key.loaded_recording(recordings),
        };
        let Some(recording) = anchored else {
            self.clear_entry_positions();
            return;
        };
        self.association.recording = Some(recording.id());
        let entry_positions = gt_logfile::associate_entries(
            self.parsed.entries(),
            &recording.placed_points(),
            self.association.window,
        );
        self.association.associated_entry_count = entry_positions
            .iter()
            .filter(|position| position.is_some())
            .count();
        self.association.entry_positions = entry_positions;
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

    /// Where on the map the entries `matches` selected are, in file order.
    /// Each position comes from the recording this log is associated against,
    /// so an entry with no fix inside the association window contributes
    /// nothing.
    fn matched_points(&self, matches: &EntryMatches) -> Vec<LogMatch> {
        matches
            .matched_entry_indices()
            .filter_map(|entry_index| {
                let (latitude, longitude) = self.entry_position(entry_index)?;
                Some(LogMatch {
                    merc: mercator::normalize(latitude, longitude),
                    entry_index,
                })
            })
            .collect()
    }

    fn clear_entry_positions(&mut self) {
        self.association.entry_positions = Vec::new();
        self.association.associated_entry_count = 0;
        self.association.recording = None;
    }
}

/// One loaded log under the identity it was loaded with.
#[derive(Debug)]
struct StoredLog {
    id: LoadedLogId,
    log: LoadedLog,
}

impl StoredLog {
    /// What the map needs to read this log's hovered lines back: its identity
    /// and the parse its layers' entries index into.
    fn source(&self) -> LogMatchSource {
        LogMatchSource {
            id: self.id,
            parsed: Arc::clone(&self.log.parsed),
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
    /// Content already loaded is refused: the answer names the log the session
    /// holds it under, and `log` is dropped.
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
    /// out, and the live filters over them: the filter being typed is the
    /// answer to what the user is doing right now.
    pub fn map_matches(&mut self) -> &LogMatches {
        if mem::take(&mut self.map_matches_stale) {
            self.map_matches = self.build_map_matches();
        }
        &self.map_matches
    }

    fn build_map_matches(&self) -> LogMatches {
        let shown = || self.logs.iter().filter(|stored| stored.log.visible);
        let mut layers = Vec::new();
        for stored in shown() {
            for (slot, chip) in stored.log.filters.enabled_layer_chips() {
                layers.push(LogMatchLayer {
                    color: LogMatchColor::LayerSlot {
                        index: slot.index(),
                        shared: self.layer_color_slots.is_shared(slot),
                    },
                    log: stored.source(),
                    matches: stored.log.matched_points(chip.matches()),
                });
            }
        }
        for stored in shown() {
            layers.push(LogMatchLayer {
                color: LogMatchColor::LiveFilter,
                log: stored.source(),
                matches: stored
                    .log
                    .matched_points(stored.log.filters.live_filter_matches()),
            });
        }
        layers.retain(|layer| !layer.matches.is_empty());
        LogMatches::from_layers(layers)
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
mod tests {
    use gt_history_types::{DatabaseRef, LogAttachmentId, StoredLogFilterMode};
    use gt_loaded_files::LoadedFiles;

    use crate::{
        LayerColorSlot,
        test_fixtures::{
            anchor_to, association_window, id_of, key_of, loaded, log_of, log_of_service,
            parsed_log, recording_at, start, stored_in_history, stored_recording_ref,
        },
    };

    use super::*;

    fn attachment_ref() -> LogAttachmentRef {
        LogAttachmentRef {
            recording: DatabaseRef {
                identity: "nav-devkit-mk2".to_owned(),
                group_name: "2026-01-01T14-02-11".to_owned(),
            },
            id: LogAttachmentId::new_random(),
        }
    }

    /// Waits for every log's filter scans, as the viewer's per-frame polling
    /// does once they land.
    fn wait_for_scans(logs: &mut LoadedLogs) {
        let ids: Vec<LoadedLogId> = logs.iter_with_ids().map(|(id, _)| id).collect();
        for id in ids {
            if let Some((stack, _)) = logs.filter_stack_mut_by_id(id) {
                stack.wait_for_queries();
            }
        }
    }

    /// Adds the live filter of the log `id` names as a layer chip, and answers
    /// with the palette colour that chip took.
    fn add_layer_chip(logs: &mut LoadedLogs, id: LoadedLogId, text: &str) -> Option<usize> {
        let (stack, slots) = logs.filter_stack_mut_by_id(id)?;
        stack.set_live_filter_text(text);
        let chip = stack.add_live_filter_as_chip(slots)?;
        stack.chip(chip)?.layer_slot().map(LayerColorSlot::index)
    }

    /// The palette colour the first chip of the log `id` names draws in.
    fn first_chip_slot(logs: &LoadedLogs, id: LoadedLogId) -> Option<usize> {
        logs.get_by_id(id)?
            .filters()
            .chips()
            .first()?
            .layer_slot()
            .map(LayerColorSlot::index)
    }

    /// The single-target rule: two recordings run at the same time in different
    /// places, and the log takes its positions from the one it was pointed at.
    #[test]
    fn entries_take_their_position_from_the_chosen_recording_alone() {
        let files = loaded(vec![recording_at(55.0, 10), recording_at(60.0, 10)]);
        let mut log = log_of(10);

        anchor_to(&mut log, &files, 1);

        assert_eq!(log.associated_entry_count(), 10);
        let latitudes: Vec<f64> = (0..10)
            .filter_map(|entry| log.entry_position(entry))
            .map(|(lat, _)| lat.as_degrees())
            .collect();
        assert!(
            latitudes.iter().all(|lat| *lat >= 60.0),
            "every entry must land in the chosen recording, got {latitudes:?}"
        );
    }

    #[test]
    fn widening_the_window_associates_the_entries_the_narrower_one_missed() {
        let files = loaded(vec![recording_at(55.0, 3)]);
        let mut log = log_of(10);

        log.set_association_window(Duration::seconds(1), &files.view());
        anchor_to(&mut log, &files, 0);
        assert_eq!(log.associated_entry_count(), 4);
        assert_eq!(log.unassociated_entry_count(), 6);

        log.set_association_window(Duration::seconds(60), &files.view());
        assert_eq!(log.associated_entry_count(), 10);
        assert_eq!(log.unassociated_entry_count(), 0);
    }

    /// Unloading the anchored recording strands the log, and never hands it to
    /// the other loaded recording.
    #[test]
    fn unloading_the_anchored_recording_leaves_the_log_anchored_without_positions() {
        let mut files = loaded(vec![recording_at(55.0, 10), recording_at(60.0, 10)]);
        let anchored = key_of(&files, 1);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 1);
        let id = logs.push(log).id();

        files.remove_file(1);
        logs.reassociate_all(&files.view());

        let log = logs.get_by_id(id).expect("the log stays loaded");
        assert_eq!(log.anchor_key(), Some(&anchored));
        assert_eq!(log.associated_recording(), None);
        assert_eq!(log.associated_entry_count(), 0);
        assert_eq!(log.entry_position(0), None);
    }

    /// A recording in history is the same recording every time it is opened,
    /// however many session identities it goes through.
    #[test]
    fn a_log_anchored_to_a_stored_recording_associates_again_when_it_is_opened_again() {
        let db_ref = stored_recording_ref();
        let mut files = LoadedFiles::new();
        files.push(recording_at(55.0, 10), stored_in_history(&db_ref));
        let first_load = id_of(&files, 0);
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 0);
        assert_eq!(log.associated_entry_count(), 10);

        files.remove_file(0);
        log.reassociate(&files.view());
        assert_eq!(log.associated_entry_count(), 0);

        files.push(recording_at(55.0, 10), stored_in_history(&db_ref));
        log.reassociate(&files.view());

        assert_eq!(log.anchor_key(), Some(&RecordingKey::Stored(db_ref)));
        assert_eq!(log.associated_entry_count(), 10);
        assert_eq!(log.associated_recording(), Some(id_of(&files, 0)));
        assert_ne!(
            log.associated_recording(),
            Some(first_load),
            "the recording came back under a session identity of its own"
        );
    }

    /// The recording an attached log is stored with is the recording it takes
    /// its positions from, until the attachment is removed.
    #[test]
    fn an_attached_log_keeps_its_anchor() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 0);
        log.record_attachment(attachment_ref(), Vec::new(), &files.view());
        let anchored = log.anchor_key().cloned();

        log.remove_anchor();

        assert_eq!(log.anchor_key(), anchored.as_ref());

        log.forget_attachment();
        log.remove_anchor();

        assert_eq!(log.anchor_key(), None);
    }

    /// A recording leaving the session takes the logs anchored to it with it,
    /// and leaves the rest of the session's logs loaded.
    #[test]
    fn unloading_a_recording_unloads_the_logs_anchored_to_it() {
        let files = loaded(vec![recording_at(55.0, 10), recording_at(60.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut anchored = log_of(10);
        anchor_to(&mut anchored, &files, 0);
        let anchored = logs.push(anchored).id();
        let mut elsewhere = log_of_service("hal-powerd", 10);
        anchor_to(&mut elsewhere, &files, 1);
        let elsewhere = logs.push(elsewhere).id();
        add_layer_chip(&mut logs, anchored, "entry 1");
        add_layer_chip(&mut logs, elsewhere, "entry 2");

        let unloaded = logs.unload_anchored_to(&[key_of(&files, 0)]);

        assert_eq!(
            unloaded.iter().map(LoadedLog::name).collect::<Vec<_>>(),
            ["navsyncd.log"]
        );
        assert_eq!(
            logs.iter_with_ids().map(|(id, _)| id).collect::<Vec<_>>(),
            [elsewhere]
        );
        assert_eq!(
            first_chip_slot(&logs, elsewhere),
            Some(1),
            "the log that stayed keeps the colour it drew in"
        );
        assert_eq!(
            add_layer_chip(&mut logs, elsewhere, "entry 3"),
            Some(0),
            "the unloaded log handed its colour back"
        );
    }

    #[test]
    fn the_summary_names_the_format_the_counts_and_what_took_no_position() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut log = log_of(10);

        assert_eq!(
            log.parse_summary_line(),
            "ISO 8601 · 10 entries · 1 boot · 10 unassociated"
        );

        anchor_to(&mut log, &files, 0);

        assert_eq!(
            log.parse_summary_line(),
            "ISO 8601 · 10 entries · 1 boot · 0 unassociated"
        );
    }

    /// A log whose lines do not all carry a timestamp, across two boots.
    #[test]
    fn the_summary_states_how_many_entries_were_timestamped_from_their_neighbours() {
        let text = "\
2026-01-01 14:02:11 navsyncd: starting
  at 0x0000c3f4 in gnss_task+0x54
2026-01-01 14:02:13 navsyncd: fix acquired
--- Device reboot ---
2026-01-01 14:02:20 navsyncd: starting
";
        let parsed = gt_logfile::parse_log(text.into(), start()).expect("the log parses");
        let log = LoadedLog::new(
            Some("navsyncd.log".to_owned()),
            parsed,
            association_window(),
        );

        assert_eq!(
            log.parse_summary_line(),
            "ISO 8601 · 4 entries (1 interpolated) · 2 boots · 4 unassociated"
        );
    }

    #[test]
    fn a_log_that_arrived_without_a_filename_is_named_after_its_first_entry() {
        let log = LoadedLog::new(None, parsed_log(3), association_window());
        assert_eq!(log.name(), "pasted 14:02:11");
    }

    /// What the map draws for a log: the entries a chip matched, at the fixes
    /// they were associated to.
    #[test]
    fn a_layer_chip_puts_the_lines_it_matched_on_the_map() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 0);
        let id = logs.push(log).id();

        add_layer_chip(&mut logs, id, "entry 1");
        wait_for_scans(&mut logs);

        let matches = logs.map_matches();
        assert_eq!(
            matches.layers().len(),
            1,
            "the live filter is empty, so it draws nothing"
        );
        assert_eq!(
            matches.layers().first().map(|layer| layer.color),
            Some(LogMatchColor::LayerSlot {
                index: 0,
                shared: false,
            })
        );
        assert_eq!(matches.match_count(), 1, "\"entry 1\" matches one line");
    }

    /// A log anchored to no recording has nothing to put on the map, however
    /// much its filters match.
    #[test]
    fn an_unassociated_log_draws_nothing() {
        let mut logs = LoadedLogs::default();
        let id = logs.push(log_of(10)).id();

        add_layer_chip(&mut logs, id, "entry");
        wait_for_scans(&mut logs);

        assert!(logs.map_matches().is_empty());
    }

    /// The whole map contribution of a log switches off with the log, and comes
    /// back with it.
    #[test]
    fn hiding_a_log_takes_its_layers_off_the_map() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 0);
        let id = logs.push(log).id();
        add_layer_chip(&mut logs, id, "entry");
        wait_for_scans(&mut logs);
        assert_eq!(logs.map_matches().match_count(), 10);

        if let Some(log) = logs.get_mut_by_id(id) {
            log.set_visible(false);
        }
        assert!(logs.map_matches().is_empty());

        if let Some(log) = logs.get_mut_by_id(id) {
            log.set_visible(true);
        }
        assert_eq!(logs.map_matches().match_count(), 10);
    }

    /// A refine chip narrows the table, never the map: it has no colour to draw
    /// in. The live filter draws over the chips that were added.
    #[test]
    fn the_map_holds_the_layer_chips_and_the_live_filter_over_them() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 0);
        let id = logs.push(log).id();
        add_layer_chip(&mut logs, id, "entry 2");
        if let Some((stack, slots)) = logs.filter_stack_mut_by_id(id) {
            stack.set_live_filter_text("entry 3");
            let refined = stack.add_live_filter_as_chip(slots);
            if let Some(chip) = refined {
                stack.switch_chip_to_refine_mode(chip, slots);
            }
            stack.set_live_filter_text("entry");
        }
        wait_for_scans(&mut logs);

        let colors: Vec<LogMatchColor> = logs
            .map_matches()
            .layers()
            .iter()
            .map(|layer| layer.color)
            .collect();
        assert_eq!(
            colors,
            [
                LogMatchColor::LayerSlot {
                    index: 0,
                    shared: false,
                },
                LogMatchColor::LiveFilter,
            ]
        );
    }

    /// Unloading a log takes what it drew with it: the cached layers are what
    /// the logs still loaded selected.
    #[test]
    fn unloading_a_log_takes_its_layers_off_the_map() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut kept = log_of(10);
        anchor_to(&mut kept, &files, 0);
        let kept = logs.push(kept).id();
        let mut unloaded = log_of_service("hal-powerd", 10);
        anchor_to(&mut unloaded, &files, 0);
        let unloaded = logs.push(unloaded).id();
        add_layer_chip(&mut logs, kept, "entry 1");
        add_layer_chip(&mut logs, unloaded, "entry");
        wait_for_scans(&mut logs);
        assert_eq!(logs.map_matches().match_count(), 11);

        logs.remove_by_id(unloaded);

        assert_eq!(
            logs.map_matches().match_count(),
            1,
            "only the log still loaded draws"
        );
    }

    /// Unloading the anchored recording strands the log's layers: nothing draws
    /// where no fix says it was.
    #[test]
    fn re_association_after_a_recording_is_unloaded_empties_the_map() {
        let mut files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        anchor_to(&mut log, &files, 0);
        let id = logs.push(log).id();
        add_layer_chip(&mut logs, id, "entry");
        wait_for_scans(&mut logs);
        assert_eq!(logs.map_matches().match_count(), 10);

        files.remove_file(0);
        logs.reassociate_all(&files.view());

        assert!(logs.map_matches().is_empty());
    }

    /// Each layer names the log its filter read, which is how the map hands a
    /// hovered hexagon back to the viewer showing that log.
    #[test]
    fn every_layer_names_the_log_it_was_filtered_out_of() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut ids = Vec::new();
        for service in ["navsyncd", "hal-powerd"] {
            let mut log = log_of_service(service, 10);
            anchor_to(&mut log, &files, 0);
            ids.push(logs.push(log).id());
        }
        if let [first, second] = ids.as_slice() {
            add_layer_chip(&mut logs, *first, "entry 1");
            add_layer_chip(&mut logs, *second, "entry 2");
        }
        wait_for_scans(&mut logs);

        let layer_logs: Vec<LoadedLogId> = logs
            .map_matches()
            .layers()
            .iter()
            .map(|layer| layer.log.id)
            .collect();

        assert_eq!(layer_logs, ids, "one layer per log, each naming its own");
    }

    /// A hexagon of an unloaded log never names the log that took its place in
    /// the list: an identity is never handed out twice.
    #[test]
    fn an_unloaded_logs_identity_is_never_handed_out_again() {
        let mut logs = LoadedLogs::default();
        let unloaded = logs.push(log_of(3)).id();
        let kept = logs.push(log_of_service("hal-powerd", 3)).id();
        assert_ne!(unloaded, kept);

        logs.remove_by_id(unloaded);
        let loaded_after = logs.push(log_of_service("telemetryd", 3)).id();

        assert_eq!(
            logs.iter_with_ids().map(|(id, _)| id).collect::<Vec<_>>(),
            [kept, loaded_after],
            "the log that stayed keeps its identity, and the new one takes its own"
        );
        assert_ne!(loaded_after, unloaded);
    }

    /// The second log's first layer chip takes the colour after the first
    /// log's: a colour means one filter across the session.
    #[test]
    fn layer_colours_are_handed_out_across_every_loaded_log() {
        let mut logs = LoadedLogs::default();
        let first = logs.push(log_of(3)).id();
        let second = logs.push(log_of_service("hal-powerd", 3)).id();

        assert_eq!(add_layer_chip(&mut logs, first, "entry 0"), Some(0));
        assert_eq!(add_layer_chip(&mut logs, second, "entry 1"), Some(1));

        logs.remove_by_id(first);

        assert_eq!(
            add_layer_chip(&mut logs, second, "entry 2"),
            Some(0),
            "unloading a log frees the colours its chips held"
        );
    }

    /// A log hands its colours back when it is unloaded, and takes them anew
    /// when it is loaded again with the chips it kept.
    #[test]
    fn a_log_loaded_again_takes_colours_for_the_chips_it_kept() {
        let mut logs = LoadedLogs::default();
        let first = logs.push(log_of(3)).id();
        let second = logs.push(log_of_service("hal-powerd", 3)).id();
        add_layer_chip(&mut logs, first, "entry 0");
        add_layer_chip(&mut logs, second, "entry 1");

        let unloaded = logs.remove_by_id(first).expect("the log is loaded");
        let loaded_again = logs.push(unloaded).id();

        assert_eq!(
            first_chip_slot(&logs, second),
            Some(1),
            "the log that stayed loaded keeps the colour it had"
        );
        assert_eq!(
            first_chip_slot(&logs, loaded_again),
            Some(0),
            "the colour the unloaded log freed is the lowest one free again"
        );
    }

    /// An attachment is written again only for the edits the database has not
    /// seen, and the log stays loaded once the attachment is gone.
    #[test]
    fn an_attached_log_reports_the_filter_stack_edits_the_database_has_not_seen() {
        let attachment = attachment_ref();
        let recordings = loaded(Vec::new());
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        log.record_attachment(attachment.clone(), Vec::new(), &recordings.view());
        let id = logs.push(log).id();

        assert!(
            logs.take_filter_stack_edits_to_store().is_empty(),
            "the database holds the stack the log was attached with"
        );

        add_layer_chip(&mut logs, id, "entry 1");

        let edits = logs.take_filter_stack_edits_to_store();
        assert_eq!(
            edits
                .iter()
                .map(|(attachment, filters)| (attachment.id, filters.as_slice()))
                .collect::<Vec<_>>(),
            [(
                attachment.id,
                [StoredLogFilter {
                    text: "entry 1".to_owned(),
                    regex: false,
                    enabled: true,
                    mode: StoredLogFilterMode::Layer { color_slot: 0 },
                }]
                .as_slice()
            )],
            "the added chip is what the attachment has yet to be written"
        );
        assert!(
            logs.take_filter_stack_edits_to_store().is_empty(),
            "an edit that was written is not written again"
        );

        if let Some(log) = logs.get_mut_by_id(id) {
            log.forget_attachment();
        }
        assert_eq!(logs.len(), 1);
        assert_eq!(logs.get_by_id(id).and_then(LoadedLog::attachment), None);
    }

    /// A log restored from an attachment comes back with its chips, drawing in
    /// the colours it was stored with.
    #[test]
    fn a_restored_attachment_puts_back_the_stack_it_was_stored_with() {
        let stored = vec![
            StoredLogFilter {
                text: "entry 1".to_owned(),
                regex: false,
                enabled: true,
                mode: StoredLogFilterMode::Layer { color_slot: 2 },
            },
            StoredLogFilter {
                text: "entry".to_owned(),
                regex: false,
                enabled: false,
                mode: StoredLogFilterMode::Refine,
            },
        ];
        let attachment = attachment_ref();
        let recordings = loaded(Vec::new());
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        log.restore_attachment(attachment.clone(), stored.clone(), &recordings.view());
        let id = logs.push(log).id();
        wait_for_scans(&mut logs);

        let log = logs.get_by_id(id).expect("the restored log is loaded");
        assert_eq!(log.attachment(), Some(&attachment));
        assert_eq!(
            log.filters().to_stored_filters(),
            stored,
            "the restored stack is the stored one, colours and all"
        );
        assert!(
            logs.take_filter_stack_edits_to_store().is_empty(),
            "a stack that came back as it was stored needs no write-back"
        );
    }

    /// One log per content: a second copy of a text the session already holds
    /// is refused, whatever name it arrived under.
    #[test]
    fn pushing_content_that_is_already_loaded_answers_with_the_loaded_log() {
        let mut logs = LoadedLogs::default();
        let loaded = logs.push(log_of(10));

        let second = logs.push(LoadedLog::new(
            Some("copy-of-navsyncd.log".to_owned()),
            parsed_log(10),
            association_window(),
        ));

        assert_eq!(second, LogPushOutcome::AlreadyLoaded(loaded.id()));
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs.get_by_id(loaded.id()).map(LoadedLog::name),
            Some("navsyncd.log"),
            "the loaded log keeps the name it was loaded under"
        );
    }

    /// The refused copy leaves the loaded log as it was, chips and colour slots
    /// included.
    #[test]
    fn a_refused_copy_takes_no_colour_slot_from_the_loaded_log() {
        let mut logs = LoadedLogs::default();
        let id = logs.push(log_of(10)).id();
        add_layer_chip(&mut logs, id, "entry 1");

        let stored = vec![StoredLogFilter {
            text: "entry 2".to_owned(),
            regex: false,
            enabled: true,
            mode: StoredLogFilterMode::Layer { color_slot: 1 },
        }];
        let mut copy = log_of(10);
        copy.restore_attachment(attachment_ref(), stored, &loaded(Vec::new()).view());
        logs.push(copy);

        assert_eq!(first_chip_slot(&logs, id), Some(0));
        assert_eq!(
            logs.get_by_id(id).and_then(LoadedLog::attachment),
            None,
            "the loaded log took nothing from the copy that was refused"
        );
        assert_eq!(
            add_layer_chip(&mut logs, id, "entry 2"),
            Some(1),
            "the refused copy left the palette as the loaded log had it"
        );
    }
}
