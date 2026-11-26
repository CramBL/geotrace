use std::borrow::Cow;
use std::fmt::Write as _;
use std::str;
use std::time::Duration;

use egui::{Button, Label, RichText, ScrollArea};
use gt_session_bridge::{TrafficCapture, TrafficDirection, TrafficFrame};
use gt_ui_components::{ToolWindow, ToolWindowSizing};

use super::App;

#[derive(Default)]
pub(super) struct SessionTrafficWindow {
    open: bool,
    selected: Option<FrameId>,
}

impl SessionTrafficWindow {
    pub(super) fn menu_ui(&mut self, ui: &mut egui::Ui, bridge_available: bool) {
        let item = ui
            .add_enabled(bridge_available, Button::new(MENU_LABEL))
            .on_hover_text("Inspect local session bridge requests and replies")
            .on_disabled_hover_text("Start GeoTrace with --session-bridge to inspect traffic");
        if item.clicked() {
            self.open = true;
            ui.close();
        }
    }

    fn show(&mut self, ui: &egui::Ui, capture: &TrafficCapture) {
        let mut open = self.open;
        ToolWindow {
            id: egui::Id::unique(WINDOW_TITLE),
            title: WINDOW_TITLE,
            viewport: ui.ctx().content_rect(),
            sizing: WINDOW_SIZING,
            movable: true,
            resizable: true,
        }
        .show_ui(ui.ctx(), &mut open, None, |ui| {
            self.contents_ui(ui, capture)
        });
        self.open = open;
        if !open {
            self.selected = None;
        }
    }

    fn contents_ui(&mut self, ui: &mut egui::Ui, capture: &TrafficCapture) {
        ui.set_min_height(ui.available_height());
        let mut enabled = capture.is_enabled();
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut enabled, CAPTURE_LABEL).changed() {
                capture.set_enabled(enabled);
            }
            if ui
                .button(CLEAR_LABEL)
                .on_hover_text("Remove captured frames")
                .clicked()
            {
                capture.clear();
                self.selected = None;
            }
        });
        if enabled {
            ui.ctx().request_repaint_after(CAPTURE_REFRESH_INTERVAL);
        }
        ui.label("Capture continues when this window is closed. GeoTrace keeps frames in memory for this session.");
        ui.separator();

        let previous_selection = self.selected;
        let frames = capture.snapshot();
        if self
            .selected
            .is_some_and(|selected| !frames.iter().any(|frame| FrameId::from(frame) == selected))
        {
            self.selected = None;
        }
        self.frame_list_ui(ui, &frames);
        ui.separator();
        let selected = self
            .selected
            .and_then(|id| frames.iter().find(|frame| FrameId::from(*frame) == id));
        self::payload_ui(ui, selected, self.selected != previous_selection);
    }

    fn frame_list_ui(&mut self, ui: &mut egui::Ui, frames: &[TrafficFrame]) {
        if frames.is_empty() {
            ui.label(EMPTY_TRAFFIC_LABEL);
            return;
        }
        ScrollArea::both()
            .id_salt(FRAME_LIST_ID)
            .max_height(ui.available_height() * FRAME_LIST_HEIGHT_FRACTION)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for frame in frames.iter().rev() {
                    let id = FrameId::from(frame);
                    let time = frame.observed_at.format(gt_fmt::UTC_SECOND_FORMAT);
                    let direction = id.direction_label();
                    let label = format!("{time} UTC   #{}   {direction}", id.exchange_id);
                    ui.push_id(id.widget_key(), |ui| {
                        if ui
                            .selectable_label(
                                self.selected == Some(id),
                                RichText::new(label).monospace(),
                            )
                            .clicked()
                        {
                            self.selected = Some(id);
                        }
                    });
                }
            });
    }
}

impl App {
    pub(super) fn show_session_traffic_window(&mut self, ui: &egui::Ui) {
        if !self.session_traffic_window.open {
            return;
        }
        if let Some(bridge) = &self.session_bridge {
            self.session_traffic_window
                .show(ui, &bridge.traffic_capture());
        } else {
            self.session_traffic_window = SessionTrafficWindow::default();
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct FrameId {
    exchange_id: u64,
    direction: TrafficDirection,
}

impl FrameId {
    fn widget_key(self) -> (u64, &'static str) {
        (self.exchange_id, self.direction_label())
    }

    fn direction_label(self) -> &'static str {
        match self.direction {
            TrafficDirection::Reply => "Reply",
            TrafficDirection::Request => "Request",
        }
    }
}

impl From<&TrafficFrame> for FrameId {
    fn from(frame: &TrafficFrame) -> Self {
        Self {
            exchange_id: frame.exchange_id,
            direction: frame.direction,
        }
    }
}

fn payload_ui(ui: &mut egui::Ui, frame: Option<&TrafficFrame>, reset_scroll: bool) {
    let Some(frame) = frame else {
        ui.add_enabled(false, Button::new(COPY_LABEL))
            .on_disabled_hover_text("Select a frame to copy its payload");
        return;
    };
    let payload = frame.payload();
    if payload.is_empty() {
        ui.add_enabled(false, Button::new(COPY_LABEL))
            .on_disabled_hover_text("This frame has an empty payload");
        ui.label("Empty payload");
        return;
    }
    let (text, encoding_note) = match str::from_utf8(payload) {
        Ok(text) if !text.contains('\0') => (Cow::Borrowed(text), None),
        Ok(_) => (
            Cow::Owned(self::hexadecimal_payload(payload)),
            Some("NUL bytes shown as hex"),
        ),
        Err(_) => (
            Cow::Owned(self::hexadecimal_payload(payload)),
            Some("Invalid UTF-8 shown as hex"),
        ),
    };
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(COPY_LABEL)
            .on_hover_text("Copy the complete payload")
            .clicked()
        {
            ui.ctx().copy_text(text.to_string());
        }
        if let Some(note) = encoding_note {
            ui.label(note);
        }
    });
    let mut scroll = ScrollArea::vertical()
        .id_salt(PAYLOAD_ID)
        .auto_shrink([false, false]);
    if reset_scroll {
        scroll = scroll.scroll_offset(egui::Vec2::ZERO);
    }
    scroll.show(ui, |ui| {
        ui.push_id(FrameId::from(frame).widget_key(), |ui| {
            ui.add(
                Label::new(RichText::new(text).monospace())
                    .selectable(true)
                    .wrap(),
            );
        });
    });
}

fn hexadecimal_payload(payload: &[u8]) -> String {
    let mut text = String::new();
    for (index, byte) in payload.iter().enumerate() {
        if index > 0 {
            text.push(if index.is_multiple_of(HEX_BYTES_PER_LINE) {
                '\n'
            } else {
                ' '
            });
        }
        write!(text, "{byte:02x}").ok();
    }
    text
}

const MENU_LABEL: &str = "Session bridge traffic…";
const WINDOW_TITLE: &str = "Session bridge traffic";
const WINDOW_SIZING: ToolWindowSizing = ToolWindowSizing {
    preferred_fraction: egui::vec2(0.58, 0.65),
    minimum_size: egui::vec2(360.0, 240.0),
    maximum_fraction: egui::vec2(0.95, 0.95),
};
const CAPTURE_LABEL: &str = "Capture traffic";
const CLEAR_LABEL: &str = "Clear";
const COPY_LABEL: &str = "Copy payload";
const EMPTY_TRAFFIC_LABEL: &str = "No captured traffic";
const FRAME_LIST_HEIGHT_FRACTION: f32 = 0.35;
const CAPTURE_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const FRAME_LIST_ID: &str = "captured-frames";
const PAYLOAD_ID: &str = "captured-payload";
const HEX_BYTES_PER_LINE: usize = 16;
