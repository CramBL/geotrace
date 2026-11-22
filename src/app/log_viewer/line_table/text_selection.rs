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
    CursorIcon, Event, InputOptions, Key, Modifiers, OutputCommand, PointerButton, Pos2, Role,
    Sense, Shape, WidgetInfo,
};

use super::LineTableRows;
use super::row_cells::CellIndex;
use super::row_grid::ShownRows;
use super::row_layout::{CellLayout, RowLayout};
use super::selection_document::{Caret, DocumentRevision, DocumentRow};

use keyboard::{SelectionMotion, VerticalMotion};

mod keyboard;

#[derive(Clone, Copy)]
struct HorizontalPosition(f32);

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

#[derive(Clone, Copy, Eq, PartialEq)]
enum SelectionUnit {
    Character,
    Line,
    Word,
}

/// egui exposes the click count on release, after a selection gesture can begin.
struct CompletedClick {
    position: Pos2,
    time: f64,
    unit: SelectionUnit,
}

impl CompletedClick {
    fn next_unit(&self, position: Pos2, time: f64, options: InputOptions) -> SelectionUnit {
        if time - self.time >= options.max_double_click_delay
            || position.distance(self.position) >= options.max_click_dist
        {
            SelectionUnit::Character
        } else if self.unit == SelectionUnit::Character {
            SelectionUnit::Word
        } else {
            SelectionUnit::Line
        }
    }
}

struct SelectionGesture {
    unit: SelectionUnit,
    origin: Pos2,
    start_time: f64,
    extending: bool,
    within_click_distance: bool,
}

struct SelectedText {
    anchor: CaretRange,
    range: CaretRange,
    gesture: Option<SelectionGesture>,
    preferred_x: Option<HorizontalPosition>,
}

impl SelectedText {
    fn extend(&mut self, pointer: Caret, unit_range: CaretRange) {
        let anchor = self.anchor.ordered();
        let target = unit_range.ordered();
        self.range =
            if pointer < anchor.start || (pointer == anchor.start && target.start < anchor.start) {
                CaretRange::point(anchor.end).with_endpoint(target.start)
            } else if pointer > anchor.end
                || anchor.is_empty()
                || (pointer == anchor.end && target.end > anchor.end)
            {
                CaretRange::point(anchor.start).with_endpoint(target.end)
            } else {
                self.anchor
            };
    }
}

#[derive(Default)]
pub(in crate::app::log_viewer) struct TableTextSelection {
    document: Option<DocumentRevision>,
    selected: Option<SelectedText>,
    last_click: Option<CompletedClick>,
    primary_reported: Option<Range<Caret>>,
}

impl TableTextSelection {
    pub(in crate::app::log_viewer) fn replace_document(
        &mut self,
        document: Option<DocumentRevision>,
    ) {
        if self.document != document {
            *self = Self {
                document,
                ..Self::default()
            };
        }
    }

    fn settled_primary(
        &mut self,
        ui: &egui::Ui,
        document: &LineTableRows,
    ) -> Option<OutputCommand> {
        let selection = self.nonempty_range();
        let range = selection.map(CaretRange::ordered);
        let settle = ui.ctx().os() == OperatingSystem::Nix
            && !ui.input(|input| input.pointer.any_down())
            && self.primary_reported != range;
        if settle {
            self.primary_reported = range;
            if let Some(selection) = selection {
                let text = document.clipboard_text(selection)?;
                return (!text.is_empty()).then_some(OutputCommand::TextSelectionSettled(text));
            }
        }
        None
    }

    fn select_all(&mut self, document: &LineTableRows) {
        if let Some(range) = document.full_range() {
            self.selected = Some(SelectedText {
                anchor: CaretRange::point(range.anchor()),
                range,
                gesture: None,
                preferred_x: None,
            });
        }
    }

    fn nonempty_range(&self) -> Option<CaretRange> {
        self.selected
            .as_ref()
            .map(|text| text.range)
            .filter(|range| !range.ordered().is_empty())
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

#[derive(PartialEq, Eq)]
enum PointerCompletion {
    Caret,
    Selection,
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
        motion: &mut VerticalMotion<'_>,
        fonts: &mut FontsView<'_>,
        layout_row: &impl Fn(&mut FontsView<'_>, &DocumentRow<'_>) -> RowLayout,
    ) -> Option<Caret> {
        let left = self.shown.bounds.left();
        let offset = if let Some(preferred) = *motion.preferred_x {
            preferred
        } else {
            let previous = motion.previous;
            let x = self.with_row_geometry(previous.row(), fonts, layout_row, |fields| {
                let field = fields
                    .iter()
                    .find(|field| field.carets().start().cell() == previous.cell())?;
                Some(field.caret_x(previous.character()))
            })?;
            let preferred = HorizontalPosition(x - left);
            *motion.preferred_x = Some(preferred);
            preferred
        };
        let viewport = self.drag_target.rect;
        let x = (left + offset.0).clamp(viewport.left(), viewport.right());
        self.with_row_geometry(motion.row, fonts, layout_row, |fields| {
            Some(self.field_at_x(fields, x)?.hit_at(x).caret)
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
        let options = ui.ctx().options(|options| options.input_options);
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
                && (selection.selected.is_some() || !label_has_selection));
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
                        .press_selection(ui, selection, &mut outcome, press, options)
                        .unwrap_or(keyboard);
                }
                Event::PointerMoved(pos) if available => {
                    self.extend_selection(selection, pos, options.max_click_dist);
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                } if available => {
                    self.extend_selection(selection, pos, options.max_click_dist);
                    let completion = Self::complete_selection(selection, pos, time, options);
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
                        outcome.keyboard_endpoint = Some(endpoint);
                        outcome.commands.consumed.push(input_index);
                    }
                }
                Event::Copy | Event::Cut if keyboard => {
                    let range = selection.nonempty_range();
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
                && selection
                    .selected
                    .as_ref()
                    .is_some_and(|text| text.gesture.is_some())
            {
                self.extend_selection(selection, position, options.max_click_dist);
                self.scroll_during_drag(ui, position.y);
            }
        } else if let Some(text) = &mut selection.selected {
            text.gesture = None;
        }
        outcome
    }

    fn press_selection(
        &self,
        ui: &egui::Ui,
        selection: &mut TableTextSelection,
        outcome: &mut SelectionOutcome,
        press: PointerPress,
        options: InputOptions,
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
            self.begin_selection(
                selection,
                press.position,
                press.modifiers,
                press.time,
                options,
            );
        } else {
            selection.selected = None;
            selection.last_click = None;
        }
        Some(owns_press)
    }

    fn begin_selection(
        &self,
        selection: &mut TableTextSelection,
        press: Pos2,
        modifiers: Modifiers,
        time: f64,
        options: InputOptions,
    ) {
        let Some(hit) = self.text_hit(press) else {
            return;
        };
        let unit = selection
            .last_click
            .as_ref()
            .map_or(SelectionUnit::Character, |click| {
                click.next_unit(press, time, options)
            });
        let caret = hit.caret;
        let unit_range = self.selection_unit(unit, hit);
        let existing = selection.selected.take().filter(|_| modifiers.shift);
        let mut text = existing.unwrap_or(SelectedText {
            anchor: unit_range,
            range: unit_range,
            gesture: None,
            preferred_x: None,
        });
        text.preferred_x = None;
        text.gesture = Some(SelectionGesture {
            unit,
            origin: press,
            start_time: time,
            extending: modifiers.shift,
            within_click_distance: true,
        });
        text.extend(caret, unit_range);
        selection.selected = Some(text);
    }

    fn extend_selection(
        &self,
        selection: &mut TableTextSelection,
        position: Pos2,
        max_click_distance: f32,
    ) {
        let Some(text) = &mut selection.selected else {
            return;
        };
        let Some(gesture) = &mut text.gesture else {
            return;
        };
        gesture.within_click_distance &= gesture.origin.distance(position) <= max_click_distance;
        if let Some(hit) = self.text_hit(position) {
            let caret = hit.caret;
            let unit_range = self.selection_unit(gesture.unit, hit);
            text.extend(caret, unit_range);
        }
    }

    fn complete_selection(
        selection: &mut TableTextSelection,
        position: Pos2,
        time: f64,
        options: InputOptions,
    ) -> Option<PointerCompletion> {
        let text = selection.selected.as_mut()?;
        let gesture = text.gesture.take()?;
        let selecting_characters =
            gesture.unit == SelectionUnit::Character && !text.range.ordered().is_empty();
        selection.last_click = (gesture.within_click_distance
            && time - gesture.start_time <= options.max_click_duration
            && !selecting_characters)
            .then_some(CompletedClick {
                position,
                time,
                unit: gesture.unit,
            });
        Some(
            if selecting_characters || gesture.unit != SelectionUnit::Character || gesture.extending
            {
                PointerCompletion::Selection
            } else {
                PointerCompletion::Caret
            },
        )
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
        let primary = selection.settled_primary(ui, self.document);
        outcome
            .commands
            .emit(ui, self.document, outcome.clear_label_selection, primary);
        let text = selection.selected.as_ref();
        if available
            && (text.is_some_and(|text| text.gesture.is_some())
                || (ui.rect_contains_pointer(self.drag_target.rect)
                    && !self
                        .scrollbar
                        .is_some_and(|rect| ui.rect_contains_pointer(rect))))
        {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        for row in self.rows {
            for (cell, shape) in row.layout.into_cells().into_iter().zip(row.shapes) {
                cell.paint(ui, shape, text.map(|text| text.range));
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
    use super::{
        DocumentRevision, DocumentRow, FontsView, ShownRows, TableTextSelection, TableTextUi,
    };

    #[test]
    fn selection_preserves_the_range_only_within_its_document() {
        let log = line_table_tests::source_log("2026-01-01 14:02:11 hello\n");
        let rows = LineTableRows::of(&log);
        let mut selection = TableTextSelection::default();
        selection.replace_document(Some(rows.document_revision()));
        selection.select_all(&rows);

        selection.replace_document(Some(rows.document_revision()));
        assert!(selection.nonempty_range().is_some());

        selection.replace_document(Some(DocumentRevision::next()));
        assert!(selection.nonempty_range().is_none());
    }

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
