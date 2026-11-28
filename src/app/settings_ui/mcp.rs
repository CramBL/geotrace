use std::io;
use std::time::Duration;

use egui::{Align, Button, Color32, Label, Layout, RichText, Sense, Stroke};
use egui_phosphor::regular::{
    CHECK as ICON_CHECK, COPY as ICON_COPY, PLAY as ICON_PLAY, STOP as ICON_STOP,
};

use super::SettingsPage;
use super::mcp_config::McpConfig;
use crate::app::App;
use crate::app::session_bridge::SessionBridge;

impl App {
    pub(super) fn show_mcp_page(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            self::mcp_header_ui(ui);
            ui.add_space(HEADER_SECTION_GAP);
            self.mcp_server_card_ui(ui);
            ui.add_space(SECTION_GAP);
            self.mcp_copy_config_button_ui(ui);
        });
    }

    fn mcp_server_card_ui(&mut self, ui: &mut egui::Ui) {
        let card = egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(CARD_CORNER_RADIUS)
            .inner_margin(CARD_INNER_MARGIN);

        card.show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.mcp_status_row_ui(ui);
            ui.add_space(CARD_DIVIDER_GAP);
            ui.separator();
            ui.add_space(CARD_DIVIDER_GAP);
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size.y = CHECKBOX_ROW_HEIGHT;
                ui.checkbox(&mut self.mcp_start_on_launch, START_ON_LAUNCH)
                    .on_hover_text(START_ON_LAUNCH_HOVER);
            });
        });
    }

    fn mcp_status_row_ui(&mut self, ui: &mut egui::Ui) {
        let action = {
            let status = McpStatus::from_bridge(&self.session_bridge);
            let is_running = matches!(status, McpStatus::Running);
            let mut requested_action = None;

            ui.horizontal(|ui| {
                status.dot_ui(ui);
                ui.label(
                    RichText::new(status.label())
                        .color(status.label_color(ui.visuals()))
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    requested_action = self.mcp_action_button_ui(ui, is_running);
                });
            });

            if let Some(error) = status.error_detail() {
                ui.add_space(ERROR_DETAIL_GAP);
                ui.add(
                    Label::new(
                        RichText::new(error.to_string())
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
            }

            requested_action
        };

        match action {
            Some(McpAction::Start) => self.start_session_bridge(),
            Some(McpAction::Stop) => self.stop_session_bridge(),
            None => {}
        }
    }

    fn mcp_copy_config_button_ui(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|input| input.time);
        let recently_copied = match self.mcp_config_copied_at {
            Some(copied_at)
                if now >= copied_at && now - copied_at < COPY_FEEDBACK_DURATION_SECS =>
            {
                let remaining = COPY_FEEDBACK_DURATION_SECS - (now - copied_at);
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(remaining));
                true
            }
            Some(_) => {
                self.mcp_config_copied_at = None;
                false
            }
            None => false,
        };

        let (icon, text_color, stroke_color) = if recently_copied {
            let success = gt_ui_theme::success_indicator(ui.visuals().dark_mode);
            (ICON_CHECK, success, success)
        } else {
            (
                ICON_COPY,
                ui.visuals().text_color(),
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            )
        };

        let clicked = ui
            .scope(|ui| {
                ui.spacing_mut().button_padding = BUTTON_PADDING;
                let button =
                    Button::new(RichText::new(format!("{icon} {COPY_CONFIG}")).color(text_color))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(BUTTON_STROKE_WIDTH, stroke_color))
                        .corner_radius(BUTTON_CORNER_RADIUS)
                        .min_size(egui::vec2(0.0, BUTTON_MIN_HEIGHT));
                ui.add(button).on_hover_text(COPY_CONFIG_HOVER).clicked()
            })
            .inner;

        if clicked {
            match McpConfig::from_current_executable() {
                Ok(config) => {
                    ui.ctx().copy_text(config.into_json());
                    self.toasts.info(COPY_CONFIG_TOAST);
                    self.mcp_config_copied_at = Some(now);
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(
                        COPY_FEEDBACK_DURATION_SECS,
                    ));
                }
                Err(error) => {
                    self.toasts.error(error.to_string());
                }
            }
        }
    }

    fn mcp_action_button_ui(&self, ui: &mut egui::Ui, is_running: bool) -> Option<McpAction> {
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = BUTTON_PADDING;
            if is_running {
                let button = Button::new(RichText::new(format!("{ICON_STOP} {STOP_MCP}")).strong())
                    .corner_radius(BUTTON_CORNER_RADIUS)
                    .min_size(egui::vec2(PRIMARY_BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT));
                return ui.add(button).clicked().then_some(McpAction::Stop);
            }

            let accent_fill = ui.visuals().selection.bg_fill;
            let accent_text = ui.visuals().selection.stroke.color;
            let button = Button::new(
                RichText::new(format!("{ICON_PLAY} {START_MCP}"))
                    .color(accent_text)
                    .strong(),
            )
            .fill(accent_fill)
            .stroke(Stroke::new(
                BUTTON_STROKE_WIDTH,
                accent_text.gamma_multiply(PRIMARY_STROKE_ALPHA),
            ))
            .corner_radius(BUTTON_CORNER_RADIUS)
            .min_size(egui::vec2(PRIMARY_BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT));

            ui.add_enabled(!self.shutdown_requested(), button)
                .on_disabled_hover_text(CLOSING_HOVER)
                .clicked()
                .then_some(McpAction::Start)
        })
        .inner
    }
}

fn mcp_header_ui(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = HEADER_ICON_GAP;
        ui.label(
            RichText::new(SettingsPage::Mcp.icon())
                .size(TITLE_FONT_SIZE)
                .color(ui.visuals().selection.stroke.color),
        );
        ui.label(
            RichText::new(SettingsPage::Mcp.rail_label())
                .size(TITLE_FONT_SIZE)
                .strong()
                .color(ui.visuals().strong_text_color()),
        );
    });
    ui.add_space(TITLE_SUBTITLE_GAP);
    ui.add(
        Label::new(RichText::new(PAGE_DESCRIPTION).color(ui.visuals().weak_text_color())).wrap(),
    );
}

#[derive(Clone, Copy)]
enum McpAction {
    Start,
    Stop,
}

enum McpStatus<'a> {
    Disconnected,
    Running,
    StartFailed(&'a io::Error),
    Stopped,
}

impl<'a> McpStatus<'a> {
    fn from_bridge(bridge: &'a SessionBridge) -> Self {
        if bridge.server().is_some() {
            return Self::Running;
        }
        match bridge {
            SessionBridge::Stopped => Self::Stopped,
            SessionBridge::StartFailed(error) => Self::StartFailed(error),
            SessionBridge::Disconnected | SessionBridge::Running(_) => Self::Disconnected,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Disconnected => "MCP stopped unexpectedly",
            Self::Running => "MCP is running",
            Self::StartFailed(_) => "MCP could not start",
            Self::Stopped => "MCP is stopped",
        }
    }

    fn dot_color(&self, visuals: &egui::Visuals) -> Color32 {
        match self {
            Self::Disconnected => gt_ui_theme::warning_amber(visuals.dark_mode),
            Self::Running => gt_ui_theme::success_indicator(visuals.dark_mode),
            Self::StartFailed(_) => gt_ui_theme::error_indicator(visuals.dark_mode),
            Self::Stopped => visuals.weak_text_color(),
        }
    }

    fn label_color(&self, visuals: &egui::Visuals) -> Color32 {
        match self {
            Self::Disconnected => gt_ui_theme::warning_amber(visuals.dark_mode),
            Self::Running => visuals.strong_text_color(),
            Self::StartFailed(_) => gt_ui_theme::error_indicator(visuals.dark_mode),
            Self::Stopped => visuals.text_color(),
        }
    }

    fn has_halo(&self) -> bool {
        !matches!(self, Self::Stopped)
    }

    fn error_detail(&self) -> Option<&io::Error> {
        match self {
            Self::StartFailed(error) => Some(error),
            Self::Disconnected | Self::Running | Self::Stopped => None,
        }
    }

    fn dot_ui(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(STATUS_DOT_SLOT_SIZE, STATUS_DOT_SLOT_SIZE),
            Sense::hover(),
        );
        let center = rect.center();
        let color = self.dot_color(ui.visuals());
        let painter = ui.painter();
        if self.has_halo() {
            painter.circle_filled(
                center,
                STATUS_DOT_HALO_RADIUS,
                color.gamma_multiply(STATUS_DOT_HALO_ALPHA),
            );
        }
        painter.circle_filled(center, STATUS_DOT_RADIUS, color);
    }
}

const BUTTON_CORNER_RADIUS: f32 = 6.0;
const BUTTON_MIN_HEIGHT: f32 = 28.0;
const BUTTON_PADDING: egui::Vec2 = egui::vec2(12.0, 5.0);
const BUTTON_STROKE_WIDTH: f32 = 1.0;
const CARD_CORNER_RADIUS: f32 = 8.0;
const CARD_DIVIDER_GAP: f32 = 6.0;
const CARD_INNER_MARGIN: egui::Margin = egui::Margin::symmetric(14, 12);
const CHECKBOX_ROW_HEIGHT: f32 = 24.0;
const CLOSING_HOVER: &str = "GeoTrace is closing";
const COPY_CONFIG: &str = "Copy MCP config";
const COPY_CONFIG_HOVER: &str = "Copy JSON to paste into your MCP client configuration";
const COPY_CONFIG_TOAST: &str = "MCP config copied";
pub(crate) const COPY_FEEDBACK_DURATION_SECS: f64 = 1.8;
const ERROR_DETAIL_GAP: f32 = 6.0;
const HEADER_ICON_GAP: f32 = 8.0;
const HEADER_SECTION_GAP: f32 = 16.0;
const PAGE_DESCRIPTION: &str =
    "Allow MCP clients under your user account to inspect and control GeoTrace.";
const PAGE_MARGIN: egui::Margin = egui::Margin::symmetric(12, 8);
const PRIMARY_BUTTON_MIN_WIDTH: f32 = 104.0;
const PRIMARY_STROKE_ALPHA: f32 = 0.35;
const SECTION_GAP: f32 = 14.0;
const START_MCP: &str = "Start MCP";
const START_ON_LAUNCH: &str = "Start MCP on launch";
const START_ON_LAUNCH_HOVER: &str = "Start MCP automatically when GeoTrace opens";
const STATUS_DOT_HALO_ALPHA: f32 = 0.25;
const STATUS_DOT_HALO_RADIUS: f32 = 7.0;
const STATUS_DOT_RADIUS: f32 = 4.0;
const STATUS_DOT_SLOT_SIZE: f32 = 16.0;
const STOP_MCP: &str = "Stop MCP";
const TITLE_FONT_SIZE: f32 = 20.0;
const TITLE_SUBTITLE_GAP: f32 = 4.0;
pub(super) const SEARCHABLE_LABELS: &[&str] = &[START_ON_LAUNCH, COPY_CONFIG];
