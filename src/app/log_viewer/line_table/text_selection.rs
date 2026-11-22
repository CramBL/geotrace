use std::mem;
use std::ops::Range;

use egui::epaint::TextShape;
use egui::epaint::text::FontsView;
use egui::layers::ShapeIdx;
use egui::os::OperatingSystem;
use egui::plugin::Plugin;
use egui::text::{CCursor, CCursorRange, CharIndex, LayoutJob};
use egui::text_selection::{LabelSelectionState, text_cursor_state, visuals};
use egui::{
    CursorIcon, Event, Key, Modifiers, OutputCommand, PointerButton, Pos2, Role, Sense, Shape,
    WidgetInfo,
};

use super::LineTableRows;
use super::row_cells::CellIndex;
use super::row_grid::ShownRows;
use super::row_layout::{CellLayout, RowLayout};
use super::selection_document::{Caret, DocumentRow};

use controller::{
    AnchorMode, GestureLimits, HorizontalPosition, PointerCompletion, PointerIntent,
    SelectionTarget, SelectionUnit,
};
use keyboard::{Direction, SelectionMotion, VerticalMotion, VerticalResult};

pub(in crate::app::log_viewer) use controller::TableTextSelection;

mod controller;
mod keyboard;

#[derive(Clone, Copy)]
struct TextHit {
    caret: Caret,
    glyph: Option<CharIndex>,
}

impl From<Caret> for TextHit {
    fn from(caret: Caret) -> Self {
        Self { caret, glyph: None }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CaretRange {
    anchor: Caret,
    endpoint: Caret,
}

impl CaretRange {
    pub(super) fn point(caret: Caret) -> Self {
        Self {
            anchor: caret,
            endpoint: caret,
        }
    }

    pub(super) fn with_endpoint(self, endpoint: Caret) -> Self {
        if self.anchor.revision() == endpoint.revision() {
            Self { endpoint, ..self }
        } else {
            self
        }
    }

    pub(super) fn anchor(self) -> Caret {
        self.anchor
    }

    pub(super) fn endpoint(self) -> Caret {
        self.endpoint
    }

    pub(super) fn ordered(self) -> Range<Caret> {
        self.anchor.min(self.endpoint)..self.anchor.max(self.endpoint)
    }

    pub(super) fn cell_characters(
        self,
        row: usize,
        cell: CellIndex,
        end: CharIndex,
    ) -> Option<Range<CharIndex>> {
        let range = self.ordered();
        let key = (row, cell);
        let first = (range.start.row(), range.start.cell());
        let last = (range.end.row(), range.end.cell());
        if range.is_empty() || key < first || key > last {
            return None;
        }
        let start = if key == first {
            range.start.character().min(end)
        } else {
            CharIndex::ZERO
        };
        let end = if key == last {
            range.end.character().min(end)
        } else {
            end
        };
        Some(start..end)
    }
}

impl CellLayout {
    fn caret_x(&self, character: CharIndex) -> f32 {
        let cursor = CCursor::new(character.min(self.galley.end().index));
        self.origin.x + self.galley.pos_from_cursor(cursor).left()
    }

    fn position(&self, character: CharIndex) -> Caret {
        self.carets().at(character)
    }

    fn hit_at(&self, x: f32) -> TextHit {
        let position = egui::vec2(x - self.origin.x, self.galley.rect.center().y);
        let cursor = self.galley.cursor_from_pos(position);
        let caret = if self.galley.elided && cursor.index == self.galley.end().index {
            self.carets().text_end()
        } else {
            cursor.index
        };
        let layout = self.galley.layout_from_cursor(cursor);
        let glyph = self.galley.rows.get(layout.row).and_then(|row| {
            let x = position.x - row.pos.x;
            let next = layout.column.0;
            [next.checked_sub(1), Some(next)]
                .into_iter()
                .flatten()
                .find(|&column| {
                    row.glyphs
                        .get(column)
                        .is_some_and(|glyph| glyph.pos.x <= x && x < glyph.max_x())
                })
                .map(|column| (cursor - (next - column)).index)
        });
        TextHit {
            caret: self.position(caret),
            glyph,
        }
    }

    fn word_at(&self, hit: TextHit) -> CaretRange {
        if let Some(glyph) = hit.glyph
            && self.galley.elided
            && glyph + 1 == self.galley.end().index
        {
            return CaretRange::point(self.position(glyph))
                .with_endpoint(self.position(self.carets().text_end()));
        }
        let text = self.galley.text();
        let start = hit.glyph.unwrap_or(hit.caret.character());
        let end = hit.glyph.map_or(start, |glyph| glyph + 1);
        let anchor = text_cursor_state::ccursor_previous_word(text, CCursor::new(end)).index;
        let endpoint = text_cursor_state::ccursor_next_word(text, CCursor::new(start)).index;
        CaretRange::point(self.position(anchor)).with_endpoint(self.position(endpoint))
    }

    fn paint(self, ui: &egui::Ui, shape: ShapeIdx, selection: Option<CaretRange>) {
        let displayed_end = self.galley.end().index;
        let text_end = if self.galley.elided && selection.is_some() {
            self.carets().text_end()
        } else {
            displayed_end
        };
        let start = self.carets().start();
        let mut galley = self.galley;
        if let Some(characters) = selection
            .and_then(|selection| selection.cell_characters(start.row(), start.cell(), text_end))
            .filter(|characters| !characters.is_empty())
        {
            let start = if galley.elided && characters.end > displayed_end {
                characters.start.min(displayed_end.saturating_sub(1))
            } else {
                characters.start
            };
            let range = CCursorRange::two(
                CCursor::new(start),
                CCursor::new(characters.end.min(displayed_end)),
            );
            visuals::paint_text_selection(&mut galley, ui.visuals(), &range, None);
        }
        ui.painter().set(
            shape,
            TextShape::new(self.origin, galley, ui.visuals().text_color()),
        );
    }
}

struct VisibleTextRow {
    layout: RowLayout,
    shapes: Vec<ShapeIdx>,
}

pub(super) struct TableTextUi<'grid> {
    drag_target: egui::Response,
    scrollbar: Option<egui::Rect>,
    document: &'grid LineTableRows,
    shown: &'grid ShownRows,
    rows: Vec<VisibleTextRow>,
}

#[derive(Clone, Copy)]
pub(super) struct TableInputFocus<'a> {
    pub(super) target: &'a egui::Response,
    pub(super) available: bool,
    pub(super) after_navigation: bool,
    pub(super) entered_by_navigation: bool,
}

#[derive(Default)]
struct SelectionOutcome {
    clear_label_selection: bool,
    selected_on_release: bool,
    keyboard_endpoint: Option<Caret>,
    request_focus: bool,
    commands: SelectionCommands,
}

#[derive(Clone, Copy)]
struct PointerPress {
    position: Pos2,
    button: PointerButton,
    modifiers: Modifiers,
    time: f64,
    released_in_frame: bool,
    available: bool,
}

#[derive(Default)]
struct SelectionCommands {
    consumed: Vec<usize>,
    copies: Vec<(usize, CaretRange)>,
    last_label_copy: Option<usize>,
}

impl SelectionCommands {
    fn emit(
        self,
        ui: &egui::Ui,
        document: &LineTableRows,
        clear_label_selection: bool,
        primary: Option<OutputCommand>,
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
        let mut after_labels = Vec::new();
        for (index, range) in self.copies {
            if let Some(text) = document
                .clipboard_text(range)
                .filter(|text| !text.is_empty())
            {
                if self
                    .last_label_copy
                    .is_some_and(|label_copy| index > label_copy)
                {
                    after_labels.push(OutputCommand::CopyText(text));
                } else {
                    ui.copy_text(text);
                }
            }
        }
        if self.last_label_copy.is_some() && (clear_label_selection || !after_labels.is_empty()) {
            after_labels.extend(primary);
            let output = ui.ctx().plugin_or_default::<LabelSelectionHandoff>();
            let mut output = output.lock();
            output.clear_label_selection |= clear_label_selection;
            output.commands.append(&mut after_labels);
        } else {
            if clear_label_selection {
                ui.ctx()
                    .plugin::<LabelSelectionState>()
                    .lock()
                    .clear_selection();
                ui.ctx().request_repaint();
            }
            if let Some(command) = primary {
                ui.ctx().send_cmd(command);
            }
        }
    }
}

#[derive(Default)]
struct LabelSelectionHandoff {
    clear_label_selection: bool,
    commands: Vec<OutputCommand>,
}

impl Plugin for LabelSelectionHandoff {
    fn debug_name(&self) -> &'static str {
        "Log table selection handoff"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        // egui publishes label selections before it runs output hooks.
        if mem::take(&mut self.clear_label_selection) {
            ctx.plugin::<LabelSelectionState>().lock().clear_selection();
            ctx.request_repaint();
        }
        output.platform_output.commands.append(&mut self.commands);
    }
}

impl SelectionMotion {
    fn of(key: Key, modifiers: Modifiers) -> Option<Self> {
        if !modifiers.shift {
            return None;
        }
        Some(match key {
            Key::ArrowLeft | Key::ArrowRight => {
                let direction = if key == Key::ArrowLeft {
                    Direction::Backward
                } else {
                    Direction::Forward
                };
                if modifiers.alt || modifiers.ctrl {
                    Self::Word(direction)
                } else if modifiers.mac_cmd {
                    Self::RowBoundary(direction)
                } else {
                    Self::Character(direction)
                }
            }
            Key::ArrowUp if modifiers.command => Self::DocumentBoundary(Direction::Backward),
            Key::ArrowDown if modifiers.command => Self::DocumentBoundary(Direction::Forward),
            Key::ArrowUp => Self::Row(Direction::Backward),
            Key::ArrowDown => Self::Row(Direction::Forward),
            Key::Home if modifiers.ctrl => Self::DocumentBoundary(Direction::Backward),
            Key::End if modifiers.ctrl => Self::DocumentBoundary(Direction::Forward),
            Key::Home => Self::RowBoundary(Direction::Backward),
            Key::End => Self::RowBoundary(Direction::Forward),
            _ => return None,
        })
    }
}

impl<'grid> TableTextUi<'grid> {
    pub(super) fn new(
        ui: &egui::Ui,
        shown: &'grid ShownRows,
        document: &'grid LineTableRows,
        scrollbar: Option<egui::Rect>,
    ) -> Self {
        let viewport = ui.clip_rect().intersect(egui::Rect::from_x_y_ranges(
            ui.max_rect().x_range(),
            ui.clip_rect().y_range(),
        ));
        let sense = Sense::click_and_drag() - Sense::FOCUSABLE;
        let drag_target = ui.interact(viewport, ui.make_persistent_id("log_text_drag"), sense);
        Self {
            drag_target,
            scrollbar,
            document,
            shown,
            rows: Vec::new(),
        }
    }

    pub(super) fn row_ui(
        &mut self,
        ui: &egui::Ui,
        layout: RowLayout,
        mut cell_hover: impl FnMut(CellIndex, egui::Response),
    ) {
        if layout.revision() != self.document.document_revision()
            || !self.shown.rows.contains(&layout.row())
            || self
                .rows
                .last()
                .is_some_and(|row| row.layout.row() >= layout.row())
        {
            return;
        }
        let shapes = if ui.is_visible() {
            let sense = if ui.memory(|memory| memory.options.screen_reader) {
                Sense::focusable_noninteractive()
            } else {
                Sense::hover()
            };
            layout
                .cells()
                .iter()
                .map(|cell| {
                    let caret = cell.carets().start();
                    let id = ui.make_persistent_id(("log_text_cell", caret.row(), caret.cell().0));
                    let mut response = ui.interact(cell.bounds, id, sense);
                    let galley = &cell.galley;
                    response.widget_info(|| {
                        WidgetInfo::labeled(Role::Label, ui.is_enabled(), galley.text())
                    });
                    if galley.elided && ui.is_rect_visible(response.rect) {
                        response = response.on_hover_text(LayoutJob {
                            text: galley.job.text.clone(),
                            sections: galley.job.sections.clone(),
                            ..LayoutJob::default()
                        });
                    }
                    cell_hover(caret.cell(), response);
                    ui.painter().add(Shape::Noop)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.rows.push(VisibleTextRow { layout, shapes });
    }

    fn text_hit(&self, pointer: Pos2) -> Option<TextHit> {
        let viewport = self.drag_target.rect;
        let top = self.shown.bounds.top();
        if self.shown.rows.start == 0 && viewport.top() <= top && pointer.y < top {
            let first = self.rows.first()?.layout.cells().first()?;
            return Some(first.position(CharIndex::ZERO).into());
        }
        let bottom = self.shown.row_top(self.shown.rows.end);
        if self.shown.rows.end == self.document.len()
            && bottom <= viewport.bottom()
            && pointer.y >= bottom
        {
            let field = self.rows.last()?.layout.cells().last()?;
            return Some(field.position(field.carets().text_end()).into());
        }
        let x = pointer.x.clamp(viewport.left(), viewport.right());
        let row = self.shown.hit_row(viewport.y_range(), pointer.y)?;
        Some(self.field_at_x(self.placed_in_row(row), x)?.hit_at(x))
    }

    fn field_at_x<'a>(&self, fields: &'a [CellLayout], x: f32) -> Option<&'a CellLayout> {
        let viewport = self.drag_target.rect;
        fields
            .iter()
            .filter(|field| {
                let bounds = field.galley.rect.translate(field.origin.to_vec2());
                bounds.right() > viewport.left() && bounds.left() < viewport.right()
            })
            .min_by(|a, b| {
                let distance = |field: &CellLayout| {
                    let bounds = field.galley.rect.translate(field.origin.to_vec2());
                    (x - x.clamp(bounds.left(), bounds.right())).abs()
                };
                distance(a).total_cmp(&distance(b))
            })
    }

    fn placed_in_row(&self, row: usize) -> &[CellLayout] {
        self.rows
            .binary_search_by_key(&row, |entry| entry.layout.row())
            .ok()
            .and_then(|index| self.rows.get(index))
            .map_or(&[], |row| row.layout.cells())
    }

    fn with_row_geometry<R>(
        &self,
        row: usize,
        fonts: &mut FontsView<'_>,
        layout_row: &impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
        read: impl FnOnce(&[CellLayout]) -> R,
    ) -> R {
        let placed = self.placed_in_row(row);
        if !placed.is_empty() {
            return read(placed);
        }
        let layout = self
            .document
            .document_row(row)
            .map(|row| layout_row(fonts, &row))
            .filter(|layout| {
                layout.revision() == self.document.document_revision() && layout.row() == row
            });
        read(layout.as_ref().map_or(&[], |layout| layout.cells()))
    }

    fn vertical_endpoint(
        &self,
        motion: VerticalMotion,
        fonts: &mut FontsView<'_>,
        layout_row: &impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
    ) -> Option<VerticalResult> {
        let left = self.shown.bounds.left();
        let offset = if let Some(preferred) = motion.preferred_x {
            preferred
        } else {
            let previous = motion.previous;
            let x = self.with_row_geometry(previous.row(), fonts, layout_row, |fields| {
                let field = fields
                    .iter()
                    .find(|field| field.carets().start().cell() == previous.cell())?;
                Some(field.caret_x(previous.character()))
            })?;
            HorizontalPosition(x - left)
        };
        let viewport = self.drag_target.rect;
        let x = (left + offset.0).clamp(viewport.left(), viewport.right());
        let caret = self.with_row_geometry(motion.row, fonts, layout_row, |fields| {
            Some(self.field_at_x(fields, x)?.hit_at(x).caret)
        });
        Some(VerticalResult {
            caret,
            preferred_x: offset,
        })
    }

    fn selection_unit(&self, unit: SelectionUnit, hit: TextHit) -> CaretRange {
        let position = hit.caret;
        let fields = self.placed_in_row(position.row());
        match unit {
            SelectionUnit::Character => {}
            SelectionUnit::Line => {
                if let (Some(first), Some(last)) = (fields.first(), fields.last()) {
                    return CaretRange::point(first.carets().start())
                        .with_endpoint(last.position(last.carets().text_end()));
                }
            }
            SelectionUnit::Word => {
                if let Some(field) = fields
                    .iter()
                    .find(|field| field.carets().start().cell() == position.cell())
                {
                    return field.word_at(hit);
                }
            }
        }
        CaretRange::point(position)
    }

    fn update_selection(
        &self,
        ui: &egui::Ui,
        selection: &mut TableTextSelection,
        focus: TableInputFocus<'_>,
        layout_row: &impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
    ) -> SelectionOutcome {
        let available = focus.available;
        let (events, position, time, primary_down) = ui.input(|input| {
            let events: Vec<_> = input
                .events
                .iter()
                .enumerate()
                .filter(|(_, event)| match event {
                    Event::Key {
                        key: Key::Tab,
                        pressed: true,
                        modifiers,
                        ..
                    } => !modifiers.any() || modifiers.shift_only(),
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => *key == Key::A || SelectionMotion::of(*key, *modifiers).is_some(),
                    event => matches!(
                        event,
                        Event::PointerMoved(_)
                            | Event::PointerButton { .. }
                            | Event::Copy
                            | Event::Cut
                    ),
                })
                .map(|(index, event)| (index, event.clone()))
                .collect();
            (
                events,
                input.pointer.interact_pos(),
                input.time,
                input.pointer.primary_down(),
            )
        });
        let limits = ui.ctx().options(|options| {
            let input = options.input_options;
            GestureLimits {
                distance: input.max_click_dist,
                duration: input.max_click_duration,
                multi_click_delay: input.max_double_click_delay,
            }
        });
        let last_release = events.iter().rposition(|(_, event)| {
            matches!(
                event,
                Event::PointerButton {
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                }
            )
        });
        let label_has_selection = ui
            .ctx()
            .plugin::<LabelSelectionState>()
            .lock()
            .has_selection();
        let tab_pressed = events
            .iter()
            .any(|(_, event)| matches!(event, Event::Key { key: Key::Tab, .. }));
        let pointer_pressed = events
            .iter()
            .any(|(_, event)| matches!(event, Event::PointerButton { pressed: true, .. }));
        let previously_focused = ui.memory(|memory| memory.had_focus_last_frame(focus.target.id));
        let entry_from_previous_frame = available && focus.entered_by_navigation && !tab_pressed;
        let table_had_focus = if tab_pressed || pointer_pressed {
            previously_focused
        } else {
            focus.after_navigation
        };
        // A label can already have processed a later press in this frame.
        let mut keyboard = entry_from_previous_frame
            || (available
                && table_had_focus
                && (selection.range().is_some() || !label_has_selection));
        let mut outcome = SelectionOutcome {
            clear_label_selection: entry_from_previous_frame,
            ..SelectionOutcome::default()
        };
        for (index, (input_index, event)) in events.into_iter().enumerate() {
            match event {
                Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                } => {
                    let press = PointerPress {
                        position: pos,
                        button,
                        modifiers,
                        time,
                        released_in_frame: last_release.is_some_and(|release| index < release),
                        available,
                    };
                    keyboard = self
                        .press_selection(ui, selection, &mut outcome, press, limits)
                        .unwrap_or(keyboard);
                }
                Event::PointerMoved(pos) if available => {
                    let _ = self.apply_pointer(selection, PointerIntent::Drag(pos), limits);
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                } if available => {
                    let intent = PointerIntent::Release {
                        position: pos,
                        time,
                    };
                    let completion = self.apply_pointer(selection, intent, limits);
                    outcome.selected_on_release = completion == Some(PointerCompletion::Selection);
                    outcome.request_focus |= completion.is_some();
                    keyboard |= completion.is_some();
                }
                Event::Key {
                    key: Key::Tab,
                    modifiers,
                    ..
                } => {
                    // egui applies backwards focus traversal in the next pass.
                    let leaves_backwards = modifiers.shift_only() && previously_focused;
                    let focused = available && focus.after_navigation && !leaves_backwards;
                    outcome.clear_label_selection |= focused && focus.entered_by_navigation;
                    outcome.request_focus &= focused;
                    keyboard = focused && (outcome.clear_label_selection || !label_has_selection);
                }
                Event::Key {
                    key: Key::A,
                    modifiers,
                    ..
                } if keyboard && modifiers.matches_logically(Modifiers::COMMAND) => {
                    selection.select_all(self.document);
                    outcome.commands.consumed.push(input_index);
                }
                Event::Key { key, modifiers, .. } if keyboard => {
                    if let Some(motion) = SelectionMotion::of(key, modifiers)
                        && let Some(endpoint) =
                            selection.move_endpoint(motion, self.document, |motion| {
                                ui.fonts_mut(|fonts| {
                                    self.vertical_endpoint(motion, fonts, layout_row)
                                })
                            })
                    {
                        outcome.keyboard_endpoint = Some(endpoint.caret);
                        outcome.commands.consumed.push(input_index);
                    }
                }
                Event::Copy | Event::Cut if keyboard => {
                    let range = selection.copy_range();
                    if range.is_some() || outcome.clear_label_selection {
                        outcome.commands.consumed.push(input_index);
                    }
                    outcome
                        .commands
                        .copies
                        .extend(range.map(|range| (input_index, range)));
                }
                Event::Copy | Event::Cut
                    if label_has_selection && !outcome.clear_label_selection =>
                {
                    outcome.commands.last_label_copy = Some(input_index);
                }
                _ => {}
            }
        }
        if available && primary_down {
            if let Some(position) = position
                && selection.dragging()
            {
                let _ = self.apply_pointer(selection, PointerIntent::Drag(position), limits);
                self.scroll_during_drag(ui, position.y);
            }
        } else {
            selection.stop_drag();
        }
        outcome
    }

    fn press_selection(
        &self,
        ui: &egui::Ui,
        selection: &mut TableTextSelection,
        outcome: &mut SelectionOutcome,
        press: PointerPress,
        limits: GestureLimits,
    ) -> Option<bool> {
        outcome.keyboard_endpoint = None;
        let on_layer = ui.ctx().layer_id_at(press.position) == Some(self.drag_target.layer_id);
        if on_layer
            && self
                .scrollbar
                .is_some_and(|rect| rect.contains(press.position))
        {
            return None;
        }
        let inside = on_layer && self.drag_target.rect.contains(press.position);
        let primary = press.button == PointerButton::Primary;
        if primary {
            outcome.selected_on_release = false;
        } else if inside {
            return None;
        }
        let owns_press = press.available
            && inside
            && (self.drag_target.is_pointer_button_down_on() || press.released_in_frame);
        outcome.clear_label_selection = owns_press;
        outcome.request_focus = owns_press;
        if owns_press {
            let anchor = if press.modifiers.shift {
                AnchorMode::Extend
            } else {
                AnchorMode::Replace
            };
            let intent = PointerIntent::Press {
                position: press.position,
                time: press.time,
                anchor,
            };
            let _ = self.apply_pointer(selection, intent, limits);
        } else {
            selection.clear();
        }
        Some(owns_press)
    }

    fn apply_pointer(
        &self,
        selection: &mut TableTextSelection,
        intent: PointerIntent,
        limits: GestureLimits,
    ) -> Option<PointerCompletion> {
        selection.apply_pointer(intent, limits, |unit, position| {
            let hit = self.text_hit(position)?;
            Some(SelectionTarget {
                caret: hit.caret,
                range: self.selection_unit(unit, hit),
            })
        })
    }

    fn settled_primary(
        &self,
        ui: &egui::Ui,
        selection: &mut TableTextSelection,
    ) -> Option<OutputCommand> {
        if ui.ctx().os() != OperatingSystem::Nix || ui.input(|input| input.pointer.any_down()) {
            return None;
        }
        let text = self.document.clipboard_text(selection.settle()?)?;
        (!text.is_empty()).then_some(OutputCommand::TextSelectionSettled(text))
    }

    fn scroll_during_drag(&self, ui: &egui::Ui, pointer_y: f32) {
        let rect = self.drag_target.rect;
        let overshoot = pointer_y.clamp(rect.top(), rect.bottom()) - pointer_y;
        let more_above = self.shown.rows.start > 0 || self.shown.bounds.top() < rect.top();
        let more_below = self.shown.rows.end < self.document.len()
            || self.shown.row_top(self.shown.rows.end) > rect.bottom();
        if (overshoot > 0.0 && more_above) || (overshoot < 0.0 && more_below) {
            let speed =
                (overshoot * SCROLL_SPEED_PER_POINT).clamp(-MAX_SCROLL_SPEED, MAX_SCROLL_SPEED);
            let delta = ui.input(|input| input.stable_dt) * speed;
            ui.scroll_with_delta(egui::vec2(0.0, delta));
        }
    }

    pub(super) fn finish(
        self,
        ui: &egui::Ui,
        selection: &mut TableTextSelection,
        focus: TableInputFocus<'_>,
        layout_row: impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
    ) -> bool {
        let available = focus.available && self.drag_target.rect.is_positive();
        let focus = TableInputFocus { available, ..focus };
        selection.replace_document(Some(self.document.document_revision()));
        let outcome = self.update_selection(ui, selection, focus, &layout_row);
        if outcome.request_focus {
            focus.target.request_focus();
        }
        if let Some(endpoint) = outcome.keyboard_endpoint {
            let viewport = self.drag_target.rect;
            let bounds = self.shown.row_bounds(endpoint.row());
            if bounds.top() < viewport.top() || bounds.bottom() > viewport.bottom() {
                ui.scroll_to_rect(
                    egui::Rect::from_x_y_ranges(
                        viewport.left()..=viewport.left(),
                        bounds.y_range(),
                    ),
                    None,
                );
            }
        }
        let primary = self.settled_primary(ui, selection);
        outcome
            .commands
            .emit(ui, self.document, outcome.clear_label_selection, primary);
        let range = selection.range();
        if available
            && (selection.dragging()
                || (ui.rect_contains_pointer(self.drag_target.rect)
                    && !self
                        .scrollbar
                        .is_some_and(|rect| ui.rect_contains_pointer(rect))))
        {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        for row in self.rows {
            for (cell, shape) in row.layout.into_cells().into_iter().zip(row.shapes) {
                cell.paint(ui, shape, range);
            }
        }
        outcome.selected_on_release
    }
}

const SCROLL_SPEED_PER_POINT: f32 = 12.0;
const MAX_SCROLL_SPEED: f32 = 900.0;

#[cfg(test)]
mod tests {
    use super::super::LineTableRows;
    use super::super::row_grid::RowGrid;
    use super::super::row_layout::{RowLayoutRequest, RowLayoutStyle};
    use super::super::tests as line_table_tests;
    use super::{DocumentRow, FontsView, ShownRows, TableTextUi};

    #[test]
    fn table_selection_rejects_geometry_for_another_document_or_row() {
        let log = line_table_tests::source_log("2026-01-01 14:02:11 hello\n");
        let rows = LineTableRows::of(&log);
        let foreign_rows = LineTableRows::of(&log);
        let context = egui::Context::default();
        let output = context.run_ui(Default::default(), |ui| {
            let shown = ShownRows {
                grid: RowGrid { height: 20.0 },
                rows: 0..1,
                bounds: ui.max_rect(),
            };
            let style = RowLayoutStyle {
                style: ui.style(),
                line_number_width: 24.0,
                gutter_width: 11.0,
            };
            let layout = |fonts: &mut FontsView<'_>, row: &DocumentRow<'_>| {
                style.layout(
                    fonts,
                    &RowLayoutRequest {
                        row,
                        bounds: shown.row_bounds(row.index()),
                        find_highlight: None,
                        entry_text: None,
                    },
                )
            };
            let mut table = TableTextUi::new(ui, &shown, &rows, None);
            let foreign = foreign_rows.document_row(0).unwrap();
            table.row_ui(ui, ui.fonts_mut(|fonts| layout(fonts, &foreign)), |_, _| {});
            assert!(table.rows.is_empty());

            let other_row = rows.document_row(0).unwrap();
            let requested_row = rows.document_row(1).unwrap();
            let cells = ui.fonts_mut(|fonts| {
                table.with_row_geometry(
                    requested_row.index(),
                    fonts,
                    &|fonts, _| layout(fonts, &other_row),
                    |cells| cells.len(),
                )
            });
            assert_eq!(cells, 0);
        });
        output.drop_without_applying_deltas();
    }
}
