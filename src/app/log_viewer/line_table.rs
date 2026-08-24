//! The virtualized line table: the entries of a log its filters leave visible,
//! in file order, with a divider row opening each boot session.

use std::ops::Range;

use chrono::Duration;
use egui::{Color32, Label, RichText, ScrollArea, Separator, Shape, text::LayoutJob};
use gt_fmt::MIDDLE_DOT;
use gt_log_view::{EntryMatches, FilterStack, LoadedLog, VisibleEntries};
use gt_logfile::{BootSession, LogEntry, ParsedLog, TimestampKind};
use gt_types::{Latitude, Longitude, mercator};
use gt_ui_theme::EM_DASH;
use gt_ui_types::{HoveredLogGlyph, LoadedLogId, LogMatchHover};
use rustc_hash::FxHashMap;

use super::{AssociationWindowUnit, LogViewerWindow, TIMESTAMP_FORMAT};

/// U+2248 ALMOST EQUAL TO, marking an interpolated timestamp.
const ALMOST_EQUAL_TO: &str = "≈";

const INTERPOLATED_TIMESTAMP_HOVER: &str = "Timestamp interpolated between neighbouring entries";

const ASSOCIATED_ROW_HOVER: &str = "Centre the map on this line";

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

/// How strongly the rows of the map's hovered hexagon are tinted in that
/// hexagon's colour: enough to find them in a scrolling table, light enough to
/// read the line through.
const CROSS_HIGHLIGHT_ROW_ALPHA: f32 = 0.3;

/// One row of the table: a boot session's divider, or one entry of the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineTableRow {
    BootSeparator { session_index: usize },
    Entry { entry_index: usize },
}

/// The table's rows over one log: the entries its filters leave visible in file
/// order, each boot session preceded by its divider row.
///
/// A boot session whose every entry is filtered out drops out of the table
/// along with its divider.
#[derive(Debug)]
pub(super) struct LineTableRows<'a> {
    visible: &'a VisibleEntries,

    /// The boot sessions holding at least one visible entry, in file order.
    shown_sessions: Vec<ShownBootSession>,

    row_count: usize,
}

/// One boot session the table draws, and the stretch of the visible set it
/// covers.
#[derive(Debug)]
struct ShownBootSession {
    session_index: usize,

    /// The table row this session's divider is drawn at.
    separator_row: usize,

    /// The rows of the visible set holding this session's entries.
    visible_rows: Range<usize>,
}

impl<'a> LineTableRows<'a> {
    pub(super) fn of(parsed: &ParsedLog, visible: &'a VisibleEntries) -> Self {
        Self::over(parsed.boot_sessions(), visible)
    }

    fn over(boot_sessions: &[BootSession], visible: &'a VisibleEntries) -> Self {
        let mut shown_sessions = Vec::with_capacity(boot_sessions.len());
        let mut row_count = 0;
        for (session_index, session) in boot_sessions.iter().enumerate() {
            let first = visible.row_at_or_after(session.entry_range.start);
            let past_last = visible.row_at_or_after(session.entry_range.end);
            if first >= past_last {
                continue;
            }
            shown_sessions.push(ShownBootSession {
                session_index,
                separator_row: row_count,
                visible_rows: first..past_last,
            });
            row_count = row_count
                .saturating_add(past_last.saturating_sub(first))
                .saturating_add(1);
        }
        Self {
            visible,
            shown_sessions,
            row_count,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.row_count
    }

    pub(super) fn at(&self, row: usize) -> Option<LineTableRow> {
        let shown = self.shown_session_at(row)?;
        if row == shown.separator_row {
            return Some(LineTableRow::BootSeparator {
                session_index: shown.session_index,
            });
        }
        let rows_into_session = row
            .saturating_sub(shown.separator_row)
            .saturating_sub(1)
            .saturating_add(shown.visible_rows.start);
        if rows_into_session >= shown.visible_rows.end {
            return None;
        }
        self.visible
            .entry_index(rows_into_session)
            .map(|entry_index| LineTableRow::Entry { entry_index })
    }

    /// The row the divider of `session_index` is drawn at, `None` for a session
    /// the filters left nothing visible of.
    pub(super) fn row_of_boot_separator(&self, session_index: usize) -> Option<usize> {
        self.shown_sessions
            .iter()
            .find(|shown| shown.session_index == session_index)
            .map(|shown| shown.separator_row)
    }

    /// The row showing the first visible entry at or after `entry_index`,
    /// `None` when the filters left none of the rest of its session visible.
    pub(super) fn row_of_entry(&self, entry_index: usize) -> Option<usize> {
        let visible_row = self.visible.row_at_or_after(entry_index);
        let shown = self
            .shown_sessions
            .iter()
            .find(|shown| shown.visible_rows.contains(&visible_row))?;
        Some(
            shown
                .separator_row
                .saturating_add(1)
                .saturating_add(visible_row.saturating_sub(shown.visible_rows.start)),
        )
    }

    /// The shown session whose divider or entries `row` falls in.
    fn shown_session_at(&self, row: usize) -> Option<&ShownBootSession> {
        let past_row = self
            .shown_sessions
            .partition_point(|shown| shown.separator_row <= row);
        self.shown_sessions.get(past_row.checked_sub(1)?)
    }
}

/// What the table hands back to the app: where a click centres the map, and
/// the hover it shares with the map's hexagons.
pub(super) struct LineTableRequests<'a> {
    pub(super) map_center: &'a mut Option<(f64, f64)>,
    pub(super) hover: &'a mut LogMatchHover,
}

/// The rows the hexagon under the cursor on the map marks in the table.
///
/// A hexagon of another log than the one shown marks nothing: filter stacks
/// are per log, and marking rows across a log switch the reader did not ask
/// for would show them the wrong log's lines.
struct CrossHighlightedRows<'a> {
    glyph: Option<&'a HoveredLogGlyph>,
    shown_log: LoadedLogId,
    fill: Color32,
}

impl<'a> CrossHighlightedRows<'a> {
    fn of(glyph: Option<&'a HoveredLogGlyph>, shown_log: LoadedLogId, dark_mode: bool) -> Self {
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
    /// row the hovered hexagon does not stand for.
    fn fill_of(&self, entry_index: usize) -> Option<Color32> {
        self.glyph?
            .covers(self.shown_log, entry_index)
            .then_some(self.fill)
    }
}

impl LogViewerWindow {
    /// The table of `log`'s visible lines. Only the rows on screen are built: a
    /// journal of a million lines costs what one of a hundred does.
    pub(super) fn line_table_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        log_id: LoadedLogId,
        requests: &mut LineTableRequests<'_>,
    ) {
        let parsed = log.parsed();
        let filters = log.filters();
        let rows = LineTableRows::of(parsed, filters.visible_entries());
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
        let association_window = log.association_window();
        let dark_mode = ui.visuals().dark_mode;
        let gutter = LayerGutter::of(filters, dark_mode);
        let highlight = gt_ui_theme::LOG_LIVE_FILTER.resolve(dark_mode);
        let cross_highlighted =
            CrossHighlightedRows::of(requests.hover.glyph.as_ref(), log_id, dark_mode);
        let mut hovered_row_position = None;

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
                            let message = parsed.message(entry);
                            let interaction = EntryRow {
                                entry,
                                message,
                                highlighted: HighlightedMessage {
                                    spans: filters.live_filter_match_spans(message),
                                    color: highlight,
                                },
                                position: log.entry_position(entry_index),
                                order_anomaly_step: anomaly_steps.get(&entry_index).copied(),
                                association_window,
                                gutter: &gutter,
                                entry_index,
                                cross_highlight_fill: cross_highlighted.fill_of(entry_index),
                            }
                            .ui(ui, unit);
                            if let Some(RowInteraction {
                                latitude,
                                longitude,
                                clicked,
                            }) = interaction
                            {
                                hovered_row_position =
                                    Some(mercator::normalize(latitude, longitude));
                                if clicked {
                                    *requests.map_center =
                                        Some((latitude.as_degrees(), longitude.as_degrees()));
                                }
                            }
                        }
                        None => {}
                    }
                }
            });
        });
        requests.hover.row_position = hovered_row_position;
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

/// One entry of the log, as the table draws it.
struct EntryRow<'a> {
    entry: &'a LogEntry,
    message: &'a str,
    highlighted: HighlightedMessage,
    position: Option<(Latitude, Longitude)>,

    /// The backwards step this entry opens, for the rows an order anomaly
    /// starts at.
    order_anomaly_step: Option<Duration>,

    association_window: Duration,
    gutter: &'a LayerGutter<'a>,
    entry_index: usize,

    /// The background of a row the map's hovered hexagon stands for.
    cross_highlight_fill: Option<Color32>,
}

/// What the cursor did to a row carrying a position: hovering it rings that
/// position on the map, clicking centres the map there.
struct RowInteraction {
    latitude: Latitude,
    longitude: Longitude,
    clicked: bool,
}

/// Where the live filter matched the message, and the colour reserved for it.
struct HighlightedMessage {
    spans: Vec<Range<usize>>,
    color: Color32,
}

impl EntryRow<'_> {
    /// Renders the row, returning what the cursor did to it.
    fn ui(&self, ui: &mut egui::Ui, unit: AssociationWindowUnit) -> Option<RowInteraction> {
        let associated = self.position.is_some();
        let timestamp = self.entry.timestamp.format(TIMESTAMP_FORMAT);
        let (prefix, interpolated) = match self.entry.timestamp_kind {
            TimestampKind::Anchored => (" ", false),
            TimestampKind::Interpolated => (ALMOST_EQUAL_TO, true),
        };
        let mut timestamp_text = RichText::new(format!("{prefix}{timestamp}")).monospace();
        if interpolated || !associated {
            timestamp_text = timestamp_text.weak();
        }
        let message_color = match associated {
            true => ui.visuals().text_color(),
            false => ui.visuals().weak_text_color(),
        };

        // Claimed before the row draws: the fill belongs behind its text.
        let background = ui.painter().add(Shape::Noop);
        let row = ui
            .horizontal(|ui| {
                self.gutter_ui(ui);
                let timestamp = ui.label(timestamp_text);
                if interpolated {
                    timestamp.on_hover_text(INTERPOLATED_TIMESTAMP_HOVER);
                }
                let message = self.message_job(ui, message_color);
                ui.add(Label::new(message));
            })
            .response;
        if let Some(fill) = self.cross_highlight_fill {
            ui.painter()
                .set(background, Shape::rect_filled(row.rect, 0, fill));
        }

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
        row.hovered().then_some(RowInteraction {
            latitude,
            longitude,
            clicked: row.clicked(),
        })
    }

    /// The message, with what the live filter matched painted in the colour
    /// reserved for it. Laid out as one truncated row: a long line must not
    /// push the rows below it out of the virtualized table's grid.
    fn message_job(&self, ui: &egui::Ui, color: Color32) -> LayoutJob {
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let mut job = LayoutJob {
            wrap: egui::text::TextWrapping::truncate_at_width(ui.available_width()),
            ..LayoutJob::default()
        };
        let mut appended_to = 0;
        for span in &self.highlighted.spans {
            let (Some(plain), Some(matched)) = (
                self.message.get(appended_to..span.start),
                self.message.get(span.clone()),
            ) else {
                continue;
            };
            job.append(plain, 0.0, egui::TextFormat::simple(font_id.clone(), color));
            job.append(
                matched,
                0.0,
                egui::TextFormat::simple(font_id.clone(), self.highlighted.color),
            );
            appended_to = span.end;
        }
        if let Some(rest) = self.message.get(appended_to..) {
            job.append(rest, 0.0, egui::TextFormat::simple(font_id, color));
        }
        job
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
    use std::sync::Arc;

    use chrono::{DateTime, TimeZone as _, Utc};
    use gt_log_view::LayerColorSlots;
    use gt_logfile::AnchoredBounds;
    use gt_ui_types::LogMatchColor;

    use super::*;

    /// The log the viewer is showing in the cross-highlight cases.
    const SHOWN_LOG: LoadedLogId = LoadedLogId::new(1);

    const OTHER_LOG: LoadedLogId = LoadedLogId::new(2);

    /// Two phenomena a filter can pick out, one of them logged twice.
    const GUTTER_LOG: &str = "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
2026-01-01 14:02:12 hal-powerd: battery low
2026-01-01 14:02:13 navsyncd: gnss fix lost
";

    fn gutter_log_start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 14, 2, 11)
            .single()
            .unwrap_or_default()
    }

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

    fn every_entry(entries_per_session: &[usize]) -> VisibleEntries {
        VisibleEntries::All {
            entry_count: entries_per_session.iter().sum(),
        }
    }

    fn drawn_rows(rows: &LineTableRows<'_>) -> Vec<LineTableRow> {
        (0..rows.len()).filter_map(|row| rows.at(row)).collect()
    }

    #[test]
    fn each_boot_session_opens_with_its_divider_and_is_followed_by_its_entries() {
        let boot_sessions = sessions(&[2, 1]);
        let visible = every_entry(&[2, 1]);

        assert_eq!(
            drawn_rows(&LineTableRows::over(&boot_sessions, &visible)),
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
        let boot_sessions = sessions(&[2]);
        let visible = every_entry(&[2]);

        assert_eq!(
            drawn_rows(&LineTableRows::over(&boot_sessions, &visible)),
            [
                LineTableRow::BootSeparator { session_index: 0 },
                LineTableRow::Entry { entry_index: 0 },
                LineTableRow::Entry { entry_index: 1 },
            ]
        );
    }

    /// A filtered table keeps the divider of every boot session it still shows
    /// a line of, and drops the ones it shows nothing of.
    #[test]
    fn a_boot_session_the_filters_emptied_drops_out_with_its_divider() {
        let boot_sessions = sessions(&[2, 2, 2]);
        let visible = VisibleEntries::Matching(vec![1, 4, 5]);

        let rows = LineTableRows::over(&boot_sessions, &visible);

        assert_eq!(
            drawn_rows(&rows),
            [
                LineTableRow::BootSeparator { session_index: 0 },
                LineTableRow::Entry { entry_index: 1 },
                LineTableRow::BootSeparator { session_index: 2 },
                LineTableRow::Entry { entry_index: 4 },
                LineTableRow::Entry { entry_index: 5 },
            ]
        );
        assert_eq!(rows.row_of_boot_separator(2), Some(2));
        assert_eq!(
            rows.row_of_boot_separator(1),
            None,
            "the middle boot has no line the filters show"
        );
    }

    /// Both accessors name the row the table draws a session or an entry at,
    /// which is the row the summary panel scrolls to.
    #[test]
    fn the_row_of_a_session_and_of_an_entry_is_where_the_table_draws_them() {
        let boot_sessions = sessions(&[2, 3]);
        let visible = every_entry(&[2, 3]);

        let rows = LineTableRows::over(&boot_sessions, &visible);

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

    /// A line the filters hid scrolls to the next line of its session that they
    /// left visible.
    #[test]
    fn a_hidden_line_scrolls_to_the_next_one_its_session_still_shows() {
        let boot_sessions = sessions(&[4]);
        let visible = VisibleEntries::Matching(vec![0, 3]);

        let rows = LineTableRows::over(&boot_sessions, &visible);

        assert_eq!(rows.row_of_entry(0), Some(1));
        assert_eq!(rows.row_of_entry(1), Some(2), "entry 3 is the next visible");
        assert_eq!(rows.row_of_entry(3), Some(2));
    }

    /// A hexagon of `log` standing for `entry_indices`, as the map publishes
    /// one while the cursor is on it.
    fn hovering(log: LoadedLogId, entry_indices: &[usize]) -> LogMatchHover {
        LogMatchHover {
            glyph: Some(HoveredLogGlyph {
                log,
                color: LogMatchColor::LayerSlot {
                    index: 0,
                    shared: false,
                },
                entry_indices: entry_indices.to_vec(),
            }),
            row_position: None,
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
            stack.add_live_filter_as_chip(&mut slots);
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
            .add_live_filter_as_chip(&mut slots)
            .expect("a written filter becomes a chip");
        stack.wait_for_queries();
        assert_eq!(LayerGutter::of(&stack, true).columns.len(), 1);

        stack.set_chip_enabled(chip, false);

        assert_eq!(LayerGutter::of(&stack, true).columns.len(), 0);
    }
}
