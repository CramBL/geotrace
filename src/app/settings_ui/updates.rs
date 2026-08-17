//! The startup update check toggle.

use crate::app::App;

impl App {
    pub(super) fn show_updates_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.separator();
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
