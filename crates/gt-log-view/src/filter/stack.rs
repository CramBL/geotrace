//! One log's filters: the live filter, and the chips added from it.

use std::{iter, mem, ops::Range, sync::Arc};

use gt_history_types::{StoredLogFilter, StoredLogFilterMode};
use gt_logfile::ParsedLog;

use crate::filter::{
    clock_ticks::ClockTicks,
    matches::{self, EntryMatches},
    pattern::{CompiledFilter, FilterPattern, InvalidFilterPattern},
    query::FilterQuery,
    slots::{LayerColorSlot, LayerColorSlots},
};

/// Identifies a chip for as long as it is in the stack it was added to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FilterChipId(u64);

/// What a chip does with the entries it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterChipMode {
    /// An independent overlay: its own colour on the map and a gutter bar on
    /// the rows it matches, without narrowing the table.
    Layer,

    /// A refinement of the table: only the entries it matches stay visible.
    Refine,
}

/// The entries the table shows, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisibleEntries {
    /// Every entry of the log: nothing narrows the table.
    All { entry_count: usize },

    /// What the live filter and every enabled refine chip all matched.
    Matching(Vec<usize>),
}

impl VisibleEntries {
    pub fn len(&self) -> usize {
        match self {
            Self::All { entry_count } => *entry_count,
            Self::Matching(entries) => entries.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Which entry of [`ParsedLog::entries`] the table's `row` shows.
    pub fn entry_index(&self, row: usize) -> Option<usize> {
        match self {
            Self::All { entry_count } => (row < *entry_count).then_some(row),
            Self::Matching(entries) => entries.get(row).copied(),
        }
    }

    pub fn entry_indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.len()).filter_map(|row| self.entry_index(row))
    }

    /// Which row shows the first entry at or after `entry_index`, or
    /// [`len`](Self::len) when this set holds no such entry.
    ///
    /// The table calls this with each boot session's bounds, to find how many
    /// rows of that session the filters left.
    pub fn row_at_or_after(&self, entry_index: usize) -> usize {
        match self {
            Self::All { entry_count } => entry_index.min(*entry_count),
            Self::Matching(entries) => entries.partition_point(|entry| *entry < entry_index),
        }
    }
}

/// The live filter and the chips of one log.
///
/// Editing a filter starts a scan of the log for it. The results of the scans
/// that finished are read in with
/// [`FilterStack::apply_finished_queries`](Self::apply_finished_queries).
#[derive(Debug)]
pub struct FilterStack {
    log: Arc<ParsedLog>,
    live: LogFilter,
    chips: Vec<FilterChip>,
    next_chip_id: u64,
    visible: VisibleEntries,
    clock_ticks: ClockTicks,
}

impl FilterStack {
    /// The unfiltered stack of a freshly loaded log.
    pub fn new(log: Arc<ParsedLog>) -> Self {
        let entry_count = log.entries().len();
        let visible = VisibleEntries::All { entry_count };
        let clock_ticks = ClockTicks::of(&log, &visible);
        Self {
            log,
            live: LogFilter::unwritten(entry_count),
            chips: Vec::new(),
            next_chip_id: 0,
            visible,
            clock_ticks,
        }
    }

    /// The stack an attachment's stored filters restore into, in the order
    /// they were stored. Each chip starts the scan that finds what it matches,
    /// as an added filter does.
    ///
    /// The layer chips carry the slots they were stored with as preferences:
    /// [`LoadedLogs::push`](crate::LoadedLogs::push) hands out the session's
    /// slots when the log is loaded.
    pub fn from_stored_filters(log: Arc<ParsedLog>, stored: &[StoredLogFilter]) -> Self {
        let mut stack = Self::new(log);
        for filter in stored {
            stack.push_stored_chip(filter);
        }
        stack.recompose_visible_entries();
        stack
    }

    /// The chips of this stack in the form an attachment stores them. The live
    /// filter is not part of it: it belongs to the field, not to the stack.
    pub fn to_stored_filters(&self) -> Vec<StoredLogFilter> {
        self.chips
            .iter()
            .map(FilterChip::to_stored_filter)
            .collect()
    }

    pub fn live_filter_text(&self) -> &str {
        &self.live.pattern.text
    }

    pub fn live_filter_is_regex(&self) -> bool {
        self.live.pattern.regex
    }

    /// What the regex engine said about a pattern it could not compile. The
    /// viewer shows it under the field, and leaves the table unfiltered.
    pub fn live_filter_error(&self) -> Option<&InvalidFilterPattern> {
        self.live.compiled.as_ref().err()
    }

    /// The entries the live filter matched, which the map draws in the colour
    /// reserved for it. Empty while the field is empty or its regex invalid.
    pub fn live_filter_matches(&self) -> &EntryMatches {
        self.live.query.matches()
    }

    /// Where in `message` the live filter matched, as byte ranges the table
    /// paints the live colour over: every occurrence of every term of a plain
    /// filter, and every match of a regex.
    pub fn live_filter_match_spans(&self, message: &str) -> Vec<Range<usize>> {
        match &self.live.compiled {
            Ok(compiled) => compiled.match_spans(message),
            Err(_) => Vec::new(),
        }
    }

    pub fn set_live_filter_text(&mut self, text: &str) {
        self.set_live_filter(FilterPattern {
            text: text.to_owned(),
            regex: self.live.pattern.regex,
        });
    }

    /// Switches the live filter between plain terms and a regex, keeping the
    /// text the user already wrote.
    pub fn set_live_filter_regex(&mut self, regex: bool) {
        self.set_live_filter(FilterPattern {
            text: self.live.pattern.text.clone(),
            regex,
        });
    }

    /// Empties the field, leaving the `.*` toggle as the user set it.
    pub fn clear_live_filter(&mut self) {
        self.set_live_filter_text("");
    }

    /// Whether the live filter can become a chip: the viewer grays "+ Add
    /// filter" out while an empty or invalid pattern leaves nothing to add.
    pub fn can_add_live_filter_as_chip(&self) -> bool {
        self.live.selects_entries()
    }

    /// Turns the live filter into a chip and clears the field.
    ///
    /// The chip keeps the filter's mode and the scan it already started:
    /// adding a filter never scans the log again.
    pub fn add_live_filter_as_chip(&mut self, slots: &mut LayerColorSlots) -> Option<FilterChipId> {
        if !self.can_add_live_filter_as_chip() {
            return None;
        }
        let id = FilterChipId(self.next_chip_id);
        self.next_chip_id = self.next_chip_id.saturating_add(1);
        let mut emptied = LogFilter::unwritten(self.log.entries().len());
        // The `.*` toggle belongs to the field and stays as the user set it.
        emptied.pattern.regex = self.live.pattern.regex;
        self.chips.push(FilterChip {
            id,
            filter: mem::replace(&mut self.live, emptied),
            layer_slot: Some(slots.allocate()),
            enabled: true,
        });
        self.recompose_visible_entries();
        Some(id)
    }

    /// The chips of this log, in the order they were added.
    pub fn chips(&self) -> &[FilterChip] {
        &self.chips
    }

    pub fn chip(&self, id: FilterChipId) -> Option<&FilterChip> {
        self.chips.iter().find(|chip| chip.id == id)
    }

    /// The layer chips drawing on the map right now, each with the palette slot
    /// it draws in.
    pub fn enabled_layer_chips(&self) -> impl Iterator<Item = (LayerColorSlot, &FilterChip)> {
        self.chips
            .iter()
            .filter(|chip| chip.enabled)
            .filter_map(|chip| Some((chip.layer_slot?, chip)))
    }

    /// Takes a chip out of the map and the table, keeping everything else about
    /// it, including its colour slot.
    pub fn set_chip_enabled(&mut self, id: FilterChipId, enabled: bool) {
        let Some(chip) = self.chips.iter_mut().find(|chip| chip.id == id) else {
            return;
        };
        if chip.enabled == enabled {
            return;
        }
        chip.enabled = enabled;
        self.recompose_visible_entries();
    }

    /// Gives the chip a colour of its own on the map, and stops it narrowing
    /// the table.
    pub fn switch_chip_to_layer_mode(&mut self, id: FilterChipId, slots: &mut LayerColorSlots) {
        let Some(chip) = self.chips.iter_mut().find(|chip| chip.id == id) else {
            return;
        };
        if chip.layer_slot.is_some() {
            return;
        }
        chip.layer_slot = Some(slots.allocate());
        self.recompose_visible_entries();
    }

    /// Narrows the table by the chip, and frees the colour slot it held.
    pub fn switch_chip_to_refine_mode(&mut self, id: FilterChipId, slots: &mut LayerColorSlots) {
        let Some(chip) = self.chips.iter_mut().find(|chip| chip.id == id) else {
            return;
        };
        let Some(slot) = chip.layer_slot.take() else {
            return;
        };
        slots.release(slot);
        self.recompose_visible_entries();
    }

    pub fn remove_chip(&mut self, id: FilterChipId, slots: &mut LayerColorSlots) {
        let Some(position) = self.chips.iter().position(|chip| chip.id == id) else {
            return;
        };
        let removed = self.chips.remove(position);
        if let Some(slot) = removed.layer_slot {
            slots.release(slot);
        }
        self.recompose_visible_entries();
    }

    /// The entries the table shows: what the live filter and every enabled
    /// refine chip matched. Layer chips never narrow it.
    pub fn visible_entries(&self) -> &VisibleEntries {
        &self.visible
    }

    /// What the clock did along the visible rows: the tick each row's
    /// timestamp draws at, and the rows a day divider opens.
    pub fn clock_ticks(&self) -> &ClockTicks {
        &self.clock_ticks
    }

    /// Entries of the log, the count the viewer's "18 of 4,812" ends in.
    pub fn entry_count(&self) -> usize {
        self.log.entries().len()
    }

    /// Whether a filter is being scanned for. The viewer says "filtering…" once
    /// that lasts long enough to notice.
    pub fn is_query_pending(&self) -> bool {
        self.live.query.is_pending() || self.chips.iter().any(|chip| chip.filter.query.is_pending())
    }

    /// Reads in the scans that finished since the last call, replacing the
    /// filters' matches, and returns whether any of them landed. Every filter
    /// keeps the matches it had until its own newer scan lands.
    pub fn apply_finished_queries(&mut self) -> bool {
        let live_landed = self.live.query.take_landed();
        let mut any_landed = live_landed;
        let mut visible_entries_changed = live_landed && self.live.narrows_visible_set();
        for chip in &mut self.chips {
            let landed = chip.filter.query.take_landed();
            any_landed |= landed;
            visible_entries_changed |= landed && chip.narrows_visible_set();
        }
        if visible_entries_changed {
            self.recompose_visible_entries();
        }
        any_landed
    }

    /// Blocks until every scan this stack started has landed.
    ///
    /// The viewer polls with
    /// [`apply_finished_queries`](Self::apply_finished_queries) once a frame,
    /// and draws the matches that have landed by then.
    pub fn wait_for_queries(&mut self) {
        self.live.query.wait_for_landing();
        for chip in &mut self.chips {
            chip.filter.query.wait_for_landing();
        }
        self.recompose_visible_entries();
    }

    /// Hands back the colour slots this stack's layer chips hold, for a log
    /// being unloaded.
    pub(crate) fn release_layer_color_slots(&self, slots: &mut LayerColorSlots) {
        for chip in &self.chips {
            if let Some(slot) = chip.layer_slot {
                slots.release(slot);
            }
        }
    }

    /// Takes a colour slot for every layer chip of this stack, for a log being
    /// loaded into a session.
    pub(crate) fn take_layer_color_slots(&mut self, slots: &mut LayerColorSlots) {
        for chip in &mut self.chips {
            if let Some(held) = chip.layer_slot {
                chip.layer_slot = Some(slots.allocate_preferring(held));
            }
        }
    }

    fn push_stored_chip(&mut self, stored: &StoredLogFilter) {
        let id = FilterChipId(self.next_chip_id);
        self.next_chip_id = self.next_chip_id.saturating_add(1);
        let mut filter = LogFilter::unwritten(self.log.entries().len());
        filter.rewrite(
            FilterPattern {
                text: stored.text.clone(),
                regex: stored.regex,
            },
            &self.log,
        );
        self.chips.push(FilterChip {
            id,
            filter,
            layer_slot: match stored.mode {
                StoredLogFilterMode::Layer { color_slot } => {
                    Some(LayerColorSlot::from_stored_index(color_slot))
                }
                StoredLogFilterMode::Refine => None,
            },
            enabled: stored.enabled,
        });
    }

    fn set_live_filter(&mut self, pattern: FilterPattern) {
        if self.live.pattern == pattern {
            return;
        }
        self.live.rewrite(pattern, &self.log);
        self.recompose_visible_entries();
    }

    fn recompose_visible_entries(&mut self) {
        let entry_count = self.entry_count();
        let narrowing: Vec<&EntryMatches> = iter::once(&self.live)
            .filter(|live| live.narrows_visible_set())
            .chain(
                self.chips
                    .iter()
                    .filter(|chip| chip.narrows_visible_set())
                    .map(|chip| &chip.filter),
            )
            .map(LogFilter::matches)
            .collect();

        let visible = match narrowing.is_empty() {
            true => VisibleEntries::All { entry_count },
            false => VisibleEntries::Matching(matches::intersecting_entry_indices(&narrowing)),
        };
        self.clock_ticks = ClockTicks::of(&self.log, &visible);
        self.visible = visible;
    }
}

/// One added filter: what it matches, how it shows those matches, and whether
/// it is doing so at all.
#[derive(Debug)]
pub struct FilterChip {
    id: FilterChipId,
    filter: LogFilter,

    /// Held while the chip is in layer mode, whether or not it is enabled.
    layer_slot: Option<LayerColorSlot>,

    enabled: bool,
}

impl FilterChip {
    pub fn id(&self) -> FilterChipId {
        self.id
    }

    pub fn pattern(&self) -> &FilterPattern {
        &self.filter.pattern
    }

    pub fn mode(&self) -> FilterChipMode {
        match self.layer_slot {
            Some(_) => FilterChipMode::Layer,
            None => FilterChipMode::Refine,
        }
    }

    /// The palette slot this chip's matches draw in, `None` for a refine chip.
    pub fn layer_slot(&self) -> Option<LayerColorSlot> {
        self.layer_slot
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The entries this chip matched, whether or not it is enabled.
    pub fn matches(&self) -> &EntryMatches {
        self.filter.query.matches()
    }

    fn narrows_visible_set(&self) -> bool {
        self.enabled && self.layer_slot.is_none() && self.filter.narrows_visible_set()
    }

    fn to_stored_filter(&self) -> StoredLogFilter {
        StoredLogFilter {
            text: self.filter.pattern.text.clone(),
            regex: self.filter.pattern.regex,
            enabled: self.enabled,
            mode: match self.layer_slot {
                Some(slot) => StoredLogFilterMode::Layer {
                    color_slot: slot.index(),
                },
                None => StoredLogFilterMode::Refine,
            },
        }
    }
}

/// The live filter or a chip's filter: the pattern, what it compiled to, and
/// the scan applying it to the log.
#[derive(Debug)]
struct LogFilter {
    pattern: FilterPattern,
    compiled: Result<Arc<CompiledFilter>, InvalidFilterPattern>,
    query: FilterQuery,
}

impl LogFilter {
    fn unwritten(entry_count: usize) -> Self {
        Self {
            pattern: FilterPattern::default(),
            compiled: Ok(Arc::new(CompiledFilter::matching_nothing())),
            query: FilterQuery::matching_nothing(entry_count),
        }
    }

    fn rewrite(&mut self, pattern: FilterPattern, log: &Arc<ParsedLog>) {
        self.pattern = pattern;
        self.compiled = self.pattern.compile().map(Arc::new);
        let compiled = match &self.compiled {
            Ok(compiled) => Arc::clone(compiled),
            // An invalid pattern selects nothing: the viewer shows the error
            // and the log stays unfiltered.
            Err(_) => Arc::new(CompiledFilter::matching_nothing()),
        };
        self.query.restart(log, compiled);
    }

    /// Whether the pattern the user wrote can match anything at all.
    fn selects_entries(&self) -> bool {
        self.compiled
            .as_ref()
            .is_ok_and(|compiled| !compiled.matches_nothing())
    }

    /// Whether the matches this filter has *now* narrow the table. A filter
    /// whose first scan is still running does not: the table stays as it was
    /// until the scan lands.
    fn narrows_visible_set(&self) -> bool {
        !self.query.landed_matches_nothing()
    }

    fn matches(&self) -> &EntryMatches {
        self.query.matches()
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, proptest};

    use super::*;
    use crate::{filter::slots::LAYER_COLOR_SLOT_COUNT, test_fixtures};

    /// A filter can select a service, a phenomenon, or one line: two services
    /// write two lines each.
    const LOG: &str = "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
2026-01-01 14:02:12 hal-powerd: battery low
2026-01-01 14:02:13 navsyncd: gnss fix lost
2026-01-01 14:02:14 hal-powerd: battery critical
";

    fn unfiltered_stack() -> (FilterStack, LayerColorSlots) {
        let log = Arc::new(test_fixtures::parsed_log_of_text(LOG));
        (FilterStack::new(log), LayerColorSlots::default())
    }

    /// The stack after `write` has been applied to it and every scan it started
    /// has landed.
    fn scanned_stack(write: impl FnOnce(&mut FilterStack, &mut LayerColorSlots)) -> FilterStack {
        let (mut stack, mut slots) = unfiltered_stack();
        write(&mut stack, &mut slots);
        stack.wait_for_queries();
        stack
    }

    fn visible(stack: &FilterStack) -> Vec<usize> {
        stack.visible_entries().entry_indices().collect()
    }

    fn chip_ids(stack: &FilterStack) -> Vec<FilterChipId> {
        stack.chips().iter().map(FilterChip::id).collect()
    }

    /// Which palette colour a chip draws in, `None` for a refine chip.
    fn slot_index(stack: &FilterStack, id: FilterChipId) -> Option<usize> {
        stack
            .chip(id)
            .and_then(FilterChip::layer_slot)
            .map(LayerColorSlot::index)
    }

    fn add_chip(stack: &mut FilterStack, slots: &mut LayerColorSlots, text: &str) -> FilterChipId {
        stack.set_live_filter_text(text);
        stack
            .add_live_filter_as_chip(slots)
            .expect("a written filter becomes a chip")
    }

    #[test]
    fn an_unfiltered_log_shows_every_line_and_draws_nothing() {
        let stack = scanned_stack(|_, _| {});

        assert_eq!(visible(&stack), [0, 1, 2, 3]);
        assert_eq!(stack.entry_count(), 4);
        assert_eq!(stack.live_filter_matches().match_count(), 0);
        assert!(!stack.can_add_live_filter_as_chip());
        assert!(!stack.is_query_pending());
    }

    #[test]
    fn the_live_filter_narrows_the_table_to_the_lines_it_matches() {
        let stack = scanned_stack(|stack, _| stack.set_live_filter_text("gnss"));

        assert_eq!(visible(&stack), [0, 2]);
        assert_eq!(stack.visible_entries().len(), 2);
        assert_eq!(stack.live_filter_matches().match_count(), 2);
    }

    #[test]
    fn clearing_the_live_filter_shows_every_line_again() {
        let stack = scanned_stack(|stack, _| {
            stack.set_live_filter_text("gnss");
            stack.clear_live_filter();
        });

        assert_eq!(visible(&stack), [0, 1, 2, 3]);
        assert_eq!(stack.live_filter_matches().match_count(), 0);
        assert!(!stack.is_query_pending(), "an empty filter needs no scan");
    }

    /// The viewer draws the frame it is in from the matches it already has, and
    /// the newer ones replace them once their scan has landed.
    #[test]
    fn the_table_stays_as_it_was_until_the_new_matches_land() {
        let (mut stack, _slots) = unfiltered_stack();

        stack.set_live_filter_text("gnss");
        assert_eq!(visible(&stack), [0, 1, 2, 3]);

        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0, 2]);
        assert!(!stack.is_query_pending());
    }

    #[test]
    fn an_invalid_regex_reports_the_error_and_leaves_the_table_unfiltered() {
        let stack = scanned_stack(|stack, _| {
            stack.set_live_filter_regex(true);
            stack.set_live_filter_text("navsyncd(");
        });

        assert!(
            stack
                .live_filter_error()
                .is_some_and(|error| error.message().contains("unclosed group"))
        );
        assert_eq!(visible(&stack), [0, 1, 2, 3]);
        assert_eq!(stack.live_filter_matches().match_count(), 0);
        assert!(
            !stack.can_add_live_filter_as_chip(),
            "there is nothing to add while the pattern does not compile"
        );
    }

    /// The table paints the live colour over what the filter selected the line
    /// for, and over nothing at all while the pattern does not compile.
    #[test]
    fn the_live_filter_names_where_it_matched_in_a_line() {
        let stack = scanned_stack(|stack, _| stack.set_live_filter_text("gnss fix"));
        assert_eq!(
            stack.live_filter_match_spans("navsyncd: gnss fix acquired"),
            [10..14, 15..18]
        );

        let stack = scanned_stack(|stack, _| {
            stack.set_live_filter_regex(true);
            stack.set_live_filter_text("navsyncd(");
        });
        assert_eq!(
            stack.live_filter_match_spans("navsyncd: gnss fix acquired"),
            Vec::<Range<usize>>::new()
        );
    }

    /// Where the table resumes a boot session under a narrowed visible set: the
    /// row showing the first line of that session still visible.
    #[test]
    fn the_visible_set_names_the_row_a_session_resumes_at() {
        let stack = scanned_stack(|stack, _| stack.set_live_filter_text("battery"));

        let visible = stack.visible_entries();
        assert_eq!(visible.row_at_or_after(0), 0);
        assert_eq!(visible.row_at_or_after(2), 1, "entry 3 is the second match");
        assert_eq!(visible.row_at_or_after(4), 2, "past the last entry");

        let unfiltered = scanned_stack(|_, _| {});
        assert_eq!(unfiltered.visible_entries().row_at_or_after(2), 2);
        assert_eq!(unfiltered.visible_entries().row_at_or_after(9), 4);
    }

    #[test]
    fn a_regex_live_filter_matches_the_message_as_one_pattern() {
        let stack = scanned_stack(|stack, _| {
            stack.set_live_filter_regex(true);
            stack.set_live_filter_text("^(navsyncd|hal-powerd): battery");
        });

        assert_eq!(visible(&stack), [1, 3]);
    }

    #[test]
    fn adding_a_chip_clears_the_field_and_leaves_the_toggle_as_it_was() {
        let (mut stack, mut slots) = unfiltered_stack();
        stack.set_live_filter_regex(true);
        stack.set_live_filter_text("gnss|battery");

        let id = stack
            .add_live_filter_as_chip(&mut slots)
            .expect("a written filter becomes a chip");
        stack.wait_for_queries();

        let chip = stack.chip(id).expect("the chip was added");
        assert_eq!(chip.pattern(), &FilterPattern::regex("gnss|battery"));
        assert_eq!(chip.matches().match_count(), 4);
        assert_eq!(chip.mode(), FilterChipMode::Layer);
        assert!(chip.is_enabled());

        assert_eq!(stack.live_filter_text(), "");
        assert!(
            stack.live_filter_is_regex(),
            "the .* toggle belongs to the field, not to the text that was in it"
        );
        assert_eq!(stack.live_filter_matches().match_count(), 0);
    }

    #[test]
    fn a_filter_that_matches_nothing_yet_cannot_become_a_chip() {
        let (mut stack, mut slots) = unfiltered_stack();

        assert_eq!(stack.add_live_filter_as_chip(&mut slots), None);

        stack.set_live_filter_regex(true);
        stack.set_live_filter_text("navsyncd(");
        assert_eq!(stack.add_live_filter_as_chip(&mut slots), None);
        assert!(stack.chips().is_empty());
    }

    /// The compare-phenomena-spatially mode: a layer chip colours the map
    /// without taking a line out of the table.
    #[test]
    fn a_layer_chip_leaves_the_table_alone_and_a_refine_chip_narrows_it() {
        let (mut stack, mut slots) = unfiltered_stack();
        let id = add_chip(&mut stack, &mut slots, "gnss");
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0, 1, 2, 3]);

        stack.switch_chip_to_refine_mode(id, &mut slots);

        assert_eq!(visible(&stack), [0, 2]);
        assert_eq!(
            stack.chip(id).map(FilterChip::mode),
            Some(FilterChipMode::Refine)
        );
    }

    #[test]
    fn the_visible_set_is_the_live_filter_and_every_enabled_refine_chip() {
        let (mut stack, mut slots) = unfiltered_stack();
        let gnss = add_chip(&mut stack, &mut slots, "gnss");
        let battery = add_chip(&mut stack, &mut slots, "battery");
        stack.switch_chip_to_refine_mode(gnss, &mut slots);
        stack.switch_chip_to_refine_mode(battery, &mut slots);
        stack.wait_for_queries();

        assert_eq!(
            visible(&stack),
            Vec::<usize>::new(),
            "no line is both a fix and a battery"
        );

        stack.set_chip_enabled(battery, false);
        assert_eq!(visible(&stack), [0, 2], "a disabled chip narrows nothing");

        stack.set_live_filter_text("lost");
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [2]);
    }

    #[test]
    fn a_disabled_chip_keeps_its_matches_and_its_colour_slot() {
        let (mut stack, mut slots) = unfiltered_stack();
        let id = add_chip(&mut stack, &mut slots, "gnss");
        stack.wait_for_queries();

        stack.set_chip_enabled(id, false);

        let chip = stack.chip(id).expect("the chip is still there");
        assert!(!chip.is_enabled());
        assert_eq!(
            slot_index(&stack, id),
            Some(0),
            "re-enabling must not reshuffle the map"
        );
        assert_eq!(chip.matches().match_count(), 2);
        assert_eq!(stack.enabled_layer_chips().count(), 0);
        assert!(
            !stack.is_query_pending(),
            "toggling a chip must not start a scan"
        );
    }

    #[test]
    fn removing_a_chip_frees_the_colour_it_held() {
        let (mut stack, mut slots) = unfiltered_stack();
        let first = add_chip(&mut stack, &mut slots, "gnss");
        let second = add_chip(&mut stack, &mut slots, "battery");
        assert_eq!(slot_index(&stack, first), Some(0));
        assert_eq!(slot_index(&stack, second), Some(1));

        stack.remove_chip(first, &mut slots);

        assert_eq!(chip_ids(&stack), [second]);
        let readded = add_chip(&mut stack, &mut slots, "critical");
        assert_eq!(
            slot_index(&stack, readded),
            Some(0),
            "the freed colour is the lowest one free again"
        );
    }

    #[test]
    fn switching_a_chip_out_of_layer_mode_and_back_takes_the_lowest_free_colour() {
        let (mut stack, mut slots) = unfiltered_stack();
        let first = add_chip(&mut stack, &mut slots, "gnss");
        let second = add_chip(&mut stack, &mut slots, "battery");

        stack.switch_chip_to_refine_mode(first, &mut slots);
        assert_eq!(slot_index(&stack, first), None);
        assert_eq!(stack.enabled_layer_chips().count(), 1);

        stack.switch_chip_to_layer_mode(first, &mut slots);

        assert_eq!(
            slot_index(&stack, first),
            Some(0),
            "the colour it freed was the lowest one free again"
        );
        assert_eq!(slot_index(&stack, second), Some(1));
    }

    #[test]
    fn every_chip_keeps_its_own_matches() {
        let (mut stack, mut slots) = unfiltered_stack();
        let gnss = add_chip(&mut stack, &mut slots, "gnss");
        let critical = add_chip(&mut stack, &mut slots, "battery critical");
        stack.wait_for_queries();

        assert_eq!(
            stack
                .chips()
                .iter()
                .map(|chip| chip.matches().matched_entry_indices().collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [vec![0, 2], vec![3]]
        );
        assert_eq!(chip_ids(&stack), [gnss, critical]);
    }

    /// The stack an attachment stores and restores: every chip's text, mode,
    /// enabled state and colour come back, and each chip scans the log again
    /// for what it matches.
    #[test]
    fn a_stored_stack_restores_every_chip_with_what_it_matched() {
        let (mut stack, mut slots) = unfiltered_stack();
        let gnss = add_chip(&mut stack, &mut slots, "gnss");
        let battery = add_chip(&mut stack, &mut slots, "battery");
        stack.switch_chip_to_refine_mode(gnss, &mut slots);
        stack.set_chip_enabled(battery, false);

        let stored = stack.to_stored_filters();
        assert_eq!(
            stored,
            [
                StoredLogFilter {
                    text: "gnss".to_owned(),
                    regex: false,
                    enabled: true,
                    mode: StoredLogFilterMode::Refine,
                },
                StoredLogFilter {
                    text: "battery".to_owned(),
                    regex: false,
                    enabled: false,
                    mode: StoredLogFilterMode::Layer { color_slot: 1 },
                },
            ]
        );

        let log = Arc::new(test_fixtures::parsed_log_of_text(LOG));
        let mut restored = FilterStack::from_stored_filters(log, &stored);
        restored.wait_for_queries();

        assert_eq!(restored.to_stored_filters(), stored);
        assert_eq!(
            visible(&restored),
            [0, 2],
            "the restored refine chip narrows the table as it did"
        );
        assert_eq!(
            restored
                .chips()
                .iter()
                .map(|chip| chip.matches().match_count())
                .collect::<Vec<_>>(),
            [2, 2],
            "every restored chip scanned the log for its own matches"
        );
    }

    /// A regex chip stays a regex chip, and a stored slot this build's palette
    /// does not have still restores as a layer chip.
    #[test]
    fn a_stored_regex_chip_and_an_unknown_colour_slot_restore_as_they_were() {
        let stored = [StoredLogFilter {
            text: "^navsyncd".to_owned(),
            regex: true,
            enabled: true,
            mode: StoredLogFilterMode::Layer {
                color_slot: LAYER_COLOR_SLOT_COUNT + 3,
            },
        }];

        let log = Arc::new(test_fixtures::parsed_log_of_text(LOG));
        let mut restored = FilterStack::from_stored_filters(log, &stored);
        restored.wait_for_queries();

        let chip = restored.chips().first().expect("the chip was restored");
        assert_eq!(chip.pattern(), &FilterPattern::regex("^navsyncd"));
        assert_eq!(chip.mode(), FilterChipMode::Layer);
        assert_eq!(chip.matches().match_count(), 2);
    }

    proptest! {
        /// Whatever the user writes into the field and adds as a refine chip,
        /// the table shows exactly the entries a walk of the log selects, in
        /// file order, and never an entry the log does not have.
        #[test]
        fn the_visible_entries_are_what_a_walk_of_the_log_selects(
            live in "[a-z ]{0,5}",
            refine in "[a-z ]{0,5}",
            chip_enabled in any::<bool>(),
        ) {
            let (mut stack, mut slots) = unfiltered_stack();
            stack.set_live_filter_text(&refine);
            let chip = stack.add_live_filter_as_chip(&mut slots);
            if let Some(id) = chip {
                stack.switch_chip_to_refine_mode(id, &mut slots);
                stack.set_chip_enabled(id, chip_enabled);
            }
            stack.set_live_filter_text(&live);
            stack.wait_for_queries();

            let live_filter = FilterPattern::plain(&live).compile().expect("plain compiles");
            let refine_filter = FilterPattern::plain(&refine).compile().expect("plain compiles");
            let refine_narrows = chip.is_some() && chip_enabled;
            let log = test_fixtures::parsed_log_of_text(LOG);
            let expected: Vec<usize> = log
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    let message = log.message(entry);
                    (live_filter.matches_nothing() || live_filter.matches(message))
                        && (!refine_narrows || refine_filter.matches(message))
                })
                .map(|(index, _)| index)
                .collect();

            prop_assert_eq!(visible(&stack), expected.clone());
            prop_assert_eq!(stack.visible_entries().len(), expected.len());
            prop_assert!(expected.len() <= stack.entry_count());
        }
    }
}
