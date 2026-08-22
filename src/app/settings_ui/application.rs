//! The startup update check, the history database's storage controls, and what
//! the environment archives take up.

use egui::Grid;
use egui_phosphor::regular::ARCHIVE as ICON_ARCHIVE;
use egui_phosphor::regular::BROOM as ICON_BROOM;
use egui_phosphor::regular::CLOUD_SUN as ICON_CLOUD_SUN;
use egui_phosphor::regular::QUESTION as ICON_QUESTION;

use crate::app::App;
use crate::app::environment_storage::PrunedDays;
use crate::app::environment_storage_ui::{
    self, AUTO_PRUNE_LABEL as ENVIRONMENT_AUTO_PRUNE_LABEL, DELETE_ALL_LABEL,
    ENVIRONMENT_DATA_LABEL, EnvironmentStorageState, PRUNE_LABEL,
};
use crate::app::settings_ui::SettingsPage;
use crate::app::storage_controls;

#[cfg(feature = "self-update")]
use egui_phosphor::regular::ARROW_CIRCLE_DOWN as ICON_ARROW_CIRCLE_DOWN;

#[cfg(feature = "self-update")]
const UPDATES_LABEL: &str = "Updates";
const RECORDING_STORAGE_LABEL: &str = "Recording storage";
const AUTO_PRUNE_LABEL: &str = "Auto-prune";
const CONFIRMATION_LABEL: &str = "Confirmation";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    #[cfg(feature = "self-update")]
    UPDATES_LABEL,
    RECORDING_STORAGE_LABEL,
    AUTO_PRUNE_LABEL,
    CONFIRMATION_LABEL,
    ENVIRONMENT_DATA_LABEL,
    PRUNE_LABEL,
    DELETE_ALL_LABEL,
    ENVIRONMENT_AUTO_PRUNE_LABEL,
];

impl App {
    pub(super) fn show_application_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Application.show_header(ui);
        let storage_before_edit = self.storage_settings;
        Grid::new("application_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                #[cfg(feature = "self-update")]
                {
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
                }

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

        ui.add_space(8.0);
        ui.separator();
        self.show_environment_data_section(ui);
    }

    /// What the day-keyed archives take up, and the controls deleting days
    /// from them.
    fn show_environment_data_section(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("{ICON_CLOUD_SUN} {ENVIRONMENT_DATA_LABEL}"))
            .on_hover_text(
                "Days downloaded for the recordings loaded this session, kept for the next one",
            );
        ui.add_space(4.0);

        let usage = self.environment_usage();
        let days_before_cutoff = self
            .environment_storage_ui
            .cutoff()
            .map(|cutoff| self.environment_days_covered(PrunedDays::Before(cutoff)))
            .unwrap_or_default();
        let state = EnvironmentStorageState {
            usage: &usage,
            days_before_cutoff,
            deletes_blocked_by: self.environment_deletes_blocked_by(),
        };
        if let Some(request) = self.environment_storage_ui.ui(ui, state) {
            self.pending_environment_prune = Some(request);
        }
        environment_storage_ui::show_auto_prune_age(ui, &mut self.environment_storage_settings);
    }
}
