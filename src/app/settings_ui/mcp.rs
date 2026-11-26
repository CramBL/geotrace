use egui::Button;

use super::SettingsPage;
use crate::app::App;
use crate::app::session_bridge::SessionBridge;

impl App {
    pub(super) fn show_mcp_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Mcp.show_header(ui);
        ui.label("Allow MCP clients under your user account to inspect and control GeoTrace.");
        ui.add_space(ui.spacing().item_spacing.y);

        if self.session_bridge.server().is_some() {
            ui.label("MCP is running");
            if ui.button("Stop MCP").clicked() {
                self.stop_session_bridge();
            }
            return;
        }
        match &self.session_bridge {
            SessionBridge::Stopped => {
                ui.label("MCP is stopped");
            }
            SessionBridge::StartFailed(error) => {
                ui.label("MCP could not start");
                ui.label(error.to_string());
            }
            SessionBridge::Disconnected | SessionBridge::Running(_) => {
                ui.label("MCP stopped unexpectedly");
            }
        }
        if ui
            .add_enabled(!self.shutdown_requested(), Button::new("Start MCP"))
            .on_disabled_hover_text("GeoTrace is closing")
            .clicked()
        {
            self.start_session_bridge();
        }
    }
}
