//! The loaded logs of a session and each log's association state.

use std::{fmt::Write as _, mem, sync::Arc};

use chrono::Duration;
use gt_fmt::MIDDLE_DOT;
use gt_loaded_files::{LoadedFileId, LoadedFilesView};
use gt_logfile::ParsedLog;
use gt_types::{Latitude, Longitude, MercPoint, TimeRange, mercator};
use gt_ui_types::{LogMatchColor, LogMatchLayer, LogMatches};

use crate::{
    association::AssociationCandidates,
    filter::{EntryMatches, FilterStack, LayerColorSlots},
};

/// How a log that arrived without a filename is named, followed by the time of
/// its first anchored entry.
const UNNAMED_LOG_NAME_PREFIX: &str = "pasted";

const UNNAMED_LOG_NAME_TIME_FORMAT: &str = "%H:%M:%S";

/// One loaded log: the text it was parsed from, the recording it is associated
/// against, the filters over it, and whether it draws on the map.
#[derive(Debug)]
pub struct LoadedLog {
    name: String,

    /// Shared with the workers scanning the log for its filters.
    parsed: Arc<ParsedLog>,

    entry_time_range: Option<TimeRange>,
    association: Association,
    filters: FilterStack,
    visible: bool,
}

/// What a log is associated against, and what that association produced.
#[derive(Debug)]
struct Association {
    target: Option<LoadedFileId>,
    window: Duration,

    /// One slot per entry of the log, in entry order. Empty while the log has
    /// no association target.
    entry_positions: Vec<Option<(Latitude, Longitude)>>,

    associated_entry_count: usize,
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
            entry_time_range: parsed.time_range(),
            filters: FilterStack::new(Arc::clone(&parsed)),
            parsed,
            association: Association {
                target: None,
                window: association_window,
                entry_positions: Vec::new(),
                associated_entry_count: 0,
            },
            visible: true,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parsed(&self) -> &ParsedLog {
        &self.parsed
    }

    /// The filters over this log. Mutating them goes through
    /// [`LoadedLogs::filter_stack_mut`], which hands out the palette slots the
    /// layer chips share with every other log.
    pub fn filters(&self) -> &FilterStack {
        &self.filters
    }

    /// The one-line parse summary the viewer shows beside the log's name:
    /// detected format, entry count with the interpolated portion, boot count,
    /// and how many entries took no position.
    pub fn parse_summary_line(&self) -> String {
        let entries = self.parsed.entries().len();
        let interpolated = self.parsed.interpolated_entry_count();
        let boots = self.parsed.boot_sessions().len();
        let unassociated = self.unassociated_entry_count();

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

    pub fn association_target(&self) -> Option<LoadedFileId> {
        self.association.target
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

    /// Points the log at `target` and associates every entry against it, or
    /// drops the association when `target` is `None`.
    pub fn associate_with(
        &mut self,
        target: Option<LoadedFileId>,
        recordings: &LoadedFilesView<'_>,
    ) {
        self.association.target = target;
        self.reassociate(recordings);
    }

    /// Sets how far an entry may be from a fix to take its position, and
    /// associates the log again under the new window.
    pub fn set_association_window(&mut self, window: Duration, recordings: &LoadedFilesView<'_>) {
        self.association.window = window;
        self.reassociate(recordings);
    }

    /// Associates every entry against the log's target again, after the loaded
    /// recordings changed.
    ///
    /// A target that is no longer loaded clears the association: no log ever
    /// re-anchors to another recording without being pointed at it.
    pub fn reassociate(&mut self, recordings: &LoadedFilesView<'_>) {
        let Some(target) = self.association.target else {
            self.clear_entry_positions();
            return;
        };
        let Some(recording) = recordings.entry_for_id(target) else {
            log::info!(
                "Log {:?} lost its association target: that recording is no longer loaded",
                self.name
            );
            self.association.target = None;
            self.clear_entry_positions();
            return;
        };
        let entry_positions = gt_logfile::associate_entries(
            self.parsed.entries(),
            &recording.nav_points(),
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

    /// Where on the map the entries `matches` selected are. Each position
    /// comes from the recording this log is associated against, so an entry
    /// with no fix inside the association window contributes nothing.
    fn matched_positions(&self, matches: &EntryMatches) -> Vec<MercPoint> {
        matches
            .matched_entry_indices()
            .filter_map(|entry_index| self.entry_position(entry_index))
            .map(|(latitude, longitude)| mercator::normalize(latitude, longitude))
            .collect()
    }

    fn clear_entry_positions(&mut self) {
        self.association.entry_positions = Vec::new();
        self.association.associated_entry_count = 0;
    }
}

/// Every log loaded in this session, in load order. The same log may be loaded
/// more than once: the copies are separate logs.
#[derive(Debug, Default)]
pub struct LoadedLogs {
    logs: Vec<LoadedLog>,

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

    pub fn iter(&self) -> std::slice::Iter<'_, LoadedLog> {
        self.logs.iter()
    }

    pub fn get(&self, index: usize) -> Option<&LoadedLog> {
        self.logs.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut LoadedLog> {
        self.map_matches_stale = true;
        self.logs.get_mut(index)
    }

    /// Loads `log`, taking a colour slot for each layer chip it arrives with.
    pub fn push(&mut self, mut log: LoadedLog) {
        log.filters
            .take_layer_color_slots(&mut self.layer_color_slots);
        self.logs.push(log);
        self.map_matches_stale = true;
    }

    /// Unloads the log at `index`, freeing the colour slots its layer chips
    /// held.
    pub fn remove(&mut self, index: usize) -> Option<LoadedLog> {
        let removed = (index < self.logs.len()).then(|| self.logs.remove(index))?;
        removed
            .filters
            .release_layer_color_slots(&mut self.layer_color_slots);
        self.map_matches_stale = true;
        Some(removed)
    }

    /// The filter stack of the log at `index`, with the palette its layer chips
    /// take their colours from.
    pub fn filter_stack_mut(
        &mut self,
        index: usize,
    ) -> Option<(&mut FilterStack, &mut LayerColorSlots)> {
        self.map_matches_stale = true;
        let log = self.logs.get_mut(index)?;
        Some((&mut log.filters, &mut self.layer_color_slots))
    }

    pub fn layer_color_slots(&self) -> &LayerColorSlots {
        &self.layer_color_slots
    }

    /// Reads in every log's finished filter scans. The viewer calls this once a
    /// frame, before it reads what the filters matched.
    pub fn apply_finished_queries(&mut self) {
        for log in &mut self.logs {
            self.map_matches_stale |= log.filters.apply_finished_queries();
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
        let shown = || self.logs.iter().filter(|log| log.visible);
        let mut layers = Vec::new();
        for log in shown() {
            for (slot, chip) in log.filters.enabled_layer_chips() {
                layers.push(LogMatchLayer {
                    color: LogMatchColor::LayerSlot {
                        index: slot.index(),
                        shared: self.layer_color_slots.is_shared(slot),
                    },
                    positions: log.matched_positions(chip.matches()),
                });
            }
        }
        for log in shown() {
            layers.push(LogMatchLayer {
                color: LogMatchColor::LiveFilter,
                positions: log.matched_positions(log.filters.live_filter_matches()),
            });
        }
        layers.retain(|layer| !layer.positions.is_empty());
        LogMatches::from_layers(layers)
    }

    /// Associates every loaded log again, after the loaded recordings changed.
    pub fn reassociate_all(&mut self, recordings: &LoadedFilesView<'_>) {
        for log in &mut self.logs {
            log.reassociate(recordings);
        }
        self.map_matches_stale = true;
    }
}

impl<'a> IntoIterator for &'a LoadedLogs {
    type Item = &'a LoadedLog;
    type IntoIter = std::slice::Iter<'a, LoadedLog>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
    use crate::{
        LayerColorSlot,
        test_fixtures::{
            association_window, id_of, loaded, log_of, parsed_log, recording_at, start,
        },
    };

    use super::*;

    /// Waits for every log's filter scans, as the viewer's per-frame polling
    /// does once they land.
    fn wait_for_scans(logs: &mut LoadedLogs) {
        for index in 0..logs.len() {
            if let Some((stack, _)) = logs.filter_stack_mut(index) {
                stack.wait_for_queries();
            }
        }
    }

    /// Adds the live filter of the log at `index` as a layer chip, and answers
    /// with the palette colour that chip took.
    fn add_layer_chip(logs: &mut LoadedLogs, index: usize, text: &str) -> Option<usize> {
        let (stack, slots) = logs.filter_stack_mut(index)?;
        stack.set_live_filter_text(text);
        let id = stack.add_live_filter_as_chip(slots)?;
        stack.chip(id)?.layer_slot().map(LayerColorSlot::index)
    }

    /// The palette colour the first chip of the log at `index` draws in.
    fn first_chip_slot(logs: &LoadedLogs, index: usize) -> Option<usize> {
        logs.get(index)?
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

        log.associate_with(Some(id_of(&files, 1)), &files.view());

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
        log.associate_with(Some(id_of(&files, 0)), &files.view());
        assert_eq!(log.associated_entry_count(), 4);
        assert_eq!(log.unassociated_entry_count(), 6);

        log.set_association_window(Duration::seconds(60), &files.view());
        assert_eq!(log.associated_entry_count(), 10);
        assert_eq!(log.unassociated_entry_count(), 0);
    }

    /// Unloading the target strands the log, and never hands it to the other
    /// loaded recording.
    #[test]
    fn removing_the_target_clears_the_association_without_re_anchoring() {
        let mut files = loaded(vec![recording_at(55.0, 10), recording_at(60.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        log.associate_with(Some(id_of(&files, 1)), &files.view());
        logs.push(log);

        files.remove_file(1);
        logs.reassociate_all(&files.view());

        let log = logs.get(0).expect("the log stays loaded");
        assert_eq!(log.association_target(), None);
        assert_eq!(log.associated_entry_count(), 0);
        assert_eq!(log.entry_position(0), None);
    }

    #[test]
    fn the_summary_names_the_format_the_counts_and_what_took_no_position() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut log = log_of(10);

        assert_eq!(
            log.parse_summary_line(),
            "ISO 8601 · 10 entries · 1 boot · 10 unassociated"
        );

        log.associate_with(Some(id_of(&files, 0)), &files.view());

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
        log.associate_with(Some(id_of(&files, 0)), &files.view());
        logs.push(log);

        add_layer_chip(&mut logs, 0, "entry 1");
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
        assert_eq!(matches.position_count(), 1, "\"entry 1\" matches one line");
    }

    /// A log with no association target has nothing to put on the map, however
    /// much its filters match.
    #[test]
    fn an_unassociated_log_draws_nothing() {
        let mut logs = LoadedLogs::default();
        logs.push(log_of(10));

        add_layer_chip(&mut logs, 0, "entry");
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
        log.associate_with(Some(id_of(&files, 0)), &files.view());
        logs.push(log);
        add_layer_chip(&mut logs, 0, "entry");
        wait_for_scans(&mut logs);
        assert_eq!(logs.map_matches().position_count(), 10);

        if let Some(log) = logs.get_mut(0) {
            log.set_visible(false);
        }
        assert!(logs.map_matches().is_empty());

        if let Some(log) = logs.get_mut(0) {
            log.set_visible(true);
        }
        assert_eq!(logs.map_matches().position_count(), 10);
    }

    /// A refine chip narrows the table, never the map: it has no colour to draw
    /// in. The live filter draws over the chips that were added.
    #[test]
    fn the_map_holds_the_layer_chips_and_the_live_filter_over_them() {
        let files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        log.associate_with(Some(id_of(&files, 0)), &files.view());
        logs.push(log);
        add_layer_chip(&mut logs, 0, "entry 2");
        if let Some((stack, slots)) = logs.filter_stack_mut(0) {
            stack.set_live_filter_text("entry 3");
            let refined = stack.add_live_filter_as_chip(slots);
            if let Some(id) = refined {
                stack.switch_chip_to_refine_mode(id, slots);
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
        for _ in 0..2 {
            let mut log = log_of(10);
            log.associate_with(Some(id_of(&files, 0)), &files.view());
            logs.push(log);
        }
        add_layer_chip(&mut logs, 0, "entry 1");
        add_layer_chip(&mut logs, 1, "entry");
        wait_for_scans(&mut logs);
        assert_eq!(logs.map_matches().position_count(), 11);

        logs.remove(1);

        assert_eq!(
            logs.map_matches().position_count(),
            1,
            "only the log still loaded draws"
        );
    }

    /// Losing the association target strands the log's layers: nothing draws
    /// where no fix says it was.
    #[test]
    fn re_association_after_a_recording_is_unloaded_empties_the_map() {
        let mut files = loaded(vec![recording_at(55.0, 10)]);
        let mut logs = LoadedLogs::default();
        let mut log = log_of(10);
        log.associate_with(Some(id_of(&files, 0)), &files.view());
        logs.push(log);
        add_layer_chip(&mut logs, 0, "entry");
        wait_for_scans(&mut logs);
        assert_eq!(logs.map_matches().position_count(), 10);

        files.remove_file(0);
        logs.reassociate_all(&files.view());

        assert!(logs.map_matches().is_empty());
    }

    /// The second log's first layer chip takes the colour after the first
    /// log's: a colour means one filter across the session.
    #[test]
    fn layer_colours_are_handed_out_across_every_loaded_log() {
        let mut logs = LoadedLogs::default();
        logs.push(log_of(3));
        logs.push(log_of(3));

        assert_eq!(add_layer_chip(&mut logs, 0, "entry 0"), Some(0));
        assert_eq!(add_layer_chip(&mut logs, 1, "entry 1"), Some(1));

        logs.remove(0);

        assert_eq!(
            add_layer_chip(&mut logs, 0, "entry 2"),
            Some(0),
            "unloading a log frees the colours its chips held"
        );
    }

    /// A log hands its colours back when it is unloaded, and takes them anew
    /// when it is loaded again with the chips it kept.
    #[test]
    fn a_log_loaded_again_takes_colours_for_the_chips_it_kept() {
        let mut logs = LoadedLogs::default();
        logs.push(log_of(3));
        logs.push(log_of(3));
        add_layer_chip(&mut logs, 0, "entry 0");
        add_layer_chip(&mut logs, 1, "entry 1");

        let unloaded = logs.remove(0).expect("the log is loaded");
        logs.push(unloaded);

        assert_eq!(
            first_chip_slot(&logs, 0),
            Some(1),
            "the log that stayed loaded keeps the colour it had"
        );
        assert_eq!(
            first_chip_slot(&logs, 1),
            Some(0),
            "the colour the unloaded log freed is the lowest one free again"
        );
    }

    /// The same log may be loaded twice: the two are separate logs with
    /// separate targets, visibility, and lifetimes.
    #[test]
    fn the_same_log_loaded_twice_stays_two_independent_logs() {
        let files = loaded(vec![recording_at(55.0, 10), recording_at(60.0, 10)]);
        let mut logs = LoadedLogs::default();
        logs.push(log_of(10));
        logs.push(log_of(10));

        if let Some(first) = logs.get_mut(0) {
            first.associate_with(Some(id_of(&files, 0)), &files.view());
            first.set_visible(false);
        }
        if let Some(second) = logs.get_mut(1) {
            second.associate_with(Some(id_of(&files, 1)), &files.view());
        }

        assert_eq!(logs.len(), 2);
        assert_eq!(
            logs.get(0).map(LoadedLog::is_visible),
            Some(false),
            "hiding one log leaves the other shown"
        );
        assert_eq!(logs.get(1).map(LoadedLog::is_visible), Some(true));

        logs.remove(0);

        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs.get(0).and_then(LoadedLog::association_target),
            Some(id_of(&files, 1)),
            "removing a log leaves the remaining one associated as it was"
        );
    }
}
