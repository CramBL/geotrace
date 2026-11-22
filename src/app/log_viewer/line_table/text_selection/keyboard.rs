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
            SelectionMotion::Character(direction) => {
                (previous.character_position(direction, &row, document), None)
            }
            SelectionMotion::Word(direction) => {
                (previous.word_position(direction, &row, document), None)
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
    fn character_position(
        self,
        direction: Direction,
        row: &DocumentRow<'_>,
        document: &LineTableRows,
    ) -> Option<Self> {
        let cursor = CCursor::new(row.column_of(self));
        let destination = match direction {
            Direction::Backward => cursor - 1,
            Direction::Forward => cursor + 1,
        };
        row.position_at_column(destination.index)
            .filter(|&position| position != self)
            .or_else(|| {
                let next = direction.adjacent_row(self, document)?;
                let range = document.document_row(next)?.range()?;
                Some(match direction {
                    Direction::Backward => range.endpoint(),
                    Direction::Forward => range.anchor(),
                })
            })
    }

    fn word_position(
        self,
        direction: Direction,
        row: &DocumentRow<'_>,
        document: &LineTableRows,
    ) -> Option<Self> {
        // The first newline beyond the caret bounds native word motion.
        let adjacent = direction
            .adjacent_row(self, document)
            .and_then(|index| document.document_row(index));
        let neighbour = adjacent.as_ref().unwrap_or(row);
        let (first, last) = match direction {
            Direction::Backward => (neighbour, row),
            Direction::Forward => (row, neighbour),
        };
        let first_range = first.range()?;
        let text = document.clipboard_text(first_range.with_endpoint(last.range()?.endpoint()))?;
        let next_row_start = first.column_of(first_range.endpoint()) + 1;
        let column = row.column_of(self);
        let cursor = CCursor::new(if row.index() == first.index() {
            column
        } else {
            next_row_start + column.0
        });
        let destination = match direction {
            Direction::Backward => text_cursor_state::ccursor_previous_word(&text, cursor),
            Direction::Forward => text_cursor_state::ccursor_next_word(&text, cursor),
        };
        if destination.index < next_row_start {
            first.position_at_column(destination.index)
        } else {
            last.position_at_column(destination.index - next_row_start.0)
        }
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

#[cfg(test)]
mod tests {
    use egui::text::{CCursor, CharIndex};
    use egui::text_selection::text_cursor_state;

    use super::super::super::{LineTableRows, tests as line_table_tests};
    use super::{CaretRange, Direction, NavigationEndpoint, SelectionMotion};

    #[test]
    fn word_motion_matches_native_motion_over_the_clipboard_text() {
        let log = line_table_tests::source_log(concat!(
            "2026-05-29 18:48:25 svc: abc!\n",
            "2026-05-29 18:48:26 svc: emoji 👩‍🔬\n",
            "2026-05-29 18:48:27 svc: plain   spaces\n",
            "2026-05-29 18:48:28 svc: final word\n",
        ));
        let document = LineTableRows::of(&log);
        let full = document.full_range().unwrap();
        let clipboard = document.clipboard_text(full).unwrap();
        for row_index in 0..document.len() {
            let row = document.document_row(row_index).unwrap();
            for (cell, _) in row.cells() {
                for index in 0..=cell.text_end().0 {
                    let caret = cell.at(CharIndex(index));
                    let cursor = CCursor::new(CharIndex(
                        document
                            .clipboard_text(CaretRange::point(full.anchor()).with_endpoint(caret))
                            .unwrap()
                            .chars()
                            .count(),
                    ));
                    for (direction, native) in [
                        (
                            Direction::Backward,
                            text_cursor_state::ccursor_previous_word(&clipboard, cursor),
                        ),
                        (
                            Direction::Forward,
                            text_cursor_state::ccursor_next_word(&clipboard, cursor),
                        ),
                    ] {
                        let endpoint = NavigationEndpoint {
                            caret,
                            preferred_x: None,
                        };
                        let moved = endpoint
                            .moved(SelectionMotion::Word(direction), &document, |_| None)
                            .unwrap();
                        let actual = document
                            .clipboard_text(CaretRange::point(caret).with_endpoint(moved.caret));
                        let expected: String = clipboard
                            .chars()
                            .skip(cursor.index.min(native.index).0)
                            .take(cursor.index.0.abs_diff(native.index.0))
                            .collect();
                        assert_eq!(
                            actual.as_deref(),
                            Some(expected.as_str()),
                            "row {row_index}, cursor {:?}",
                            cursor.index
                        );
                    }
                }
            }
        }
    }
}
