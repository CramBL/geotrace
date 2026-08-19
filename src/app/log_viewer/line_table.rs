//! The virtualized line table: every entry of a log in file order, with a
//! divider row opening each boot session.

use std::collections::HashMap;

use chrono::Duration;
use egui::{Label, RichText, ScrollArea, Separator};
use gt_fmt::MIDDLE_DOT;
use gt_log_view::LoadedLog;
use gt_logfile::{BootSession, LogEntry, ParsedLog, TimestampKind};
use gt_types::{Latitude, Longitude};
use gt_ui_theme::EM_DASH;

use super::{AssociationWindowUnit, LogViewerWindow, TIMESTAMP_FORMAT};

/// U+2248 ALMOST EQUAL TO, marking an interpolated timestamp.
const ALMOST_EQUAL_TO: &str = "≈";

const INTERPOLATED_TIMESTAMP_HOVER: &str = "Timestamp interpolated between neighbouring entries";

const ASSOCIATED_ROW_HOVER: &str = "Centre the map on this line";

/// Width of the gutter left of every row, wide enough for the order-anomaly
/// marker and keeping the timestamp column aligned on the rows without one.
const GUTTER_WIDTH_PX: f32 = 6.0;

const ANOMALY_MARKER_WIDTH_PX: f32 = 3.0;

/// One row of the table: a boot session's divider, or one entry of the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineTableRow {
    BootSeparator { session_index: usize },
    Entry { entry_index: usize },
}

/// The table's rows over one log: every entry in file order, each boot session
/// preceded by its divider row.
#[derive(Debug, Clone, Copy)]
pub(super) struct LineTableRows<'a> {
    boot_sessions: &'a [BootSession],
    entry_count: usize,
}

impl<'a> LineTableRows<'a> {
    pub(super) fn of(parsed: &'a ParsedLog) -> Self {
        Self {
            boot_sessions: parsed.boot_sessions(),
            entry_count: parsed.entries().len(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entry_count.saturating_add(self.boot_sessions.len())
    }

    pub(super) fn at(&self, row: usize) -> Option<LineTableRow> {
        let session_index = self.session_at(row)?;
        let separator_row = self.row_of_boot_separator(session_index)?;
        if row == separator_row {
            return Some(LineTableRow::BootSeparator { session_index });
        }
        let entries_into_session = row.saturating_sub(separator_row).saturating_sub(1);
        let entry_index = self
            .boot_sessions
            .get(session_index)?
            .entry_range
            .start
            .saturating_add(entries_into_session);
        (entry_index < self.entry_count).then_some(LineTableRow::Entry { entry_index })
    }

    pub(super) fn row_of_boot_separator(&self, session_index: usize) -> Option<usize> {
        let session = self.boot_sessions.get(session_index)?;
        Some(session.entry_range.start.saturating_add(session_index))
    }

    pub(super) fn row_of_entry(&self, entry_index: usize) -> Option<usize> {
        if entry_index >= self.entry_count {
            return None;
        }
        let sessions_before = self
            .boot_sessions
            .partition_point(|session| session.entry_range.start <= entry_index);
        Some(entry_index.saturating_add(sessions_before))
    }

    /// The boot session whose divider or entries `row` falls in.
    fn session_at(&self, row: usize) -> Option<usize> {
        let mut first = 0;
        let mut past_last = self.boot_sessions.len();
        while first < past_last {
            let middle = first + (past_last - first) / 2;
            match self.row_of_boot_separator(middle) {
                Some(separator_row) if separator_row <= row => first = middle + 1,
                _ => past_last = middle,
            }
        }
        first.checked_sub(1)
    }
}

impl LogViewerWindow {
    /// The table of `log`'s lines. Only the rows on screen are built: a journal
    /// of a million lines costs what one of a hundred does.
    pub(super) fn line_table_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        map_center_request: &mut Option<(f64, f64)>,
    ) {
        let parsed = log.parsed();
        let rows = LineTableRows::of(parsed);
        let anomaly_steps: HashMap<usize, Duration> = parsed
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
        let association_window = log.association_window();

        ui.scope(|ui| {
            // Rows sit directly on top of each other, so the table reads as one
            // block of text and a row's index times its height is its offset.
            ui.spacing_mut().item_spacing.y = 0.0;
            // Selectable labels would swallow the click that pans the map.
            ui.style_mut().interaction.selectable_labels = false;
            let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
            let mut scroll_area = ScrollArea::vertical()
                .id_salt("log_viewer_line_table")
                .auto_shrink([false, false]);
            if let Some(row) = self.scroll_to_row.take() {
                scroll_area = scroll_area.vertical_scroll_offset(row as f32 * row_height);
            }
            scroll_area.show_rows(ui, row_height, rows.len(), |ui, shown| {
                for row in shown {
                    match rows.at(row) {
                        Some(LineTableRow::BootSeparator { session_index }) => {
                            if let Some(session) = parsed.boot_sessions().get(session_index) {
                                boot_separator_row_ui(ui, session);
                            }
                        }
                        Some(LineTableRow::Entry { entry_index }) => {
                            let Some(entry) = parsed.entries().get(entry_index) else {
                                continue;
                            };
                            let clicked_position = EntryRow {
                                entry,
                                message: parsed.message(entry),
                                position: log.entry_position(entry_index),
                                order_anomaly_step: anomaly_steps.get(&entry_index).copied(),
                                association_window,
                            }
                            .ui(ui, unit);
                            if let Some(position) = clicked_position {
                                *map_center_request = Some(position);
                            }
                        }
                        None => {}
                    }
                }
            });
        });
    }
}

/// One entry of the log, as the table draws it.
struct EntryRow<'a> {
    entry: &'a LogEntry,
    message: &'a str,
    position: Option<(Latitude, Longitude)>,

    /// The backwards step this entry opens, for the rows an order anomaly
    /// starts at.
    order_anomaly_step: Option<Duration>,

    association_window: Duration,
}

impl EntryRow<'_> {
    /// Renders the row, returning where the map should centre when it was clicked.
    fn ui(&self, ui: &mut egui::Ui, unit: AssociationWindowUnit) -> Option<(f64, f64)> {
        let associated = self.position.is_some();
        let timestamp = self.entry.timestamp.format(TIMESTAMP_FORMAT);
        let (prefix, interpolated) = match self.entry.timestamp_kind {
            TimestampKind::Anchored => (" ", false),
            TimestampKind::Interpolated => (ALMOST_EQUAL_TO, true),
        };
        let mut timestamp_text = RichText::new(format!("{prefix}{timestamp}")).monospace();
        let mut message_text = RichText::new(self.message).monospace();
        if interpolated {
            timestamp_text = timestamp_text.weak();
        }
        if !associated {
            timestamp_text = timestamp_text.weak();
            message_text = message_text.weak();
        }

        let row = ui
            .horizontal(|ui| {
                self.gutter_ui(ui);
                let timestamp = ui.label(timestamp_text);
                if interpolated {
                    timestamp.on_hover_text(INTERPOLATED_TIMESTAMP_HOVER);
                }
                ui.add(Label::new(message_text).truncate());
            })
            .response;

        let hover = match (self.order_anomaly_step, self.position) {
            (Some(step), _) => format!(
                "Timestamp steps back {} here with no recorded clock change {EM_DASH} the log \
                 may have been edited or spliced",
                gt_fmt::format_human_terse_duration(step.abs())
            ),
            (None, Some(_)) => ASSOCIATED_ROW_HOVER.to_owned(),
            (None, None) => format!(
                "No GPS fix within {} of this line",
                unit.describe(self.association_window)
            ),
        };
        let Some((latitude, longitude)) = self.position else {
            row.on_hover_text(hover);
            return None;
        };
        let row = row.interact(egui::Sense::click()).on_hover_text(hover);
        row.clicked()
            .then(|| (latitude.as_degrees(), longitude.as_degrees()))
    }

    /// The warning-amber marker on the row an unexplained backwards timestamp
    /// step starts at.
    fn gutter_ui(&self, ui: &mut egui::Ui) {
        let height = ui.text_style_height(&egui::TextStyle::Monospace);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(GUTTER_WIDTH_PX, height), egui::Sense::hover());
        if self.order_anomaly_step.is_none() {
            return;
        }
        let marker =
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(ANOMALY_MARKER_WIDTH_PX, height));
        ui.painter().rect_filled(
            marker,
            1.0,
            gt_ui_theme::warning_amber(ui.visuals().dark_mode),
        );
    }
}

/// The divider opening one boot session, naming the run it starts.
fn boot_separator_row_ui(ui: &mut egui::Ui, session: &BootSession) {
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
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).monospace().strong());
        ui.add(Separator::default().horizontal());
    });
}

#[cfg(test)]
mod tests {
    use gt_logfile::AnchoredBounds;

    use super::*;

    fn sessions(entries_per_session: &[usize]) -> Vec<BootSession> {
        let mut sessions = Vec::new();
        let mut start = 0;
        for (index, count) in entries_per_session.iter().enumerate() {
            sessions.push(BootSession {
                boot_number: u32::try_from(index + 1).unwrap_or(u32::MAX),
                entry_range: start..start + count,
                anchored: None::<AnchoredBounds>,
            });
            start += count;
        }
        sessions
    }

    fn drawn_rows(entries_per_session: &[usize]) -> Vec<LineTableRow> {
        let boot_sessions = sessions(entries_per_session);
        let rows = LineTableRows {
            boot_sessions: &boot_sessions,
            entry_count: entries_per_session.iter().sum(),
        };
        (0..rows.len()).filter_map(|row| rows.at(row)).collect()
    }

    #[test]
    fn each_boot_session_opens_with_its_divider_and_is_followed_by_its_entries() {
        assert_eq!(
            drawn_rows(&[2, 1]),
            [
                LineTableRow::BootSeparator { session_index: 0 },
                LineTableRow::Entry { entry_index: 0 },
                LineTableRow::Entry { entry_index: 1 },
                LineTableRow::BootSeparator { session_index: 1 },
                LineTableRow::Entry { entry_index: 2 },
            ]
        );
    }

    #[test]
    fn a_log_with_no_reboot_separator_is_one_session_of_every_entry() {
        assert_eq!(
            drawn_rows(&[2]),
            [
                LineTableRow::BootSeparator { session_index: 0 },
                LineTableRow::Entry { entry_index: 0 },
                LineTableRow::Entry { entry_index: 1 },
            ]
        );
    }

    /// Both accessors name the row the table draws a session or an entry at,
    /// which is the row the summary panel scrolls to.
    #[test]
    fn the_row_of_a_session_and_of_an_entry_is_where_the_table_draws_them() {
        let boot_sessions = sessions(&[2, 3]);
        let rows = LineTableRows {
            boot_sessions: &boot_sessions,
            entry_count: 5,
        };

        assert_eq!(rows.row_of_boot_separator(1), Some(3));
        assert_eq!(
            rows.at(3),
            Some(LineTableRow::BootSeparator { session_index: 1 })
        );
        assert_eq!(rows.row_of_entry(4), Some(6));
        assert_eq!(rows.at(6), Some(LineTableRow::Entry { entry_index: 4 }));
        assert_eq!(rows.row_of_entry(5), None);
        assert_eq!(rows.at(7), None);
    }
}
