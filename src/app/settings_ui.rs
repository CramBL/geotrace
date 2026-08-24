mod analysis;
mod application;
mod geomagnetic_indices;
mod interface;
mod interference;
mod persist;
pub(super) mod processing;
pub(super) mod search;
mod snap;
mod solar_flares;
mod source_page;
mod tec;

use egui::{ScrollArea, Window};
use egui_phosphor::regular::AIRPLANE_TILT as ICON_AIRPLANE_TILT;
use egui_phosphor::regular::GAUGE as ICON_GAUGE;
use egui_phosphor::regular::MAGNET as ICON_MAGNET;
use egui_phosphor::regular::MONITOR as ICON_MONITOR;
use egui_phosphor::regular::PATH as ICON_PATH;
use egui_phosphor::regular::SLIDERS_HORIZONTAL as ICON_SLIDERS_HORIZONTAL;
use egui_phosphor::regular::SUN as ICON_SUN;
use egui_phosphor::regular::WAVES as ICON_WAVES;
use gt_map::MapLayer;
use gt_store::EnvironmentArchive;
use strum::{EnumIter, IntoEnumIterator};

use egui_phosphor::regular::APP_WINDOW as ICON_APP_WINDOW;

use super::App;
use super::backfill_ui::BackfillReadiness;
use super::storage::DatabasesPending;

/// One category of the settings window, in the order the rail lists them.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug, EnumIter)]
pub(super) enum SettingsPage {
    #[default]
    Processing,
    Analysis,
    AircraftInterference,
    GeomagneticIndices,
    IonosphericTec,
    SolarFlares,
    SnapToRoad,
    Interface,
    Application,
}

impl SettingsPage {
    pub(super) fn rail_label(self) -> &'static str {
        match self {
            Self::Processing => "Processing",
            Self::Analysis => "Analysis",
            Self::AircraftInterference => gt_jam::text::LAYER_LABEL,
            Self::GeomagneticIndices => "Geomagnetic indices",
            Self::IonosphericTec => "Ionospheric TEC",
            Self::SolarFlares => gt_flare::text::LAYER_LABEL,
            Self::SnapToRoad => "Snap to road",
            Self::Interface => "Interface",
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
            Self::SolarFlares => ICON_SUN,
            Self::SnapToRoad => ICON_PATH,
            Self::Interface => ICON_MONITOR,
            Self::Application => ICON_APP_WINDOW,
        }
    }

    /// The labels the query field matches, alongside the page name. Each one
    /// must be a label the page renders, which
    /// `every_settings_page_renders_the_labels_it_declares` checks.
    pub(super) fn searchable_labels(self) -> &'static [&'static str] {
        match self {
            Self::Processing => processing::SEARCHABLE_LABELS,
            Self::Analysis => analysis::SEARCHABLE_LABELS,
            Self::AircraftInterference => interference::SEARCHABLE_LABELS,
            Self::GeomagneticIndices => geomagnetic_indices::SEARCHABLE_LABELS,
            Self::IonosphericTec => tec::SEARCHABLE_LABELS,
            Self::SolarFlares => solar_flares::SEARCHABLE_LABELS,
            Self::SnapToRoad => snap::SEARCHABLE_LABELS,
            Self::Interface => interface::SEARCHABLE_LABELS,
            Self::Application => application::SEARCHABLE_LABELS,
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

const NO_MATCHES_TEXT: &str = "No matching settings";

impl App {
    /// What a download control may do right now. An archive still opening or
    /// one that could not be opened outranks offline mode: there is nowhere
    /// to download to either way.
    fn backfill_readiness(&self, archive: EnvironmentArchive) -> BackfillReadiness {
        if !self.pending_writes.write_access().allows_writing() {
            return BackfillReadiness::ReadOnlySession;
        }
        match self.storage_open.databases_pending() {
            Some(DatabasesPending::WaitingForTheDataDirectory) => {
                BackfillReadiness::WaitingForTheDataDirectory
            }
            Some(DatabasesPending::AwaitingAnInterruptedDeleteAnswer) => {
                BackfillReadiness::AwaitingAnInterruptedDeleteAnswer
            }
            Some(DatabasesPending::Opening) => BackfillReadiness::ArchiveStillOpening,
            None => match self.unavailable_archives.of(archive) {
                Some(reason) => BackfillReadiness::ArchiveUnavailable(reason),
                None if !self.environment_archive_available(archive) => {
                    BackfillReadiness::WithoutArchive
                }
                None if self.offline => BackfillReadiness::Offline,
                None => BackfillReadiness::Ready,
            },
        }
    }

    /// The flare download's readiness, which also depends on the key its
    /// endpoint needs.
    fn solar_flare_backfill_readiness(&self) -> BackfillReadiness {
        let readiness = self.backfill_readiness(EnvironmentArchive::SolarFlares);
        if readiness == BackfillReadiness::Ready && !self.solar_flares.has_api_key() {
            return BackfillReadiness::WithoutApiKey;
        }
        readiness
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
            if self.settings_search.is_active() {
                self.settings_search.clear();
            } else {
                open = false;
            }
        }

        self.settings_open = open;
        apply
    }

    fn show_category_rail(&mut self, ui: &mut egui::Ui) {
        self.settings_search.show_query_field(ui);
        ui.add_space(4.0);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.settings_search.is_active() {
                    self.show_search_matches(ui);
                } else {
                    for page in SettingsPage::iter() {
                        self.show_rail_entry(ui, page);
                    }
                }
            });
    }

    fn show_rail_entry(&mut self, ui: &mut egui::Ui, page: SettingsPage) {
        let label = format!("{} {}", page.icon(), page.rail_label());
        if ui
            .selectable_label(self.settings_page == page, label)
            .clicked()
        {
            self.settings_page = page;
        }
    }

    /// The rail filtered to the matching pages, each followed by the labels
    /// that matched on it. Clicking either opens the page.
    fn show_search_matches(&mut self, ui: &mut egui::Ui) {
        let matches = self.settings_search.page_matches();
        if matches.is_empty() {
            ui.weak(NO_MATCHES_TEXT);
            return;
        }
        for page_match in matches {
            self.show_rail_entry(ui, page_match.page);
            ui.indent(page_match.page, |ui| {
                for label in page_match.labels {
                    if ui
                        .selectable_label(false, egui::RichText::new(label).weak())
                        .clicked()
                    {
                        self.settings_page = page_match.page;
                    }
                }
            });
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
            SettingsPage::SolarFlares => {
                self.show_solar_flare_page(ui);
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
