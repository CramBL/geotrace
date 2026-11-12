use std::mem;

use egui::text::{ByteIndex, LayoutJob};
use egui::{
    Button, ComboBox, Label, Popup, Stroke, StrokeKind, TextEdit, TextFormat, WidgetInfo,
    WidgetType,
};
use egui_phosphor::regular::{
    ARROW_DOWN as ICON_NEXT, ARROW_UP as ICON_PREVIOUS, MAGNIFYING_GLASS as ICON_FIND,
    X as ICON_CLOSE,
};
use gt_log_view::{
    LoadedLog, LoadedLogs, SourceOccurrence, SourceSearch, SourceSearchScope, SourceSearchStatus,
};
use gt_logfile::ParsedLog;
use gt_ui_types::LoadedLogId;

use super::LogViewerWindow;
use super::keyboard_navigation::TableFocusScope;
use super::line_table::SourceRequest;

pub(super) struct LogFind {
    pub(super) open: bool,
    pub(super) text: String,
    pub(super) scope: SourceSearchScope,
    pub(super) search: SourceSearch,
    focus_requested: bool,
}

impl LogFind {
    fn new(id: LoadedLogId) -> Self {
        Self {
            open: false,
            text: String::new(),
            scope: SourceSearchScope::default(),
            search: SourceSearch::new(id),
            focus_requested: false,
        }
    }

    pub(super) fn current(&self) -> Option<&SourceOccurrence> {
        self.open.then(|| self.search.current()).flatten()
    }

    fn controls_ui(&mut self, ui: &mut egui::Ui, id: LoadedLogId) -> Option<FindAction> {
        let mut action = self.keyboard_action(ui, id);
        ui.horizontal_wrapped(|ui| {
            self.query_field_ui(ui, id);
            ui.label("Literal, ignore case");
            self.scope_ui(ui, id);
            action = self.navigation_ui(ui).or(action);
        });
        action
    }

    fn query_field_ui(&mut self, ui: &mut egui::Ui, id: LoadedLogId) {
        let field = ui
            .add(
                TextEdit::singleline(&mut self.text)
                    .id(egui::Id::new((FIND_FIELD_ID, id)))
                    .hint_text(FIND_LABEL)
                    .desired_width(FIND_FIELD_WIDTH_PX),
            )
            .on_hover_text(
                "Literal text, ignoring case. Enter selects next. Shift+Enter selects previous.",
            );
        field.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, FIND_LABEL));
        field.ctx.accesskit_node_builder(field.id, |node| {
            node.set_description(
                "Enter selects next. Shift+Enter selects previous. Escape closes Find.",
            );
        });
        if mem::take(&mut self.focus_requested) {
            field.request_focus();
            field.scroll_to_me(Some(egui::Align::Min));
        }
    }

    fn scope_ui(&mut self, ui: &mut egui::Ui, id: LoadedLogId) {
        let response = ComboBox::from_id_salt(("log_find_scope", id))
            .selected_text(match self.scope {
                SourceSearchScope::SearchWholeLog => "Whole log",
                SourceSearchScope::SearchFilteredEntries => "Filtered results",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.scope,
                    SourceSearchScope::SearchWholeLog,
                    "Whole log",
                );
                ui.selectable_value(
                    &mut self.scope,
                    SourceSearchScope::SearchFilteredEntries,
                    "Filtered results",
                );
            })
            .response
            .on_hover_text("Choose the source lines to search");
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_label("Find scope");
            node.set_description("Search the whole source or filtered entries");
        });
    }

    fn keyboard_action(&self, ui: &egui::Ui, id: LoadedLogId) -> Option<FindAction> {
        if ui.ctx().top_layer_id() != Some(ui.layer_id()) || Popup::is_any_open(ui.ctx()) {
            return None;
        }
        let field = egui::Id::new((FIND_FIELD_ID, id));
        let focused = ui.memory(|memory| memory.focused());
        if focused != Some(field) {
            return (focused.is_none()
                && ui.memory(|memory| memory.had_focus_last_frame(field))
                && ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                }))
            .then_some(FindAction::Close);
        }
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter) {
                Some(FindAction::SelectPrevious)
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                Some(FindAction::SelectNext)
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(FindAction::Close)
            } else {
                None
            }
        })
    }

    fn navigation_ui(&self, ui: &mut egui::Ui) -> Option<FindAction> {
        let mut action = None;
        let available = self.search.status() == &SourceSearchStatus::ShowResults
            && !self.search.occurrences().is_empty();
        for (step, icon, label) in [
            (FindAction::SelectPrevious, ICON_PREVIOUS, PREVIOUS_LABEL),
            (FindAction::SelectNext, ICON_NEXT, NEXT_LABEL),
        ] {
            let response = ui
                .add_enabled(available, Button::new(icon))
                .on_hover_text(label)
                .on_disabled_hover_text("No completed Find matches");
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, available, label));
            if response.clicked() {
                action = Some(step);
            }
        }
        ui.label(self.status_label());
        let response = ui.button(ICON_CLOSE).on_hover_text(CLOSE_LABEL);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, CLOSE_LABEL));
        if response.clicked() {
            action = Some(FindAction::Close);
        }
        action
    }

    fn status_label(&self) -> String {
        match self.search.status() {
            SourceSearchStatus::AwaitFilters | SourceSearchStatus::AwaitSearch => {
                "Searching…".into()
            }
            SourceSearchStatus::ReportEmptyQuery => "Enter source text".into(),
            SourceSearchStatus::ReportFailure(error) => error.clone(),
            SourceSearchStatus::ReportUnavailableLog => "Source unavailable".into(),
            SourceSearchStatus::ShowResults => format!(
                "{} / {}",
                self.search.current_index().map_or(0, |index| index + 1),
                self.search.occurrences().len()
            ),
        }
    }
}

impl LogViewerWindow {
    pub(super) fn find_button_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            return;
        };
        let open = self
            .log_state_mut(id)
            .find
            .as_ref()
            .is_some_and(|find| find.open);
        let response = ui
            .add(Button::new(ICON_FIND).selected(open))
            .on_hover_text("Find source text (Ctrl/Cmd+F)");
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, FIND_LABEL));
        if response.clicked() {
            self.set_find_open(id, !open);
            self.log_state_mut(id).table_focus_requested = open;
        }
    }

    pub(super) fn find_shortcut_ui(&mut self, ui: &egui::Ui, dialog_open: bool) {
        let Some(id) = self.selected else {
            return;
        };
        let focused = ui.memory(|memory| memory.focused());
        let owns_keyboard = focused == Some(egui::Id::new((FIND_FIELD_ID, id)))
            || TableFocusScope::for_log(id).has_focus(ui)
            || (focused.is_none() && ui.rect_contains_pointer(ui.max_rect()));
        if !dialog_open
            && !Popup::is_any_open(ui.ctx())
            && owns_keyboard
            && ui.ctx().top_layer_id() == Some(ui.layer_id())
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            self.set_find_open(id, true);
        }
    }

    pub(super) fn find_bar_ui(&mut self, ui: &mut egui::Ui, logs: &LoadedLogs, id: LoadedLogId) {
        let Some(find) = self
            .log_state_mut(id)
            .find
            .as_mut()
            .filter(|find| find.open)
        else {
            return;
        };
        let previous = find.search.current().cloned();
        find.search.update(logs, &find.text, find.scope);
        let action = find.controls_ui(ui, id);
        let changed = find.search.update(logs, &find.text, find.scope);
        match action {
            Some(FindAction::SelectNext) => {
                find.search.select_next();
            }
            Some(FindAction::SelectPrevious) => {
                find.search.select_previous();
            }
            Some(FindAction::Close) => {
                ui.memory_mut(|memory| memory.surrender_focus(egui::Id::new((FIND_FIELD_ID, id))));
                self.set_find_open(id, false);
                self.log_state_mut(id).table_focus_requested = true;
                return;
            }
            None => {}
        }
        if changed
            || action.is_some()
            || matches!(
                find.search.status(),
                SourceSearchStatus::AwaitSearch | SourceSearchStatus::AwaitFilters
            )
        {
            ui.ctx().request_repaint();
        }
        let current = find.search.current().cloned();
        if let Some(hit) = current
            && let Some(log) = logs.get_by_id(id)
        {
            let navigate = previous.as_ref() != Some(&hit)
                || matches!(
                    action,
                    Some(FindAction::SelectNext | FindAction::SelectPrevious)
                );
            if navigate {
                self.navigate_to_source(log, id, SourceRequest::NavigateToLine(hit.line_number()));
            }
            self.find_preview_ui(ui, log, &hit, navigate);
        }
    }

    pub(super) fn set_find_open(&mut self, id: LoadedLogId, open: bool) {
        let state = self.log_state_mut(id);
        if open {
            state.table_focus_requested = false;
            state.jump.close();
        }
        if open || state.find.is_some() {
            let find = state.find.get_or_insert_with(|| LogFind::new(id));
            find.open = open;
            find.focus_requested = open;
        }
        state.scroll_to_source_line = state.scroll_to_source_line.or(state.table_reading_anchor);
    }

    fn find_preview_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        hit: &SourceOccurrence,
        scroll_to_hit: bool,
    ) {
        if let Some(highlight) = FindHighlight::of(hit, log.parsed()) {
            let preview = ui.horizontal(|ui| {
                self.source_link_ui(
                    ui,
                    log,
                    hit.log(),
                    SourceRequest::NavigateToLine(hit.line_number()),
                    Button::new(format!("Line {}", hit.line_number())).small(),
                );
                ui.add(
                    Label::new(highlight.preview_job(ui))
                        .truncate()
                        .selectable(true),
                )
                .on_hover_text(highlight.source);
            });
            if scroll_to_hit {
                preview.response.scroll_to_me(Some(egui::Align::Max));
            }
        }
    }
}

pub(super) struct FindHighlight<'a> {
    hit: &'a SourceOccurrence,
    source: &'a str,
}

impl<'a> FindHighlight<'a> {
    pub(super) fn of(hit: &'a SourceOccurrence, parsed: &'a ParsedLog) -> Option<Self> {
        Some(Self {
            hit,
            source: parsed.source_line(hit.line_number())?,
        })
    }

    pub(super) fn line_number(&self) -> u32 {
        self.hit.line_number()
    }

    pub(super) fn decorate_row(&self, ui: &egui::Ui, rect: egui::Rect) {
        ui.painter().rect_stroke(
            rect,
            0,
            Stroke::new(FIND_OUTLINE_WIDTH_PX, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }

    pub(super) fn highlight_job(&self, ui: &egui::Ui, job: &mut LayoutJob) {
        let Some(before) = self
            .source
            .strip_suffix(&job.text)
            .or_else(|| self.source.trim_end().strip_suffix(&job.text))
        else {
            return;
        };
        let hit = self.hit.byte_range();
        let Some(end) = hit.end.checked_sub(before.len()) else {
            return;
        };
        let range = hit.start.saturating_sub(before.len())..end.min(job.text.len());
        for section in mem::take(&mut job.sections) {
            let start = section.byte_range.start.max(ByteIndex(range.start));
            let end = section.byte_range.end.min(ByteIndex(range.end));
            if start >= end {
                job.sections.push(section);
                continue;
            }
            if section.byte_range.start < start {
                let mut before = section.clone();
                before.byte_range.end = start;
                job.sections.push(before);
            }
            let mut matched = section.clone();
            matched.byte_range = start..end;
            matched.format.background = ui.visuals().selection.bg_fill;
            job.sections.push(matched);
            if end < section.byte_range.end {
                let mut after = section;
                after.byte_range.start = end;
                job.sections.push(after);
            }
        }
    }

    fn preview_job(&self, ui: &egui::Ui) -> LayoutJob {
        let hit = self.hit.byte_range();
        let start = self
            .source
            .get(..hit.start)
            .and_then(|before| before.char_indices().rev().nth(FIND_PREVIEW_CONTEXT_CHARS))
            .map_or(0, |(index, _)| index);
        let end = self
            .source
            .get(hit.end..)
            .and_then(|after| after.char_indices().nth(FIND_PREVIEW_CONTEXT_CHARS))
            .map_or(self.source.len(), |(index, _)| hit.end + index);
        let mut job = LayoutJob::default();
        for (range, matched) in [
            (start..hit.start, false),
            (hit, true),
            (self.hit.byte_range().end..end, false),
        ] {
            if let Some(text) = self.source.get(range) {
                let mut format = TextFormat::simple(
                    egui::TextStyle::Monospace.resolve(ui.style()),
                    ui.visuals().text_color(),
                );
                if matched {
                    format.background = ui.visuals().selection.bg_fill;
                    format.color = ui.visuals().selection.stroke.color;
                }
                job.append(text, 0.0, format);
            }
        }
        job
    }
}

#[derive(Clone, Copy)]
enum FindAction {
    Close,
    SelectNext,
    SelectPrevious,
}

pub(super) const FIND_LABEL: &str = "Find source text";
pub(super) const FIND_FIELD_ID: &str = "log-source-find";
pub(super) const NEXT_LABEL: &str = "Next Find match";
pub(super) const PREVIOUS_LABEL: &str = "Previous Find match";
pub(super) const CLOSE_LABEL: &str = "Close Find";
const FIND_FIELD_WIDTH_PX: f32 = 140.0;
const FIND_OUTLINE_WIDTH_PX: f32 = 2.0;
const FIND_PREVIEW_CONTEXT_CHARS: usize = 40;
