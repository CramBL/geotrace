use std::mem;

use egui::epaint::text::FontsView;
use egui::os::OperatingSystem;
use egui::plugin::Plugin;
use egui::text_selection::LabelSelectionState;
use egui::{
    CursorIcon, Event, EventFilter, FocusDirection, IdSalt, Key, LayerId, Modifiers, OutputCommand,
    PointerButton, Pos2, Sense,
};
use gt_ui_types::LoadedLogId;

use super::super::LineTableRows;
use super::super::row_layout::RowLayout;
use super::super::selection_document::{Caret, DocumentRow};
use super::controller::{
    AnchorMode, GestureLimits, KeyboardIntent, PointerCompletion, PointerIntent,
};
use super::keyboard::{Direction, SelectionMotion};
use super::{CaretRange, TableTextSelection, TableTextUi};

pub(in crate::app::log_viewer::line_table) struct TableInputFocus {
    target: egui::Response,
    available: bool,
    after_navigation: bool,
    entered_by_navigation: bool,
}

impl TableInputFocus {
    pub(in crate::app::log_viewer::line_table) fn capture(
        target: egui::Response,
        available: bool,
        focus_requested: &mut bool,
    ) -> Self {
        let entered_by_navigation = target.gained_focus() && target.has_focus();
        if available && mem::take(focus_requested) {
            target.request_focus();
        }
        Self {
            after_navigation: target.has_focus(),
            target,
            available,
            entered_by_navigation,
        }
    }

    fn update_arrow_lock_ui(&self, ui: &egui::Ui, horizontal_arrows: bool) {
        if self.available {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    self.target.id,
                    EventFilter {
                        horizontal_arrows,
                        vertical_arrows: true,
                        ..EventFilter::default()
                    },
                );
            });
        }
    }
}

pub(in crate::app::log_viewer::line_table) struct TableSelectionInput<'controls> {
    drag_target: egui::Response,
    scrollbar: Option<egui::Response>,
    previous_row_controls: &'controls [egui::Response],
}

impl<'controls> TableSelectionInput<'controls> {
    pub(in crate::app::log_viewer::line_table) fn previous_scrollbar_ui(
        ui: &egui::Ui,
        scroll_salt: (&str, LoadedLogId),
    ) -> Option<egui::Response> {
        // egui registers each scrollbar under its axis index.
        ui.ctx()
            .read_response(
                ui.make_persistent_id(IdSalt::new(scroll_salt))
                    .with(VERTICAL_SCROLLBAR_AXIS),
            )
            .filter(egui::Response::enabled)
    }

    fn apply_events_ui(
        ui: &egui::Ui,
        text: &TableTextUi<'_>,
        selection: &mut TableTextSelection,
        focus: &TableInputFocus,
        targets: PointerTargets<'_>,
        limits: GestureLimits,
        layout_row: &impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
    ) -> SelectionOutcome {
        let available = focus.available && targets.text.rect.is_positive();
        let os = ui.ctx().os();
        let (mut events, time, last_spatial_key) = ui.input(|input| {
            (
                input
                    .events
                    .iter()
                    .enumerate()
                    .filter_map(|(index, event)| {
                        Some((index, SelectionInput::from_event(event, os)?))
                    })
                    .collect::<Vec<_>>(),
                input.time,
                SelectionInput::last_spatial_key(input),
            )
        });
        let last_release = events.iter().rev().find_map(|(index, input)| {
            matches!(input, SelectionInput::Release(_)).then_some(*index)
        });
        let label_has_selection = ui
            .ctx()
            .plugin::<LabelSelectionState>()
            .lock()
            .has_selection();
        let tab_pressed = SelectionInput::coalesce_traversal(&mut events);
        let pointer_pressed = events
            .iter()
            .any(|(_, input)| matches!(input, SelectionInput::Press(_)));
        let previously_focused = ui.memory(|memory| memory.had_focus_last_frame(focus.target.id));
        let entry_from_previous_frame = available && focus.entered_by_navigation && !tab_pressed;
        let table_had_focus = if tab_pressed || pointer_pressed {
            previously_focused
        } else {
            focus.after_navigation
        };
        let mut table_focused = entry_from_previous_frame || (available && table_had_focus);
        // A label can already have processed a later press in this frame.
        let mut text_owner = TextCommandOwner::from_table_focus(
            table_focused
                && (entry_from_previous_frame
                    || selection.range().is_some()
                    || !label_has_selection),
        );
        let mut outcome = SelectionOutcome {
            clear_label_selection: entry_from_previous_frame,
            ..SelectionOutcome::default()
        };
        for (index, input) in events {
            match input {
                SelectionInput::Press(press) => {
                    outcome.keyboard_endpoint = None;
                    let captured = available
                        && (targets.text.is_pointer_button_down_on()
                            || last_release.is_some_and(|release| index < release));
                    let action = targets.press_action(
                        ui.ctx().layer_id_at(press.position),
                        &press,
                        captured,
                    );
                    if action == PressAction::Preserve {
                        continue;
                    }
                    outcome.selected_on_release &= press.button != PointerButton::Primary;
                    let owns_press = action == PressAction::Select;
                    outcome.clear_label_selection = owns_press;
                    outcome.request_focus = owns_press;
                    table_focused = owns_press;
                    text_owner = TextCommandOwner::from_table_focus(owns_press);
                    if owns_press {
                        let _ = text.apply_pointer(
                            selection,
                            PointerIntent::Press {
                                position: press.position,
                                time,
                                anchor: press.anchor,
                            },
                            limits,
                        );
                    } else {
                        selection.clear();
                    }
                }
                SelectionInput::Drag(position) if available => {
                    let _ = text.apply_pointer(selection, PointerIntent::Drag(position), limits);
                }
                SelectionInput::Release(position) if available => {
                    let completion = text.apply_pointer(
                        selection,
                        PointerIntent::Release { position, time },
                        limits,
                    );
                    outcome.selected_on_release = completion == Some(PointerCompletion::Selection);
                    if completion.is_some() {
                        outcome.request_focus = true;
                        table_focused = true;
                        text_owner = TextCommandOwner::Table;
                    }
                }
                SelectionInput::Traverse => {
                    let leaves_table = previously_focused || table_focused;
                    table_focused = available && focus.after_navigation && !leaves_table;
                    outcome.clear_label_selection |= table_focused && focus.entered_by_navigation;
                    outcome.request_focus &= table_focused;
                    text_owner = TextCommandOwner::from_table_focus(
                        table_focused && (outcome.clear_label_selection || !label_has_selection),
                    );
                }
                SelectionInput::Scroll(step) if table_focused => {
                    *outcome.scroll_delta_y.get_or_insert(0.0) +=
                        step.delta_y(text, ui.clip_rect().height());
                    outcome.consume_navigation(index, last_spatial_key);
                }
                SelectionInput::SelectAll if text_owner == TextCommandOwner::Table => {
                    selection.select_all(text.document);
                    outcome.commands.consumed.push(index);
                }
                SelectionInput::Navigate(intent) if text_owner == TextCommandOwner::Table => {
                    if let Some(endpoint) = selection.navigate(intent, text.document, |motion| {
                        ui.fonts_mut(|fonts| text.vertical_endpoint(motion, fonts, layout_row))
                    }) {
                        outcome.keyboard_endpoint = Some(endpoint.caret);
                        outcome.consume_navigation(index, last_spatial_key);
                    }
                }
                SelectionInput::Copy if text_owner == TextCommandOwner::Table => {
                    outcome.record_table_copy(index, selection.copy_range());
                }
                SelectionInput::Copy if label_has_selection && !outcome.clear_label_selection => {
                    outcome.commands.last_label_copy = Some(index);
                }
                _ => {}
            }
        }
        outcome
    }

    /// Read each overlapping control's previous response before registering it in this pass.
    pub(in crate::app::log_viewer::line_table) fn register_ui(
        ui: &egui::Ui,
        scrollbar: Option<egui::Response>,
        previous_row_controls: &'controls [egui::Response],
    ) -> Self {
        let viewport = ui.clip_rect().intersect(egui::Rect::from_x_y_ranges(
            ui.max_rect().x_range(),
            ui.clip_rect().y_range(),
        ));
        let sense = Sense::click_and_drag() - Sense::FOCUSABLE;
        Self {
            drag_target: ui.interact(viewport, ui.make_persistent_id("log_text_drag"), sense),
            scrollbar,
            previous_row_controls,
        }
    }

    pub(in crate::app::log_viewer::line_table) fn viewport(&self) -> egui::Rect {
        self.drag_target.rect
    }

    pub(in crate::app::log_viewer::line_table) fn finish_ui(
        self,
        ui: &egui::Ui,
        text: TableTextUi<'_>,
        selection: &mut TableTextSelection,
        focus: &TableInputFocus,
        layout_row: impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
    ) -> bool {
        let available = focus.available && self.viewport().is_positive();
        let targets = PointerTargets {
            text: &self.drag_target,
            scrollbar: self.scrollbar.as_ref(),
            previous_controls: self.previous_row_controls,
        };
        selection.replace_document(Some(text.document.document_revision()));
        let limits = ui.ctx().options(|options| {
            let input = options.input_options;
            GestureLimits {
                distance: input.max_click_dist,
                duration: input.max_click_duration,
                multi_click_delay: input.max_double_click_delay,
            }
        });
        let outcome =
            Self::apply_events_ui(ui, &text, selection, focus, targets, limits, &layout_row);
        let (position, primary_down) =
            ui.input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        if available && primary_down {
            if let Some(position) = position
                && selection.dragging()
            {
                let _ = text.apply_pointer(selection, PointerIntent::Drag(position), limits);
                text.scroll_during_drag_ui(ui, position.y);
            }
        } else {
            selection.stop_drag();
        }

        if outcome.cancel_spatial_focus {
            ui.memory_mut(|memory| memory.move_focus(FocusDirection::None));
        }
        if outcome.request_focus {
            focus.target.request_focus();
        }
        focus.update_arrow_lock_ui(
            ui,
            available && selection.range().is_some() && !selection.dragging(),
        );
        if let Some(endpoint) = outcome.keyboard_endpoint {
            text.reveal_endpoint_ui(ui, endpoint);
        }
        if let Some(delta_y) = outcome.scroll_delta_y {
            ui.scroll_with_delta(egui::vec2(0.0, delta_y));
        }
        let primary = if ui.ctx().os() == OperatingSystem::Nix
            && !ui.input(|input| input.pointer.any_down())
        {
            selection.settle()
        } else {
            None
        };
        outcome
            .commands
            .emit_ui(ui, text.document, outcome.clear_label_selection, primary);
        let over_text = position.is_some_and(|position| {
            targets.at(ui.ctx().layer_id_at(position), position) == PressTarget::Text
                && ui.rect_contains_pointer(self.viewport())
        });
        if available && (selection.dragging() || over_text) {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        text.paint(ui, selection.range());
        outcome.selected_on_release
    }
}

#[derive(Default)]
struct SelectionCommands {
    consumed: Vec<usize>,
    copies: Vec<(usize, CaretRange)>,
    last_label_copy: Option<usize>,
}

impl SelectionCommands {
    fn emit_ui(
        self,
        ui: &egui::Ui,
        document: &LineTableRows,
        clear_label_selection: bool,
        primary: Option<CaretRange>,
    ) {
        if !self.consumed.is_empty() {
            ui.input_mut(|input| {
                let mut consumed = self.consumed.into_iter().peekable();
                let mut index = 0;
                input.events.retain(|_| {
                    let keep = consumed.next_if_eq(&index).is_none();
                    index += 1;
                    keep
                });
            });
        }
        let mut handoff = NativeLabelHandoff::default();
        for (index, range) in self.copies {
            if let Some(text) = document
                .clipboard_text(range)
                .filter(|text| !text.is_empty())
            {
                if self
                    .last_label_copy
                    .is_some_and(|label_copy| index > label_copy)
                {
                    handoff.writes.push(ClipboardWrite::Copy(text));
                } else {
                    ui.ctx().send_cmd(ClipboardWrite::Copy(text).into());
                }
            }
        }
        let primary = primary
            .and_then(|range| document.clipboard_text(range))
            .filter(|text| !text.is_empty())
            .map(ClipboardWrite::Primary);
        if self.last_label_copy.is_some() && (clear_label_selection || !handoff.writes.is_empty()) {
            handoff.clear_selection = clear_label_selection;
            handoff.writes.extend(primary);
            let plugin = ui.ctx().plugin_or_default::<NativeLabelHandoff>();
            let mut plugin = plugin.lock();
            plugin.clear_selection |= handoff.clear_selection;
            plugin.writes.extend(handoff.writes);
        } else {
            if clear_label_selection {
                ui.ctx()
                    .plugin::<LabelSelectionState>()
                    .lock()
                    .clear_selection();
                ui.ctx().request_repaint();
            }
            if let Some(write) = primary {
                ui.ctx().send_cmd(write.into());
            }
        }
    }
}

enum ClipboardWrite {
    Copy(String),
    Primary(String),
}

impl From<ClipboardWrite> for OutputCommand {
    fn from(write: ClipboardWrite) -> Self {
        match write {
            ClipboardWrite::Copy(text) => Self::CopyText(text),
            ClipboardWrite::Primary(text) => Self::TextSelectionSettled(text),
        }
    }
}

#[derive(Default)]
struct NativeLabelHandoff {
    clear_selection: bool,
    writes: Vec<ClipboardWrite>,
}

impl Plugin for NativeLabelHandoff {
    fn debug_name(&self) -> &'static str {
        "Log table native label selection"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        // egui publishes label selections before it runs output hooks.
        if mem::take(&mut self.clear_selection) {
            ctx.plugin::<LabelSelectionState>().lock().clear_selection();
            ctx.request_repaint();
        }
        output
            .platform_output
            .commands
            .extend(self.writes.drain(..).map(OutputCommand::from));
    }
}

#[derive(Clone, Copy)]
struct PointerTargets<'a> {
    text: &'a egui::Response,
    scrollbar: Option<&'a egui::Response>,
    previous_controls: &'a [egui::Response],
}

impl PointerTargets<'_> {
    fn at(&self, layer: Option<LayerId>, position: Pos2) -> PressTarget {
        let contains = |response: &egui::Response| {
            layer == Some(response.layer_id)
                && response.interact_rect.is_positive()
                && response.interact_rect.contains(position)
        };
        if self.scrollbar.is_some_and(contains) {
            PressTarget::Scrollbar
        } else if self.previous_controls.iter().any(contains) {
            PressTarget::Control
        } else if layer == Some(self.text.layer_id) && self.text.rect.contains(position) {
            PressTarget::Text
        } else {
            PressTarget::Outside
        }
    }

    fn press_action(
        &self,
        layer: Option<LayerId>,
        press: &PointerPress,
        captured: bool,
    ) -> PressAction {
        match self.at(layer, press.position) {
            PressTarget::Scrollbar => PressAction::Preserve,
            PressTarget::Text if press.button != PointerButton::Primary => PressAction::Preserve,
            PressTarget::Text if captured => PressAction::Select,
            PressTarget::Text | PressTarget::Control | PressTarget::Outside => PressAction::Clear,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PressTarget {
    Control,
    Outside,
    Scrollbar,
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PressAction {
    Clear,
    Preserve,
    Select,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TextCommandOwner {
    Native,
    Table,
}

impl TextCommandOwner {
    fn from_table_focus(table: bool) -> Self {
        if table { Self::Table } else { Self::Native }
    }
}

#[derive(Default)]
struct SelectionOutcome {
    cancel_spatial_focus: bool,
    clear_label_selection: bool,
    selected_on_release: bool,
    keyboard_endpoint: Option<Caret>,
    request_focus: bool,
    scroll_delta_y: Option<f32>,
    commands: SelectionCommands,
}

impl SelectionOutcome {
    fn consume_navigation(&mut self, index: usize, last_spatial_key: Option<usize>) {
        self.cancel_spatial_focus |= Some(index) == last_spatial_key;
        self.commands.consumed.push(index);
    }

    fn record_table_copy(&mut self, index: usize, range: Option<CaretRange>) {
        if range.is_some() || self.clear_label_selection {
            self.commands.consumed.push(index);
        }
        self.commands
            .copies
            .extend(range.map(|range| (index, range)));
    }
}

enum SelectionInput {
    Copy,
    Drag(Pos2),
    Navigate(KeyboardIntent),
    Press(PointerPress),
    Release(Pos2),
    Scroll(ScrollStep),
    SelectAll,
    Traverse,
}

impl SelectionInput {
    fn from_event(event: &Event, os: OperatingSystem) -> Option<Self> {
        Some(match *event {
            Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers,
            } => {
                let anchor = if modifiers.shift {
                    AnchorMode::Extend
                } else {
                    AnchorMode::Replace
                };
                Self::Press(PointerPress {
                    position: pos,
                    button,
                    anchor,
                })
            }
            Event::PointerMoved(position) => Self::Drag(position),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                ..
            } => Self::Release(pos),
            Event::Key {
                key: Key::Tab,
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none() || modifiers.shift_only() => Self::Traverse,
            Event::Key {
                key: Key::A,
                pressed: true,
                modifiers,
                ..
            } if modifiers.matches_logically(Modifiers::COMMAND) => Self::SelectAll,
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if modifiers.is_none()
                    && let Some(step) = ScrollStep::from_key(key)
                {
                    Self::Scroll(step)
                } else {
                    Self::Navigate(Self::navigation(key, modifiers, os)?)
                }
            }
            Event::Copy | Event::Cut => Self::Copy,
            _ => return None,
        })
    }

    fn last_spatial_key(input: &egui::InputState) -> Option<usize> {
        let filter = EventFilter::default();
        let last = input.raw.events.iter().rfind(|event| {
            matches!(event, Event::Key { key, modifiers, pressed: true, .. }
                if (!modifiers.any() || (*key == Key::Tab && modifiers.shift_only()))
                    && !filter.matches(event))
        })?;
        if !matches!(
            last,
            Event::Key {
                key: Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown,
                ..
            }
        ) {
            return None;
        }
        // egui schedules focus traversal before widgets consume input events.
        let mut matches = input
            .events
            .iter()
            .enumerate()
            .filter(|(_, event)| *event == last);
        let (index, _) = matches.next_back()?;
        let raw_count = input
            .raw
            .events
            .iter()
            .filter(|event| *event == last)
            .count();
        (matches.count() + 1 == raw_count).then_some(index)
    }

    fn coalesce_traversal(events: &mut Vec<(usize, Self)>) -> bool {
        // egui resolves one Tab traversal per input pass.
        let last = events
            .iter()
            .rev()
            .find_map(|(index, input)| matches!(input, Self::Traverse).then_some(*index));
        if let Some(last) = last {
            events.retain(|(index, input)| !matches!(input, Self::Traverse) || *index == last);
        }
        last.is_some()
    }

    fn navigation(key: Key, modifiers: Modifiers, os: OperatingSystem) -> Option<KeyboardIntent> {
        match key {
            Key::ArrowLeft => {
                return Self::horizontal_navigation(Direction::Backward, modifiers, os);
            }
            Key::ArrowRight => {
                return Self::horizontal_navigation(Direction::Forward, modifiers, os);
            }
            _ => {}
        }
        if !modifiers.shift {
            return None;
        }
        let motion = match key {
            Key::ArrowUp if modifiers.command => {
                SelectionMotion::DocumentBoundary(Direction::Backward)
            }
            Key::ArrowDown if modifiers.command => {
                SelectionMotion::DocumentBoundary(Direction::Forward)
            }
            Key::ArrowUp => SelectionMotion::Row(Direction::Backward),
            Key::ArrowDown => SelectionMotion::Row(Direction::Forward),
            Key::Home if modifiers.ctrl => SelectionMotion::DocumentBoundary(Direction::Backward),
            Key::End if modifiers.ctrl => SelectionMotion::DocumentBoundary(Direction::Forward),
            Key::Home => SelectionMotion::RowBoundary(Direction::Backward),
            Key::End => SelectionMotion::RowBoundary(Direction::Forward),
            _ => return None,
        };
        Some(KeyboardIntent::Extend(motion))
    }

    fn horizontal_navigation(
        direction: Direction,
        modifiers: Modifiers,
        os: OperatingSystem,
    ) -> Option<KeyboardIntent> {
        if modifiers.is_none() {
            return Some(KeyboardIntent::CollapseOrMoveCharacter(direction));
        }
        if os == OperatingSystem::Mac && modifiers.ctrl && !modifiers.shift {
            return None;
        }
        let motion = if modifiers.alt || modifiers.ctrl {
            SelectionMotion::Word(direction)
        } else if modifiers.mac_cmd {
            SelectionMotion::RowBoundary(direction)
        } else if !modifiers.shift {
            return None;
        } else {
            SelectionMotion::Character(direction)
        };
        Some(if modifiers.shift {
            KeyboardIntent::Extend(motion)
        } else {
            KeyboardIntent::MoveCaret(motion)
        })
    }
}

enum ScrollStep {
    Document(Direction),
    Page(Direction),
    Row(Direction),
}

impl ScrollStep {
    fn from_key(key: Key) -> Option<Self> {
        Some(match key {
            Key::Home => Self::Document(Direction::Backward),
            Key::End => Self::Document(Direction::Forward),
            Key::PageUp => Self::Page(Direction::Backward),
            Key::PageDown => Self::Page(Direction::Forward),
            Key::ArrowUp => Self::Row(Direction::Backward),
            Key::ArrowDown => Self::Row(Direction::Forward),
            _ => return None,
        })
    }

    fn delta_y(&self, text: &TableTextUi<'_>, page_height: f32) -> f32 {
        let grid = text.shown.grid;
        let (direction, distance) = match *self {
            Self::Document(direction) => (direction, grid.offset(text.document.len())),
            Self::Page(direction) => (direction, page_height),
            Self::Row(direction) => (direction, grid.height),
        };
        match direction {
            Direction::Backward => distance,
            Direction::Forward => -distance,
        }
    }
}

struct PointerPress {
    position: Pos2,
    button: PointerButton,
    anchor: AnchorMode,
}

const VERTICAL_SCROLLBAR_AXIS: usize = 1;

#[cfg(test)]
mod tests {
    use egui::{PointerButton, Sense};

    use super::super::controller::AnchorMode;
    use super::{PointerPress, PointerTargets, PressAction, PressTarget};

    #[test]
    fn a_batched_press_checks_the_clipped_control_region_from_the_previous_pass() {
        let context = egui::Context::default();
        let control_id = egui::Id::unique("row_button");
        let control = egui::Rect::from_min_max(egui::pos2(20.0, 20.0), egui::pos2(50.0, 50.0));
        let viewport = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(90.0, 90.0));
        let previous = context.run_ui(Default::default(), |ui| {
            ui.scope(|ui| {
                ui.set_clip_rect(egui::Rect::from_min_max(
                    egui::pos2(30.0, 30.0),
                    viewport.max,
                ));
                ui.interact(control, control_id, Sense::click());
            });
        });
        previous.drop_without_applying_deltas();
        let output = context.run_ui(Default::default(), |ui| {
            let previous_control = ui.ctx().read_response(control_id).unwrap();
            let table = ui.interact(
                viewport,
                ui.scope_id().with("table"),
                Sense::click_and_drag(),
            );
            let moved_control = ui.interact(
                control.translate(egui::vec2(40.0, 0.0)),
                control_id,
                Sense::click(),
            );
            let targets = PointerTargets {
                text: &table,
                scrollbar: None,
                previous_controls: &[previous_control],
            };
            let position = egui::pos2(40.0, 40.0);
            assert!(!moved_control.interact_rect.contains(position));
            let press = PointerPress {
                position,
                button: PointerButton::Primary,
                anchor: AnchorMode::Replace,
            };
            let layer = Some(ui.layer_id());
            assert_eq!(
                targets.press_action(layer, &press, true),
                PressAction::Clear
            );
            assert_eq!(targets.at(layer, control.min), PressTarget::Text);
            assert_eq!(targets.at(None, position), PressTarget::Outside);
        });
        output.drop_without_applying_deltas();
    }
}
