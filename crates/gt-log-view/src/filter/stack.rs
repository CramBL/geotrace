//! One log's filters: the live filter, and the chips added from it.

use std::{iter, mem, ops::Range, sync::Arc};

use gt_history_types::{
    InvalidStoredLogFilterStack, StoredLogFilter, StoredLogFilterGroup, StoredLogFilterMode,
    StoredLogFilterOperator, StoredLogFilterStack, StoredLogFilterStackParts,
};
use gt_logfile::ParsedLog;

use crate::filter::{
    clock_ticks::ClockTicks,
    composition::FilterGroupOperator,
    matches::EntryMatches,
    pattern::{CompiledFilter, FilterPattern, InvalidFilterPattern},
    query::FilterQuery,
    slots::{LayerColorSlot, LayerColorSlots},
};

/// Identifies a chip for as long as it is in the stack it was added to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FilterChipId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FilterGroupId(u64);

#[derive(Clone, Copy, Debug)]
pub struct FilterGroup {
    id: FilterGroupId,
    operator: FilterGroupOperator,
}

impl FilterGroup {
    pub fn id(&self) -> FilterGroupId {
        self.id
    }

    pub fn operator(&self) -> FilterGroupOperator {
        self.operator
    }
}

/// What a chip does with the entries it matches.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumIter)]
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
    All {
        entry_count: usize,
    },

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
    groups: Vec<FilterGroup>,
    selected_group: FilterGroupId,
    visible: VisibleEntries,
    clock_ticks: ClockTicks,
}

impl FilterStack {
    pub(crate) fn share_equal_parsed_log(&mut self, parsed: Arc<ParsedLog>) {
        debug_assert_eq!(self.log, parsed);
        self.log = parsed;
    }

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
            groups: vec![FilterGroup {
                id: FilterGroupId(0),
                operator: FilterGroupOperator::All,
            }],
            selected_group: FilterGroupId(0),
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
    pub fn from_stored_stack(log: Arc<ParsedLog>, stored: &StoredLogFilterStack) -> Self {
        let mut stack = Self::new(log);
        stack.groups = stored
            .groups()
            .iter()
            .map(|group| FilterGroup {
                id: FilterGroupId(group.id),
                operator: match group.operator {
                    StoredLogFilterOperator::All => FilterGroupOperator::All,
                    StoredLogFilterOperator::Any => FilterGroupOperator::Any,
                },
            })
            .collect();
        stack.selected_group = FilterGroupId(stored.selected_group_id());
        for filter in stored.chips() {
            stack.push_stored_chip(filter);
        }
        stack.recompose_visible_entries();
        stack
    }

    pub fn create_group(&mut self) -> FilterGroupId {
        let mut id = FilterGroupId(0);
        while self.groups.iter().any(|group| group.id == id) {
            id.0 = id.0.saturating_add(1);
        }
        self.groups.push(FilterGroup {
            id,
            operator: FilterGroupOperator::All,
        });
        self.select_group(id);
        id
    }

    pub fn select_group(&mut self, id: FilterGroupId) {
        if self.selected_group != id && self.groups.iter().any(|group| group.id == id) {
            self.selected_group = id;
            self.recompose_visible_entries();
        }
    }

    pub fn remove_group(&mut self, id: FilterGroupId) {
        let Some(survivor) = self
            .groups
            .iter()
            .find(|group| group.id != id)
            .map(FilterGroup::id)
        else {
            return;
        };
        let Some(index) = self.groups.iter().position(|group| group.id == id) else {
            return;
        };
        for chip in &mut self.chips {
            if chip.group == id {
                chip.group = survivor;
            }
        }
        if self.selected_group == id {
            self.selected_group = survivor;
        }
        self.groups.remove(index);
        self.recompose_visible_entries();
    }

    pub fn move_chip_to_group(&mut self, chip_id: FilterChipId, group: FilterGroupId) {
        if !self.groups.iter().any(|candidate| candidate.id == group) {
            return;
        }
        let Some(chip) = self.chips.iter_mut().find(|chip| chip.id == chip_id) else {
            return;
        };
        if chip.group != group {
            chip.group = group;
            self.recompose_visible_entries();
        }
    }

    pub fn set_group_operator(&mut self, id: FilterGroupId, operator: FilterGroupOperator) {
        let Some(group) = self.groups.iter_mut().find(|group| group.id == id) else {
            return;
        };
        if group.operator != operator {
            group.operator = operator;
            self.recompose_visible_entries();
        }
    }

    pub fn to_stored_stack(&self) -> Result<StoredLogFilterStack, InvalidStoredLogFilterStack> {
        StoredLogFilterStack::try_from_parts(StoredLogFilterStackParts {
            groups: self
                .groups
                .iter()
                .map(|group| StoredLogFilterGroup {
                    id: group.id.0,
                    operator: match group.operator {
                        FilterGroupOperator::All => StoredLogFilterOperator::All,
                        FilterGroupOperator::Any => StoredLogFilterOperator::Any,
                    },
                })
                .collect(),
            selected_group_id: self.selected_group.0,
            chips: self
                .chips
                .iter()
                .map(FilterChip::to_stored_filter)
                .collect(),
        })
    }

    pub fn groups(&self) -> &[FilterGroup] {
        &self.groups
    }

    pub fn selected_group(&self) -> FilterGroupId {
        self.selected_group
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

    pub fn can_add_live_filter_as_chip(&self) -> bool {
        self.live.selects_entries()
    }

    /// Adds an enabled table filter with the existing scan and clears the field.
    pub fn add_live_filter_as_chip(&mut self) -> Option<FilterChipId> {
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
            group: self.selected_group,
            filter: mem::replace(&mut self.live, emptied),
            layer_slot: None,
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
            group: FilterGroupId(stored.group_id),
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
        let composed_groups: Vec<EntryMatches> = self
            .groups
            .iter()
            .filter_map(|group| {
                let conditions: Vec<&EntryMatches> = iter::once(&self.live)
                    .filter(|live| self.selected_group == group.id && live.narrows_visible_set())
                    .chain(
                        self.chips
                            .iter()
                            .filter(|chip| chip.group == group.id && chip.narrows_visible_set())
                            .map(|chip| &chip.filter),
                    )
                    .map(LogFilter::matches)
                    .collect();
                group.operator.compose(&conditions)
            })
            .collect();
        let group_matches: Vec<_> = composed_groups.iter().collect();
        let visible = FilterGroupOperator::All
            .compose(&group_matches)
            .map_or(VisibleEntries::All { entry_count }, |matches| {
                VisibleEntries::Matching(matches.matched_entry_indices().collect())
            });
        self.clock_ticks = ClockTicks::of(&self.log, &visible);
        self.visible = visible;
    }
}

/// One added filter: what it matches, how it shows those matches, and whether
/// it is doing so at all.
#[derive(Debug)]
pub struct FilterChip {
    id: FilterChipId,
    group: FilterGroupId,
    filter: LogFilter,

    /// Held while the chip is in layer mode, whether or not it is enabled.
    layer_slot: Option<LayerColorSlot>,

    enabled: bool,
}

impl FilterChip {
    pub fn id(&self) -> FilterChipId {
        self.id
    }

    pub fn group(&self) -> FilterGroupId {
        self.group
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
            group_id: self.group.0,
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
    use std::ptr;

    use gt_history_types::LogAttachment;
    use proptest::{prelude::*, proptest};
    use rstest::rstest;

    use super::*;
    use crate::{filter::slots::LAYER_COLOR_SLOT_COUNT, test_util};

    fn unfiltered_stack() -> (FilterStack, LayerColorSlots) {
        let log = Arc::new(test_util::parsed_log_of_text(LOG));
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

    fn add_refine_chip(stack: &mut FilterStack, text: &str) -> FilterChipId {
        stack.set_live_filter_text(text);
        stack.add_live_filter_as_chip().expect("valid filter")
    }

    #[rstest]
    #[case::mixed(FilterGroupOperator::Any, vec![0, 1])]
    #[case::all(FilterGroupOperator::All, vec![1])]
    fn groups_intersect_their_composed_matches(
        #[case] operator: FilterGroupOperator,
        #[case] expected: Vec<usize>,
    ) {
        let log = Arc::new(test_util::parsed_log_of_text(COMPOSITION_LOG));
        let mut stack = FilterStack::new(log);
        let first_group = stack.selected_group();
        add_refine_chip(&mut stack, "first");
        add_refine_chip(&mut stack, "second");
        stack.set_group_operator(first_group, operator);
        let second_group = stack.create_group();
        let third = add_refine_chip(&mut stack, "first");
        stack.wait_for_queries();
        assert_eq!(visible(&stack), expected);
        assert_eq!(stack.chip(third).unwrap().group(), second_group);
        if operator == FilterGroupOperator::All {
            stack.move_chip_to_group(third, first_group);
            assert_eq!(visible(&stack), expected);
        }
    }

    #[test]
    fn groups_without_participating_conditions_leave_visibility_unchanged() {
        let (mut stack, mut slots) = unfiltered_stack();
        let chip = add_refine_chip(&mut stack, "acquired");
        stack.create_group();
        stack.set_group_operator(stack.selected_group(), FilterGroupOperator::Any);
        let disabled = add_refine_chip(&mut stack, "missing");
        stack.set_chip_enabled(disabled, false);
        add_layer_chip(&mut stack, &mut slots, "battery");
        stack.create_group();
        stack.set_live_filter_regex(true);
        stack.set_live_filter_text("[");
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0]);
        stack.set_chip_enabled(chip, false);
        assert_eq!(visible(&stack), [0, 1, 2, 3]);
        stack.set_chip_enabled(disabled, true);
        assert_eq!(visible(&stack), Vec::<usize>::new());
    }

    #[rstest]
    #[case::first_pending(ScanState::FirstPending)]
    #[case::landed(ScanState::Landed)]
    #[case::replacement_pending(ScanState::ReplacementPending)]
    fn selecting_a_group_and_adding_live_preserve_query_identity(#[case] state: ScanState) {
        let log = Arc::new(test_util::parsed_log_of_text(COMPOSITION_LOG));
        let mut stack = FilterStack::new(log);
        let any_group = stack.selected_group();
        stack.set_group_operator(any_group, FilterGroupOperator::Any);
        add_refine_chip(&mut stack, "first");
        let all_group = stack.create_group();
        add_refine_chip(&mut stack, "first");
        stack.wait_for_queries();
        if matches!(state, ScanState::ReplacementPending) {
            stack.set_live_filter_text("second");
            stack.wait_for_queries();
        }
        stack.set_live_filter_text("second first");
        if matches!(state, ScanState::Landed) {
            stack.wait_for_queries();
        }
        let identity = stack.live.query.scan_identity();
        let matches = stack.live_filter_matches().clone();
        let pending = stack.is_query_pending();
        stack.select_group(any_group);
        assert_eq!(visible(&stack), [0, 1]);
        assert_eq!(stack.live.query.scan_identity(), identity);
        assert_eq!(*stack.live_filter_matches(), matches);
        stack.select_group(all_group);
        assert_eq!(
            visible(&stack),
            if matches!(state, ScanState::FirstPending) {
                vec![0, 1]
            } else {
                vec![1]
            }
        );
        stack.select_group(any_group);
        let before_add = visible(&stack);
        let chip = stack.add_live_filter_as_chip().expect("valid live filter");
        assert_eq!(visible(&stack), before_add);
        assert_eq!(stack.chip(chip).unwrap().group(), any_group);
        assert_eq!(
            stack.chip(chip).unwrap().filter.query.scan_identity(),
            identity
        );
        assert_eq!(*stack.chip(chip).unwrap().matches(), matches);
        assert_eq!(stack.is_query_pending(), pending);
        assert_eq!(stack.live_filter_text(), "");
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0, 1]);
    }

    #[rstest]
    #[case::pending(false)]
    #[case::landed(true)]
    fn group_edits_and_highlight_transitions_preserve_condition_scans(#[case] landed: bool) {
        let (mut stack, mut slots) = unfiltered_stack();
        let first_group = stack.selected_group();
        add_refine_chip(&mut stack, "acquired");
        stack.wait_for_queries();
        let second_group = stack.create_group();
        let moved = add_refine_chip(&mut stack, "lost");
        if landed {
            stack.wait_for_queries();
        }
        let identity = stack.chip(moved).unwrap().filter.query.scan_identity();
        let matches = ptr::from_ref(stack.chip(moved).unwrap().matches());
        let pending = stack.is_query_pending();
        stack.move_chip_to_group(moved, first_group);
        stack.set_group_operator(first_group, FilterGroupOperator::Any);
        assert_eq!(visible(&stack), if landed { vec![0, 2] } else { vec![0] });
        stack.switch_chip_to_layer_mode(moved, &mut slots);
        assert_eq!(visible(&stack), [0]);
        stack.select_group(second_group);
        stack.switch_chip_to_refine_mode(moved, &mut slots);
        assert_eq!(stack.chip(moved).unwrap().group(), first_group);
        assert_eq!(
            stack.chip(moved).unwrap().filter.query.scan_identity(),
            identity
        );
        assert_eq!(ptr::from_ref(stack.chip(moved).unwrap().matches()), matches);
        assert_eq!(stack.is_query_pending(), pending);
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0, 2]);
    }

    #[rstest]
    #[case::remove_first(true)]
    #[case::remove_later(false)]
    fn removing_groups_reassigns_active_and_remembered_memberships(#[case] remove_first: bool) {
        let (mut stack, mut slots) = unfiltered_stack();
        let first_group = stack.selected_group();
        let second_group = stack.create_group();
        let (removed, survivor) = if remove_first {
            (first_group, second_group)
        } else {
            (second_group, first_group)
        };
        stack.select_group(removed);
        let active = add_refine_chip(&mut stack, "gnss");
        let highlighted = add_layer_chip(&mut stack, &mut slots, "fix");
        stack.set_chip_enabled(active, false);
        stack.set_live_filter_text("lost");
        stack.wait_for_queries();
        let live_identity = stack.live.query.scan_identity();
        let chip_identity = stack
            .chip(highlighted)
            .unwrap()
            .filter
            .query
            .scan_identity();
        stack.remove_group(removed);
        assert_eq!(stack.groups().len(), 1);
        assert_eq!(stack.selected_group(), survivor);
        assert_eq!(stack.chip(active).unwrap().group(), survivor);
        assert_eq!(stack.chip(highlighted).unwrap().group(), survivor);
        assert_eq!(stack.live.query.scan_identity(), live_identity);
        assert_eq!(
            stack
                .chip(highlighted)
                .unwrap()
                .filter
                .query
                .scan_identity(),
            chip_identity
        );
        assert_eq!(visible(&stack), [2]);
        stack.switch_chip_to_refine_mode(highlighted, &mut slots);
        assert_eq!(stack.chip(highlighted).unwrap().group(), survivor);
        stack.remove_group(survivor);
        assert_eq!(stack.groups().len(), 1);
        assert_eq!(visible(&stack), [2]);
        stack.clear_live_filter();
        stack.set_chip_enabled(highlighted, false);
        assert_eq!(visible(&stack), [0, 1, 2, 3]);
    }

    #[test]
    fn grouped_filters_round_trip_with_order_selection_and_highlight_memberships() {
        let (mut stack, mut slots) = unfiltered_stack();
        let first_group = stack.selected_group();
        let first = add_refine_chip(&mut stack, "acquired");
        let second = add_refine_chip(&mut stack, "lost");
        stack.set_group_operator(first_group, FilterGroupOperator::Any);
        let second_group = stack.create_group();
        add_refine_chip(&mut stack, "gnss");
        stack.switch_chip_to_layer_mode(first, &mut slots);
        let empty_group = stack.create_group();
        stack.select_group(second_group);
        stack.wait_for_queries();
        let before = visible(&stack);
        let stored = stack.to_stored_stack().expect("valid groups");
        let mut restored = FilterStack::from_stored_stack(Arc::clone(&stack.log), &stored);
        restored.wait_for_queries();
        assert_eq!(restored.to_stored_stack().expect("valid groups"), stored);
        assert_eq!(visible(&restored), before);
        assert_eq!(
            restored
                .groups()
                .iter()
                .map(FilterGroup::id)
                .collect::<Vec<_>>(),
            [first_group, second_group, empty_group]
        );
        restored.switch_chip_to_refine_mode(first, &mut slots);
        assert_eq!(restored.chip(first).unwrap().group(), first_group);
        assert_eq!(visible(&restored), [0, 2]);
        restored.move_chip_to_group(second, second_group);
        assert_eq!(visible(&restored), Vec::<usize>::new());
    }

    #[rstest]
    #[case::flat(1, None, vec![1])]
    #[case::single_all(2, Some("all"), vec![1])]
    #[case::single_any(2, Some("any"), vec![0, 1, 2])]
    fn legacy_attachment_filters_restore_identical_visible_entries(
        #[case] version: u32,
        #[case] operator: Option<&str>,
        #[case] expected: Vec<usize>,
    ) {
        let chips = serde_json::json!([
            {"text": "first", "regex": false, "enabled": true, "mode": "refine"},
            {"text": "second", "regex": false, "enabled": true, "mode": "refine"}
        ]);
        let filters = operator.map_or(
            chips.clone(),
            |operator| serde_json::json!({"operator": operator, "chips": chips}),
        );
        let json = serde_json::json!({"format_version": version, "name": "legacy.log", "content_hash": "0", "filters": filters}).to_string();
        let attachment = LogAttachment::from_attribute_json(&json).expect("legacy attachment");
        let log = Arc::new(test_util::parsed_log_of_text(COMPOSITION_LOG));
        let mut restored = FilterStack::from_stored_stack(log, &attachment.filters);
        restored.wait_for_queries();
        assert_eq!(restored.groups().len(), 1);
        assert_eq!(visible(&restored), expected);
    }

    fn add_layer_chip(
        stack: &mut FilterStack,
        slots: &mut LayerColorSlots,
        text: &str,
    ) -> FilterChipId {
        stack.set_live_filter_text(text);
        let chip = stack
            .add_live_filter_as_chip()
            .expect("a written filter becomes a chip");
        stack.switch_chip_to_layer_mode(chip, slots);
        chip
    }

    #[rstest]
    #[case::all_with_live(FilterGroupOperator::All, false, vec![1])]
    #[case::any_with_live(FilterGroupOperator::Any, false, vec![0, 1, 2])]
    #[case::all_with_chips(FilterGroupOperator::All, true, vec![1])]
    #[case::any_with_chips(FilterGroupOperator::Any, true, vec![0, 1, 2])]
    fn group_composition_excludes_disabled_and_highlight_conditions(
        #[case] operator: FilterGroupOperator,
        #[case] add_second_chip: bool,
        #[case] expected: Vec<usize>,
    ) {
        let log = Arc::new(test_util::parsed_log_of_text(COMPOSITION_LOG));
        let mut stack = FilterStack::new(Arc::clone(&log));
        let mut slots = LayerColorSlots::default();
        add_layer_chip(&mut stack, &mut slots, "excluded");
        stack.set_live_filter_text("excluded");
        let disabled = stack.add_live_filter_as_chip().expect("valid filter");
        stack.set_chip_enabled(disabled, false);
        stack.set_live_filter_text("first");
        stack.add_live_filter_as_chip().expect("valid filter");
        stack.set_live_filter_text("second");
        if add_second_chip {
            stack.add_live_filter_as_chip().expect("valid filter");
        }
        stack.set_group_operator(stack.selected_group(), operator);
        stack.wait_for_queries();
        assert_eq!(visible(&stack), expected);
        if add_second_chip {
            let stored = stack.to_stored_stack().expect("valid groups");
            let mut restored = FilterStack::from_stored_stack(log, &stored);
            restored.wait_for_queries();
            assert_eq!(visible(&restored), expected);
            assert_eq!(restored.to_stored_stack().expect("valid groups"), stored);
        }
    }

    #[rstest]
    #[case::all_empty(FilterGroupOperator::All, "", false)]
    #[case::any_empty(FilterGroupOperator::Any, "", false)]
    #[case::all_invalid(FilterGroupOperator::All, "[", true)]
    #[case::any_invalid(FilterGroupOperator::Any, "[", true)]
    #[case::all_whitespace(FilterGroupOperator::All, "  ", false)]
    #[case::any_whitespace(FilterGroupOperator::Any, "  ", false)]
    fn empty_or_invalid_live_conditions_do_not_participate(
        #[case] operator: FilterGroupOperator,
        #[case] live: &str,
        #[case] regex: bool,
    ) {
        let (mut stack, mut slots) = unfiltered_stack();
        add_layer_chip(&mut stack, &mut slots, "gnss");
        stack.set_live_filter_text("battery");
        let disabled = stack.add_live_filter_as_chip().expect("valid filter");
        stack.set_chip_enabled(disabled, false);
        stack.set_group_operator(stack.selected_group(), operator);
        stack.set_live_filter_regex(regex);
        stack.set_live_filter_text(live);
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0, 1, 2, 3]);
        stack.set_live_filter_regex(false);
        stack.set_live_filter_text("acquired");
        stack.add_live_filter_as_chip().expect("valid filter");
        stack.set_live_filter_regex(regex);
        stack.set_live_filter_text(live);
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0]);
    }

    #[rstest]
    #[case::first_pending(ScanState::FirstPending, vec![0])]
    #[case::landed(ScanState::Landed, vec![0, 2])]
    #[case::replacement_pending(ScanState::ReplacementPending, vec![0, 2])]
    fn operator_changes_preserve_landed_matches_and_pending_scans(
        #[case] state: ScanState,
        #[case] expected_any: Vec<usize>,
    ) {
        let (mut stack, _) = unfiltered_stack();
        stack.set_live_filter_text("acquired");
        let chip = stack.add_live_filter_as_chip().expect("valid filter");
        stack.wait_for_queries();
        if matches!(state, ScanState::ReplacementPending) {
            stack.set_live_filter_text("gnss");
            stack.wait_for_queries();
        }
        stack.set_live_filter_text("gnss fix");
        if matches!(state, ScanState::Landed) {
            stack.wait_for_queries();
        }
        let live_identity = stack.live.query.scan_identity();
        let chip_identity = stack.chip(chip).unwrap().filter.query.scan_identity();
        let live_matches = ptr::from_ref(stack.live_filter_matches());
        let chip_matches = ptr::from_ref(stack.chip(chip).unwrap().matches());
        let pending = stack.is_query_pending();
        let all_visible = visible(&stack);
        stack.set_group_operator(stack.selected_group(), FilterGroupOperator::Any);
        assert_eq!(visible(&stack), expected_any);
        stack.set_group_operator(stack.selected_group(), FilterGroupOperator::All);
        assert_eq!(visible(&stack), all_visible);
        assert_eq!(stack.live.query.scan_identity(), live_identity);
        assert_eq!(
            stack.chip(chip).unwrap().filter.query.scan_identity(),
            chip_identity
        );
        assert_eq!(ptr::from_ref(stack.live_filter_matches()), live_matches);
        assert_eq!(
            ptr::from_ref(stack.chip(chip).unwrap().matches()),
            chip_matches
        );
        assert_eq!(stack.is_query_pending(), pending);
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0]);
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
        let (mut stack, _slots) = unfiltered_stack();
        stack.set_live_filter_regex(true);
        stack.set_live_filter_text("gnss|battery");

        let id = stack
            .add_live_filter_as_chip()
            .expect("a written filter becomes a chip");
        stack.wait_for_queries();

        let chip = stack.chip(id).expect("the chip was added");
        assert_eq!(chip.pattern(), &FilterPattern::regex("gnss|battery"));
        assert_eq!(chip.matches().match_count(), 4);
        assert_eq!(chip.mode(), FilterChipMode::Refine);
        assert_eq!(chip.layer_slot(), None);
        assert!(chip.is_enabled());

        assert_eq!(stack.live_filter_text(), "");
        assert!(
            stack.live_filter_is_regex(),
            "the .* toggle belongs to the field, not to the text that was in it"
        );
        assert_eq!(stack.live_filter_matches().match_count(), 0);
    }

    #[derive(Debug)]
    enum ScanState {
        FirstPending,
        Landed,
        ReplacementPending,
    }

    #[rstest]
    #[case::first_pending(ScanState::FirstPending, vec![0, 1, 2, 3])]
    #[case::replacement_pending(ScanState::ReplacementPending, vec![1, 3])]
    #[case::landed(ScanState::Landed, vec![0, 2])]
    fn adding_a_table_filter_preserves_visible_entries_and_scan(
        #[case] state: ScanState,
        #[case] expected_before: Vec<usize>,
    ) {
        let (mut stack, slots) = unfiltered_stack();
        if matches!(state, ScanState::ReplacementPending) {
            stack.set_live_filter_text("battery");
            stack.wait_for_queries();
        }
        stack.set_live_filter_text("gnss");
        if matches!(state, ScanState::Landed) {
            stack.wait_for_queries();
        }
        let identity = stack.live.query.scan_identity();
        let matches = ptr::from_ref(stack.live_filter_matches());
        let pending = stack.is_query_pending();
        assert_eq!(visible(&stack), expected_before);

        let id = stack
            .add_live_filter_as_chip()
            .expect("the filter is valid");
        let chip = stack.chip(id).expect("the chip was added");
        assert_eq!(chip.filter.query.scan_identity(), identity);
        assert_eq!(ptr::from_ref(chip.matches()), matches);
        assert_eq!(chip.mode(), FilterChipMode::Refine);
        assert!(chip.is_enabled());
        assert_eq!(chip.layer_slot(), None);
        assert_eq!(stack.is_query_pending(), pending);
        assert_eq!(visible(&stack), expected_before);
        for index in 0..LAYER_COLOR_SLOT_COUNT {
            assert_eq!(
                slots.holders_of(LayerColorSlot::from_stored_index(index)),
                0
            );
        }
        stack.wait_for_queries();
        assert_eq!(visible(&stack), [0, 2]);
    }

    #[test]
    fn a_filter_that_matches_nothing_yet_cannot_become_a_chip() {
        let (mut stack, _slots) = unfiltered_stack();

        assert_eq!(stack.add_live_filter_as_chip(), None);

        stack.set_live_filter_regex(true);
        stack.set_live_filter_text("navsyncd(");
        assert_eq!(stack.add_live_filter_as_chip(), None);
        assert!(stack.chips().is_empty());
    }

    /// The compare-phenomena-spatially mode: a layer chip colours the map
    /// without taking a line out of the table.
    #[test]
    fn a_layer_chip_leaves_the_table_alone_and_a_refine_chip_narrows_it() {
        let (mut stack, mut slots) = unfiltered_stack();
        let id = add_layer_chip(&mut stack, &mut slots, "gnss");
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
        let gnss = add_layer_chip(&mut stack, &mut slots, "gnss");
        let battery = add_layer_chip(&mut stack, &mut slots, "battery");
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
        let id = add_layer_chip(&mut stack, &mut slots, "gnss");
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
        let first = add_layer_chip(&mut stack, &mut slots, "gnss");
        let second = add_layer_chip(&mut stack, &mut slots, "battery");
        assert_eq!(slot_index(&stack, first), Some(0));
        assert_eq!(slot_index(&stack, second), Some(1));

        stack.remove_chip(first, &mut slots);

        assert_eq!(chip_ids(&stack), [second]);
        let readded = add_layer_chip(&mut stack, &mut slots, "critical");
        assert_eq!(
            slot_index(&stack, readded),
            Some(0),
            "the freed colour is the lowest one free again"
        );
    }

    #[test]
    fn switching_a_chip_out_of_layer_mode_and_back_takes_the_lowest_free_colour() {
        let (mut stack, mut slots) = unfiltered_stack();
        let first = add_layer_chip(&mut stack, &mut slots, "gnss");
        let second = add_layer_chip(&mut stack, &mut slots, "battery");

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
        let gnss = add_layer_chip(&mut stack, &mut slots, "gnss");
        let critical = add_layer_chip(&mut stack, &mut slots, "battery critical");
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
        let gnss = add_layer_chip(&mut stack, &mut slots, "gnss");
        let battery = add_layer_chip(&mut stack, &mut slots, "battery");
        stack.switch_chip_to_refine_mode(gnss, &mut slots);
        stack.set_chip_enabled(battery, false);

        let stored = stack.to_stored_stack().expect("valid groups");
        assert_eq!(
            stored.chips(),
            [
                StoredLogFilter {
                    group_id: 0,
                    text: "gnss".to_owned(),
                    regex: false,
                    enabled: true,
                    mode: StoredLogFilterMode::Refine,
                },
                StoredLogFilter {
                    group_id: 0,
                    text: "battery".to_owned(),
                    regex: false,
                    enabled: false,
                    mode: StoredLogFilterMode::Layer { color_slot: 1 },
                },
            ]
        );

        let log = Arc::new(test_util::parsed_log_of_text(LOG));
        let mut restored = FilterStack::from_stored_stack(log, &stored);
        restored.wait_for_queries();

        assert_eq!(restored.to_stored_stack().expect("valid groups"), stored);
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
        let stored: StoredLogFilterStack = vec![StoredLogFilter {
            group_id: 0,
            text: "^navsyncd".to_owned(),
            regex: true,
            enabled: true,
            mode: StoredLogFilterMode::Layer {
                color_slot: LAYER_COLOR_SLOT_COUNT + 3,
            },
        }]
        .into();

        let log = Arc::new(test_util::parsed_log_of_text(LOG));
        let mut restored = FilterStack::from_stored_stack(log, &stored);
        restored.wait_for_queries();

        let chip = restored.chips().first().expect("the chip was restored");
        assert_eq!(chip.pattern(), &FilterPattern::regex("^navsyncd"));
        assert_eq!(chip.mode(), FilterChipMode::Layer);
        assert_eq!(chip.matches().match_count(), 2);
    }

    proptest! {
        /// Whatever the user writes into the field and adds as a refine chip,
        /// the table shows exactly the entries a walk of the log selects, in
        /// file order, and never an entry missing from the log.
        #[test]
        fn the_visible_entries_are_what_a_walk_of_the_log_selects(
            live in "[a-z ]{0,5}",
            refine in "[a-z ]{0,5}",
            chip_enabled in any::<bool>(),
            any_operator in any::<bool>(),
        ) {
            let (mut stack, mut slots) = unfiltered_stack();
            stack.set_live_filter_text(&refine);
            let chip = stack.add_live_filter_as_chip();
            if let Some(id) = chip {
                stack.switch_chip_to_refine_mode(id, &mut slots);
                stack.set_chip_enabled(id, chip_enabled);
            }
            stack.set_live_filter_text(&live);
            stack.set_group_operator(stack.selected_group(), if any_operator { FilterGroupOperator::Any } else { FilterGroupOperator::All });
            stack.wait_for_queries();

            let live_filter = FilterPattern::plain(&live).compile().expect("plain compiles");
            let refine_filter = FilterPattern::plain(&refine).compile().expect("plain compiles");
            let refine_narrows = chip.is_some() && chip_enabled;
            let log = test_util::parsed_log_of_text(LOG);
            let expected: Vec<usize> = log
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    let message = log.message(entry);
                    let conditions: Vec<_> = [
                        (!live_filter.matches_nothing()).then(|| live_filter.matches(message)),
                        refine_narrows.then(|| refine_filter.matches(message)),
                    ].into_iter().flatten().collect();
                    conditions.is_empty() || if any_operator {
                        conditions.iter().any(|matched| *matched)
                    } else {
                        conditions.iter().all(|matched| *matched)
                    }
                })
                .map(|(index, _)| index)
                .collect();

            prop_assert_eq!(visible(&stack), expected.clone());
            prop_assert_eq!(stack.visible_entries().len(), expected.len());
            prop_assert!(expected.len() <= stack.entry_count());
        }
    }

    const COMPOSITION_LOG: &str = "\
2026-01-01 14:02:11 first
2026-01-01 14:02:12 first second
2026-01-01 14:02:13 second
2026-01-01 14:02:14 excluded
";

    /// A filter can select a service, a phenomenon, or one line: two services
    /// write two lines each.
    const LOG: &str = "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
2026-01-01 14:02:12 hal-powerd: battery low
2026-01-01 14:02:13 navsyncd: gnss fix lost
2026-01-01 14:02:14 hal-powerd: battery critical
";
}
