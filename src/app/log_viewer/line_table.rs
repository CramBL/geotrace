use std::ops::Range;
use std::sync::Arc;

use chrono::Duration;
use egui::{
    Color32, InteractOptions, Label, RichText, ScrollArea, Separator, Shape, TextFormat,
    text::LayoutJob,
};
use gt_fmt::MIDDLE_DOT;
use gt_log_view::{
    ClockTicks, EntryMatches, FilterStack, LoadedLog, TimestampTick, VisibleEntries,
};
use gt_logfile::{
    BootSession, LogEntry, LogLevelKind, ParsedLog, RecognisedMessage, StructuralLineKind,
    TimestampKind,
};
use gt_types::{Latitude, Longitude, mercator};
use gt_ui_theme::ALMOST_EQUAL_TO;
use gt_ui_theme::EM_DASH;
use gt_ui_types::{LoadedLogId, LogMatchGlyph, LogMatchHover, LogRowPlacement};
use rustc_hash::FxHashMap;

use super::{AssociationWindowUnit, DATE_FORMAT, LogViewerWindow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LineTableRow {
    BootDivider {
        session_index: usize,
        structural_index: Option<usize>,
    },
    DayDivider {
        entry_index: usize,
    },
    Entry {
        entry_index: usize,
        visible_row: usize,
    },
    Structural {
        structural_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiagnosticTarget {
    BootSession(usize),
    Entry(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiagnosticReveal {
    pub(super) log: LoadedLogId,
    pub(super) semantic_revision: u64,
    pub(super) entry_index: usize,
}

#[derive(Debug)]
pub(super) struct LineTableRows {
    rows: Vec<LineTableRow>,
    ticks: ClockTicks,
    largest_line_number: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RowCacheKey {
    log: LoadedLogId,
    visible_revision: u64,
    show_structural_lines: bool,
    revealed_entry: Option<usize>,
}

pub(super) struct LineTableCache {
    key: RowCacheKey,
    rows: Arc<LineTableRows>,
}

#[derive(Default)]
struct SourceBootDividers {
    separator_for_session: Vec<Option<usize>>,
    session_for_separator: Vec<Option<usize>>,
}

impl SourceBootDividers {
    fn of(parsed: &ParsedLog, show_structural_lines: bool) -> Self {
        if !show_structural_lines {
            return Self::default();
        }
        let mut dividers = Self {
            separator_for_session: vec![None; parsed.boot_sessions().len()],
            session_for_separator: vec![None; parsed.structural_lines().len()],
        };
        for (index, session) in parsed.boot_sessions().iter().enumerate() {
            let Some(first) = parsed.entries().get(session.entry_range.start) else {
                continue;
            };
            let Some(separator_index) = parsed
                .structural_lines()
                .partition_point(|line| line.line_number < first.line_number)
                .checked_sub(1)
            else {
                continue;
            };
            if parsed
                .structural_lines()
                .get(separator_index)
                .is_some_and(|line| line.kind == StructuralLineKind::RebootSeparator)
            {
                if let Some(slot) = dividers.separator_for_session.get_mut(index) {
                    *slot = Some(separator_index);
                }
                if let Some(slot) = dividers.session_for_separator.get_mut(separator_index) {
                    *slot = Some(index);
                }
            }
        }
        dividers
    }

    fn row_of_structural_line(&self, structural_index: usize) -> LineTableRow {
        self.session_for_separator
            .get(structural_index)
            .copied()
            .flatten()
            .map_or(
                LineTableRow::Structural { structural_index },
                |session_index| LineTableRow::BootDivider {
                    session_index,
                    structural_index: Some(structural_index),
                },
            )
    }
}

impl LineTableRows {
    #[cfg(test)]
    pub(super) fn of(log: &LoadedLog) -> Self {
        Self::with_overlay(log, false, None)
    }

    fn with_overlay(
        log: &LoadedLog,
        show_structural_lines: bool,
        revealed_entry: Option<usize>,
    ) -> Self {
        let parsed = log.parsed();
        let filtered = log.filters().visible_entries();
        let overlay;
        let visible = if let Some(entry_index) =
            revealed_entry.filter(|index| *index < parsed.entries().len())
        {
            let mut entries: Vec<_> = filtered.entry_indices().collect();
            if let Err(position) = entries.binary_search(&entry_index) {
                entries.insert(position, entry_index);
            }
            overlay = VisibleEntries::Matching(entries);
            &overlay
        } else {
            filtered
        };
        let ticks = ClockTicks::of(parsed, visible);
        let source_boot_dividers = SourceBootDividers::of(parsed, show_structural_lines);
        let mut rows = Vec::with_capacity(visible.len());
        let mut structural = parsed.structural_lines().iter().enumerate().peekable();
        let mut previous_session = None;
        let mut largest_line_number = 0;
        let mut dividers = ticks.day_dividers().iter().peekable();
        let mut session_index = 0;
        for (visible_row, entry_index) in visible.entry_indices().enumerate() {
            let Some(entry) = parsed.entries().get(entry_index) else {
                continue;
            };
            while parsed
                .boot_sessions()
                .get(session_index)
                .is_some_and(|session| session.entry_range.end <= entry_index)
            {
                session_index += 1;
            }
            let opens_session = previous_session != Some(session_index);
            let separator_index = source_boot_dividers
                .separator_for_session
                .get(session_index)
                .copied()
                .flatten();
            while let Some(&(structural_index, line)) = structural.peek() {
                if line.line_number >= entry.line_number {
                    break;
                }
                structural.next();
                if !show_structural_lines {
                    continue;
                }
                largest_line_number = largest_line_number.max(line.line_number);
                if opens_session && separator_index == Some(structural_index) {
                    if dividers
                        .peek()
                        .is_some_and(|divider| divider.visible_row == visible_row)
                    {
                        dividers.next();
                        rows.push(LineTableRow::DayDivider { entry_index });
                    }
                    rows.push(LineTableRow::BootDivider {
                        session_index,
                        structural_index: Some(structural_index),
                    });
                } else {
                    rows.push(source_boot_dividers.row_of_structural_line(structural_index));
                }
            }
            if dividers
                .peek()
                .is_some_and(|divider| divider.visible_row == visible_row)
            {
                dividers.next();
                rows.push(LineTableRow::DayDivider { entry_index });
            }
            if opens_session && separator_index.is_none() {
                rows.push(LineTableRow::BootDivider {
                    session_index,
                    structural_index: None,
                });
            }
            previous_session = Some(session_index);
            largest_line_number = largest_line_number.max(entry.line_number);
            rows.push(LineTableRow::Entry {
                entry_index,
                visible_row,
            });
        }
        if show_structural_lines {
            for (structural_index, line) in structural {
                largest_line_number = largest_line_number.max(line.line_number);
                rows.push(source_boot_dividers.row_of_structural_line(structural_index));
            }
        }
        Self {
            rows,
            ticks,
            largest_line_number,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn at(&self, row: usize) -> Option<LineTableRow> {
        self.rows.get(row).copied()
    }

    pub(super) fn row_of_boot_divider(&self, target: usize) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, LineTableRow::BootDivider { session_index, .. } if *session_index == target))
    }

    pub(super) fn row_of_exact_entry(&self, target: usize) -> Option<usize> {
        self.row_of_entry(target).filter(|&row| {
            matches!(self.at(row), Some(LineTableRow::Entry { entry_index, .. }) if entry_index == target)
        })
    }

    pub(super) fn row_of_entry(&self, target: usize) -> Option<usize> {
        self.rows.iter().position(
            |row| matches!(row, LineTableRow::Entry { entry_index, .. } if *entry_index >= target),
        )
    }
}

impl LogViewerWindow {
    pub(super) fn clear_invalid_diagnostic_reveal(&mut self, log: &LoadedLog, log_id: LoadedLogId) {
        if self.diagnostic_reveal.is_some_and(|reveal| {
            reveal.log != log_id || reveal.semantic_revision != log.filters().semantic_revision()
        }) {
            self.diagnostic_reveal = None;
            self.scroll_to_row = None;
        }
    }

    pub(super) fn table_rows(
        &mut self,
        log: &LoadedLog,
        log_id: LoadedLogId,
    ) -> Arc<LineTableRows> {
        self.clear_invalid_diagnostic_reveal(log, log_id);
        let key = RowCacheKey {
            log: log_id,
            visible_revision: log.filters().visible_revision(),
            show_structural_lines: self.show_structural_lines,
            revealed_entry: self.diagnostic_reveal.map(|reveal| reveal.entry_index),
        };
        match &mut self.line_table_cache {
            Some(cache) if cache.key == key => Arc::clone(&cache.rows),
            slot => {
                let rows = Arc::new(LineTableRows::with_overlay(
                    log,
                    self.show_structural_lines,
                    key.revealed_entry,
                ));
                *slot = Some(LineTableCache {
                    key,
                    rows: Arc::clone(&rows),
                });
                rows
            }
        }
    }

    pub(super) fn navigate_to_diagnostic(
        &mut self,
        log: &LoadedLog,
        log_id: LoadedLogId,
        target: DiagnosticTarget,
    ) {
        let filtered = log.filters().visible_entries();
        let (entry_index, hidden) = match target {
            DiagnosticTarget::BootSession(index) => {
                let Some(session) = log.parsed().boot_sessions().get(index) else {
                    return;
                };
                (
                    session.entry_range.start,
                    filtered.row_at_or_after(session.entry_range.start)
                        == filtered.row_at_or_after(session.entry_range.end),
                )
            }
            DiagnosticTarget::Entry(index) => {
                let visible_row = filtered.row_at_or_after(index);
                (index, filtered.entry_index(visible_row) != Some(index))
            }
        };
        if log.parsed().entries().get(entry_index).is_none() {
            return;
        }
        self.diagnostic_reveal = None;
        if hidden {
            self.diagnostic_reveal = Some(DiagnosticReveal {
                log: log_id,
                semantic_revision: log.filters().semantic_revision(),
                entry_index,
            });
        }
        let rows = self.table_rows(log, log_id);
        self.scroll_to_row = match target {
            DiagnosticTarget::BootSession(session) => rows.row_of_boot_divider(session),
            DiagnosticTarget::Entry(entry_index) => rows.row_of_exact_entry(entry_index),
        };
    }
}

/// What the table hands back to the app: where a click centres the map, and
/// the hover it shares with the map's hexagons.
pub(super) struct LineTableRequests<'a> {
    pub(super) map_center: &'a mut Option<(f64, f64)>,
    pub(super) hover: &'a mut LogMatchHover,
}

/// The line the pointer rests on and the moment it arrived there.
///
/// egui's `tooltip_delay` runs from the last pointer movement anywhere on
/// screen, and its grace time opens the next tooltip at once after another one
/// closed. The table times the pointer's stay on one line itself.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RowHoverDwell {
    entry_index: Option<usize>,
    arrived_at_secs: f64,
}

impl RowHoverDwell {
    /// Records the line the pointer is on this frame. Any change of line -
    /// including the pointer leaving the table - restarts the dwell.
    fn moved_to(&mut self, entry_index: Option<usize>, now_secs: f64) {
        if self.entry_index != entry_index {
            *self = Self {
                entry_index,
                arrived_at_secs: now_secs,
            };
        }
    }

    /// [`ASSOCIATED_ROW_HOVER`] once the pointer has rested on `entry_index`
    /// for [`ASSOCIATED_ROW_HOVER_DWELL_SECS`], and `None` until then. It
    /// requests a repaint for the time still to run, which opens the text
    /// under a pointer that stays where it is.
    fn associated_row_hover(
        &self,
        ctx: &egui::Context,
        entry_index: usize,
    ) -> Option<&'static str> {
        let rested_secs = match self.entry_index == Some(entry_index) {
            true => ctx.input(|input| input.time) - self.arrived_at_secs,
            false => 0.0,
        };
        let remaining_secs = f64::from(ASSOCIATED_ROW_HOVER_DWELL_SECS) - rested_secs;
        if remaining_secs > 0.0 {
            ctx.request_repaint_after_secs(remaining_secs as f32);
            return None;
        }
        Some(ASSOCIATED_ROW_HOVER)
    }
}

/// The rows a hexagon on the map marks in the table: the one under the cursor,
/// or the one last clicked while the cursor is on none.
///
/// A hexagon of another log than the one shown marks nothing: filter stacks
/// are per log, and marking rows across a log switch the reader did not
/// request would show them the wrong log's lines.
struct CrossHighlightedRows<'a> {
    glyph: Option<&'a LogMatchGlyph>,
    shown_log: LoadedLogId,
    fill: Color32,
}

impl<'a> CrossHighlightedRows<'a> {
    fn of(glyph: Option<&'a LogMatchGlyph>, shown_log: LoadedLogId, dark_mode: bool) -> Self {
        let fill = glyph.map_or(Color32::TRANSPARENT, |glyph| {
            gt_ui_theme::log_match_color(glyph.color, dark_mode)
                .gamma_multiply(CROSS_HIGHLIGHT_ROW_ALPHA)
        });
        Self {
            glyph,
            shown_log,
            fill,
        }
    }

    /// The background the row of `entry_index` draws behind it, `None` for a
    /// row whose entry the marking hexagon does not group.
    fn fill_of(&self, entry_index: usize) -> Option<Color32> {
        self.glyph?
            .covers(self.shown_log, entry_index)
            .then_some(self.fill)
    }
}

impl LogViewerWindow {
    pub(super) fn line_table_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        log_id: LoadedLogId,
        pointer_over_the_window: bool,
        requests: &mut LineTableRequests<'_>,
    ) {
        let parsed = log.parsed();
        let filters = log.filters();
        let rows = self.table_rows(log, log_id);
        let ticks = &rows.ticks;
        let line_numbers = LineNumberColumn::new(ui, rows.largest_line_number);
        let anomaly_steps: FxHashMap<usize, Duration> = parsed
            .order_anomalies()
            .iter()
            .map(|anomaly| {
                (
                    parsed
                        .entries()
                        .partition_point(|entry| entry.line_number < anomaly.line_number),
                    anomaly.timestamp_step,
                )
            })
            .collect();
        let unit = self.association_window_unit;
        let recognised_messages = parsed.recognised_messages();
        let color_switches = MessageColorSwitches {
            services: self.color_services,
            levels: self.color_levels,
        };
        let association_window = log.association_window();
        let dark_mode = ui.visuals().dark_mode;
        let gutter = LayerGutter::of(filters, dark_mode);
        let highlight = gt_ui_theme::LOG_LIVE_FILTER.resolve(dark_mode);
        let marking_hexagon = requests
            .hover
            .glyph
            .as_ref()
            .or(self.clicked_glyph.as_ref());
        let cross_highlighted = CrossHighlightedRows::of(marking_hexagon, log_id, dark_mode);
        let hover_dwell = self.row_hover_dwell;
        let mut hovered_row_placement = None;
        let mut hovered_entry_index = None;

        ui.scope(|ui| {
            // Rows sit directly on top of each other, so the table reads as one
            // block of text and a row's index times its height is its offset.
            ui.spacing_mut().item_spacing.y = 0.0;
            let row_height = row_height(ui);
            let mut scroll_area = ScrollArea::vertical()
                .id_salt(("log_viewer_line_table", log_id))
                .auto_shrink([false, false])
                // A keyboard step lands on a row boundary and a held key counts
                // every repeat, which an animation in flight would round off.
                .animated(false)
                // egui keeps a 64px floor by default, which on a short window
                // pushes the footer off the bottom. The table takes exactly the
                // room the window has left.
                .min_scrolled_height(0.0);
            if let Some(row) = self.scroll_to_row.take() {
                scroll_area = scroll_area.vertical_scroll_offset(row as f32 * row_height);
            }
            scroll_area.show_rows(ui, row_height, rows.len(), |ui, shown| {
                if pointer_over_the_window && ui.memory(|memory| memory.focused()).is_none() {
                    KeyboardScrollSteps {
                        row_height_px: row_height,
                        page_height_px: ui.clip_rect().height(),
                    }
                    .scroll_the_table(ui);
                }
                for row in shown {
                    match rows.at(row) {
                        Some(LineTableRow::BootDivider {
                            session_index,
                            structural_index,
                        }) => {
                            if let Some(session) = parsed.boot_sessions().get(session_index) {
                                ui.horizontal(|ui| {
                                    if let Some(line) = structural_index
                                        .and_then(|index| parsed.structural_lines().get(index))
                                    {
                                        ui.allocate_space(egui::vec2(gutter.width_px(), 0.0));
                                        line_numbers.ui(ui, line.line_number);
                                        let source = line.text.in_text(parsed.text());
                                        let source_width = ui
                                            .painter()
                                            .layout_no_wrap(
                                                source.to_owned(),
                                                egui::TextStyle::Monospace.resolve(ui.style()),
                                                ui.visuals().text_color(),
                                            )
                                            .size()
                                            .x
                                            .min(
                                                ui.available_width()
                                                    * BOOT_DIVIDER_SOURCE_WIDTH_FRACTION,
                                            );
                                        ui.add_sized(
                                            egui::vec2(source_width, row_height),
                                            Label::new(RichText::new(source).monospace())
                                                .truncate()
                                                .selectable(true),
                                        )
                                        .on_hover_text(source);
                                    }
                                    boot_divider_row_ui(ui, session);
                                });
                            }
                        }
                        Some(LineTableRow::DayDivider { entry_index }) => {
                            if let Some(entry) = parsed.entries().get(entry_index) {
                                let date = entry.timestamp.format(DATE_FORMAT).to_string();
                                divider_row_ui(ui, RichText::new(date).monospace());
                            }
                        }
                        Some(LineTableRow::Structural { structural_index }) => {
                            if let Some(line) = parsed.structural_lines().get(structural_index) {
                                ui.horizontal(|ui| {
                                    ui.allocate_space(egui::vec2(gutter.width_px(), 0.0));
                                    line_numbers.ui(ui, line.line_number);
                                    ui.add(
                                        Label::new(
                                            RichText::new(line.text.in_text(parsed.text()))
                                                .monospace(),
                                        )
                                        .truncate()
                                        .selectable(true),
                                    );
                                });
                            }
                        }
                        Some(LineTableRow::Entry {
                            entry_index,
                            visible_row,
                        }) => {
                            let Some(entry) = parsed.entries().get(entry_index) else {
                                continue;
                            };
                            let message = parsed.message(entry);
                            let placement = log.entry_placement(entry_index);
                            let interaction = EntryRow {
                                entry,
                                line_numbers,
                                message,
                                highlighted: HighlightedMessage {
                                    spans: filters.live_filter_match_spans(message),
                                    color: highlight,
                                },
                                position: placement.map(|placement| placement.position),
                                order_anomaly_step: anomaly_steps.get(&entry_index).copied(),
                                recognised: recognised_messages.get(entry_index).copied(),
                                color_switches,
                                association_window,
                                gutter: &gutter,
                                entry_index,
                                tick: ticks.tick(visible_row),
                                cross_highlight_fill: cross_highlighted.fill_of(entry_index),
                                hover_dwell,
                            }
                            .ui(ui, unit);
                            let Some(RowInteraction { clicked }) = interaction else {
                                continue;
                            };
                            hovered_entry_index = Some(entry_index);
                            let Some(placement) = placement else {
                                continue;
                            };
                            let (latitude, longitude) = placement.position;
                            hovered_row_placement = Some(LogRowPlacement {
                                merc: mercator::normalize(latitude, longitude),
                                track: placement.fix.track,
                            });
                            if clicked {
                                *requests.map_center =
                                    Some((latitude.as_degrees(), longitude.as_degrees()));
                            }
                        }
                        None => {}
                    }
                }
            });
        });
        self.row_hover_dwell
            .moved_to(hovered_entry_index, ui.input(|input| input.time));
        requests.hover.row_placement = hovered_row_placement;
    }
}

/// The height one row of the table draws at. A row is a `ui.horizontal`, which
/// claims the interactive height whenever the text is shorter than it. The
/// virtualized rows keep step with the drawn ones only at this height.
pub(super) fn row_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Monospace)
        .max(ui.spacing().interact_size.y)
}

struct KeyboardScrollSteps {
    row_height_px: f32,
    page_height_px: f32,
}

impl KeyboardScrollSteps {
    /// Scrolls the table by the arrow and page keys pressed this frame. Each
    /// press is consumed, which leaves nothing for another widget to act on.
    /// An auto-repeat counts as a press of its own: holding a key keeps
    /// scrolling.
    fn scroll_the_table(&self, ui: &egui::Ui) {
        let distance_px: f32 = [
            (egui::Key::ArrowDown, self.row_height_px),
            (egui::Key::ArrowUp, -self.row_height_px),
            (egui::Key::PageDown, self.page_height_px),
            (egui::Key::PageUp, -self.page_height_px),
        ]
        .into_iter()
        .map(|(key, step_px)| {
            let presses = ui
                .ctx()
                .input_mut(|input| input.count_and_consume_key(egui::Modifiers::NONE, key));
            presses as f32 * step_px
        })
        .sum();
        // egui reads the delta as a movement of the content, which runs
        // against the scroll offset: a step towards the end of the log is
        // negative.
        ui.scroll_with_delta(egui::vec2(0.0, -distance_px));
    }
}

/// The gutter left of the table: the order-anomaly column, then one column per
/// enabled layer chip in chip order. One chip's bars line up down the table.
struct LayerGutter<'a> {
    columns: Vec<LayerGutterColumn<'a>>,
}

struct LayerGutterColumn<'a> {
    matched_entries: &'a EntryMatches,
    color: Color32,
}

impl<'a> LayerGutter<'a> {
    fn of(filters: &'a FilterStack, dark_mode: bool) -> Self {
        Self {
            columns: filters
                .enabled_layer_chips()
                .map(|(slot, chip)| LayerGutterColumn {
                    matched_entries: chip.matches(),
                    color: gt_ui_theme::log_layer_slot_color(slot.index()).resolve(dark_mode),
                })
                .collect(),
        }
    }

    fn width_px(&self) -> f32 {
        ANOMALY_COLUMN_WIDTH_PX + self.columns.len() as f32 * LAYER_COLUMN_WIDTH_PX
    }
}

#[derive(Clone, Copy)]
struct LineNumberColumn {
    width_px: f32,
}

impl LineNumberColumn {
    fn new(ui: &egui::Ui, largest: u32) -> Self {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let width_px = ui
            .painter()
            .layout_no_wrap(largest.to_string(), font, ui.visuals().weak_text_color())
            .size()
            .x;
        Self { width_px }
    }

    fn ui(self, ui: &mut egui::Ui, line_number: u32) {
        ui.allocate_ui_with_layout(
            egui::vec2(self.width_px, ui.spacing().interact_size.y),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.add(
                    Label::new(RichText::new(line_number.to_string()).monospace().weak())
                        .selectable(true),
                );
            },
        );
    }
}

/// One entry of the log, as the table draws it.
struct EntryRow<'a> {
    entry: &'a LogEntry,
    line_numbers: LineNumberColumn,
    message: &'a str,
    highlighted: HighlightedMessage,
    position: Option<(Latitude, Longitude)>,

    /// The backwards step this entry opens, for the rows an order anomaly
    /// starts at.
    order_anomaly_step: Option<Duration>,

    recognised: Option<RecognisedMessage>,

    color_switches: MessageColorSwitches,

    association_window: Duration,
    gutter: &'a LayerGutter<'a>,
    entry_index: usize,

    /// How strongly this row's hour and minute draw.
    tick: TimestampTick,

    /// The background of a row the map's hovered hexagon stands for.
    cross_highlight_fill: Option<Color32>,

    hover_dwell: RowHoverDwell,
}

/// What the cursor did to the row under it, built only for the row the pointer
/// is on.
struct RowInteraction {
    clicked: bool,
}

/// Where the live filter matched the message, and the colour reserved for it.
struct HighlightedMessage {
    spans: Vec<Range<usize>>,
    color: Color32,
}

/// One stretch of a message the table draws in its own colour.
#[derive(Debug, PartialEq, Eq)]
struct MessageRun {
    range: Range<usize>,
    color: Color32,
}

/// Which recognised parts of a message the table draws in their own colour, as
/// the filter row's two tickboxes set them. The hostname follows the services
/// tickbox.
#[derive(Clone, Copy)]
struct MessageColorSwitches {
    services: bool,
    levels: bool,
}

/// The colours one row's message is drawn in.
struct MessageColors {
    /// Everything the parse recognised nothing in.
    base: Color32,

    /// What the hostname and a debug level draw in.
    weak: Color32,

    dark_mode: bool,

    /// Whether the service takes its own colour, which it does with the
    /// services tickbox ticked and only on a line that has a position:
    /// association is the stronger signal.
    color_the_service: bool,

    color_the_hostname: bool,

    color_the_level: bool,
}

impl MessageColors {
    fn of(ui: &egui::Ui, associated: bool, switches: MessageColorSwitches) -> Self {
        Self {
            base: match associated {
                true => ui.visuals().text_color(),
                false => ui.visuals().weak_text_color(),
            },
            weak: ui.visuals().weak_text_color(),
            dark_mode: ui.visuals().dark_mode,
            color_the_service: switches.services && associated,
            color_the_hostname: switches.services,
            color_the_level: switches.levels,
        }
    }

    /// The runs of one message, in message order and never overlapping: the
    /// hostname, then the service, then the level. An info level draws in the
    /// base colour, which needs no run.
    fn runs(&self, recognised: RecognisedMessage) -> Vec<MessageRun> {
        let mut runs = Vec::with_capacity(3);
        if let Some(range) = recognised.hostname().filter(|_| self.color_the_hostname) {
            runs.push(MessageRun {
                range,
                color: self.weak,
            });
        }
        if let Some(service) = recognised.service().filter(|_| self.color_the_service) {
            runs.push(MessageRun {
                range: service.span(),
                color: gt_ui_theme::log_service_slot_color(usize::from(service.slot()))
                    .resolve(self.dark_mode),
            });
        }
        if let Some(level) = recognised.level().filter(|_| self.color_the_level) {
            let color = match level.kind() {
                LogLevelKind::Error => Some(gt_ui_theme::error_indicator(self.dark_mode)),
                LogLevelKind::Warning => Some(gt_ui_theme::warning_amber(self.dark_mode)),
                LogLevelKind::Debug => Some(self.weak),
                LogLevelKind::Info => None,
            };
            if let Some(color) = color {
                runs.push(MessageRun {
                    range: level.span(),
                    color,
                });
            }
        }
        runs
    }
}

impl EntryRow<'_> {
    /// Renders the row, returning what the cursor did to it.
    fn ui(&self, ui: &mut egui::Ui, unit: AssociationWindowUnit) -> Option<RowInteraction> {
        let associated = self.position.is_some();
        let interpolated = self.entry.timestamp_kind == TimestampKind::Interpolated;
        let tick = match associated {
            true => self.tick,
            false => self.tick.min(TimestampTick::Plain),
        };
        // Claimed before the row draws: the fill belongs behind its text.
        let background = ui.painter().add(Shape::Noop);
        let row = ui
            .horizontal(|ui| {
                self.gutter_ui(ui);
                self.line_numbers.ui(ui, self.entry.line_number);
                let timestamp = ui.add(Label::new(self.timestamp_job(ui, tick)).selectable(true));
                if interpolated {
                    timestamp.on_hover_text(INTERPOLATED_TIMESTAMP_HOVER);
                }
                let message = self.message_job(ui);
                ui.add(Label::new(message).truncate().selectable(true));
            })
            .response;
        if let Some(fill) = self.cross_highlight_fill {
            ui.painter()
                .set(background, Shape::rect_filled(row.rect, 0, fill));
        }

        // Registered above the labels the row just drew: a selectable label
        // senses the pointer, and the topmost sensing widget under the cursor
        // takes the hover and the click.
        let row = ui.interact_opt(
            row.rect,
            row.id,
            match associated {
                true => egui::Sense::click(),
                false => egui::Sense::hover(),
            },
            InteractOptions { move_to_top: true },
        );
        if !row.hovered() {
            return None;
        }
        let row = match self.hover_text(ui.ctx(), unit) {
            Some(hover) => row.on_hover_text(hover),
            None => row,
        };
        Some(RowInteraction {
            clicked: row.clicked(),
        })
    }

    fn hover_text(&self, ctx: &egui::Context, unit: AssociationWindowUnit) -> Option<String> {
        match (self.order_anomaly_step, self.position) {
            (Some(step), _) => Some(format!(
                "Timestamp steps back {} here with no recorded clock change {EM_DASH} the log \
                 may have been edited or spliced",
                gt_fmt::format_human_terse_duration(step.abs())
            )),
            (None, Some(_)) => self
                .hover_dwell
                .associated_row_hover(ctx, self.entry_index)
                .map(str::to_owned),
            (None, None) => Some(format!(
                "No GPS fix within {} of this line",
                unit.describe(self.association_window)
            )),
        }
    }

    /// The timestamp column in three runs: the date, the hour and minute, and
    /// the seconds. The date and the seconds are always drawn in the quiet
    /// colour, and only the hour and minute take the timestamp tick.
    fn timestamp_job(&self, ui: &egui::Ui, tick: TimestampTick) -> LayoutJob {
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let quiet = ui.visuals().weak_text_color();
        let moved = match tick {
            TimestampTick::Weak => ui.visuals().weak_text_color(),
            TimestampTick::Plain => ui.visuals().text_color(),
            TimestampTick::Strong => ui.visuals().strong_text_color(),
        };
        let prefix = match self.entry.timestamp_kind {
            TimestampKind::Anchored => ANCHORED_TIMESTAMP_PREFIX,
            TimestampKind::Interpolated => ALMOST_EQUAL_TO,
        };
        let timestamp = self.entry.timestamp;
        let mut job = LayoutJob::default();
        job.append(
            &format!("{prefix}{} ", timestamp.format(DATE_FORMAT)),
            0.0,
            TextFormat::simple(font_id.clone(), quiet),
        );
        job.append(
            &timestamp.format(HOUR_MINUTE_FORMAT).to_string(),
            0.0,
            TextFormat::simple(font_id.clone(), moved),
        );
        job.append(
            &timestamp.format(SECONDS_FORMAT).to_string(),
            0.0,
            TextFormat::simple(font_id, quiet),
        );
        job
    }

    /// The message: the service, level and hostname the parse recognised in
    /// their own colours, and what the live filter matched over those. The
    /// caller truncates it to one row: a long line must not push the rows
    /// below it out of the virtualized table's grid, nor stretch the window
    /// past the screen.
    fn message_job(&self, ui: &egui::Ui) -> LayoutJob {
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let colors = MessageColors::of(ui, self.position.is_some(), self.color_switches);
        let runs = self
            .recognised
            .map(|recognised| colors.runs(recognised))
            .unwrap_or_default();
        let mut job = LayoutJob::default();
        let mut appended_to = 0;
        for span in &self.highlighted.spans {
            let Some(matched) = self.message.get(span.clone()) else {
                continue;
            };
            self.append_runs(&mut job, &font_id, appended_to..span.start, &runs, &colors);
            job.append(
                matched,
                0.0,
                egui::TextFormat::simple(font_id.clone(), self.highlighted.color),
            );
            appended_to = span.end;
        }
        self.append_runs(
            &mut job,
            &font_id,
            appended_to..self.message.len(),
            &runs,
            &colors,
        );
        job
    }

    /// Appends `range` of the message, cut at each recognised run inside it:
    /// the run in its own colour, everything else in the base colour.
    fn append_runs(
        &self,
        job: &mut LayoutJob,
        font_id: &egui::FontId,
        range: Range<usize>,
        runs: &[MessageRun],
        colors: &MessageColors,
    ) {
        let mut appended_to = range.start;
        for run in runs {
            let start = run.range.start.max(appended_to);
            let end = run.range.end.min(range.end);
            if start >= end {
                continue;
            }
            if let Some(before) = self.message.get(appended_to..start) {
                job.append(
                    before,
                    0.0,
                    egui::TextFormat::simple(font_id.clone(), colors.base),
                );
            }
            if let Some(text) = self.message.get(start..end) {
                job.append(
                    text,
                    0.0,
                    egui::TextFormat::simple(font_id.clone(), run.color),
                );
            }
            appended_to = end;
        }
        if let Some(rest) = self.message.get(appended_to..range.end) {
            job.append(
                rest,
                0.0,
                egui::TextFormat::simple(font_id.clone(), colors.base),
            );
        }
    }

    /// The warning-amber marker on the row an unexplained backwards timestamp
    /// step starts at, and a bar in every enabled layer chip's column this row
    /// matched.
    fn gutter_ui(&self, ui: &mut egui::Ui) {
        let height = ui.text_style_height(&egui::TextStyle::Monospace);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(self.gutter.width_px(), height),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        if self.order_anomaly_step.is_some() {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    rect.left_top(),
                    egui::vec2(ANOMALY_MARKER_WIDTH_PX, height),
                ),
                GUTTER_MARKER_CORNER_RADIUS,
                gt_ui_theme::warning_amber(ui.visuals().dark_mode),
            );
        }
        for (column, layer) in self.gutter.columns.iter().enumerate() {
            if !layer.matched_entries.contains(self.entry_index) {
                continue;
            }
            let left =
                rect.left() + ANOMALY_COLUMN_WIDTH_PX + column as f32 * LAYER_COLUMN_WIDTH_PX;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(left, rect.top()),
                    egui::vec2(LAYER_BAR_WIDTH_PX, height),
                ),
                GUTTER_MARKER_CORNER_RADIUS,
                layer.color,
            );
        }
    }
}

/// One divider row: its label, and the rule running from it to the right
/// edge of the table.
fn divider_row_ui(ui: &mut egui::Ui, label: RichText) {
    ui.horizontal(|ui| {
        let full_label = label.text().to_owned();
        ui.add(Label::new(label).truncate().selectable(true))
            .on_hover_text(full_label);
        ui.add(Separator::default().horizontal());
    });
}

/// The divider opening one boot session, stating the run it starts.
fn boot_divider_row_ui(ui: &mut egui::Ui, session: &BootSession) {
    let uptime = session
        .uptime()
        .map_or_else(|| EM_DASH.to_owned(), gt_fmt::format_human_terse_duration);
    let entries = session.entry_count();
    let label = format!(
        "Boot {} {MIDDLE_DOT} up {uptime} {MIDDLE_DOT} {} {}",
        session.boot_number,
        gt_fmt::format_count(entries),
        gt_fmt::pluralize(entries, "entry", "entries"),
    );
    divider_row_ui(ui, RichText::new(label).monospace().strong());
}

const BOOT_DIVIDER_SOURCE_WIDTH_FRACTION: f32 = 0.5;

/// The prefix of an anchored timestamp, as wide as the interpolated marker so
/// that the timestamp column stays aligned.
const ANCHORED_TIMESTAMP_PREFIX: &str = " ";

/// The run of the timestamp column the timestamp tick colours.
const HOUR_MINUTE_FORMAT: &str = "%H:%M";

/// The run of the timestamp column after the hour and the minute, always drawn
/// in the quiet colour.
const SECONDS_FORMAT: &str = ":%S";

pub(super) const INTERPOLATED_TIMESTAMP_HOVER: &str =
    "Timestamp interpolated between neighbouring entries";

pub(super) const ASSOCIATED_ROW_HOVER: &str = "Centre the map on this line";

pub(super) const ASSOCIATED_ROW_HOVER_DWELL_SECS: u32 = 4;

/// Width of the gutter column holding the order-anomaly marker, keeping the
/// timestamp column aligned on the rows without one.
const ANOMALY_COLUMN_WIDTH_PX: f32 = 6.0;

const ANOMALY_MARKER_WIDTH_PX: f32 = 3.0;

/// Width of the bar a row takes in the gutter column of a layer chip it
/// matches.
const LAYER_BAR_WIDTH_PX: f32 = 4.0;

/// Gap between one layer chip's bars and the next chip's.
const LAYER_BAR_GAP_PX: f32 = 1.0;

/// Width one layer chip claims in the gutter: its bar and the gap after it.
const LAYER_COLUMN_WIDTH_PX: f32 = LAYER_BAR_WIDTH_PX + LAYER_BAR_GAP_PX;

const GUTTER_MARKER_CORNER_RADIUS: u8 = 1;

/// How strongly the rows of a marking hexagon are tinted in that hexagon's
/// colour: enough to find them in a scrolling table, light enough to read the
/// line through.
const CROSS_HIGHLIGHT_ROW_ALPHA: f32 = 0.3;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{DateTime, TimeZone as _, Utc};
    use gt_log_view::{FilterEffect, LayerColorSlots, LoadedLogs};
    use gt_ui_types::LogMatchColor;

    use super::*;

    fn gutter_log_start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 14, 2, 11)
            .single()
            .unwrap_or_default()
    }

    fn source_log(text: &str) -> LoadedLog {
        let parsed =
            gt_logfile::parse_log(text.into(), gutter_log_start()).expect("the log parses");
        LoadedLog::new(None, parsed, Duration::seconds(60))
    }

    #[test]
    fn source_rows_preserve_boot_day_entry_and_summary_order() {
        let log = source_log(
            "2026-01-01 23:59:59 first\n--- Device reboot ---\n2026-01-02 00:00:01 second\n----------- Journal summary -----------\ntrailing summary\n",
        );
        let rows = LineTableRows::with_overlay(&log, true, None);
        assert_eq!(
            rows.rows,
            [
                LineTableRow::BootDivider {
                    session_index: 0,
                    structural_index: None
                },
                LineTableRow::Entry {
                    entry_index: 0,
                    visible_row: 0
                },
                LineTableRow::DayDivider { entry_index: 1 },
                LineTableRow::BootDivider {
                    session_index: 1,
                    structural_index: Some(0)
                },
                LineTableRow::Entry {
                    entry_index: 1,
                    visible_row: 1
                },
                LineTableRow::Structural {
                    structural_index: 1
                },
                LineTableRow::Structural {
                    structural_index: 2
                },
            ]
        );
        assert_eq!(rows.largest_line_number, 5);
        assert_eq!(rows.row_of_boot_divider(1), Some(3));
        assert_eq!(rows.row_of_exact_entry(1), Some(4));
        assert_eq!(rows.at(rows.len()), None);
        assert_eq!(rows.ticks.tick(1), TimestampTick::Strong);
        let hidden = LineTableRows::of(&log);
        assert_eq!(hidden.largest_line_number, 3);
        assert_eq!(hidden.len(), 5);
    }

    #[test]
    fn a_revealed_entry_recomputes_day_dividers_and_ticks_from_displayed_entries() {
        let log = source_log(
            "2026-01-01 23:59:59 keep\n2026-01-02 00:00:01 hidden\n2026-01-02 00:00:02 keep\n",
        );
        let mut logs = LoadedLogs::default();
        let id = logs.push(log).id();
        let (stack, _) = logs.filter_stack_mut_by_id(id).expect("loaded");
        stack.set_live_filter_text("keep");
        stack.wait_for_queries();
        let log = logs.get_by_id(id).expect("loaded");
        let filtered = LineTableRows::of(log);
        assert_eq!(filtered.row_of_exact_entry(1), None);
        assert_eq!(filtered.row_of_entry(1), filtered.row_of_exact_entry(2));
        let revealed = LineTableRows::with_overlay(log, false, Some(1));
        assert_eq!(
            revealed.rows,
            [
                LineTableRow::BootDivider {
                    session_index: 0,
                    structural_index: None
                },
                LineTableRow::Entry {
                    entry_index: 0,
                    visible_row: 0
                },
                LineTableRow::DayDivider { entry_index: 1 },
                LineTableRow::Entry {
                    entry_index: 1,
                    visible_row: 1
                },
                LineTableRow::Entry {
                    entry_index: 2,
                    visible_row: 2
                },
            ]
        );
        assert_eq!(revealed.ticks.tick(1), TimestampTick::Strong);
        assert_eq!(revealed.ticks.tick(2), TimestampTick::Weak);
        assert_eq!(
            log.filters()
                .visible_entries()
                .entry_indices()
                .collect::<Vec<_>>(),
            [0, 2]
        );
    }

    /// A hexagon of `log` standing for `entry_indices`, as the map publishes
    /// one while the cursor is on it.
    fn hovering(log: LoadedLogId, entry_indices: &[usize]) -> LogMatchHover {
        LogMatchHover {
            glyph: Some(LogMatchGlyph {
                log,
                color: LogMatchColor::LayerSlot {
                    index: 0,
                    shared: false,
                },
                entry_indices: entry_indices.to_vec(),
            }),
            row_placement: None,
        }
    }

    /// The background a marked row draws: the hexagon's own palette colour,
    /// tinted down.
    fn marked_row_fill() -> Color32 {
        gt_ui_theme::log_layer_slot_color(0)
            .dark()
            .gamma_multiply(CROSS_HIGHLIGHT_ROW_ALPHA)
    }

    /// The map's hovered hexagon marks the rows of the lines it stands for,
    /// and only while it belongs to the log the viewer is showing.
    #[rstest::rstest]
    #[case::a_line_the_hexagon_stands_for(hovering(SHOWN_LOG, &[2, 5]), 5, Some(marked_row_fill()))]
    #[case::a_line_it_does_not(hovering(SHOWN_LOG, &[2, 5]), 4, None)]
    #[case::a_hexagon_of_another_log(hovering(OTHER_LOG, &[2, 5]), 5, None)]
    #[case::no_hexagon_hovered(LogMatchHover::default(), 5, None)]
    fn the_table_marks_the_rows_of_the_hovered_hexagon(
        #[case] hover: LogMatchHover,
        #[case] entry_index: usize,
        #[case] expected: Option<Color32>,
    ) {
        let marked = CrossHighlightedRows::of(hover.glyph.as_ref(), SHOWN_LOG, true);

        assert_eq!(marked.fill_of(entry_index), expected);
    }

    /// What the parse read out of the one entry of `text`.
    fn recognised_message_of(text: &str) -> RecognisedMessage {
        let parsed =
            gt_logfile::parse_log(text.into(), gutter_log_start()).expect("the log parses");
        parsed
            .recognised_messages()
            .first()
            .copied()
            .expect("the log has one entry")
    }

    /// The text each run covers, with the colour it draws in.
    fn coloured<'a>(message: &'a str, runs: &[MessageRun]) -> Vec<(&'a str, Color32)> {
        runs.iter()
            .filter_map(|run| Some((message.get(run.range.clone())?, run.color)))
            .collect()
    }

    /// Association is the stronger signal: a line the window found no fix for
    /// keeps its service in the weak colour of the rest of its message, and
    /// only its level stays coloured.
    #[test]
    fn a_line_without_a_position_colours_its_level_and_not_its_service() {
        let recognised = recognised_message_of(ERROR_LINE);
        let associated = MessageColors {
            base: Color32::from_gray(200),
            weak: Color32::from_gray(100),
            dark_mode: DARK_MODE,
            color_the_service: true,
            color_the_hostname: true,
            color_the_level: true,
        };
        let unassociated = MessageColors {
            color_the_service: false,
            ..associated
        };

        assert_eq!(
            coloured(ERROR_MESSAGE, &associated.runs(recognised)),
            [
                (
                    "hal-modem:",
                    gt_ui_theme::log_service_slot_color(0).resolve(DARK_MODE)
                ),
                (
                    "[ERROR modem::manager::modem]",
                    gt_ui_theme::error_indicator(DARK_MODE)
                ),
            ]
        );
        assert_eq!(
            coloured(ERROR_MESSAGE, &unassociated.runs(recognised)),
            [(
                "[ERROR modem::manager::modem]",
                gt_ui_theme::error_indicator(DARK_MODE)
            )]
        );
    }

    /// Two logged phenomena compared side by side: each enabled layer chip
    /// claims a column of the gutter, in its own palette colour.
    #[test]
    fn each_enabled_layer_chip_marks_the_rows_it_matched_in_its_own_column() {
        let log = Arc::new(
            gt_logfile::parse_log(GUTTER_LOG.into(), gutter_log_start()).expect("the log parses"),
        );
        let mut stack = FilterStack::new(log);
        let mut slots = LayerColorSlots::default();
        for text in ["gnss", "battery"] {
            stack.set_live_filter_text(text);
            let chip = stack
                .add_live_filter_as_chip()
                .expect("the filter is valid");
            stack.add_chip_effect(chip, FilterEffect::Map, &mut slots);
            stack.remove_chip_effect(chip, FilterEffect::Table, &mut slots);
        }
        stack.wait_for_queries();

        let gutter = LayerGutter::of(&stack, true);

        assert_eq!(
            gutter
                .columns
                .iter()
                .map(|column| column.color)
                .collect::<Vec<_>>(),
            [
                gt_ui_theme::log_layer_slot_color(0).dark(),
                gt_ui_theme::log_layer_slot_color(1).dark(),
            ]
        );
        assert_eq!(
            gutter
                .columns
                .iter()
                .map(|column| column.matched_entries.matched_entry_indices().collect())
                .collect::<Vec<Vec<usize>>>(),
            [vec![0, 2], vec![1]]
        );
        let expected_width = ANOMALY_COLUMN_WIDTH_PX + 2.0 * LAYER_COLUMN_WIDTH_PX;
        assert!(
            (gutter.width_px() - expected_width).abs() < f32::EPSILON,
            "the gutter makes room for both columns beside the anomaly marker, got {}",
            gutter.width_px()
        );
    }

    /// The gutter narrows back to the columns still marking rows: a chip
    /// switched off draws nothing.
    #[test]
    fn a_chip_that_is_switched_off_gives_up_its_gutter_column() {
        let log = Arc::new(
            gt_logfile::parse_log(GUTTER_LOG.into(), gutter_log_start()).expect("the log parses"),
        );
        let mut stack = FilterStack::new(log);
        let mut slots = LayerColorSlots::default();
        stack.set_live_filter_text("gnss");
        let chip = stack
            .add_live_filter_as_chip()
            .expect("a written filter becomes a chip");
        stack.add_chip_effect(chip, FilterEffect::Map, &mut slots);
        stack.remove_chip_effect(chip, FilterEffect::Table, &mut slots);
        stack.wait_for_queries();
        assert_eq!(LayerGutter::of(&stack, true).columns.len(), 1);

        stack.set_chip_effect_enabled(chip, FilterEffect::Map, false);

        assert_eq!(LayerGutter::of(&stack, true).columns.len(), 0);
    }

    /// The log the viewer is showing in the cross-highlight cases.
    const SHOWN_LOG: LoadedLogId = LoadedLogId::new(1);

    const OTHER_LOG: LoadedLogId = LoadedLogId::new(2);

    /// Two phenomena a filter can pick out, one of them logged twice.
    const GUTTER_LOG: &str = "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
2026-01-01 14:02:12 hal-powerd: battery low
2026-01-01 14:02:13 navsyncd: gnss fix lost
";

    /// A line stating a service and an error level, as the two colouring cases
    /// read it.
    const ERROR_LINE: &str =
        "2026-01-01 14:02:11 hal-modem: [ERROR modem::manager::modem] timed out";

    const ERROR_MESSAGE: &str = "hal-modem: [ERROR modem::manager::modem] timed out";

    /// The theme the runs resolve in.
    const DARK_MODE: bool = true;
}
