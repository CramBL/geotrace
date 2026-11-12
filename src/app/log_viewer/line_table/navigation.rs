use std::fmt::Write as _;

use gt_log_view::LoadedLog;
use gt_logfile::ParsedLog;
use gt_ui_types::LoadedLogId;

use super::{LineTableRow, LogViewerWindow, TableReadingAnchor, TableRowIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::log_viewer) enum SourceRequest {
    NavigateToEntry(usize),
    NavigateToLine(u32),
    ShowBootSession(usize),
    ShowSurroundingLines(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::log_viewer) enum SourceRow {
    RenderRawLine(u32),
    ShowEntry(usize),
    ShowStructuralLine(usize),
}

impl SourceRow {
    pub(super) fn at_line(parsed: &ParsedLog, line: u32) -> Option<Self> {
        parsed.source_line(line)?;
        if let Ok(index) = parsed
            .entries()
            .binary_search_by_key(&line, |entry| entry.line_number)
        {
            Some(Self::ShowEntry(index))
        } else if let Ok(index) = parsed
            .structural_lines()
            .binary_search_by_key(&line, |source| source.line_number)
        {
            Some(Self::ShowStructuralLine(index))
        } else {
            Some(Self::RenderRawLine(line))
        }
    }

    #[cfg(test)]
    pub(super) fn entry_index(self) -> Option<usize> {
        match self {
            Self::ShowEntry(index) => Some(index),
            Self::RenderRawLine(_) | Self::ShowStructuralLine(_) => None,
        }
    }

    pub(super) fn line_number(self, parsed: &ParsedLog) -> Option<u32> {
        match self {
            Self::RenderRawLine(line) => Some(line),
            Self::ShowEntry(index) => parsed.entries().get(index).map(|entry| entry.line_number),
            Self::ShowStructuralLine(index) => parsed
                .structural_lines()
                .get(index)
                .map(|line| line.line_number),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExactSourceTarget {
    log: LoadedLogId,
    source: SourceRow,
    line: u32,
}

impl ExactSourceTarget {
    fn resolve(log: &LoadedLog, id: LoadedLogId, request: SourceRequest) -> Option<Self> {
        let parsed = log.parsed();
        let source = match request {
            SourceRequest::NavigateToEntry(index) => SourceRow::ShowEntry(index),
            SourceRequest::ShowBootSession(index) => {
                let session = parsed.boot_sessions().get(index)?;
                if session.entry_range.is_empty() {
                    return None;
                }
                return Self::resolve(
                    log,
                    id,
                    SourceRequest::NavigateToEntry(session.entry_range.start),
                );
            }
            SourceRequest::NavigateToLine(line) | SourceRequest::ShowSurroundingLines(line) => {
                SourceRow::at_line(parsed, line)?
            }
        };
        Some(Self {
            log: id,
            line: source.line_number(parsed)?,
            source,
        })
    }

    fn is_hidden(self, log: &LoadedLog, show_structural_lines: bool) -> bool {
        match self.source {
            SourceRow::RenderRawLine(_) => true,
            SourceRow::ShowEntry(entry) => {
                let filtered = log.filters().visible_entries();
                filtered.entry_index(filtered.row_at_or_after(entry)) != Some(entry)
            }
            SourceRow::ShowStructuralLine(_) => !show_structural_lines,
        }
    }
}

/// The viewer retains currently visible targets during pending table scans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app::log_viewer) struct SourceReveal {
    target: ExactSourceTarget,
    table_semantic_revision: u64,
    return_anchor: Option<TableReadingAnchor>,
    context: Option<SourceContext>,
}

impl SourceReveal {
    #[cfg(test)]
    pub(in crate::app::log_viewer) fn entry_index(self) -> Option<usize> {
        self.target.source.entry_index()
    }

    #[cfg(test)]
    pub(in crate::app::log_viewer) fn log(self) -> LoadedLogId {
        self.target.log
    }

    #[cfg(test)]
    pub(super) fn source(self) -> SourceRow {
        self.target.source
    }

    pub(super) fn overlay(self, parsed: &ParsedLog) -> SourceOverlay {
        let radius = self.context.map_or(0, |context| context.radius);
        SourceOverlay {
            first_line: self.target.line.saturating_sub(radius).max(1),
            last_line: self
                .target
                .line
                .saturating_add(radius)
                .min(u32::try_from(parsed.source_line_count()).unwrap_or(u32::MAX)),
        }
    }

    pub(super) fn center_pending(self) -> bool {
        self.context.is_some_and(|context| context.center_pending)
    }

    pub(super) fn finish_centering(&mut self) {
        if let Some(context) = &mut self.context {
            context.center_pending = false;
        }
    }

    pub(in crate::app::log_viewer) fn context_request(self) -> SourceRequest {
        SourceRequest::ShowSurroundingLines(self.target.line)
    }

    pub(in crate::app::log_viewer) fn can_extend(self, parsed: &ParsedLog) -> bool {
        let overlay = self.overlay(parsed);
        self.context
            .is_none_or(|context| context.radius < MAX_CONTEXT_RADIUS)
            && (overlay.first_line > 1
                || usize::try_from(overlay.last_line).unwrap_or(usize::MAX)
                    < parsed.source_line_count())
    }

    pub(in crate::app::log_viewer) fn reason(
        self,
        log: &LoadedLog,
        show_structural: bool,
    ) -> Option<String> {
        if self.context.is_some() {
            let overlay = self.overlay(log.parsed());
            return Some(format!(
                "Source lines {}–{} are temporarily shown",
                overlay.first_line, overlay.last_line
            ));
        }
        if !self.target.is_hidden(log, show_structural) {
            return None;
        }
        Some(match self.target.source {
            SourceRow::RenderRawLine(_) => format!("Line {} has no parsed entry", self.target.line),
            SourceRow::ShowEntry(entry) => {
                let mut reason =
                    format!("Line {} is outside the table conditions", self.target.line);
                if log.filters().is_table_query_pending() {
                    reason.push_str("\nTable conditions are updating");
                } else {
                    for condition in log.filters().excluding_table_conditions(entry) {
                        write!(reason, "\n{} {}", condition.scope(), condition.text()).ok();
                    }
                }
                reason
            }
            SourceRow::ShowStructuralLine(_) => format!(
                "Line {} is hidden while structural source lines are off",
                self.target.line
            ),
        })
    }

    pub(super) fn row_reason(
        self,
        row: LineTableRow,
        log: &LoadedLog,
        show_structural: bool,
    ) -> Option<String> {
        let source = match row {
            LineTableRow::RenderRawSource { line_number } => SourceRow::RenderRawLine(line_number),
            LineTableRow::Entry { entry_index, .. } => SourceRow::ShowEntry(entry_index),
            LineTableRow::Structural { structural_index } => {
                SourceRow::ShowStructuralLine(structural_index)
            }
            _ => return None,
        };
        let line = source.line_number(log.parsed())?;
        let overlay = self.overlay(log.parsed());
        if overlay.contains(line) {
            Self {
                target: ExactSourceTarget {
                    source,
                    line,
                    ..self.target
                },
                context: None,
                ..self
            }
            .reason(log, show_structural)
        } else {
            None
        }
    }
}

impl LogViewerWindow {
    pub(in crate::app::log_viewer) fn clear_invalid_source_reveal(
        &mut self,
        log: &LoadedLog,
        id: LoadedLogId,
    ) {
        let Some(reveal) = self.log_state_mut(id).source_reveal else {
            return;
        };
        if reveal.target.log != id
            || reveal.table_semantic_revision != log.filters().table_semantic_revision()
        {
            self.return_to_filtered_results(id);
        } else if reveal.context.is_none()
            && !log.filters().is_table_query_pending()
            && !reveal.target.is_hidden(log, self.show_structural_lines)
        {
            self.log_state_mut(id).source_reveal = None;
        }
    }

    pub(in crate::app::log_viewer) fn navigate_to_source(
        &mut self,
        log: &LoadedLog,
        id: LoadedLogId,
        request: SourceRequest,
    ) -> bool {
        let Some(target) = ExactSourceTarget::resolve(log, id, request) else {
            return false;
        };
        self.clear_invalid_source_reveal(log, id);
        let hidden = target.is_hidden(log, self.show_structural_lines);
        let state = self.log_state_mut(id);
        let context = matches!(request, SourceRequest::ShowSurroundingLines(_)).then(|| {
            SourceContext::extend(
                state
                    .source_reveal
                    .filter(|reveal| reveal.target == target)
                    .and_then(|reveal| reveal.context),
            )
        });
        let return_anchor = state.filtered_return.map_or_else(
            || match state.source_reveal {
                Some(reveal) => reveal.return_anchor,
                None => state.table_reading_anchor,
            },
            |origin| origin.anchor,
        );
        state.filtered_return = None;
        state.source_reveal = (hidden
            || context.is_some()
            || log.filters().is_table_query_pending())
        .then_some(SourceReveal {
            target,
            table_semantic_revision: log.filters().table_semantic_revision(),
            return_anchor,
            context,
        });
        let rows = self.table_rows(log, id);
        let row = match request {
            SourceRequest::ShowBootSession(session) => rows.row_of_boot_divider(session),
            _ => match target.source {
                SourceRow::RenderRawLine(line) => {
                    rows.row_of_identity(TableRowIdentity::RenderRawSource(line))
                }
                SourceRow::ShowEntry(entry) => rows.row_of_exact_entry(entry),
                SourceRow::ShowStructuralLine(index) => {
                    rows.row_of_identity(TableRowIdentity::Structural(index))
                }
            },
        };
        if let Some(row) = row.and_then(|row| rows.at(row)) {
            let state = self.log_state_mut(id);
            state.scroll_to_row = None;
            state.scroll_to_source_line = Some(TableReadingAnchor {
                log: id,
                source_line: target.line,
                row: row.into(),
                offset_within_row_px: 0.0,
            });
        }
        true
    }

    pub(in crate::app::log_viewer) fn return_to_filtered_results(&mut self, id: LoadedLogId) {
        let state = self.log_state_mut(id);
        if let Some(reveal) = state.source_reveal.take() {
            state.scroll_to_row = None;
            let anchor = reveal.return_anchor.filter(|anchor| anchor.log == id);
            state.filtered_return = Some(FilteredSourceReturn { log: id, anchor });
            state.scroll_to_source_line = anchor;
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::app::log_viewer) struct FilteredSourceReturn {
    pub(super) log: LoadedLogId,
    pub(super) anchor: Option<TableReadingAnchor>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceContext {
    radius: u32,
    center_pending: bool,
}

impl SourceContext {
    fn extend(previous: Option<Self>) -> Self {
        Self {
            radius: previous.map_or(INITIAL_CONTEXT_RADIUS, |context| {
                (context.radius + CONTEXT_EXTENSION_RADIUS).min(MAX_CONTEXT_RADIUS)
            }),
            center_pending: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SourceOverlay {
    pub(super) first_line: u32,
    pub(super) last_line: u32,
}

impl SourceOverlay {
    pub(super) fn contains(self, line: u32) -> bool {
        (self.first_line..=self.last_line).contains(&line)
    }
}

const INITIAL_CONTEXT_RADIUS: u32 = 5;
const CONTEXT_EXTENSION_RADIUS: u32 = 5;
const MAX_CONTEXT_RADIUS: u32 = 50;
