//! The log viewer window: the loaded logs, the parse summary each of them
//! expands into, and the virtualized table of the selected log's lines.

pub(super) mod association_dialog;
mod association_window;
pub(super) mod filters;
mod line_table;
pub(super) mod log_list;
pub(super) mod restored_logs_badge;
mod summary_panel;
#[cfg(test)]
mod tests;

use egui::{Button, ComboBox, DragValue, Label, RichText, Window};
use egui_phosphor::regular::X as ICON_X;
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs, LogAttachmentRef, RecordingKey, SessionLogAttachments};
use gt_pending_writes::WriteAccess;
use gt_store::DatabaseRef;
use gt_types::FileIdx;
use gt_ui_theme::EM_DASH;
use gt_ui_types::{LoadedLogId, LogMatchGlyph, LogMatchHover};
use rustc_hash::FxHashMap;
use strum::IntoEnumIterator as _;

use association_window::AssociationWindowUnit;
use line_table::LineTableRequests;
use restored_logs_badge::RestoredLogsBadge;

use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

/// The three ways a log gets in, shown by the viewer's empty state and by the
/// drag-and-drop overlay alike.
pub(super) const LOG_LOAD_HINT: &str = "Open a log file, drop it here, or paste log text (Ctrl+V)";

/// How the viewer writes a moment, in the table and in the summary panel alike.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub(super) const LOG_VIEWER_TITLE: &str = "Log viewer";

/// Wide enough for the footer's association controls to sit on one row.
const DEFAULT_WINDOW_WIDTH_PX: f32 = 800.0;

/// Rows of the line table the sections above it leave room for before they
/// start scrolling among themselves.
const TABLE_ROWS_THE_HEADER_LEAVES: usize = 3;

const SUMMARY_HOVER: &str = "Show what the parse read from this log";

const ASSOCIATION_WINDOW_HOVER: &str = "Furthest a line's timestamp may be from a fix of the anchored recording for the line \
     to take its position";

const NO_RECORDING_HOVER: &str = "Take this log's positions from no recording";

const NO_RECORDING_ATTACHED_HOVER: &str =
    "Remove the attachment first: this log is stored with a recording in history";

/// Why a recording that ran at no time the log covers is still a choice: a
/// clock-skewed source is a recording too.
pub(super) const NO_OVERLAP_HOVER: &str =
    "This recording ran at no time the log covers: every line would stay unassociated";

pub(in crate::app) const ATTACH_LABEL: &str = "Attach to recording…";

const ATTACH_HOVER: &str = "Choose the recording this log belongs to, and store it there";

const ATTACH_NO_RECORDING_HOVER: &str = "Load a recording to attach this log to it";

pub(in crate::app) const DETACH_LABEL: &str = "Remove attachment";

const DETACH_HOVER: &str =
    "Take this log out of the recording in history. It stays loaded in this session.";

const DETACH_UNATTACHED_HOVER: &str = "This log is not stored with a recording in history";

const NOTICE_DISMISS_HOVER: &str = "Dismiss this warning";

/// What the list and the footer append to the name of a recording that is not
/// loaded.
pub(super) const NOT_LOADED_MARKER: &str = "not loaded";

pub(in crate::app) const LOAD_RECORDING_LABEL: &str = "Load recording";

const LOAD_RECORDING_HOVER: &str =
    "Gives this log's lines positions by opening its recording from history";

const LOAD_RECORDING_NO_DATABASE_HOVER: &str = "The recordings database is unavailable";

/// Fixed width for the unit dropdown: a wider unit label must not shift the
/// controls beside it.
const UNIT_DROPDOWN_WIDTH_PX: f32 = 52.0;

/// The floating log viewer: which log it shows, how much of that log's parse
/// summary is unfolded, and the unit its association window is entered in.
pub(super) struct LogViewerWindow {
    pub open: bool,

    /// The log the window shows. Resolved against the loaded logs before every
    /// render: `None`, or an id no longer loaded, falls back to the log that
    /// loaded first.
    selected: Option<LoadedLogId>,

    summary_expanded: bool,
    association_window_unit: AssociationWindowUnit,

    /// When the shown log's filters started scanning, for the note the viewer
    /// shows once a scan runs long enough to notice.
    query_pending_since: Option<f64>,

    /// The table row the summary panel asked to scroll to, consumed by the
    /// table on the frame after it was asked for.
    scroll_to_row: Option<usize>,

    /// The hexagon the map was last clicked on. The table marks the rows of
    /// its lines while it shows that hexagon's log.
    clicked_glyph: Option<LogMatchGlyph>,

    /// What went wrong with this session's attachments, shown until dismissed.
    notices: Vec<String>,

    /// The logs that came back with a recording since this window was last
    /// open, announced on the toolbar's log button.
    pub(super) restored_logs: RestoredLogsBadge,
}

/// The app state the viewer reads and writes while it renders.
pub(super) struct LogViewerContext<'a> {
    pub recordings: LoadedFilesView<'a>,
    pub recording_names: &'a RecordingNames,

    /// The attachments the loaded recordings hold in the history database,
    /// shown beside the loaded logs.
    pub attachments: &'a SessionLogAttachments,

    pub map_center_request: &'a mut Option<(f64, f64)>,
    /// The hexagon the map found under the cursor, and the row this viewer
    /// puts under it in return.
    pub log_hover: &'a mut LogMatchHover,

    /// The hexagon the map was clicked on, which this viewer takes and shows
    /// the log of.
    pub clicked_glyph: &'a mut Option<LogMatchGlyph>,

    pub requests: &'a mut LogViewerRequests,

    /// Whether this session writes to the recording history, which is what
    /// attaching a log to a recording and taking it back out do.
    pub write_access: WriteAccess,

    /// Whether the recordings database is open, which is what grays the
    /// footer's "Load recording" while it is not.
    pub history_available: bool,

    /// Whether the association dialog is over the viewer, which then takes the
    /// Escape press for itself.
    pub dialog_open: bool,
}

/// What the viewer requests of the app for the log it is showing. The app owns
/// the history database, the dialog, and the recording a log is anchored to.
#[derive(Debug, Default)]
pub(super) struct LogViewerRequests {
    /// Open the association dialog on this log.
    pub open_association_dialog: Option<LoadedLogId>,

    /// Remove this log's attachment from the history database.
    pub detach: Option<LoadedLogId>,

    /// Read this attachment back out of the history database and load it as a
    /// log.
    pub load_attachment: Option<AttachmentToLoad>,

    /// Open this recording from history. The footer sets it while the shown
    /// log's anchored recording is not loaded.
    pub open_recording: Option<DatabaseRef>,
}

/// One of a loaded recording's attachments, as the viewer's list requested it.
#[derive(Debug)]
pub(super) struct AttachmentToLoad {
    pub attachment: LogAttachmentRef,

    /// The name the list drew, which names the attachment in a warning when it
    /// does not come back.
    pub name: String,
}

impl LogViewerWindow {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            selected: None,
            summary_expanded: false,
            association_window_unit: AssociationWindowUnit::Seconds,
            query_pending_since: None,
            scroll_to_row: None,
            clicked_glyph: None,
            notices: Vec::new(),
            restored_logs: RestoredLogsBadge::default(),
        }
    }

    /// Draws the toolbar's log button, which opens and closes this window and
    /// counts the logs that came back with a recording while it was closed.
    pub(super) fn toolbar_button_ui(&mut self, ui: &mut egui::Ui) {
        let label = self.restored_logs.toolbar_label(ui);
        let hover = self.restored_logs.toolbar_hover_text();
        if ui
            .selectable_label(self.open, label)
            .on_hover_text(hover)
            .clicked()
        {
            self.open = !self.open;
        }
    }

    /// Shows `notice` in the viewer until the user dismisses it, opening the
    /// window on it: an attachment that did not come back is only visible here.
    pub(super) fn report_warning(&mut self, notice: String) {
        log::warn!("{notice}");
        self.notices.push(notice);
        self.open = true;
    }

    /// Shows the log `id` names, opening the window on it.
    pub(super) fn open_on_log(&mut self, id: LoadedLogId) {
        self.selected = Some(id);
        self.open = true;
    }

    /// Shows the log the map's clicked hexagon draws matches of, with the table
    /// scrolled to that hexagon's first line and the rows of all its lines
    /// marked.
    ///
    /// A hexagon of a log that is no longer loaded leaves the window as it is.
    fn open_on_clicked_glyph(&mut self, clicked: LogMatchGlyph, logs: &LoadedLogs) {
        let Some(log) = logs.get_by_id(clicked.log) else {
            return;
        };
        let rows = line_table::LineTableRows::of(log.parsed(), log.filters().visible_entries());
        self.scroll_to_row = clicked
            .entry_indices
            .first()
            .and_then(|&entry_index| rows.row_of_entry(entry_index));
        self.open_on_log(clicked.log);
        self.clicked_glyph = Some(clicked);
    }

    /// Which of the loaded logs the window is showing, as of the last render.
    #[cfg(test)]
    pub(super) fn selected_log(&self) -> Option<LoadedLogId> {
        self.selected
    }

    /// Points the selection at the log this frame draws, which is the selected
    /// one while it stays loaded and the log that loaded first otherwise.
    fn resolve_selected_log(&mut self, logs: &LoadedLogs) {
        self.selected = self
            .selected
            .filter(|id| logs.get_by_id(*id).is_some())
            .or_else(|| logs.first_id());
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        logs: &mut LoadedLogs,
        LogViewerContext {
            recordings,
            recording_names,
            attachments,
            map_center_request,
            log_hover,
            clicked_glyph,
            requests,
            write_access,
            history_available,
            dialog_open,
        }: LogViewerContext<'_>,
    ) {
        // The scans run on worker threads: whatever finished since the last
        // frame becomes what this one draws.
        logs.apply_finished_queries();
        // The ring on the map lives exactly as long as the cursor is on a
        // row: the rows below fill this in again while they draw.
        log_hover.row_position = None;
        if let Some(clicked) = clicked_glyph.take() {
            self.open_on_clicked_glyph(clicked, logs);
        }
        self.resolve_selected_log(logs);
        if !self.open {
            return;
        }
        self.restored_logs.clear();

        let mut open = self.open;
        let mut unload = None;
        Window::new(LOG_VIEWER_TITLE)
            .open(&mut open)
            .default_width(DEFAULT_WINDOW_WIDTH_PX)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                // The footer claims its height first: the sections above it
                // divide what is left.
                egui::Panel::bottom("log_viewer_footer")
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        self.footer_ui(
                            ui,
                            logs,
                            recordings,
                            recording_names,
                            requests,
                            FooterAccess {
                                write_access,
                                history_available,
                            },
                        );
                    });
                // Notices, the list, the parse summary and the filter rows
                // scroll among themselves once they need more room than the
                // window can spare, leaving the table
                // `TABLE_ROWS_THE_HEADER_LEAVES` rows to draw in.
                let reserved_for_the_table = ui.text_style_height(&egui::TextStyle::Monospace)
                    * TABLE_ROWS_THE_HEADER_LEAVES as f32;
                egui::ScrollArea::both()
                    .id_salt("log_viewer_header")
                    .max_height((ui.available_height() - reserved_for_the_table).max(0.0))
                    .show(ui, |ui| {
                        self.notices_ui(ui);
                        let actions =
                            self.log_list_ui(ui, logs, recordings, recording_names, attachments);
                        unload = actions.unload;
                        if let Some(chosen) = actions.load_attachment {
                            requests.load_attachment = Some(chosen);
                        }
                        if logs.is_empty() {
                            ui.label(RichText::new(LOG_LOAD_HINT).weak());
                        }
                        let Some(shown) = self.selected else {
                            return;
                        };
                        if let Some(log) = logs.get_by_id(shown) {
                            self.parse_summary_row_ui(ui, log);
                        }
                        if self.summary_expanded
                            && let Some(log) = logs.get_by_id(shown)
                        {
                            self.summary_panel_ui(ui, log);
                        }
                        self.filters_ui(ui, logs, shown);
                    });
                let Some(shown) = self.selected else {
                    return;
                };
                ui.separator();
                if let Some(log) = logs.get_by_id(shown) {
                    self.line_table_ui(
                        ui,
                        log,
                        shown,
                        &mut LineTableRequests {
                            map_center: map_center_request,
                            hover: log_hover,
                        },
                    );
                }
            });

        if !dialog_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }
        self.open = open;

        if let Some(id) = unload {
            if let Some(log) = logs.remove_by_id(id) {
                log::info!("Unloaded log {:?}", log.name());
            }
            // The next frame resolves the selection against what is left.
            self.selected = None;
        }
    }

    /// The warnings this session's attachments produced, each dismissed on its
    /// own.
    fn notices_ui(&mut self, ui: &mut egui::Ui) {
        let mut dismissed = None;
        for (index, notice) in self.notices.iter().enumerate() {
            // Wrapped: a notice names a recording and a reason.
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(notice).color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                );
                if ui
                    .small_button(ICON_X)
                    .on_hover_text(NOTICE_DISMISS_HOVER)
                    .clicked()
                {
                    dismissed = Some(index);
                }
            });
        }
        if let Some(index) = dismissed {
            self.notices.remove(index);
        }
    }

    /// The loaded logs and the available attachments, grouped by recording.
    fn log_list_ui(
        &mut self,
        ui: &mut egui::Ui,
        logs: &mut LoadedLogs,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
        attachments: &SessionLogAttachments,
    ) -> log_list::LogListActions {
        let groups =
            log_list::group_logs_by_recording(logs, recordings, recording_names, attachments);
        log_list::log_list_ui(ui, &groups, logs, &mut self.selected)
    }

    /// The parse summary of the shown log, which unfolds the summary panel.
    fn parse_summary_row_ui(&mut self, ui: &mut egui::Ui, log: &LoadedLog) {
        if ui
            .add(
                Label::new(RichText::new(log.parse_summary_line()).weak())
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(SUMMARY_HOVER)
            .clicked()
        {
            self.summary_expanded = !self.summary_expanded;
        }
    }

    /// The recording the log takes its positions from, and how far an entry may
    /// be from a fix to take one.
    fn footer_ui(
        &mut self,
        ui: &mut egui::Ui,
        logs: &mut LoadedLogs,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
        requests: &mut LogViewerRequests,
        FooterAccess {
            write_access,
            history_available,
        }: FooterAccess,
    ) {
        let selected = self.selected;
        let Some(log) = selected.and_then(|id| logs.get_by_id(id)) else {
            return;
        };
        let target = log.associated_recording();
        let candidates = log.rank_association_candidates(&recordings);
        let entered_unit = self.association_window_unit;
        let mut value = entered_unit.measure(log.association_window());
        let mut unit = entered_unit;
        let mut chosen_target = target;
        let mut window_edited = false;

        let attached = log.attachment().is_some();
        // The anchored recording while it is not loaded, which is always one of
        // the history database's: a log anchored to a recording the database
        // does not hold unloads with that recording.
        let unresolved_anchor = match target {
            Some(_) => None,
            None => log.anchor_key().and_then(RecordingKey::database_ref),
        };
        let names = recording_names_by_id(recordings, recording_names);

        ui.horizontal_wrapped(|ui| {
            ui.label("Associated with");
            let ranked = candidates.ranked();
            ui.add_enabled_ui(!ranked.is_empty(), |ui| {
                ComboBox::from_id_salt("log_viewer_association_target")
                    .selected_text(
                        target
                            .and_then(|id| names.get(&id).copied())
                            .unwrap_or(EM_DASH),
                    )
                    .show_ui(ui, |ui| {
                        let no_recording = ui
                            .add_enabled(!attached, Button::selectable(target.is_none(), EM_DASH));
                        if no_recording.clicked() {
                            chosen_target = None;
                        }
                        no_recording
                            .on_hover_text(NO_RECORDING_HOVER)
                            .on_disabled_hover_text(NO_RECORDING_ATTACHED_HOVER);
                        for candidate in ranked {
                            let name = names.get(&candidate.recording).copied().unwrap_or(EM_DASH);
                            let overlapping = candidate.overlaps_the_log();
                            let label = if overlapping {
                                RichText::new(name)
                            } else {
                                RichText::new(name).weak()
                            };
                            let hover = if overlapping {
                                format!(
                                    "Ran alongside {} of the log, {} of its span",
                                    gt_fmt::format_human_terse_duration(candidate.overlap),
                                    gt_fmt::format_fraction_percent(candidate.fraction_of_log)
                                )
                            } else {
                                NO_OVERLAP_HOVER.to_owned()
                            };
                            if ui
                                .selectable_label(target == Some(candidate.recording), label)
                                .on_hover_text(hover)
                                .clicked()
                            {
                                chosen_target = Some(candidate.recording);
                            }
                        }
                    })
                    .response
                    .on_disabled_hover_text("Load a recording to associate this log against");
            });

            if let Some(db_ref) = unresolved_anchor {
                let recording = gt_loaded_files::display_identity(&db_ref.identity).0;
                ui.label(
                    RichText::new(format!("Anchored to {recording} ({NOT_LOADED_MARKER})")).weak(),
                );
                let load = ui.add_enabled(history_available, Button::new(LOAD_RECORDING_LABEL));
                if load.clicked() {
                    requests.open_recording = Some(db_ref.clone());
                }
                load.on_hover_text(LOAD_RECORDING_HOVER)
                    .on_disabled_hover_text(LOAD_RECORDING_NO_DATABASE_HOVER);
            }

            ui.separator();
            ui.label("Association window");
            window_edited = ui
                .add(DragValue::new(&mut value).range(0.0..=entered_unit.largest_value()))
                .on_hover_text(ASSOCIATION_WINDOW_HOVER)
                .changed();
            ComboBox::from_id_salt("log_viewer_association_window_unit")
                .selected_text(unit.label())
                .width(UNIT_DROPDOWN_WIDTH_PX)
                .show_ui(ui, |ui| {
                    for choice in AssociationWindowUnit::iter() {
                        ui.selectable_value(&mut unit, choice, choice.label());
                    }
                });

            ui.separator();
            let writes_recordings = write_access.allows_writing();
            let attach = ui.add_enabled(
                !recordings.is_empty() && writes_recordings,
                Button::new(ATTACH_LABEL),
            );
            if attach.clicked() {
                requests.open_association_dialog = selected;
            }
            attach
                .on_hover_text(ATTACH_HOVER)
                .on_disabled_hover_text(if writes_recordings {
                    ATTACH_NO_RECORDING_HOVER
                } else {
                    READ_ONLY_RECORDING_HISTORY_HOVER
                });
            let detach = ui.add_enabled(attached && writes_recordings, Button::new(DETACH_LABEL));
            if detach.clicked() {
                requests.detach = selected;
            }
            detach
                .on_hover_text(DETACH_HOVER)
                .on_disabled_hover_text(if writes_recordings {
                    DETACH_UNATTACHED_HOVER
                } else {
                    READ_ONLY_RECORDING_HISTORY_HOVER
                });
        });

        self.association_window_unit = unit;
        let Some(log) = selected.and_then(|id| logs.get_mut_by_id(id)) else {
            return;
        };
        if window_edited {
            log.set_association_window(entered_unit.window_of(value), &recordings);
        }
        if chosen_target != target {
            log.anchor_to_loaded_recording(chosen_target, &recordings);
        }
    }
}

/// What the footer's attachment and recording controls are allowed to do in
/// this session.
struct FooterAccess {
    write_access: WriteAccess,
    history_available: bool,
}

/// The name each loaded recording goes by, keyed by its session identity.
fn recording_names_by_id<'a>(
    recordings: LoadedFilesView<'a>,
    recording_names: &'a RecordingNames,
) -> FxHashMap<LoadedFileId, &'a str> {
    recordings
        .entries()
        .enumerate()
        .map(|(index, entry)| {
            (
                entry.id(),
                recording_names.get(FileIdx::new(index)).unwrap_or(EM_DASH),
            )
        })
        .collect()
}
