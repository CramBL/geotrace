mod analysis;
#[cfg(feature = "self-update")]
mod application;
mod geomagnetic_indices;
mod interface;
mod interference;
mod persist;
mod processing;
mod snap;
mod source_page;
mod tec;

use egui::{ScrollArea, Window};
use egui_phosphor::regular::AIRPLANE_TILT as ICON_AIRPLANE_TILT;
use egui_phosphor::regular::GAUGE as ICON_GAUGE;
use egui_phosphor::regular::MAGNET as ICON_MAGNET;
use egui_phosphor::regular::MONITOR as ICON_MONITOR;
use egui_phosphor::regular::PATH as ICON_PATH;
use egui_phosphor::regular::SLIDERS_HORIZONTAL as ICON_SLIDERS_HORIZONTAL;
use egui_phosphor::regular::WAVES as ICON_WAVES;
use gt_map::MapLayer;
use strum::{EnumIter, IntoEnumIterator};

#[cfg(feature = "self-update")]
use egui_phosphor::regular::APP_WINDOW as ICON_APP_WINDOW;

use super::App;
use super::backfill_ui::BackfillReadiness;

/// One category of the settings window, in the order the rail lists them.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug, EnumIter)]
pub(super) enum SettingsPage {
    #[default]
    Processing,
    Analysis,
    AircraftInterference,
    GeomagneticIndices,
    IonosphericTec,
    SnapToRoad,
    Interface,
    /// Gated on `self-update` because the update check is the page's only
    /// control.
    #[cfg(feature = "self-update")]
    Application,
}

impl SettingsPage {
    fn rail_label(self) -> &'static str {
        match self {
            Self::Processing => "Processing",
            Self::Analysis => "Analysis",
            Self::AircraftInterference => gt_jam::text::LAYER_LABEL,
            Self::GeomagneticIndices => "Geomagnetic indices",
            Self::IonosphericTec => "Ionospheric TEC",
            Self::SnapToRoad => "Snap to road",
            Self::Interface => "Interface",
            #[cfg(feature = "self-update")]
            Self::Application => "Application",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Processing => ICON_SLIDERS_HORIZONTAL,
            Self::Analysis => ICON_GAUGE,
            Self::AircraftInterference => ICON_AIRPLANE_TILT,
            Self::GeomagneticIndices => ICON_MAGNET,
            Self::IonosphericTec => ICON_WAVES,
            Self::SnapToRoad => ICON_PATH,
            Self::Interface => ICON_MONITOR,
            #[cfg(feature = "self-update")]
            Self::Application => ICON_APP_WINDOW,
        }
    }

    /// The header the page opens with: the same icon and label the rail entry
    /// shows.
    fn show_header(self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(self.icon());
            ui.strong(self.rail_label());
        });
        ui.separator();
    }
}

/// Id the settings window registers its area under, and the key egui remembers
/// its position and size by.
pub(super) const WINDOW_ID: &str = "settings_window";

/// Width of the category rail, wide enough for the longest label with its icon.
const RAIL_WIDTH: f32 = 176.0;

/// Size the window opens at, large enough for the tallest page to render
/// without scrolling. Changing the page never resizes the window: egui keeps
/// the size it remembers under [`WINDOW_ID`].
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(700.0, 480.0);

const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(520.0, 380.0);

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
            .id(egui::Id::new(WINDOW_ID))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(DEFAULT_WINDOW_SIZE)
            .min_size(MIN_WINDOW_SIZE)
            .show(ui.ctx(), |ui| {
                egui::Panel::left("settings_category_rail")
                    .resizable(false)
                    .exact_size(RAIL_WIDTH)
                    .show(ui, |ui| {
                        self.show_category_rail(ui);
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            apply = self.show_selected_page(ui);
                        });
                });
            });

        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }

        self.settings_open = open;
        apply
    }

    fn show_category_rail(&mut self, ui: &mut egui::Ui) {
        for page in SettingsPage::iter() {
            let label = format!("{} {}", page.icon(), page.rail_label());
            if ui
                .selectable_label(self.settings_page == page, label)
                .clicked()
            {
                self.settings_page = page;
            }
        }
    }

    /// Returns `true` in the frame when the user clicks the Processing page's
    /// "Apply to loaded data".
    fn show_selected_page(&mut self, ui: &mut egui::Ui) -> bool {
        match self.settings_page {
            SettingsPage::Processing => self.show_processing_page(ui),
            SettingsPage::Analysis => {
                self.show_analysis_page(ui);
                false
            }
            SettingsPage::AircraftInterference => {
                self.show_interference_page(ui);
                false
            }
            SettingsPage::GeomagneticIndices => {
                self.show_geomagnetic_index_page(ui);
                false
            }
            SettingsPage::IonosphericTec => {
                self.show_tec_page(ui);
                false
            }
            SettingsPage::SnapToRoad => {
                self.show_snap_page(ui);
                false
            }
            SettingsPage::Interface => {
                self.show_interface_page(ui);
                false
            }
            #[cfg(feature = "self-update")]
            SettingsPage::Application => {
                self.show_application_page(ui);
                false
            }
        }
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
