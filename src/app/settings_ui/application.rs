//! The startup update check and the history database's storage controls.

use egui::Grid;
use egui_phosphor::regular::ARCHIVE as ICON_ARCHIVE;
use egui_phosphor::regular::ARROW_CIRCLE_DOWN as ICON_ARROW_CIRCLE_DOWN;
use egui_phosphor::regular::BROOM as ICON_BROOM;
use egui_phosphor::regular::QUESTION as ICON_QUESTION;

use crate::app::App;
use crate::app::settings_ui::SettingsPage;
use crate::app::storage_controls;

const UPDATES_LABEL: &str = "Updates";
const RECORDING_STORAGE_LABEL: &str = "Recording storage";
const AUTO_PRUNE_LABEL: &str = "Auto-prune";
const CONFIRMATION_LABEL: &str = "Confirmation";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    UPDATES_LABEL,
    RECORDING_STORAGE_LABEL,
    AUTO_PRUNE_LABEL,
    CONFIRMATION_LABEL,
];

impl App {
    pub(super) fn show_application_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Application.show_header(ui);
        let storage_before_edit = self.storage_settings;
        Grid::new("application_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let update_help =
                    "Check for a newer GeoTrace release on startup and prompt to install it. \
                     Always off in development builds and in offline mode.";
                ui.label(format!("{ICON_ARROW_CIRCLE_DOWN} {UPDATES_LABEL}"))
                    .on_hover_text(update_help);
                ui.checkbox(
                    &mut self.update_check_on_startup,
                    "Check for updates on startup",
                )
                .on_hover_text(update_help);
                ui.end_row();

                ui.label(format!("{ICON_ARCHIVE} {RECORDING_STORAGE_LABEL}"));
                storage_controls::show_auto_store_checkbox(ui, &mut self.storage_settings);
                ui.end_row();

                ui.label(format!("{ICON_BROOM} {AUTO_PRUNE_LABEL}"));
                storage_controls::show_auto_prune_limit(ui, &mut self.storage_settings);
                ui.end_row();

                ui.label(format!("{ICON_QUESTION} {CONFIRMATION_LABEL}"));
                storage_controls::show_auto_prune_confirm_checkbox(ui, &mut self.storage_settings);
                ui.end_row();
            });
        self.sync_db_path_if_auto_store_changed(storage_before_edit);
    }
}
