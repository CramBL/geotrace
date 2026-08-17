mod analysis;
mod display;
mod geomagnetic_indices;
mod interference;
mod persist;
mod processing;
mod snap;
mod tec;
#[cfg(feature = "self-update")]
mod updates;

use egui::Window;
use gt_map::MapLayer;

use super::App;
use super::backfill_ui::BackfillReadiness;

impl App {
    /// What a download control may do right now. An archive that could not be
    /// opened outranks offline mode: it is the permanent condition.
    fn backfill_readiness(&self, archive_available: bool) -> BackfillReadiness {
        if !archive_available {
            BackfillReadiness::WithoutArchive
        } else if self.offline {
            BackfillReadiness::Offline
        } else {
            BackfillReadiness::Ready
        }
    }

    /// Render the Settings window.
    ///
    /// Returns `true` in the frame when the user clicks "Apply to loaded data",
    /// signalling that the caller should call `apply_resegmentation`.
    pub(super) fn show_settings_window(&mut self, ui: &egui::Ui) -> bool {
        if !self.settings_open {
            return false;
        }
        // The name-template preview reads a stored recording from the History
        // window's cached list when nothing is loaded.
        self.history_window
            .request_recording_list_if_missing(&self.history);
        let mut open = self.settings_open;
        let mut apply = false;
        Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(360.0)
            .show(ui.ctx(), |ui| {
                apply = self.show_processing_page(ui);
                self.show_analysis_page(ui);
                self.show_display_page(ui);
                self.show_snap_page(ui);
                self.show_interference_page(ui);
                self.show_geomagnetic_index_page(ui);
                self.show_tec_page(ui);
                // Only meaningful in dist builds. Builds without the self-update
                // feature carry no update check to toggle.
                #[cfg(feature = "self-update")]
                self.show_updates_page(ui);
            });

        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }

        self.settings_open = open;
        apply
    }

    /// Whether to run the startup update check: enabled in settings, a release
    /// build (avoids hitting GitHub during development), and not offline.
    #[cfg(feature = "self-update")]
    pub(super) fn should_check_for_updates(&self) -> bool {
        self.update_check_on_startup && !cfg!(debug_assertions) && !self.offline
    }
}

pub(super) fn map_layer_to_setting(layer: MapLayer) -> crate::settings::MapLayerSetting {
    match layer {
        MapLayer::OpenStreetMap => crate::settings::MapLayerSetting::Osm,
        MapLayer::Satellite => crate::settings::MapLayerSetting::Satellite,
    }
}

fn map_layer_from_setting(s: crate::settings::MapLayerSetting) -> MapLayer {
    match s {
        crate::settings::MapLayerSetting::Osm => MapLayer::OpenStreetMap,
        crate::settings::MapLayerSetting::Satellite => MapLayer::Satellite,
    }
}

pub(super) fn theme_pref_to_setting(p: egui::ThemePreference) -> crate::settings::ThemeSetting {
    match p {
        egui::ThemePreference::System => crate::settings::ThemeSetting::System,
        egui::ThemePreference::Light => crate::settings::ThemeSetting::Light,
        egui::ThemePreference::Dark => crate::settings::ThemeSetting::Dark,
    }
}

fn theme_pref_from_setting(s: crate::settings::ThemeSetting) -> egui::ThemePreference {
    match s {
        crate::settings::ThemeSetting::System => egui::ThemePreference::System,
        crate::settings::ThemeSetting::Light => egui::ThemePreference::Light,
        crate::settings::ThemeSetting::Dark => egui::ThemePreference::Dark,
    }
}
