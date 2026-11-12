use std::mem;

use egui::style::ScrollAnimation;
use egui::{Button, Popup, TextEdit, WidgetInfo, WidgetType};
use gt_log_view::{InvalidLogJump, LoadedLog, LoadedLogs, LogJumpRequest};
use gt_ui_types::LoadedLogId;
use strum::{AsRefStr, EnumIter, IntoEnumIterator as _};

use super::line_table::SourceRequest;
use super::{InspectorSection, LogViewerWindow};

#[derive(Default)]
pub(super) struct LogJumpEditor {
    mode: Option<JumpMode>,
    line: String,
    time: String,
    feedback: Option<String>,
    focus_requested: bool,
}

impl LogJumpEditor {
    pub(super) fn close(&mut self) {
        self.mode = None;
    }

    fn editor_ui(
        &mut self,
        ui: &mut egui::Ui,
        id: LoadedLogId,
        available: bool,
    ) -> Option<JumpAction> {
        let mode = self.mode?;
        let field_id = egui::Id::new((JUMP_FIELD_ID, id, mode));
        let mut action = self.keyboard_action(ui, field_id, available);
        let reveal_editor = self.focus_requested;
        let row = ui.horizontal_wrapped(|ui| {
            self.input_ui(ui, field_id, mode, available);
            if ui
                .add_enabled(available, Button::new("Go"))
                .on_hover_text("Show the requested source location (Enter)")
                .on_disabled_hover_text(UNAVAILABLE_TIME_HELP)
                .clicked()
            {
                action = Some(JumpAction::Submit);
            }
            let close = ui.small_button(super::ICON_X).on_hover_text(CLOSE_LABEL);
            close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, CLOSE_LABEL));
            if close.clicked() {
                action = Some(JumpAction::Close);
            }
            if !available {
                ui.weak("Dates unresolved");
                if ui
                    .button("Set dates…")
                    .on_hover_text(UNAVAILABLE_TIME_HELP)
                    .clicked()
                {
                    action = Some(JumpAction::OpenDates);
                }
            }
            if let Some(feedback) = &self.feedback {
                ui.label(feedback);
            }
        });
        if reveal_editor {
            row.response
                .scroll_to_me_animation(Some(egui::Align::Max), ScrollAnimation::none());
        }
        action
    }

    fn input_ui(&mut self, ui: &mut egui::Ui, field_id: egui::Id, mode: JumpMode, available: bool) {
        let (label, hint, value) = match mode {
            JumpMode::GoToLine => (LINE_LABEL, "Physical line number", &mut self.line),
            JumpMode::GoToTime => (TIME_LABEL, "2026-05-29 18:48:25 UTC", &mut self.time),
        };
        ui.label(label);
        let field = ui
            .add_enabled(
                available,
                TextEdit::singleline(value)
                    .id(field_id)
                    .desired_width(match mode {
                        JumpMode::GoToLine => LINE_FIELD_WIDTH_PX,
                        JumpMode::GoToTime => TIME_FIELD_WIDTH_PX,
                    })
                    .hint_text(hint),
            )
            .on_hover_text(match mode {
                JumpMode::GoToLine => LINE_HELP,
                JumpMode::GoToTime => TIME_HELP,
            })
            .on_disabled_hover_text(UNAVAILABLE_TIME_HELP);
        field.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, available, label));
        field.ctx.accesskit_node_builder(field.id, |node| {
            node.set_author_id(JUMP_FIELD_ID);
            node.set_description("Enter goes to the location. Escape closes this editor.");
        });
        if field.changed() {
            self.feedback = None;
        }
        if mem::take(&mut self.focus_requested) && available {
            field.request_focus();
        }
    }

    fn keyboard_action(
        &self,
        ui: &egui::Ui,
        field_id: egui::Id,
        available: bool,
    ) -> Option<JumpAction> {
        if ui.ctx().top_layer_id() != Some(ui.layer_id()) || Popup::is_any_open(ui.ctx()) {
            return None;
        }
        let focused = ui.memory(|memory| memory.focused());
        if focused != Some(field_id) {
            return (focused.is_none()
                && ui.memory(|memory| memory.had_focus_last_frame(field_id))
                && ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                }))
            .then_some(JumpAction::Close);
        }
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(JumpAction::Close)
            } else if available && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                Some(JumpAction::Submit)
            } else {
                None
            }
        })
    }

    fn request(&self) -> Option<Result<LogJumpRequest, InvalidLogJump>> {
        self.mode.map(|mode| match mode {
            JumpMode::GoToLine => LogJumpRequest::parse_line(&self.line),
            JumpMode::GoToTime => LogJumpRequest::parse_time(&self.time),
        })
    }
}

impl LogViewerWindow {
    pub(super) fn jump_menu_ui(&mut self, ui: &mut egui::Ui, log: &LoadedLog, id: LoadedLogId) {
        ui.menu_button("Go to…", |ui| {
            for mode in JumpMode::iter() {
                if ui
                    .button(mode.as_ref())
                    .on_hover_text(match mode {
                        JumpMode::GoToLine => LINE_HELP,
                        JumpMode::GoToTime => TIME_HELP,
                    })
                    .clicked()
                {
                    self.set_find_open(id, false);
                    let state = self.log_state_mut(id);
                    state.jump.mode = Some(mode);
                    state.jump.feedback = None;
                    state.table_focus_requested = false;
                    state.jump.focus_requested = true;
                    if mode == JumpMode::GoToTime && !log.parsed().chronology().is_resolved() {
                        self.open_jump_dates(id);
                    }
                    ui.close();
                }
                if mode == JumpMode::GoToTime && !log.parsed().chronology().is_resolved() {
                    ui.weak("Dates unresolved")
                        .on_hover_text(UNAVAILABLE_TIME_HELP);
                }
            }
        })
        .response
        .on_hover_text("Jump to a physical source line or an absolute time");
    }

    pub(super) fn jump_bar_ui(&mut self, ui: &mut egui::Ui, logs: &LoadedLogs, id: LoadedLogId) {
        let Some(log) = logs.get_by_id(id) else {
            return;
        };
        let available = self.log_state_mut(id).jump.mode != Some(JumpMode::GoToTime)
            || log.parsed().chronology().is_resolved();
        match self.log_state_mut(id).jump.editor_ui(ui, id, available) {
            Some(JumpAction::Close) => {
                let state = self.log_state_mut(id);
                if let Some(mode) = state.jump.mode {
                    ui.memory_mut(|memory| {
                        memory.surrender_focus(egui::Id::new((JUMP_FIELD_ID, id, mode)))
                    });
                }
                state.jump.close();
                state.table_focus_requested = true;
                state.scroll_to_source_line =
                    state.scroll_to_source_line.or(state.table_reading_anchor);
            }
            Some(JumpAction::OpenDates) => self.open_jump_dates(id),
            Some(JumpAction::Submit) => {
                if let Some(request) = self.log_state_mut(id).jump.request() {
                    let result = request
                        .map_err(|error| error.to_string())
                        .and_then(|request| {
                            request.resolve(logs, id).map_err(|error| error.to_string())
                        });
                    match result {
                        Ok(target) => {
                            self.navigate_to_source(
                                log,
                                target.log(),
                                SourceRequest::NavigateToLine(target.line_number()),
                            );
                            self.log_state_mut(id).jump.feedback = None;
                        }
                        Err(error) => self.log_state_mut(id).jump.feedback = Some(error),
                    }
                    self.log_state_mut(id).jump.focus_requested = true;
                    ui.ctx().request_repaint();
                }
            }
            None => {}
        }
    }

    fn open_jump_dates(&mut self, id: LoadedLogId) {
        if self.log_state_mut(id).inspector != Some(InspectorSection::ShowChronology) {
            self.toggle_inspector(id, InspectorSection::ShowChronology);
        }
    }
}

#[derive(AsRefStr, Clone, Copy, Debug, EnumIter, Eq, Hash, PartialEq)]
enum JumpMode {
    #[strum(serialize = "Go to line…")]
    GoToLine,
    #[strum(serialize = "Go to time…")]
    GoToTime,
}

enum JumpAction {
    Close,
    OpenDates,
    Submit,
}

pub(super) const JUMP_FIELD_ID: &str = "log-source-jump";
pub(super) const LINE_LABEL: &str = "Go to line";
pub(super) const TIME_LABEL: &str = "Go to time";
const CLOSE_LABEL: &str = "Close source jump";
const LINE_HELP: &str = "Use original physical line numbers, including hidden and unparsed lines";
const TIME_HELP: &str =
    "Enter a full date and time with UTC, Z or a numeric offset. Fractional seconds are supported.";
const UNAVAILABLE_TIME_HELP: &str =
    "Absolute time requires resolved dates. Open date controls to set the years.";
const LINE_FIELD_WIDTH_PX: f32 = 100.0;
const TIME_FIELD_WIDTH_PX: f32 = 260.0;
