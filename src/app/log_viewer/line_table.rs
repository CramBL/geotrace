use std::mem;
use std::ops::Range;
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Duration, NaiveTime, Utc};
use egui::epaint::text::FontsView;
use egui::scroll_area::ScrollSource;
use egui::text::LayoutJob;
use egui::{Color32, FontId, IdSalt, ScrollArea, Separator, Stroke, StrokeKind, TextFormat};
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_DEVICE_CLOCK;
use gt_fmt::MIDDLE_DOT;
use gt_log_view::{
    ClockTicks, EntryMatches, FilterStack, LoadedLog, TimestampTick, VisibleEntries,
};
use gt_logfile::{
    BootSession, ClockEligibility, LogChronology, LogEntry, LogLevelKind, ParsedLog,
    RecognisedMessage, StructuralLineKind, TimestampKind, YearlessTimestamp,
};
use gt_types::{AssociationWindow, Latitude, Longitude, mercator};
use gt_ui_theme::{ALMOST_EQUAL_TO, EM_DASH};
use gt_ui_types::{LoadedLogId, LogMatchGlyph, LogMatchHover, LogRowPlacement};
use rustc_hash::FxHashMap;

use super::find::FindHighlight;
use super::keyboard_navigation::{KeyboardScrollSteps, TableFocusScope};
use super::{AssociationWindowUnit, DATE_FORMAT, LogViewerWindow};

use navigation::SourceOverlay;
use row_cells::{CopyProjection, RowCell, RowCells};
use row_grid::{RowGrid, ShownRows};
use row_layout::{RowLayoutRequest, RowLayoutStyle};
use text_selection::{TableInputFocus, TableTextUi};

pub(super) use navigation::{FilteredSourceReturn, SourceRequest, SourceReveal, SourceRow};
pub(super) use text_selection::TableTextSelection;

mod navigation;
mod row_cells;
mod row_grid;
mod row_layout;
mod text_selection;

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
    RenderRawSource {
        line_number: u32,
    },
    Structural {
        structural_index: usize,
    },
}

impl LineTableRow {
    fn source_line(self, parsed: &ParsedLog) -> Option<u32> {
        match self {
            Self::RenderRawSource { line_number } => Some(line_number),
            Self::Entry { entry_index, .. } | Self::DayDivider { entry_index } => parsed
                .entries()
                .get(entry_index)
                .map(|entry| entry.line_number),
            Self::Structural { structural_index } => parsed
                .structural_lines()
                .get(structural_index)
                .map(|line| line.line_number),
            Self::BootDivider {
                session_index,
                structural_index,
            } => structural_index
                .and_then(|index| parsed.structural_lines().get(index))
                .map(|line| line.line_number)
                .or_else(|| {
                    let session = parsed.boot_sessions().get(session_index)?;
                    parsed
                        .entries()
                        .get(session.entry_range.start)
                        .map(|entry| entry.line_number)
                }),
        }
    }

    fn content(self, parsed: &ParsedLog) -> Option<LineTableContent<'_>> {
        Some(match self {
            Self::Entry {
                entry_index,
                visible_row,
            } => {
                let entry = parsed.entries().get(entry_index)?;
                let timestamp = RowTimestamp::of(parsed, entry_index, entry);
                LineTableContent {
                    presentation: RowPresentation::Entry {
                        entry_index,
                        visible_row,
                        entry,
                        timestamp,
                    },
                    cells: RowCells([
                        Some(RowCell::Number(entry.line_number.to_string())),
                        Some(RowCell::Timestamp(TimestampText::of(timestamp))),
                        Some(RowCell::Message(parsed.message(entry))),
                    ]),
                }
            }
            Self::RenderRawSource { line_number } => NumberedSource {
                line_number,
                text: parsed.source_line(line_number)?,
            }
            .content(),
            Self::Structural { structural_index } => {
                NumberedSource::structural(parsed, structural_index)?.content()
            }
            Self::BootDivider {
                session_index,
                structural_index,
            } => {
                let session = parsed.boot_sessions().get(session_index)?;
                let source =
                    structural_index.and_then(|index| NumberedSource::structural(parsed, index));
                let title =
                    boot_divider_title(session, parsed.chronology().session_uptime(session_index));
                LineTableContent {
                    presentation: RowPresentation::Divider,
                    cells: RowCells([
                        source.map(|source| RowCell::Number(source.line_number.to_string())),
                        source.map(|source| RowCell::Source {
                            text: source.text,
                            boot: true,
                        }),
                        Some(RowCell::Divider {
                            text: title,
                            strong: true,
                        }),
                    ]),
                }
            }
            Self::DayDivider { entry_index } => {
                let time = parsed.chronology().resolved()?.eligible_time(entry_index)?;
                LineTableContent {
                    presentation: RowPresentation::Divider,
                    cells: RowCells([
                        Some(RowCell::Divider {
                            text: time.timestamp().format(DATE_FORMAT).to_string(),
                            strong: false,
                        }),
                        None,
                        None,
                    ]),
                }
            }
        })
    }
}

struct LineTableContent<'a> {
    presentation: RowPresentation<'a>,
    cells: RowCells<'a>,
}

enum RowPresentation<'a> {
    Divider,
    Entry {
        entry_index: usize,
        visible_row: usize,
        entry: &'a LogEntry,
        timestamp: Option<RowTimestamp>,
    },
    Source,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableRowIdentity {
    BootSession(usize),
    DayDivider(usize),
    Entry(usize),
    RenderRawSource(u32),
    Structural(usize),
}

impl From<LineTableRow> for TableRowIdentity {
    fn from(row: LineTableRow) -> Self {
        match row {
            LineTableRow::BootDivider { session_index, .. } => Self::BootSession(session_index),
            LineTableRow::DayDivider { entry_index } => Self::DayDivider(entry_index),
            LineTableRow::Entry { entry_index, .. } => Self::Entry(entry_index),
            LineTableRow::RenderRawSource { line_number } => Self::RenderRawSource(line_number),
            LineTableRow::Structural { structural_index } => Self::Structural(structural_index),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TableReadingAnchor {
    pub(super) log: LoadedLogId,
    pub(super) source_line: u32,
    row: TableRowIdentity,
    offset_within_row_px: f32,
}

#[derive(Clone, Copy, Debug)]
enum InsertedRowKind {
    BootDivider {
        session_index: usize,
        structural_index: Option<usize>,
    },
    DayDivider {
        entry_index: usize,
    },
    RenderRawSource {
        line_number: u32,
    },
    Structural {
        structural_index: usize,
    },
}

impl From<InsertedRowKind> for LineTableRow {
    fn from(row: InsertedRowKind) -> Self {
        match row {
            InsertedRowKind::BootDivider {
                session_index,
                structural_index,
            } => Self::BootDivider {
                session_index,
                structural_index,
            },
            InsertedRowKind::DayDivider { entry_index } => Self::DayDivider { entry_index },
            InsertedRowKind::RenderRawSource { line_number } => {
                Self::RenderRawSource { line_number }
            }
            InsertedRowKind::Structural { structural_index } => {
                Self::Structural { structural_index }
            }
        }
    }
}

#[derive(Debug)]
struct RevealedEntry {
    entry_index: usize,
    visible_row: usize,
}

#[derive(Debug)]
struct DisplayedEntries {
    filtered: Arc<VisibleEntries>,
    reveals: Vec<RevealedEntry>,
}

impl DisplayedEntries {
    fn new(log: &LoadedLog, overlay: Option<SourceOverlay>) -> Self {
        let filtered = log.filters().shared_visible_entries();
        let parsed = log.parsed();
        let mut reveals = Vec::new();
        if let Some(overlay) = overlay {
            let first = parsed
                .entries()
                .partition_point(|entry| entry.line_number < overlay.first_line);
            for (entry_index, entry) in parsed.entries().iter().enumerate().skip(first) {
                if entry.line_number > overlay.last_line {
                    break;
                }
                let visible_row = filtered.row_at_or_after(entry_index);
                if filtered.entry_index(visible_row) != Some(entry_index) {
                    reveals.push(RevealedEntry {
                        entry_index,
                        visible_row: visible_row + reveals.len(),
                    });
                }
            }
        }
        Self { filtered, reveals }
    }

    fn len(&self) -> usize {
        self.filtered.len() + self.reveals.len()
    }

    fn entry_index(&self, visible_row: usize) -> Option<usize> {
        let before = self
            .reveals
            .partition_point(|reveal| reveal.visible_row < visible_row);
        if let Some(reveal) = self
            .reveals
            .get(before)
            .filter(|reveal| reveal.visible_row == visible_row)
        {
            Some(reveal.entry_index)
        } else {
            self.filtered.entry_index(visible_row - before)
        }
    }

    fn row_at_or_after(&self, entry_index: usize) -> usize {
        self.filtered.row_at_or_after(entry_index)
            + self
                .reveals
                .partition_point(|reveal| reveal.entry_index < entry_index)
    }

    fn largest_line_number(&self, parsed: &ParsedLog, show_structural_lines: bool) -> u32 {
        let last_entry = self
            .len()
            .checked_sub(1)
            .and_then(|row| self.entry_index(row))
            .and_then(|entry| parsed.entries().get(entry))
            .map_or(0, |entry| entry.line_number);
        let last_structural = match show_structural_lines {
            true => parsed
                .structural_lines()
                .last()
                .map_or(0, |line| line.line_number),
            false => 0,
        };
        last_entry.max(last_structural)
    }
}

#[derive(Debug)]
struct PendingInsertion {
    visible_row: usize,
    source_line: u32,
    row: InsertedRowKind,
}

impl PendingInsertion {
    fn index_in_source_order(mut pending: Vec<Self>) -> (Vec<InsertedRow>, Vec<(usize, usize)>) {
        pending.sort_unstable_by_key(|insertion| {
            (
                insertion.visible_row,
                insertion.source_line,
                !matches!(insertion.row, InsertedRowKind::DayDivider { .. }),
            )
        });
        let mut boot_rows = Vec::new();
        let insertions = pending
            .into_iter()
            .enumerate()
            .map(|(index, insertion)| {
                let table_row = insertion.visible_row + index;
                if let InsertedRowKind::BootDivider { session_index, .. } = insertion.row {
                    boot_rows.push((session_index, table_row));
                }
                InsertedRow {
                    visible_row: insertion.visible_row,
                    table_row,
                    source_line: insertion.source_line,
                    row: insertion.row,
                }
            })
            .collect::<Vec<_>>();
        debug_assert!(
            insertions
                .iter()
                .zip(insertions.iter().skip(1))
                .all(|(previous, next)| previous.source_line <= next.source_line)
        );
        boot_rows.sort_unstable_by_key(|(session, _)| *session);
        (insertions, boot_rows)
    }
}

#[derive(Debug)]
struct InsertedRow {
    visible_row: usize,
    table_row: usize,
    source_line: u32,
    row: InsertedRowKind,
}

#[derive(Debug)]
struct RevealedTicks {
    visible_row: usize,
    revealed: TimestampTick,
    following: TimestampTick,
}

#[derive(Debug)]
struct DisplayedClockTicks {
    filtered: Arc<ClockTicks>,
    reveals: Vec<RevealedTicks>,
}

impl DisplayedClockTicks {
    fn new(log: &LoadedLog, entries: &DisplayedEntries) -> Self {
        let reveals = entries
            .reveals
            .iter()
            .map(|reveal| RevealedTicks {
                visible_row: reveal.visible_row,
                revealed: ClockTicks::tick_after_entry(
                    log.parsed(),
                    reveal.entry_index,
                    reveal
                        .visible_row
                        .checked_sub(1)
                        .and_then(|row| entries.entry_index(row)),
                ),
                following: entries.entry_index(reveal.visible_row + 1).map_or(
                    TimestampTick::Weak,
                    |following| {
                        ClockTicks::tick_after_entry(
                            log.parsed(),
                            following,
                            Some(reveal.entry_index),
                        )
                    },
                ),
            })
            .collect();
        Self {
            filtered: log.filters().shared_clock_ticks(),
            reveals,
        }
    }

    fn day_divider_rows<'a>(
        &'a self,
        parsed: &'a ParsedLog,
        entries: &'a DisplayedEntries,
    ) -> impl Iterator<Item = usize> + 'a {
        let mut boundary_rows = Vec::new();
        if let (Some(first), Some(last), Some(resolved)) = (
            entries.reveals.first(),
            entries.reveals.last(),
            parsed.chronology().resolved(),
        ) {
            boundary_rows.extend(first.visible_row..=last.visible_row);
            boundary_rows.extend((last.visible_row + 1..entries.len()).find(|&row| {
                entries
                    .entry_index(row)
                    .and_then(|entry| resolved.eligible_time(entry))
                    .is_some()
            }));
        }
        let day_rows = self
            .filtered
            .day_dividers()
            .iter()
            .filter_map(move |divider| entries.filtered.entry_index(divider.visible_row))
            .map(move |entry| entries.row_at_or_after(entry))
            .filter(|row| !boundary_rows.contains(row))
            .collect::<Vec<_>>();
        let revealed_day_rows = parsed
            .chronology()
            .resolved()
            .map(move |resolved| {
                boundary_rows.into_iter().filter(move |&row| {
                    let Some(current) = entries
                        .entry_index(row)
                        .and_then(|entry| resolved.eligible_time(entry))
                    else {
                        return false;
                    };
                    (0..row)
                        .rev()
                        .filter_map(|row| entries.entry_index(row))
                        .find_map(|entry| resolved.eligible_time(entry))
                        .is_some_and(|previous| {
                            previous.timestamp().date_naive() != current.timestamp().date_naive()
                        })
                })
            })
            .into_iter()
            .flatten();
        day_rows.into_iter().chain(revealed_day_rows)
    }

    fn append_day_divider_insertions(
        &self,
        parsed: &ParsedLog,
        entries: &DisplayedEntries,
        boot_dividers: &SourceBootDividers,
        pending: &mut Vec<PendingInsertion>,
    ) {
        for visible_row in self.day_divider_rows(parsed, entries) {
            let Some(entry_index) = entries.entry_index(visible_row) else {
                continue;
            };
            let Some(entry) = parsed.entries().get(entry_index) else {
                continue;
            };
            let session_index = parsed
                .boot_sessions()
                .partition_point(|session| session.entry_range.end <= entry_index);
            let separator = parsed
                .boot_sessions()
                .get(session_index)
                .filter(|session| entries.row_at_or_after(session.entry_range.start) == visible_row)
                .and_then(|_| boot_dividers.separator_for_session.get(session_index))
                .copied()
                .flatten()
                .and_then(|index| parsed.structural_lines().get(index));
            pending.push(PendingInsertion {
                visible_row,
                source_line: separator.map_or(entry.line_number, |line| line.line_number),
                row: InsertedRowKind::DayDivider { entry_index },
            });
        }
    }

    fn tick(&self, visible_row: usize) -> TimestampTick {
        let before = self
            .reveals
            .partition_point(|reveal| reveal.visible_row <= visible_row);
        if let Some(reveal) = before
            .checked_sub(1)
            .and_then(|index| self.reveals.get(index))
        {
            if visible_row == reveal.visible_row {
                return reveal.revealed;
            }
            if visible_row == reveal.visible_row + 1 {
                return reveal.following;
            }
        }
        self.filtered.tick(visible_row - before)
    }
}

#[derive(Debug)]
pub(super) struct LineTableRows {
    entries: DisplayedEntries,
    insertions: Vec<InsertedRow>,
    boot_rows: Vec<(usize, usize)>,
    ticks: DisplayedClockTicks,
    largest_line_number: u32,
    additional_source_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RowCacheKey {
    log: LoadedLogId,
    visible_revision: u64,
    show_structural_lines: bool,
    revealed_source: Option<SourceOverlay>,
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

    fn append_boot_divider_insertions(
        &self,
        parsed: &ParsedLog,
        entries: &DisplayedEntries,
        pending: &mut Vec<PendingInsertion>,
    ) {
        for (session_index, session) in parsed.boot_sessions().iter().enumerate() {
            let visible_row = entries.row_at_or_after(session.entry_range.start);
            let end = entries.row_at_or_after(session.entry_range.end);
            if visible_row == end
                || self
                    .separator_for_session
                    .get(session_index)
                    .is_some_and(Option::is_some)
            {
                continue;
            }
            let Some(entry) = entries
                .entry_index(visible_row)
                .and_then(|entry| parsed.entries().get(entry))
            else {
                continue;
            };
            pending.push(PendingInsertion {
                visible_row,
                source_line: entry.line_number,
                row: InsertedRowKind::BootDivider {
                    session_index,
                    structural_index: None,
                },
            });
        }
    }

    fn append_structural_insertions(
        &self,
        parsed: &ParsedLog,
        entries: &DisplayedEntries,
        pending: &mut Vec<PendingInsertion>,
    ) {
        for (structural_index, line) in parsed.structural_lines().iter().enumerate() {
            let entry_index = parsed
                .entries()
                .partition_point(|entry| entry.line_number < line.line_number);
            pending.push(PendingInsertion {
                visible_row: entries.row_at_or_after(entry_index),
                source_line: line.line_number,
                row: self.row_of_structural_line(structural_index),
            });
        }
    }

    fn row_of_structural_line(&self, structural_index: usize) -> InsertedRowKind {
        self.session_for_separator
            .get(structural_index)
            .copied()
            .flatten()
            .map_or(
                InsertedRowKind::Structural { structural_index },
                |session_index| InsertedRowKind::BootDivider {
                    session_index,
                    structural_index: Some(structural_index),
                },
            )
    }
}

impl LineTableRows {
    fn append_revealed_source_insertions(
        parsed: &ParsedLog,
        visible: &DisplayedEntries,
        overlay: SourceOverlay,
        show_structural_lines: bool,
        pending: &mut Vec<PendingInsertion>,
    ) -> usize {
        let mut added = 0;
        for line_number in overlay.first_line..=overlay.last_line {
            let row = match SourceRow::at_line(parsed, line_number) {
                Some(SourceRow::RenderRawLine(_)) => {
                    InsertedRowKind::RenderRawSource { line_number }
                }
                Some(SourceRow::ShowStructuralLine(structural_index)) if !show_structural_lines => {
                    InsertedRowKind::Structural { structural_index }
                }
                _ => continue,
            };
            added += 1;
            let entry = parsed
                .entries()
                .partition_point(|entry| entry.line_number < line_number);
            pending.push(PendingInsertion {
                visible_row: visible.row_at_or_after(entry),
                source_line: line_number,
                row,
            });
        }
        added
    }

    #[cfg(test)]
    pub(super) fn of(log: &LoadedLog) -> Self {
        Self::with_overlay(log, false, None)
    }

    #[cfg(test)]
    fn with_overlay(log: &LoadedLog, show_structural_lines: bool, entry: Option<usize>) -> Self {
        Self::with_source_overlay(
            log,
            show_structural_lines,
            entry
                .and_then(|entry| log.parsed().entries().get(entry))
                .map(|entry| SourceOverlay {
                    first_line: entry.line_number,
                    last_line: entry.line_number,
                }),
        )
    }

    fn with_source_overlay(
        log: &LoadedLog,
        show_structural_lines: bool,
        revealed_source: Option<SourceOverlay>,
    ) -> Self {
        let parsed = log.parsed();
        let visible = DisplayedEntries::new(log, revealed_source);
        let ticks = DisplayedClockTicks::new(log, &visible);
        let source_boot_dividers = SourceBootDividers::of(parsed, show_structural_lines);
        let mut pending = Vec::new();
        ticks.append_day_divider_insertions(parsed, &visible, &source_boot_dividers, &mut pending);
        source_boot_dividers.append_boot_divider_insertions(parsed, &visible, &mut pending);
        if show_structural_lines {
            source_boot_dividers.append_structural_insertions(parsed, &visible, &mut pending);
        }
        let additional_source_count = visible.reveals.len()
            + revealed_source.map_or(0, |overlay| {
                Self::append_revealed_source_insertions(
                    parsed,
                    &visible,
                    overlay,
                    show_structural_lines,
                    &mut pending,
                )
            });
        let largest_line_number = visible
            .largest_line_number(parsed, show_structural_lines)
            .max(revealed_source.map_or(0, |overlay| overlay.last_line));
        let (insertions, boot_rows) = PendingInsertion::index_in_source_order(pending);
        Self {
            entries: visible,
            insertions,
            boot_rows,
            ticks,
            largest_line_number,
            additional_source_count,
        }
    }

    pub(super) fn additional_source_count(&self) -> usize {
        self.additional_source_count
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len() + self.insertions.len()
    }

    pub(super) fn at(&self, row: usize) -> Option<LineTableRow> {
        if row >= self.len() {
            return None;
        }
        let inserted_before = self
            .insertions
            .partition_point(|insertion| insertion.table_row < row);
        if let Some(insertion) = self
            .insertions
            .get(inserted_before)
            .filter(|insertion| insertion.table_row == row)
        {
            return Some(insertion.row.into());
        }
        let visible_row = row - inserted_before;
        self.entries
            .entry_index(visible_row)
            .map(|entry_index| LineTableRow::Entry {
                entry_index,
                visible_row,
            })
    }

    pub(super) fn row_of_boot_divider(&self, target: usize) -> Option<usize> {
        self.boot_rows
            .binary_search_by_key(&target, |(session, _)| *session)
            .ok()
            .and_then(|index| self.boot_rows.get(index).map(|(_, row)| *row))
    }

    pub(super) fn row_of_exact_entry(&self, target: usize) -> Option<usize> {
        self.row_of_entry_at_or_after(target).filter(|&row| {
            matches!(self.at(row), Some(LineTableRow::Entry { entry_index, .. }) if entry_index == target)
        })
    }

    pub(super) fn row_of_entry_at_or_after(&self, target: usize) -> Option<usize> {
        let visible_row = self.entries.row_at_or_after(target);
        (visible_row < self.entries.len()).then(|| self.table_row_of_visible_entry(visible_row))
    }

    fn row_of_identity(&self, target: TableRowIdentity) -> Option<usize> {
        match target {
            TableRowIdentity::Entry(entry) => self.row_of_exact_entry(entry),
            TableRowIdentity::BootSession(session) => self.row_of_boot_divider(session),
            TableRowIdentity::Structural(index) => self.insertions.iter().find_map(|insertion| {
                matches!(insertion.row,
                    InsertedRowKind::Structural { structural_index } |
                    InsertedRowKind::BootDivider { structural_index: Some(structural_index), .. }
                    if structural_index == index
                )
                .then_some(insertion.table_row)
            }),
            TableRowIdentity::DayDivider(_) | TableRowIdentity::RenderRawSource(_) => {
                self.insertions.iter().find_map(|insertion| {
                    (TableRowIdentity::from(LineTableRow::from(insertion.row)) == target)
                        .then_some(insertion.table_row)
                })
            }
        }
    }

    fn row_at_source_line(&self, parsed: &ParsedLog, line: u32) -> usize {
        let entry = parsed
            .entries()
            .partition_point(|entry| entry.line_number < line);
        let entry_row = self.row_of_entry_at_or_after(entry);
        let insertion = self
            .insertions
            .partition_point(|insertion| insertion.source_line < line);
        let inserted_row = self
            .insertions
            .get(insertion)
            .map(|insertion| insertion.table_row);
        entry_row
            .into_iter()
            .chain(inserted_row)
            .min()
            .unwrap_or_else(|| self.len().saturating_sub(1))
    }

    fn row_at_reading_anchor(&self, parsed: &ParsedLog, anchor: TableReadingAnchor) -> usize {
        self.row_of_identity(anchor.row)
            .unwrap_or_else(|| self.row_at_source_line(parsed, anchor.source_line))
    }

    fn table_row_of_visible_entry(&self, visible_row: usize) -> usize {
        visible_row
            + self
                .insertions
                .partition_point(|insertion| insertion.visible_row <= visible_row)
    }
}

impl LogViewerWindow {
    pub(in crate::app) fn preserve_source_position_after_chronology_change(
        &mut self,
        log: &LoadedLog,
        log_id: LoadedLogId,
    ) {
        let pending_target = self.log_state_mut(log_id).scroll_to_row.and_then(|row| {
            let cache = self.log_state_mut(log_id).line_table_cache.as_ref()?;
            if cache.key.log != log_id {
                return None;
            }
            let row = cache.rows.at(row)?;
            Some(TableReadingAnchor {
                log: log_id,
                source_line: row.source_line(log.parsed())?,
                row: row.into(),
                offset_within_row_px: 0.0,
            })
        });
        self.log_state_mut(log_id).scroll_to_source_line = pending_target
            .or(self
                .log_state_mut(log_id)
                .scroll_to_source_line
                .filter(|anchor| anchor.log == log_id))
            .or(self
                .log_state_mut(log_id)
                .table_reading_anchor
                .filter(|anchor| anchor.log == log_id));
        self.log_state_mut(log_id).scroll_to_row = None;
    }

    pub(super) fn table_rows(
        &mut self,
        log: &LoadedLog,
        log_id: LoadedLogId,
    ) -> Arc<LineTableRows> {
        self.clear_invalid_source_reveal(log, log_id);
        let key = RowCacheKey {
            log: log_id,
            visible_revision: log.filters().visible_revision(),
            show_structural_lines: self.show_structural_lines,
            revealed_source: self
                .log_state_mut(log_id)
                .source_reveal
                .map(|reveal| reveal.overlay(log.parsed())),
        };
        let state = self.log_state_mut(log_id);
        match &mut state.line_table_cache {
            Some(cache) if cache.key == key => Arc::clone(&cache.rows),
            slot => {
                state.table_text_selection = TableTextSelection::default();
                let rows = Arc::new(LineTableRows::with_source_overlay(
                    log,
                    key.show_structural_lines,
                    key.revealed_source,
                ));
                *slot = Some(LineTableCache {
                    key,
                    rows: Arc::clone(&rows),
                });
                rows
            }
        }
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
        requests: &mut LineTableRequests<'_>,
    ) {
        let parsed = log.parsed();
        let filters = log.filters();
        let rows = self.table_rows(log, log_id);
        let color_switches = MessageColorSwitches {
            services: self.color_services,
            levels: self.color_levels,
        };
        let show_structural_lines = self.show_structural_lines;
        let keyboard_access = self.keyboard_access;
        let state = self.log_state_mut(log_id);
        let find_hit = state.find.as_ref().and_then(|find| find.current()).cloned();
        let find_highlight = find_hit
            .as_ref()
            .and_then(|hit| FindHighlight::of(hit, parsed));
        let reveal = state.source_reveal;
        let filtered_return = state.filtered_return.filter(|origin| origin.log == log_id);
        let source_anchor = filtered_return.map_or_else(
            || {
                state
                    .scroll_to_source_line
                    .filter(|anchor| anchor.log == log_id)
            },
            |origin| origin.anchor,
        );
        if !filters.is_table_query_pending() {
            state.scroll_to_source_line = None;
            state.filtered_return = None;
            if let Some(reveal) = &mut state.source_reveal {
                reveal.finish_centering();
            }
        }
        let ticks = &rows.ticks;
        let line_number_width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    rows.largest_line_number.to_string(),
                    egui::TextStyle::Monospace.resolve(ui.style()),
                    ui.visuals().weak_text_color(),
                )
                .size()
                .x
        });
        let anomaly_steps: FxHashMap<usize, Duration> = parsed
            .chronology()
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
        let unit = state.association_window_unit;
        let recognised_messages = parsed.recognised_messages();
        let association_window = log.association_window();
        let years_resolved = parsed.chronology().is_resolved();
        let dark_mode = ui.visuals().dark_mode;
        let gutter = LayerGutter::of(filters, dark_mode);
        let highlight = gt_ui_theme::LOG_LIVE_FILTER.resolve(dark_mode);
        let marking_hexagon = requests
            .hover
            .glyph
            .as_ref()
            .or(state.clicked_glyph.as_ref());
        let cross_highlighted = CrossHighlightedRows::of(marking_hexagon, log_id, dark_mode);
        let hover_dwell = state.row_hover_dwell;
        let mut context_line = None;
        let mut hovered_row_placement = None;
        let mut hovered_entry_index = None;

        ui.scope(|ui| {
            // Rows sit directly on top of each other, so the table reads as one
            // block of text and a row's index times its height is its offset.
            ui.spacing_mut().item_spacing.y = 0.0;
            let grid = RowGrid {
                height: row_height(ui),
            };
            let focus = TableFocusScope::for_log(log_id);
            let target = focus.register_ui(ui);
            let entered_by_navigation = target.gained_focus() && target.has_focus();
            let keyboard_available = keyboard_access.available(ui.ctx());
            let scroll_salt = ("log_viewer_line_table", log_id);
            // egui's `ScrollArea` registers each scrollbar with its axis index.
            let scrollbar = ui.ctx().read_response(
                ui.make_persistent_id(IdSalt::new(scroll_salt))
                    .with(VERTICAL_SCROLLBAR_AXIS),
            );
            let scrollbar_rect = scrollbar
                .filter(egui::Response::enabled)
                .map(|bar| bar.interact_rect);
            if keyboard_available && mem::take(&mut state.table_focus_requested) {
                target.request_focus();
            }
            let after_navigation = target.has_focus();
            if keyboard_available {
                focus.lock_vertical_arrows(ui);
            }
            let scroll_area = ScrollArea::vertical()
                .id_salt(scroll_salt)
                .scroll_source(ScrollSource::SCROLL_BAR | ScrollSource::MOUSE_WHEEL)
                .auto_shrink([false, false])
                // A keyboard step lands on a row boundary and a held key counts
                // every repeat, which an animation in flight would round off.
                .animated(false)
                // egui keeps a 64px floor by default, which on a short window
                // pushes the footer off the bottom. The table takes exactly the
                // room the window has left.
                .min_scrolled_height(0.0);
            let requested_offset = if let Some(anchor) = source_anchor {
                let row = rows.row_at_reading_anchor(parsed, anchor);
                state.scroll_to_row = None;
                Some(if reveal.is_some_and(SourceReveal::center_pending) {
                    (grid.offset(row)
                        - (ui.available_height() - grid.height).max(0.0)
                            * CONTEXT_TARGET_VERTICAL_FRACTION)
                        .max(0.0)
                } else {
                    grid.offset(row) + anchor.offset_within_row_px
                })
            } else if filtered_return.is_some() {
                state.scroll_to_row = None;
                Some(0.0)
            } else {
                state.scroll_to_row.take().map(|row| grid.offset(row))
            };
            let output = scroll_area.show_rows(ui, grid.height, rows.len(), |ui, shown| {
                let positioned = ShownRows {
                    grid,
                    rows: shown.clone(),
                    bounds: ui.max_rect(),
                };
                if let Some(offset) = requested_offset {
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            egui::pos2(positioned.bounds.left(), positioned.screen_y(offset)),
                            egui::Vec2::ZERO,
                        ),
                        Some(egui::Align::Min),
                    );
                }
                let style = Arc::clone(ui.style());
                let metrics = RowLayoutStyle {
                    style: &style,
                    line_number_width,
                    gutter_width: gutter.width_px(),
                };
                let find_at = |row: LineTableRow| {
                    find_highlight
                        .as_ref()
                        .filter(|hit| row.source_line(parsed) == Some(hit.line_number()))
                };
                let layout_row =
                    |fonts: &mut FontsView<'_>, row, table_row, content: &LineTableContent<'_>| {
                        let entry_text = if let RowPresentation::Entry {
                            entry_index,
                            visible_row,
                            ..
                        } = content.presentation
                        {
                            let tick = ticks.tick(visible_row);
                            Some(EntryText {
                                tick: if log.entry_placement(entry_index).is_some() {
                                    tick
                                } else {
                                    tick.min(TimestampTick::Plain)
                                },
                                highlighted: HighlightedMessage {
                                    spans: filters.condition_match_spans(entry_index),
                                    color: highlight,
                                },
                                recognised: recognised_messages.get(entry_index).copied(),
                                color_switches,
                            })
                        } else {
                            None
                        };
                        metrics.layout(
                            fonts,
                            &RowLayoutRequest {
                                row,
                                bounds: positioned.row_bounds(row),
                                content,
                                find_highlight: find_at(table_row),
                                entry_text,
                            },
                        )
                    };
                let mut text = TableTextUi::new(ui, &positioned, rows.len(), scrollbar_rect);
                let render_row = |ui: &mut egui::Ui, text: &mut TableTextUi<'_>, row| {
                    let table_row = rows.at(row)?;
                    let content = table_row.content(parsed)?;
                    let layout = ui.fonts_mut(|fonts| layout_row(fonts, row, table_row, &content));
                    let bounds = layout.bounds;
                    ui.advance_cursor_after_rect(bounds);
                    let revealed_reason = reveal.and_then(|reveal| {
                        reveal.row_reason(table_row, log, show_structural_lines)
                    });
                    let decoration = RowDecoration {
                        find_highlight: find_at(table_row),
                        revealed_reason: revealed_reason.as_deref(),
                    };
                    let entry = if ui.is_visible()
                        && let RowPresentation::Entry {
                            entry_index,
                            entry,
                            timestamp,
                            ..
                        } = content.presentation
                    {
                        let placement = log.entry_placement(entry_index);
                        let entry_row = EntryRow {
                            timestamp,
                            position: placement.map(|placement| placement.position),
                            order_anomaly_step: anomaly_steps.get(&entry_index).copied(),
                            association_window,
                            years_resolved,
                            gutter: &gutter,
                            entry_index,
                            hover_dwell,
                            revealed_reason: revealed_reason.as_deref(),
                        };
                        if let Some(fill) = cross_highlighted.fill_of(entry_index) {
                            ui.painter().rect_filled(bounds, 0, fill);
                        }
                        if let Some(rect) = layout.gutter {
                            entry_row.paint_gutter(ui, rect);
                        }
                        let id = ui.make_persistent_id(("log_text_entry", entry_index));
                        let response = ui.interact(bounds, id, egui::Sense::CLICK);
                        Some((entry_row, entry.line_number, placement, response))
                    } else {
                        None
                    };
                    let separator = layout.separator;
                    text.row_ui(ui, row, layout, |index, response| {
                        if let Some((_, cell)) = content
                            .cells
                            .indexed()
                            .find(|(cell_index, _)| *cell_index == index)
                        {
                            decoration.cell_hover(ui, cell, response);
                        }
                    });
                    if !ui.is_visible() {
                        return None;
                    }
                    if let Some(rect) = separator {
                        ui.place(rect, Separator::default().horizontal());
                    }
                    if !matches!(content.presentation, RowPresentation::Divider) {
                        decoration.paint(ui, bounds);
                    }
                    let (entry_row, line_number, placement, response) = entry?;
                    let interaction = entry_row.interaction_ui(ui, response, unit)?;
                    Some((entry_row.entry_index, line_number, placement, interaction))
                };
                let mut clicked_position = None;
                for row in shown {
                    let Some((entry_index, line_number, placement, interaction)) =
                        render_row(ui, &mut text, row)
                    else {
                        continue;
                    };
                    if interaction == RowInteraction::ShowSurroundingLines {
                        context_line = Some(line_number);
                        continue;
                    }
                    hovered_entry_index = Some(entry_index);
                    let Some(placement) = placement else {
                        continue;
                    };
                    let (latitude, longitude) = placement.position;
                    hovered_row_placement = Some(LogRowPlacement {
                        merc: mercator::normalize(latitude, longitude),
                        track: placement.fix.track,
                    });
                    if interaction == RowInteraction::CentreMap {
                        clicked_position = Some((latitude.as_degrees(), longitude.as_degrees()));
                    }
                }
                let selected_by_pointer = text.finish(
                    ui,
                    &mut state.table_text_selection,
                    TableInputFocus {
                        target: &target,
                        available: keyboard_available,
                        after_navigation,
                        entered_by_navigation,
                    },
                    |row| Some(rows.at(row)?.content(parsed)?.cells),
                    |fonts, row| {
                        let table_row = rows.at(row)?;
                        let content = table_row.content(parsed)?;
                        Some(layout_row(fonts, row, table_row, &content))
                    },
                );
                if keyboard_available && target.has_focus() {
                    KeyboardScrollSteps {
                        row_height_px: grid.height,
                        page_height_px: ui.clip_rect().height(),
                        content_height_px: grid.offset(rows.len()),
                    }
                    .scroll_the_table(ui);
                }
                if !selected_by_pointer && let Some(position) = clicked_position {
                    *requests.map_center = Some(position);
                }
            });
            focus.draw_focus(ui, output.inner_rect);
            let offset = output.state.unclamped_offset().y;
            let first_visible_row = grid.row_at(offset);
            state.table_reading_anchor = rows.at(first_visible_row).and_then(|row| {
                Some(TableReadingAnchor {
                    log: log_id,
                    source_line: row.source_line(parsed)?,
                    row: TableRowIdentity::from(row),
                    offset_within_row_px: grid.offset_within_row(offset),
                })
            });
        });
        state
            .row_hover_dwell
            .moved_to(hovered_entry_index, ui.input(|input| input.time));
        requests.hover.row_placement = hovered_row_placement;
        if let Some(line) = context_line {
            self.navigate_to_source(log, log_id, SourceRequest::ShowSurroundingLines(line));
        }
    }
}

pub(super) fn row_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Monospace)
        .max(ui.spacing().interact_size.y)
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
enum RowTimestamp {
    Resolved {
        timestamp: DateTime<Utc>,
        kind: TimestampKind,
        clock_eligibility: ClockEligibility,
    },

    /// The timestamp the line writes, in a log whose years are unresolved.
    Written(YearlessTimestamp),
}

impl RowTimestamp {
    /// `None` for a line without a timestamp in a log whose years are unresolved.
    fn of(parsed: &ParsedLog, entry_index: usize, entry: &LogEntry) -> Option<Self> {
        match parsed.chronology() {
            LogChronology::Resolved(resolved) => resolved
                .timestamp(entry_index)
                .zip(resolved.clock_eligibility(entry_index))
                .map(|(timestamp, clock_eligibility)| Self::Resolved {
                    timestamp,
                    kind: entry.timestamp_kind,
                    clock_eligibility,
                }),
            LogChronology::Unresolved(unresolved) => {
                unresolved.written_timestamp(entry_index).map(Self::Written)
            }
        }
    }

    fn is_interpolated(self) -> bool {
        matches!(
            self,
            Self::Resolved {
                kind: TimestampKind::Interpolated,
                ..
            }
        )
    }

    fn is_device_clock(self) -> bool {
        matches!(
            self,
            Self::Resolved {
                clock_eligibility: ClockEligibility::BeforeSynchronization,
                ..
            }
        )
    }

    fn prefix(self) -> &'static str {
        if self.is_device_clock() {
            return ICON_DEVICE_CLOCK;
        }
        match self.is_interpolated() {
            true => ALMOST_EQUAL_TO,
            false => ANCHORED_TIMESTAMP_PREFIX,
        }
    }

    /// The date as [`DATE_FORMAT`] writes it, or as `--MM-DD` for a timestamp without a year.
    fn date_text(self) -> String {
        match self {
            Self::Resolved { timestamp, .. } => timestamp.format(DATE_FORMAT).to_string(),
            Self::Written(written) => format!("--{:02}-{:02}", written.month(), written.day()),
        }
    }

    fn time(self) -> NaiveTime {
        match self {
            Self::Resolved { timestamp, .. } => timestamp.time(),
            Self::Written(written) => written.time(),
        }
    }
}

#[derive(Clone, Copy)]
struct NumberedSource<'a> {
    line_number: u32,
    text: &'a str,
}

impl<'a> NumberedSource<'a> {
    fn structural(parsed: &'a ParsedLog, index: usize) -> Option<Self> {
        let line = parsed.structural_lines().get(index)?;
        Some(Self {
            line_number: line.line_number,
            text: line.text.in_text(parsed.text()),
        })
    }

    fn content(self) -> LineTableContent<'a> {
        LineTableContent {
            presentation: RowPresentation::Source,
            cells: RowCells([
                Some(RowCell::Number(self.line_number.to_string())),
                Some(RowCell::Source {
                    text: self.text,
                    boot: false,
                }),
                None,
            ]),
        }
    }
}

struct RowDecoration<'a> {
    find_highlight: Option<&'a FindHighlight<'a>>,
    revealed_reason: Option<&'a str>,
}

impl RowDecoration<'_> {
    fn cell_hover(&self, ui: &egui::Ui, cell: &RowCell<'_>, response: egui::Response) {
        match cell {
            RowCell::Timestamp(timestamp) if timestamp.prefix == ALMOST_EQUAL_TO => {
                response.on_hover_text(INTERPOLATED_TIMESTAMP_HOVER);
            }
            RowCell::Source { text, boot: true } => {
                let response = response.on_hover_text(*text);
                if let Some(hit) = self.find_highlight {
                    hit.decorate_row(ui, response.rect);
                }
            }
            RowCell::Source { boot: false, .. } => {
                if let Some(reason) = self.revealed_reason {
                    response.on_hover_text(reason);
                }
            }
            RowCell::Divider { text, .. } => {
                response.on_hover_text(text);
            }
            _ => {}
        }
    }

    fn paint(self, ui: &egui::Ui, rect: egui::Rect) {
        if let Some(hit) = self.find_highlight {
            hit.decorate_row(ui, rect);
        }
        if self.revealed_reason.is_some() {
            ui.painter().rect_stroke(
                rect,
                0,
                Stroke::new(
                    REVEAL_OUTLINE_WIDTH_PX,
                    gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                ),
                StrokeKind::Inside,
            );
        }
    }
}

struct EntryRow<'a> {
    timestamp: Option<RowTimestamp>,
    position: Option<(Latitude, Longitude)>,

    /// The backwards step this entry opens, for the rows an order anomaly
    /// starts at.
    order_anomaly_step: Option<Duration>,

    association_window: AssociationWindow,
    years_resolved: bool,
    gutter: &'a LayerGutter<'a>,
    entry_index: usize,

    hover_dwell: RowHoverDwell,
    revealed_reason: Option<&'a str>,
}

#[derive(PartialEq)]
enum RowInteraction {
    CentreMap,
    HoverOverRow,
    ShowSurroundingLines,
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

    color_the_service: bool,

    color_the_hostname: bool,

    color_the_level: bool,
}

impl MessageColors {
    fn of(style: &egui::Style, switches: MessageColorSwitches) -> Self {
        Self {
            base: style.visuals.text_color(),
            weak: style.visuals.weak_text_color(),
            dark_mode: style.visuals.dark_mode,
            color_the_service: switches.services,
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

struct EntryText {
    tick: TimestampTick,
    highlighted: HighlightedMessage,
    recognised: Option<RecognisedMessage>,
    color_switches: MessageColorSwitches,
}

impl EntryText {
    fn append_runs(
        message: &str,
        job: &mut LayoutJob,
        font_id: &FontId,
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
            if let Some(before) = message.get(appended_to..start) {
                job.append(
                    before,
                    0.0,
                    TextFormat::simple(font_id.clone(), colors.base),
                );
            }
            if let Some(text) = message.get(start..end) {
                job.append(text, 0.0, TextFormat::simple(font_id.clone(), run.color));
            }
            appended_to = end;
        }
        if let Some(rest) = message.get(appended_to..range.end) {
            job.append(rest, 0.0, TextFormat::simple(font_id.clone(), colors.base));
        }
    }

    fn message_job(&self, style: &egui::Style, font_id: &FontId, message: &str) -> LayoutJob {
        let colors = MessageColors::of(style, self.color_switches);
        let runs = self
            .recognised
            .map(|recognised| colors.runs(recognised))
            .unwrap_or_default();
        let mut job = LayoutJob::default();
        let mut appended_to = 0;
        for span in &self.highlighted.spans {
            let Some(matched) = message.get(span.clone()) else {
                continue;
            };
            Self::append_runs(
                message,
                &mut job,
                font_id,
                appended_to..span.start,
                &runs,
                &colors,
            );
            job.append(
                matched,
                0.0,
                TextFormat::simple(font_id.clone(), self.highlighted.color),
            );
            appended_to = span.end;
        }
        Self::append_runs(
            message,
            &mut job,
            font_id,
            appended_to..message.len(),
            &runs,
            &colors,
        );
        job
    }
}

impl EntryRow<'_> {
    fn interaction_ui(
        &self,
        ui: &egui::Ui,
        row: egui::Response,
        unit: AssociationWindowUnit,
    ) -> Option<RowInteraction> {
        let mut context_requested = false;
        row.context_menu(|ui| {
            if ui
                .button("Show surrounding lines")
                .on_hover_text("Temporarily show nearby physical source lines")
                .clicked()
            {
                context_requested = true;
                ui.close();
            }
        });
        if context_requested {
            return Some(RowInteraction::ShowSurroundingLines);
        }
        if !row.hovered() {
            return None;
        }
        let row = match self.hover_text(ui.ctx(), unit) {
            Some(hover) => row.on_hover_text(hover),
            None => row,
        };
        Some(if row.clicked() {
            RowInteraction::CentreMap
        } else {
            RowInteraction::HoverOverRow
        })
    }

    fn hover_text(&self, ctx: &egui::Context, unit: AssociationWindowUnit) -> Option<String> {
        if let Some(reason) = self.revealed_reason {
            return Some(reason.to_owned());
        }
        if self.timestamp.is_some_and(RowTimestamp::is_device_clock) {
            return Some(
                match self.timestamp.is_some_and(RowTimestamp::is_interpolated) {
                    true => format!("{DEVICE_CLOCK_ROW_HOVER}. {INTERPOLATED_TIMESTAMP_HOVER}."),
                    false => DEVICE_CLOCK_ROW_HOVER.to_owned(),
                },
            );
        }
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
            (None, None) if self.years_resolved => Some(format!(
                "No GPS fix within {} of this line",
                unit.describe(self.association_window)
            )),
            (None, None) => Some(UNRESOLVED_YEARS_ROW_HOVER.to_owned()),
        }
    }

    fn paint_gutter(&self, ui: &egui::Ui, rect: egui::Rect) {
        let height = rect.height();
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

/// `time` as `format` writes it, padded with spaces to `width`, and all spaces without a time.
fn time_run(time: Option<NaiveTime>, format: &str, width: usize) -> String {
    let text = time.map_or_else(String::new, |time| time.format(format).to_string());
    format!("{text:width$}")
}

fn boot_divider_title(session: &BootSession, uptime: Option<Duration>) -> String {
    let uptime = uptime.map_or_else(|| EM_DASH.to_owned(), gt_fmt::format_human_terse_duration);
    let entries = session.entry_count();
    format!(
        "Boot {} {MIDDLE_DOT} up {uptime} {MIDDLE_DOT} {} {}",
        session.boot_number,
        gt_fmt::format_count(entries),
        gt_fmt::pluralize(entries, "entry", "entries"),
    )
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

const CONTEXT_TARGET_VERTICAL_FRACTION: f32 = 0.5;
const VERTICAL_SCROLLBAR_AXIS: usize = 1;
const REVEAL_OUTLINE_WIDTH_PX: f32 = 1.0;

pub(super) const DEVICE_CLOCK_ROW_HOVER: &str =
    "Absolute time and map position unavailable before clock synchronization";

/// The characters each run of the timestamp column takes: as many as its format writes for the
/// Unix epoch, which is as many as it writes for any instant with a four-digit year.
struct TimestampRunWidths {
    date: usize,
    hour_minute: usize,
    seconds: usize,
}

static TIMESTAMP_RUN_WIDTHS: LazyLock<TimestampRunWidths> = LazyLock::new(|| {
    let width_of = |format: &str| {
        DateTime::UNIX_EPOCH
            .format(format)
            .to_string()
            .chars()
            .count()
    };
    TimestampRunWidths {
        date: width_of(DATE_FORMAT),
        hour_minute: width_of(HOUR_MINUTE_FORMAT),
        seconds: width_of(SECONDS_FORMAT),
    }
});

pub(super) struct TimestampText {
    prefix: &'static str,
    date: String,
    hour_minute: String,
    seconds: String,
}

impl TimestampText {
    fn of(timestamp: Option<RowTimestamp>) -> Self {
        let widths = &*TIMESTAMP_RUN_WIDTHS;
        let date = timestamp.map_or_else(String::new, RowTimestamp::date_text);
        let time = timestamp.map(RowTimestamp::time);
        Self {
            prefix: timestamp.map_or(ANCHORED_TIMESTAMP_PREFIX, RowTimestamp::prefix),
            date: format!("{date:>width$} ", width = widths.date),
            hour_minute: time_run(time, HOUR_MINUTE_FORMAT, widths.hour_minute),
            seconds: time_run(time, SECONDS_FORMAT, widths.seconds),
        }
    }

    /// The device clock glyph and its trailing padding occupy one monospace cell.
    fn layout_job(
        &self,
        fonts: &mut FontsView<'_>,
        font_id: &FontId,
        visuals: &egui::Visuals,
        tick: TimestampTick,
    ) -> LayoutJob {
        let prefix = self.prefix;
        let quiet = visuals.weak_text_color();
        let moved = match tick {
            TimestampTick::Weak => visuals.weak_text_color(),
            TimestampTick::Plain => visuals.text_color(),
            TimestampTick::Strong => visuals.strong_text_color(),
        };
        let mut job = LayoutJob::default();
        if prefix == ICON_DEVICE_CLOCK {
            let (icon_font, leading_space) = {
                let cell_width = fonts.glyph_width(font_id, ' ');
                let mut icon_font = FontId::proportional(font_id.size);
                let icon_width = fonts
                    .layout_no_wrap(prefix.to_owned(), icon_font.clone(), quiet)
                    .size()
                    .x;
                icon_font.size *= cell_width / icon_width;
                let scaled_width = fonts
                    .layout_no_wrap(prefix.to_owned(), icon_font.clone(), quiet)
                    .size()
                    .x;
                (icon_font, cell_width - scaled_width)
            };
            job.append(prefix, 0.0, TextFormat::simple(icon_font, quiet));
            job.append(
                &self.date,
                leading_space,
                TextFormat::simple(font_id.clone(), quiet),
            );
        } else {
            job.append(
                &format!("{prefix}{}", self.date),
                0.0,
                TextFormat::simple(font_id.clone(), quiet),
            );
        }
        job.append(
            &self.hour_minute,
            0.0,
            TextFormat::simple(font_id.clone(), moved),
        );
        job.append(
            &self.seconds,
            0.0,
            TextFormat::simple(font_id.clone(), quiet),
        );
        job
    }

    fn clipboard_text(&self) -> String {
        let prefix = self.prefix;
        let projection = match prefix {
            ICON_DEVICE_CLOCK => CopyProjection::ReplaceFirst(' '),
            _ => CopyProjection::Verbatim,
        };
        let mut output = String::with_capacity(
            prefix.len() + self.date.len() + self.hour_minute.len() + self.seconds.len(),
        );
        output.extend(projection.characters(prefix));
        output.push_str(&self.date);
        output.push_str(&self.hour_minute);
        output.push_str(&self.seconds);
        output
    }
}

pub(super) const INTERPOLATED_TIMESTAMP_HOVER: &str =
    "Timestamp interpolated between neighbouring entries";

pub(super) const ASSOCIATED_ROW_HOVER: &str = "Centre the map on this line";

pub(super) const UNRESOLVED_YEARS_ROW_HOVER: &str =
    "This line takes no position while the years of this log are unresolved";

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
    mod navigation;
    mod row_plan;

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
        LoadedLog::new(
            None,
            parsed,
            AssociationWindow::try_from_seconds(60).unwrap(),
        )
    }

    #[test]
    fn source_rows_preserve_boot_day_entry_and_summary_order() {
        let log = source_log(
            "2026-01-01 23:59:59 first\n--- Device reboot ---\n2026-01-02 00:00:01 second\n----------- Journal summary -----------\ntrailing summary\n",
        );
        let rows = LineTableRows::with_overlay(&log, true, None);
        assert_eq!(
            (0..rows.len())
                .map(|row| rows.at(row).unwrap())
                .collect::<Vec<_>>(),
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
        assert_eq!(
            filtered.row_of_entry_at_or_after(1),
            filtered.row_of_exact_entry(2)
        );
        let revealed = LineTableRows::with_overlay(log, false, Some(1));
        assert_eq!(
            (0..revealed.len())
                .map(|row| revealed.at(row).unwrap())
                .collect::<Vec<_>>(),
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
}
