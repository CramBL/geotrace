use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Align, Button, Label, Layout, PopupCloseBehavior, RichText, ScrollArea};
use egui_phosphor::regular::{CHECK as ICON_CHECK, COPY as ICON_COPY, WARNING as ICON_WARNING};
use gt_log_view::{AttachmentSettingsStatus, LoadedLog, LoadedLogs};
use gt_pending_writes::WriteAccess;
use gt_ui_types::LoadedLogId;

use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

use super::LogViewerRequests;

pub(in crate::app) fn toolbar_ui(
    ui: &mut egui::Ui,
    logs: &LoadedLogs,
    access: WriteAccess,
    requests: &mut LogViewerRequests,
) {
    let failures = logs
        .iter()
        .filter(|log| log.settings_save_error().is_some())
        .count();
    let saving = logs
        .iter()
        .filter(|log| log.settings_save_status(access) == AttachmentSettingsStatus::Saving)
        .count();
    let label = match (failures, saving) {
        (0, 0) => "Log saves".to_owned(),
        (0, saving) => format!("Log saves … {saving}"),
        (failures, 0) => format!("Log saves {ICON_WARNING} {failures}"),
        (failures, saving) => format!("Log saves {ICON_WARNING} {failures} … {saving}"),
    };
    let label = if failures > 0 {
        RichText::new(label).color(gt_ui_theme::warning_amber(ui.visuals().dark_mode))
    } else {
        RichText::new(label).weak()
    };
    ui.add_enabled_ui(!logs.is_empty(), |ui| {
        let response = MenuButton::new(label)
            .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
                ui.set_width(DETAILS_MAX_WIDTH_PX);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ScrollArea::vertical()
                        .max_height(INSPECTOR_MAX_HEIGHT_PX)
                        .show(ui, |ui| {
                            for (id, log) in logs.iter_with_ids() {
                                ui.push_id(id, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(log.name());
                                        ui.label(StatusPresentation::of(log, access).text(ui));
                                    });
                                    if let Some(attachment) = log.attachment() {
                                        ui.add(
                                            Label::new(
                                                RichText::new(format!(
                                                    "{}/{}",
                                                    gt_loaded_files::display_identity(
                                                        &attachment.recording.identity
                                                    )
                                                    .0,
                                                    attachment.recording.group_name,
                                                ))
                                                .small(),
                                            )
                                            .selectable(true)
                                            .wrap(),
                                        );
                                    }
                                    details_ui(ui, log, id, access, requests);
                                });
                                ui.separator();
                            }
                        });
                });
            })
            .0
            .on_hover_text("Review settings saves for all loaded logs")
            .on_disabled_hover_text("Load a log to review its settings");
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_description(if logs.is_empty() {
                "Load a log to review its settings"
            } else {
                "Review settings saves for all loaded logs"
            });
        });
    });
}

pub(super) fn selected_log_ui(
    ui: &mut egui::Ui,
    log: &LoadedLog,
    id: LoadedLogId,
    access: WriteAccess,
    requests: &mut LogViewerRequests,
) {
    let response = MenuButton::new(StatusPresentation::of(log, access).text(ui))
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, |ui| {
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                details_ui(ui, log, id, access, requests);
            });
        })
        .0
        .on_hover_text("Review settings save status");
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_description("Review settings save status");
    });
}

struct StatusPresentation {
    label: &'static str,
    attention: bool,
    icon: &'static str,
}

impl StatusPresentation {
    fn of(log: &LoadedLog, access: WriteAccess) -> Self {
        let (label, attention, icon) = match log.settings_save_status(access) {
            AttachmentSettingsStatus::Dirty => ("Settings unsaved", false, ""),
            AttachmentSettingsStatus::Failed(_) => ("Settings save failed", true, ICON_WARNING),
            AttachmentSettingsStatus::Saved => ("Settings saved", false, ICON_CHECK),
            AttachmentSettingsStatus::Saving => ("Saving settings…", false, ""),
            AttachmentSettingsStatus::SessionOnly => ("Session-only settings", false, ""),
        };
        Self {
            label,
            attention,
            icon,
        }
    }

    fn text(&self, ui: &egui::Ui) -> RichText {
        let text = if self.icon.is_empty() {
            RichText::new(self.label)
        } else {
            RichText::new(format!("{} {}", self.icon, self.label))
        };
        if self.attention {
            text.color(gt_ui_theme::warning_amber(ui.visuals().dark_mode))
        } else {
            text.weak()
        }
    }
}

fn details_ui(
    ui: &mut egui::Ui,
    log: &LoadedLog,
    id: LoadedLogId,
    access: WriteAccess,
    requests: &mut LogViewerRequests,
) {
    ui.set_max_width(DETAILS_MAX_WIDTH_PX);
    let explanation = match log.settings_save_status(access) {
        AttachmentSettingsStatus::Dirty => "Settings changes are waiting to save",
        AttachmentSettingsStatus::Failed(_) => "Settings changes remain in this session",
        AttachmentSettingsStatus::Saved => "Current settings are saved with this log",
        AttachmentSettingsStatus::Saving => "Saving settings with this log in recording history",
        AttachmentSettingsStatus::SessionOnly if log.attachment().is_some() => {
            "Changes apply in this session while recording history is read-only"
        }
        AttachmentSettingsStatus::SessionOnly => {
            "Save this log with a recording to persist its settings"
        }
    };
    ui.label(explanation);
    if let Some(error) = log.settings_save_error() {
        let retry_action = match log.attachment() {
            Some(attachment) => format!(
                "Save the latest settings for {} ({}/{})",
                log.name(),
                gt_loaded_files::display_identity(&attachment.recording.identity).0,
                attachment.recording.group_name,
            ),
            None => format!("Save the latest settings for {}", log.name()),
        };
        let retry = ui
            .add_enabled(access.allows_writing(), Button::new("Retry"))
            .on_hover_text("Save the latest settings for this log")
            .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER);
        retry.ctx.accesskit_node_builder(retry.id, |node| {
            node.set_description(if access.allows_writing() {
                &retry_action
            } else {
                READ_ONLY_RECORDING_HISTORY_HOVER
            });
        });
        if retry.clicked() {
            requests.retry_settings_save = Some(id);
        }
        ui.collapsing("Error details", |ui| {
            ui.add(Label::new(error).selectable(true).wrap());
            if ui
                .button(format!("{ICON_COPY} Copy error"))
                .on_hover_text("Copy the settings save error")
                .clicked()
            {
                ui.ctx().copy_text(error.to_owned());
            }
        });
    }
}

const INSPECTOR_MAX_HEIGHT_PX: f32 = 360.0;
const DETAILS_MAX_WIDTH_PX: f32 = 360.0;

#[cfg(test)]
mod tests;
