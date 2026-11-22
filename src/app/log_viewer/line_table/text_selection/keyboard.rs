use egui::text::{CCursor, CharIndex};
use egui::text_selection::text_cursor_state;

use super::super::LineTableRows;
use super::super::selection_document::{Caret, DocumentRow};
use super::CaretRange;
use super::controller::HorizontalPosition;

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

#[derive(Clone, Copy)]
pub(super) struct VerticalMotion {
    pub(super) previous: Caret,
    pub(super) row: usize,
    pub(super) preferred_x: Option<HorizontalPosition>,
}

pub(super) struct VerticalResult {
    pub(super) caret: Option<Caret>,
    pub(super) preferred_x: HorizontalPosition,
}

pub(super) struct NavigationEndpoint {
    pub(super) caret: Caret,
    pub(super) preferred_x: Option<HorizontalPosition>,
}

impl NavigationEndpoint {
    pub(super) fn moved(
        self,
        motion: SelectionMotion,
        document: &LineTableRows,
        vertical_endpoint: impl FnOnce(VerticalMotion) -> Option<VerticalResult>,
    ) -> Option<Self> {
        let previous = self.caret;
        if previous.revision() != document.document_revision() {
            return None;
        }
        let row = document.document_row(previous.row())?;
        let (caret, preferred_x) = match motion {
            SelectionMotion::Character(direction) | SelectionMotion::Word(direction) => {
                let by_word = matches!(motion, SelectionMotion::Word(_));
                let caret = previous.horizontal(direction, &row, by_word).or_else(|| {
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
                });
                (caret, None)
            }
            SelectionMotion::Row(direction) => {
                if let Some(row) = direction.adjacent_row(previous, document) {
                    let hit = vertical_endpoint(VerticalMotion {
                        previous,
                        row,
                        preferred_x: self.preferred_x,
                    })
                    .filter(|hit| {
                        hit.caret.is_none_or(|caret| {
                            caret.row() == row && caret.revision() == previous.revision()
                        })
                    });
                    let preferred_x = self.preferred_x.or(hit.as_ref().map(|hit| hit.preferred_x));
                    (hit.and_then(|hit| hit.caret), preferred_x)
                } else {
                    (Some(direction.boundary(row.range()?)), None)
                }
            }
            SelectionMotion::RowBoundary(direction) => {
                (Some(direction.boundary(row.range()?)), None)
            }
            SelectionMotion::DocumentBoundary(direction) => {
                let row = match direction {
                    Direction::Backward => 0,
                    Direction::Forward => document.len().checked_sub(1)?,
                };
                (
                    Some(direction.boundary(document.document_row(row)?.range()?)),
                    None,
                )
            }
        };
        Some(Self {
            caret: caret.unwrap_or(previous),
            preferred_x,
        })
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
