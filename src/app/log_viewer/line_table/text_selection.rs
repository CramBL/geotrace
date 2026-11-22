use std::ops::Range;

use egui::epaint::TextShape;
use egui::epaint::text::FontsView;
use egui::layers::ShapeIdx;
use egui::text::{CCursor, CCursorRange, CharIndex, LayoutJob};
use egui::text_selection::{text_cursor_state, visuals};
use egui::{Pos2, Role, Sense, Shape, WidgetInfo};

use super::LineTableRows;
use super::row_cells::CellIndex;
use super::row_grid::ShownRows;
use super::row_layout::{CellLayout, RowLayout};
use super::selection_document::{Caret, DocumentRow};

use controller::{
    GestureLimits, HorizontalPosition, PointerCompletion, PointerIntent, SelectionTarget,
    SelectionUnit,
};
use keyboard::{VerticalMotion, VerticalResult};

pub(in crate::app::log_viewer) use controller::TableTextSelection;

mod controller;
pub(super) mod input;
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
    viewport: egui::Rect,
    document: &'grid LineTableRows,
    shown: &'grid ShownRows,
    rows: Vec<VisibleTextRow>,
}

impl<'grid> TableTextUi<'grid> {
    pub(super) fn new(
        viewport: egui::Rect,
        shown: &'grid ShownRows,
        document: &'grid LineTableRows,
    ) -> Self {
        Self {
            viewport,
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
        let viewport = self.viewport;
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
        let viewport = self.viewport;
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
        let viewport = self.viewport;
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

    fn scroll_during_drag_ui(&self, ui: &egui::Ui, pointer_y: f32) {
        let rect = self.viewport;
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

    fn reveal_endpoint_ui(&self, ui: &egui::Ui, endpoint: Caret) {
        let bounds = self.shown.row_bounds(endpoint.row());
        if bounds.top() < self.viewport.top() || bounds.bottom() > self.viewport.bottom() {
            ui.scroll_to_rect(
                egui::Rect::from_x_y_ranges(
                    self.viewport.left()..=self.viewport.left(),
                    bounds.y_range(),
                ),
                None,
            );
        }
    }

    fn paint(self, ui: &egui::Ui, range: Option<CaretRange>) {
        for row in self.rows {
            for (cell, shape) in row.layout.into_cells().into_iter().zip(row.shapes) {
                cell.paint(ui, shape, range);
            }
        }
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
            let mut table = TableTextUi::new(ui.max_rect(), &shown, &rows);
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
