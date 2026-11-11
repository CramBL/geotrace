use std::fmt::Write as _;
use std::path::Path;

use egui::accesskit::Role;
use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Button, Label, PopupCloseBehavior, RichText, ScrollArea, TextEdit, TextStyle};
use egui_phosphor::regular::{
    CLOCK as ICON_CLOCK, LIST as ICON_LIST, PAPERCLIP as ICON_PAPERCLIP, X as ICON_X,
};
use gt_fmt::MIDDLE_DOT;
use gt_loaded_files::{LoadedFilesView, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs, LogAttachmentRef, RecordingKey, SessionLogAttachments};
use gt_logfile::YearResolution;
use gt_types::FileIdx;
use gt_ui_theme::EM_DASH;
use gt_ui_types::LoadedLogId;
use rustc_hash::FxHashMap;

use super::AttachmentToLoad;
use super::restored_logs_badge::RestoredLogsBadge;

/// One group of the list: the rows sharing a [`LogGroupHeading`].
#[derive(Debug, PartialEq)]
pub(super) struct LogGroup {
    pub heading: LogGroupHeading,

    pub rows: Vec<LogRow>,
}

/// What the rows of a group have in common, as the list heads them.
#[derive(Debug, PartialEq)]
pub(super) enum LogGroupHeading {
    /// A recording of this session, under the name the app gives it, with the
    /// logs anchored to it and the attachments it holds that are not loaded.
    LoadedRecording(String),

    /// The logs anchored to no recording.
    NotAnchored,

    /// A recording that is not loaded, under the display form of its identity
    /// in the history database, with the logs anchored to it.
    RecordingNotLoaded(String),
}

impl LogGroupHeading {
    /// The text over the group's rows.
    pub(super) fn text(&self) -> String {
        match self {
            Self::LoadedRecording(recording) => recording.clone(),
            Self::RecordingNotLoaded(recording) => {
                format!("{recording} ({})", super::NOT_LOADED_MARKER)
            }
            Self::NotAnchored => NOT_ANCHORED_HEADING.to_owned(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum LogRow {
    Available(AvailableAttachmentRow),
    Loaded(LoadedLogRow),
}

/// A log of this session, selectable and unloadable, drawn on the map while
/// its own toggle is on.
#[derive(Debug, PartialEq)]
pub(super) struct LoadedLogRow {
    pub id: LoadedLogId,
    pub name: String,
}

/// An attachment of a loaded recording that no loaded log holds.
#[derive(Debug, PartialEq)]
pub(super) struct AvailableAttachmentRow {
    pub attachment: LogAttachmentRef,
    pub name: String,

    /// The app's name for the recording holding this attachment.
    pub recording: String,
}

pub(super) enum LogNavigationAction {
    Activate(LoadedLogId),
    Close(LoadedLogId),
    Open(AttachmentToLoad),
}

pub(super) struct LogTab {
    pub id: LoadedLogId,
    pub title: String,
    pub hover: String,
    pub origin: String,
    date_status: Option<LogDateStatus>,
    saved_with: Option<String>,
    close_hover: String,
}

enum LogDateStatus {
    ShowManualDates,
    ShowUnresolvedDates,
}

/// Every loaded log under the recording it is anchored to, in the order the
/// recordings loaded, then the logs whose recording is not loaded, then the
/// logs anchored to no recording.
pub(super) fn group_logs_by_recording(
    logs: &LoadedLogs,
    recordings: LoadedFilesView<'_>,
    recording_names: &RecordingNames,
    attachments: &SessionLogAttachments,
) -> Vec<LogGroup> {
    let mut groups = Vec::new();
    let mut grouped: Vec<LoadedLogId> = Vec::new();
    for (index, entry) in recordings.entries().enumerate() {
        let recording = recording_names
            .get(FileIdx::new(index))
            .unwrap_or(EM_DASH)
            .to_owned();
        let key = RecordingKey::of_loaded_recording(entry);
        let anchored: Vec<(LoadedLogId, &LoadedLog)> =
            logs.anchored_to(std::slice::from_ref(&key)).collect();
        grouped.extend(anchored.iter().map(|(id, _)| *id));
        let mut rows: Vec<LogRow> = anchored
            .into_iter()
            .map(|(id, log)| LogRow::of_loaded_log(id, log))
            .collect();
        if let Some(db_ref) = entry.history().db_ref() {
            rows.extend(attachments.of_recording(db_ref).iter().filter_map(|entry| {
                let attachment = LogAttachmentRef {
                    recording: db_ref.clone(),
                    id: entry.id,
                };
                (!logs.any_loaded_log_holds(&attachment)).then(|| {
                    LogRow::Available(AvailableAttachmentRow {
                        attachment,
                        name: entry.display_name().into_owned(),
                        recording: recording.clone(),
                    })
                })
            }));
        }
        if !rows.is_empty() {
            groups.push(LogGroup {
                heading: LogGroupHeading::LoadedRecording(recording),
                rows,
            });
        }
    }

    let mut not_loaded: Vec<(RecordingKey, String, Vec<LogRow>)> = Vec::new();
    let mut anchorless: Vec<LogRow> = Vec::new();
    for (id, log) in logs.iter_with_ids().filter(|(id, _)| !grouped.contains(id)) {
        let Some(key) = log.anchor_key() else {
            anchorless.push(LogRow::of_loaded_log(id, log));
            continue;
        };
        let recording = key.database_ref().map_or(EM_DASH, |db_ref| {
            gt_loaded_files::display_identity(&db_ref.identity).0
        });
        let row = LogRow::of_loaded_log(id, log);
        match not_loaded
            .iter_mut()
            .find(|(anchored, ..)| anchored == &key)
        {
            Some((.., rows)) => rows.push(row),
            None => not_loaded.push((key.clone(), recording.to_owned(), vec![row])),
        }
    }
    for (_, recording, rows) in not_loaded {
        groups.push(LogGroup {
            heading: LogGroupHeading::RecordingNotLoaded(recording),
            rows,
        });
    }
    if !anchorless.is_empty() {
        groups.push(LogGroup {
            heading: LogGroupHeading::NotAnchored,
            rows: anchorless,
        });
    }
    groups
}

pub(super) fn tabs_in_session_order(logs: &LoadedLogs, groups: &[LogGroup]) -> Vec<LogTab> {
    let origins: FxHashMap<_, _> = groups
        .iter()
        .flat_map(|group| {
            group.rows.iter().filter_map(|row| match row {
                LogRow::Loaded(loaded) => Some((loaded.id, group.heading.text())),
                LogRow::Available(_) => None,
            })
        })
        .collect();
    let mut tabs: Vec<_> = logs
        .iter_with_ids()
        .map(|(id, log)| {
            let origin = origins
                .get(&id)
                .cloned()
                .unwrap_or_else(|| NOT_ANCHORED_HEADING.to_owned());
            let name = log.name();
            let title = Path::new(name)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(name)
                .to_owned();
            let mut hover = format!("{name}\nPositions from {origin}");
            if let Some(attachment) = log.attachment() {
                write!(
                    hover,
                    "\nSaved with {} / {}\nAttachment {}",
                    attachment.recording.identity, attachment.recording.group_name, attachment.id
                )
                .ok();
            }
            let date_status = if !log.parsed().chronology().is_resolved() {
                Some(LogDateStatus::ShowUnresolvedDates)
            } else if log.parsed().year_resolution() != YearResolution::Infer {
                Some(LogDateStatus::ShowManualDates)
            } else {
                None
            };
            LogTab {
                id,
                title,
                hover,
                origin: origin.clone(),
                date_status,
                saved_with: log.attachment().map(|_| origin.clone()),
                close_hover: if log.attachment().is_some() {
                    format!("Unload this log while keeping it saved with {origin}")
                } else {
                    UNLOAD_HOVER.to_owned()
                },
            }
        })
        .collect();
    let mut counts = FxHashMap::<String, usize>::default();
    for tab in &tabs {
        *counts.entry(tab.title.clone()).or_default() += 1;
    }
    for tab in &mut tabs {
        if counts.get(&tab.title).copied().unwrap_or_default() > 1 {
            tab.title = format!("{} {MIDDLE_DOT} {}", tab.title, tab.origin);
        }
    }
    loop {
        counts.clear();
        for tab in &tabs {
            *counts.entry(tab.title.clone()).or_default() += 1;
        }
        if counts.values().all(|count| *count == 1) {
            break;
        }
        for tab in &mut tabs {
            if counts.get(&tab.title).copied().unwrap_or_default() > 1 {
                tab.title = format!("{} {MIDDLE_DOT} #{}", tab.title, tab.id);
            }
        }
    }
    tabs
}

pub(super) fn tabs_ui(
    ui: &mut egui::Ui,
    tabs: &[LogTab],
    selected: Option<LoadedLogId>,
    restored: &RestoredLogsBadge,
    reveal_selected: bool,
) -> Option<LogNavigationAction> {
    let mut action = None;
    ScrollArea::horizontal()
        .id_salt("log_viewer_tabs")
        .max_height(ui.spacing().interact_size.y)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.ctx()
                    .accesskit_node_builder(ui.unique_id(), |node| node.set_role(Role::TabList));
                for tab in tabs {
                    ui.push_id(tab.id, |ui| {
                        let text = ui.painter().layout_no_wrap(
                            tab.title.clone(),
                            TextStyle::Button.resolve(ui.style()),
                            ui.visuals().text_color(),
                        );
                        let width = (text.size().x + 2.0 * ui.spacing().button_padding.x)
                            .min(MAX_TAB_WIDTH_PX);
                        let response = ui
                            .add_sized(
                                [width, ui.spacing().interact_size.y],
                                Button::selectable(selected == Some(tab.id), &tab.title).truncate(),
                            )
                            .on_hover_text(&tab.hover);
                        response.ctx.accesskit_node_builder(response.id, |node| {
                            node.set_role(Role::Tab);
                            node.clear_toggled();
                            node.set_selected(selected == Some(tab.id));
                            node.set_description(tab.hover.as_str());
                        });
                        if response.clicked() {
                            action = Some(LogNavigationAction::Activate(tab.id));
                        }
                        if reveal_selected && selected == Some(tab.id) {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if let Some(status) = &tab.date_status {
                            let (label, color) = match status {
                                LogDateStatus::ShowManualDates => {
                                    ("Manual dates", ui.visuals().weak_text_color())
                                }
                                LogDateStatus::ShowUnresolvedDates => (
                                    "Dates unresolved",
                                    gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                                ),
                            };
                            let status_response = ui
                                .label(RichText::new(ICON_CLOCK).color(color))
                                .on_hover_text(label);
                            status_response
                                .ctx
                                .accesskit_node_builder(status_response.id, |node| {
                                    node.set_label(label)
                                });
                        }
                        if let Some(recording) = &tab.saved_with {
                            ui.label(ICON_PAPERCLIP)
                                .on_hover_text(format!("Saved with {recording}"));
                        }
                        if restored.is_new(tab.id) {
                            ui.label(
                                RichText::new("New")
                                    .small()
                                    .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                            )
                            .on_hover_text("Restored with a recording");
                        }
                        let close = ui.small_button(ICON_X).on_hover_text(&tab.close_hover);
                        close.ctx.accesskit_node_builder(close.id, |node| {
                            node.set_label(format!("Close {}", tab.title));
                            node.set_author_id(format!("{CLOSE_TAB_ID_PREFIX}{}", tab.id));
                            node.set_description(tab.hover.as_str());
                        });
                        if close.clicked() {
                            action = Some(LogNavigationAction::Close(tab.id));
                        }
                    });
                }
            });
        });
    action
}

pub(super) fn picker_ui(
    ui: &mut egui::Ui,
    groups: &[LogGroup],
    tabs: &[LogTab],
    selected: Option<LoadedLogId>,
    query: &mut String,
) -> Option<LogNavigationAction> {
    let mut action = None;
    let response = MenuButton::new(format!("{ICON_LIST} {OPEN_LOG_LABEL}"))
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, |ui| {
            ui.set_max_width(PICKER_WIDTH_PX);
            let field = ui.add(
                TextEdit::singleline(query)
                    .hint_text("Find a log")
                    .desired_width(PICKER_WIDTH_PX),
            );
            field.ctx.accesskit_node_builder(field.id, |node| {
                node.set_label("Find a log");
                node.set_description("Filter loaded logs and saved attachments");
            });
            let query = query.to_lowercase();
            let tab_by_id: FxHashMap<_, _> = tabs.iter().map(|tab| (tab.id, tab)).collect();
            ScrollArea::vertical()
                .max_height(PICKER_HEIGHT_PX)
                .show(ui, |ui| {
                    for group in groups {
                        let matches: Vec<_> = group
                            .rows
                            .iter()
                            .filter(|row| {
                                query.is_empty()
                                    || match row {
                                        LogRow::Loaded(log) => {
                                            tab_by_id.get(&log.id).is_some_and(|tab| {
                                                format!("{} {}", tab.title, tab.hover)
                                                    .to_lowercase()
                                                    .contains(&query)
                                            })
                                        }
                                        LogRow::Available(log) => {
                                            format!("{} {}", log.recording, log.name)
                                                .to_lowercase()
                                                .contains(&query)
                                        }
                                    }
                            })
                            .collect();
                        if matches.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(group.heading.text()).strong());
                        for row in matches {
                            match row {
                                LogRow::Loaded(log) => {
                                    if let Some(tab) = tab_by_id.get(&log.id)
                                        && ui
                                            .add(
                                                Button::selectable(
                                                    selected == Some(tab.id),
                                                    &tab.title,
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&tab.hover)
                                            .clicked()
                                    {
                                        action = Some(LogNavigationAction::Activate(tab.id));
                                        ui.close();
                                    }
                                }
                                LogRow::Available(log) => {
                                    if log.ui(ui) {
                                        action =
                                            Some(LogNavigationAction::Open(AttachmentToLoad {
                                                attachment: log.attachment.clone(),
                                                name: log.name.clone(),
                                            }));
                                        ui.close();
                                    }
                                }
                            }
                        }
                    }
                });
        })
        .0
        .on_hover_text(PICKER_HOVER);
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_description(PICKER_HOVER));
    action
}

impl LogRow {
    fn of_loaded_log(id: LoadedLogId, log: &LoadedLog) -> Self {
        Self::Loaded(LoadedLogRow {
            id,
            name: log.name().to_owned(),
        })
    }
}

impl AvailableAttachmentRow {
    /// Whether the row's load button was pressed.
    fn ui(&self, ui: &mut egui::Ui) -> bool {
        ui.horizontal(|ui| {
            ui.add(
                Label::new(RichText::new(format!("{ICON_PAPERCLIP} {}", self.name)).weak())
                    .truncate(),
            )
            .on_hover_text(format!(
                "Saved with {}. This log is not loaded in this session.",
                self.recording
            ));
            ui.add(Button::new(LOAD_ATTACHMENT_LABEL).small())
                .on_hover_text(LOAD_ATTACHMENT_HOVER)
                .clicked()
        })
        .inner
    }
}

/// Heading of the group holding the logs that take their positions from no
/// recording.
pub(super) const NOT_ANCHORED_HEADING: &str = "No position source";

pub(super) const UNLOAD_HOVER: &str = "Unload this log";

pub(in crate::app) const OPEN_LOG_LABEL: &str = "Open log…";
pub(in crate::app) const CLOSE_TAB_ID_PREFIX: &str = "log-tab-close-";
const MAX_TAB_WIDTH_PX: f32 = 240.0;
const PICKER_WIDTH_PX: f32 = 260.0;
const PICKER_HEIGHT_PX: f32 = 240.0;
const PICKER_HOVER: &str = "Choose a loaded log or open a saved attachment";

pub(in crate::app) const LOAD_ATTACHMENT_LABEL: &str = "Load";

const LOAD_ATTACHMENT_HOVER: &str = "Load this log saved with the recording";
