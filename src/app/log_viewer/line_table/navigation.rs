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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::log_viewer) enum SourceRow {
    ShowEntry(usize),
    ShowStructuralLine(usize),
}

impl SourceRow {
    pub(super) fn entry_index(self) -> Option<usize> {
        match self {
            Self::ShowEntry(index) => Some(index),
            Self::ShowStructuralLine(_) => None,
        }
    }

    pub(super) fn line_number(self, parsed: &ParsedLog) -> Option<u32> {
        match self {
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
            SourceRequest::NavigateToLine(line) => {
                let entry = parsed
                    .entries()
                    .partition_point(|entry| entry.line_number < line);
                if parsed
                    .entries()
                    .get(entry)
                    .is_some_and(|entry| entry.line_number == line)
                {
                    SourceRow::ShowEntry(entry)
                } else {
                    SourceRow::ShowStructuralLine(
                        parsed
                            .structural_lines()
                            .binary_search_by_key(&line, |source| source.line_number)
                            .ok()?,
                    )
                }
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

    pub(super) fn source(self) -> SourceRow {
        self.target.source
    }

    pub(in crate::app::log_viewer) fn reason(
        self,
        log: &LoadedLog,
        show_structural: bool,
    ) -> Option<String> {
        if !self.target.is_hidden(log, show_structural) {
            return None;
        }
        Some(match self.target.source {
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
            LineTableRow::Entry { entry_index, .. } => SourceRow::ShowEntry(entry_index),
            LineTableRow::Structural { structural_index } => {
                SourceRow::ShowStructuralLine(structural_index)
            }
            _ => return None,
        };
        (source == self.target.source)
            .then(|| self.reason(log, show_structural))
            .flatten()
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
        } else if !log.filters().is_table_query_pending()
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
        let return_anchor = state.filtered_return.map_or_else(
            || match state.source_reveal {
                Some(reveal) => reveal.return_anchor,
                None => state.table_reading_anchor,
            },
            |origin| origin.anchor,
        );
        state.filtered_return = None;
        state.source_reveal =
            (hidden || log.filters().is_table_query_pending()).then_some(SourceReveal {
                target,
                table_semantic_revision: log.filters().table_semantic_revision(),
                return_anchor,
            });
        let rows = self.table_rows(log, id);
        let row = match request {
            SourceRequest::ShowBootSession(session) => rows.row_of_boot_divider(session),
            _ => match target.source {
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
