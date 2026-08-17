//! The startup update check toggle.

use crate::app::App;
use crate::app::settings_ui::SettingsPage;

impl App {
    pub(super) fn show_application_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Application.show_header(ui);
        ui.checkbox(
            &mut self.update_check_on_startup,
            "Check for updates on startup",
        )
        .on_hover_text(
            "Check for a newer GeoTrace release on startup and prompt to install it. \
             Always off in development builds and in offline mode.",
        );
    }
}
