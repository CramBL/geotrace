use egui::text::{CCursor, CharIndex};
use egui::text_selection::text_cursor_state;
use egui::{Key, Modifiers};

use super::super::row_cells::RowCells;
use super::{HorizontalPosition, TableTextSelection, TextPosition, TextRange};

#[derive(Clone, Copy)]
pub(super) enum Direction {
    Backward,
    Forward,
}

impl Direction {
    fn adjacent_row(self, position: TextPosition, row_count: usize) -> Option<usize> {
        match self {
            Self::Backward => position.row.checked_sub(1),
            Self::Forward => position.row.checked_add(1).filter(|&row| row < row_count),
        }
    }

    fn boundary(self, range: TextRange) -> TextPosition {
        match self {
            Self::Backward => range.anchor,
            Self::Forward => range.endpoint,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum SelectionMotion {
    Character(Direction),
    DocumentBoundary(Direction),
    Row(Direction),
    RowBoundary(Direction),
    Word(Direction),
}

pub(super) struct VerticalMotion<'a> {
    pub(super) previous: TextPosition,
    pub(super) row: usize,
    pub(super) preferred_x: &'a mut Option<HorizontalPosition>,
}

impl SelectionMotion {
    pub(super) fn of(key: Key, modifiers: Modifiers) -> Option<Self> {
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

impl TableTextSelection {
    pub(super) fn move_endpoint<'a>(
        &mut self,
        motion: SelectionMotion,
        row_count: usize,
        row_cells: impl Fn(usize) -> Option<RowCells<'a>>,
        vertical_endpoint: impl FnOnce(&mut VerticalMotion<'_>) -> Option<TextPosition>,
    ) -> Option<TextPosition> {
        let text = self
            .selected
            .as_mut()
            .filter(|text| text.gesture.is_none())?;
        let previous = text.range.endpoint;
        let fields = row_cells(previous.row)?;
        if !matches!(motion, SelectionMotion::Row(_)) {
            text.preferred_x = None;
        }
        let endpoint = match motion {
            SelectionMotion::Character(direction) | SelectionMotion::Word(direction) => {
                let by_word = matches!(motion, SelectionMotion::Word(_));
                previous
                    .horizontal(direction, &fields, by_word)
                    .or_else(|| {
                        let row = direction.adjacent_row(previous, row_count)?;
                        let fields = row_cells(row)?;
                        let range = fields.range_at(row)?;
                        let boundary = match direction {
                            Direction::Backward => range.endpoint,
                            Direction::Forward => range.anchor,
                        };
                        if by_word {
                            boundary
                                .horizontal(direction, &fields, true)
                                .or(Some(boundary))
                        } else {
                            Some(boundary)
                        }
                    })
            }
            SelectionMotion::Row(direction) => {
                if let Some(row) = direction.adjacent_row(previous, row_count) {
                    vertical_endpoint(&mut VerticalMotion {
                        previous,
                        row,
                        preferred_x: &mut text.preferred_x,
                    })
                } else {
                    text.preferred_x = None;
                    Some(direction.boundary(fields.range_at(previous.row)?))
                }
            }
            SelectionMotion::RowBoundary(direction) => {
                Some(direction.boundary(fields.range_at(previous.row)?))
            }
            SelectionMotion::DocumentBoundary(direction) => {
                let row = match direction {
                    Direction::Backward => 0,
                    Direction::Forward => row_count.checked_sub(1)?,
                };
                Some(direction.boundary(row_cells(row)?.range_at(row)?))
            }
        }
        .unwrap_or(previous);
        text.anchor = TextRange::point(text.range.anchor);
        text.range.endpoint = endpoint;
        self.last_click = None;
        Some(endpoint)
    }
}

impl TextPosition {
    fn horizontal(
        self,
        direction: Direction,
        fields: &RowCells<'_>,
        by_word: bool,
    ) -> Option<Self> {
        let cursor = CCursor::new(fields.column_of(self));
        let destination = if by_word {
            let line = fields
                .indexed()
                .map(|(_, cell)| cell.clipboard_text())
                .collect::<Vec<_>>()
                .join(" ");
            match direction {
                Direction::Backward => text_cursor_state::ccursor_previous_word(&line, cursor),
                Direction::Forward => text_cursor_state::ccursor_next_word(&line, cursor),
            }
        } else {
            match direction {
                Direction::Backward => cursor - 1,
                Direction::Forward => cursor + 1,
            }
        };
        fields
            .position_at_column(self.row, destination.index)
            .filter(|&position| position != self)
    }
}

impl RowCells<'_> {
    fn column_of(&self, position: TextPosition) -> CharIndex {
        position.character
            + self
                .indexed()
                .take_while(|(index, _)| *index < position.cell)
                .map(|(_, cell)| cell.clipboard_text().chars().count() + 1)
                .sum::<usize>()
    }

    fn position_at_column(&self, row: usize, column: CharIndex) -> Option<TextPosition> {
        let mut remaining = column.0;
        let mut last = None;
        for (cell, spec) in self.indexed() {
            let length = spec.clipboard_text().chars().count();
            let position = TextPosition {
                row,
                cell,
                character: CharIndex(remaining.min(length)),
            };
            if remaining <= length {
                return Some(position);
            }
            last = Some(position);
            remaining -= length + 1;
        }
        last
    }
}
