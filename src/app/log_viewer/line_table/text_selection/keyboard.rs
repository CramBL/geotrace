use egui::text::{CCursor, CharIndex};
use egui::text_selection::text_cursor_state;
use egui::{Key, Modifiers};

use super::super::LineTableRows;
use super::super::selection_document::{Caret, DocumentRow};
use super::{CaretRange, HorizontalPosition, TableTextSelection};

#[derive(Clone, Copy)]
pub(super) enum Direction {
    Backward,
    Forward,
}

impl Direction {
    fn adjacent_row(self, position: Caret, document: &LineTableRows) -> Option<usize> {
        match self {
            Self::Backward => position.row().checked_sub(1),
            Self::Forward => position
                .row()
                .checked_add(1)
                .filter(|&row| row < document.len()),
        }
    }

    fn boundary(self, range: CaretRange) -> Caret {
        match self {
            Self::Backward => range.anchor(),
            Self::Forward => range.endpoint(),
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
    pub(super) previous: Caret,
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
    pub(super) fn move_endpoint(
        &mut self,
        motion: SelectionMotion,
        document: &LineTableRows,
        vertical_endpoint: impl FnOnce(&mut VerticalMotion<'_>) -> Option<Caret>,
    ) -> Option<Caret> {
        let text = self
            .selected
            .as_mut()
            .filter(|text| text.gesture.is_none())?;
        let previous = text.range.endpoint();
        let row = document.document_row(previous.row())?;
        if !matches!(motion, SelectionMotion::Row(_)) {
            text.preferred_x = None;
        }
        let endpoint = match motion {
            SelectionMotion::Character(direction) | SelectionMotion::Word(direction) => {
                let by_word = matches!(motion, SelectionMotion::Word(_));
                previous.horizontal(direction, &row, by_word).or_else(|| {
                    let row = direction.adjacent_row(previous, document)?;
                    let row = document.document_row(row)?;
                    let range = row.range()?;
                    let boundary = match direction {
                        Direction::Backward => range.endpoint(),
                        Direction::Forward => range.anchor(),
                    };
                    if by_word {
                        boundary
                            .horizontal(direction, &row, true)
                            .or(Some(boundary))
                    } else {
                        Some(boundary)
                    }
                })
            }
            SelectionMotion::Row(direction) => {
                if let Some(row) = direction.adjacent_row(previous, document) {
                    vertical_endpoint(&mut VerticalMotion {
                        previous,
                        row,
                        preferred_x: &mut text.preferred_x,
                    })
                } else {
                    text.preferred_x = None;
                    Some(direction.boundary(row.range()?))
                }
            }
            SelectionMotion::RowBoundary(direction) => Some(direction.boundary(row.range()?)),
            SelectionMotion::DocumentBoundary(direction) => {
                let row = match direction {
                    Direction::Backward => 0,
                    Direction::Forward => document.len().checked_sub(1)?,
                };
                Some(direction.boundary(document.document_row(row)?.range()?))
            }
        }
        .unwrap_or(previous);
        text.anchor = CaretRange::point(text.range.anchor());
        text.range = text.range.with_endpoint(endpoint);
        self.last_click = None;
        Some(text.range.endpoint())
    }
}

impl Caret {
    fn horizontal(
        self,
        direction: Direction,
        row: &DocumentRow<'_>,
        by_word: bool,
    ) -> Option<Self> {
        let cursor = CCursor::new(row.column_of(self));
        let destination = if by_word {
            let line = row
                .content()
                .cells
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
        row.position_at_column(destination.index)
            .filter(|&position| position != self)
    }
}

impl DocumentRow<'_> {
    fn column_of(&self, position: Caret) -> CharIndex {
        position.character()
            + self
                .content()
                .cells
                .indexed()
                .take_while(|(index, _)| *index < position.cell())
                .map(|(_, cell)| cell.clipboard_text().chars().count() + 1)
                .sum::<usize>()
    }

    fn position_at_column(&self, column: CharIndex) -> Option<Caret> {
        let mut remaining = column.0;
        let mut last = None;
        for (carets, _) in self.cells() {
            let length = carets.text_end().0;
            let position = carets.at(CharIndex(remaining));
            if remaining <= length {
                return Some(position);
            }
            last = Some(position);
            remaining -= length + 1;
        }
        last
    }
}
