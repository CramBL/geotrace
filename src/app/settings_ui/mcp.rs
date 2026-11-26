use egui::Button;

use super::SettingsPage;
use super::mcp_config::McpConfig;
use crate::app::App;
use crate::app::session_bridge::SessionBridge;

impl App {
    pub(super) fn show_mcp_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Mcp.show_header(ui);
        ui.label("Allow MCP clients under your user account to inspect and control GeoTrace.");
        ui.add_space(ui.spacing().item_spacing.y);
        self.mcp_bridge_controls_ui(ui);

        ui.separator();
        if ui
            .button(COPY_CONFIG)
            .on_hover_text("Copy JSON to paste into your MCP client configuration")
            .clicked()
        {
            match McpConfig::from_current_executable() {
                Ok(config) => {
                    ui.ctx().copy_text(config.into_json());
                    self.toasts.info("MCP config copied");
                }
                Err(error) => {
                    self.toasts.error(error.to_string());
                }
            }
        }
    }

    fn mcp_bridge_controls_ui(&mut self, ui: &mut egui::Ui) {
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

const COPY_CONFIG: &str = "Copy MCP config";
pub(super) const SEARCHABLE_LABELS: &[&str] = &[COPY_CONFIG];
