//! The loaded logs of a session and each log's association state.

use std::fmt::Write as _;

use chrono::Duration;
use gt_fmt::MIDDLE_DOT;
use gt_loaded_files::{LoadedFileId, LoadedFilesView};
use gt_logfile::ParsedLog;
use gt_types::{Latitude, Longitude, TimeRange};

use crate::association::AssociationCandidates;

/// How a log that arrived without a filename is named, followed by the time of
/// its first anchored entry.
const UNNAMED_LOG_NAME_PREFIX: &str = "pasted";

const UNNAMED_LOG_NAME_TIME_FORMAT: &str = "%H:%M:%S";

/// One loaded log: the text it was parsed from, the recording it is associated
/// against, and whether it draws on the map.
#[derive(Debug)]
pub struct LoadedLog {
    name: String,
    parsed: ParsedLog,
    entry_time_range: Option<TimeRange>,
    association: Association,
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
        Self {
            name,
            entry_time_range: parsed.time_range(),
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
        self.logs.get_mut(index)
    }

    pub fn push(&mut self, log: LoadedLog) {
        self.logs.push(log);
    }

    /// Unloads the log at `index`.
    pub fn remove(&mut self, index: usize) -> Option<LoadedLog> {
        (index < self.logs.len()).then(|| self.logs.remove(index))
    }

    /// Associates every loaded log again, after the loaded recordings changed.
    pub fn reassociate_all(&mut self, recordings: &LoadedFilesView<'_>) {
        for log in &mut self.logs {
            log.reassociate(recordings);
        }
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
    use crate::test_fixtures::{
        association_window, id_of, loaded, log_of, parsed_log, recording_at, start,
    };

    use super::*;

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
