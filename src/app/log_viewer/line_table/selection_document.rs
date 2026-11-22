use std::sync::atomic::{AtomicU64, Ordering};

use egui::text::CharIndex;

use super::row_cells::{CellIndex, RowCell};
use super::text_selection::CaretRange;
use super::{LineTableContent, LineTableRow, LineTableRows};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::app::log_viewer) struct DocumentRevision(u64);

impl DocumentRevision {
    pub(super) fn next() -> Self {
        Self(NEXT_DOCUMENT_REVISION.fetch_add(1, Ordering::Relaxed))
    }
}

impl LineTableRows {
    fn copy_rows<'a>(
        selection: CaretRange,
        row_at: impl Fn(usize) -> Option<DocumentRow<'a>>,
    ) -> String {
        let ordered = selection.ordered();
        let first_row = ordered.start.row();
        let mut output = String::new();
        for index in first_row..=ordered.end.row() {
            let Some(row) = row_at(index) else {
                continue;
            };
            if index != first_row {
                output.push('\n');
            }
            let mut first_cell = true;
            for (index, cell) in row.content.cells.indexed() {
                let text = cell.clipboard_text();
                let Some(characters) =
                    selection.cell_characters(row.index, index, CharIndex(text.chars().count()))
                else {
                    continue;
                };
                if !first_cell {
                    output.push(' ');
                }
                first_cell = false;
                output.extend(
                    text.chars()
                        .skip(characters.start.0)
                        .take(characters.end.0 - characters.start.0),
                );
            }
        }
        output
    }

    pub(super) fn document_revision(&self) -> DocumentRevision {
        self.revision
    }

    pub(super) fn document_row(&self, index: usize) -> Option<DocumentRow<'_>> {
        let kind = self.at(index)?;
        Some(DocumentRow {
            revision: self.revision,
            index,
            kind,
            content: kind.content(&self.parsed)?,
        })
    }

    pub(super) fn full_range(&self) -> Option<CaretRange> {
        let first = self.document_row(0)?.range()?;
        let last = self.document_row(self.len().checked_sub(1)?)?.range()?;
        Some(first.with_endpoint(last.endpoint()))
    }

    pub(super) fn clipboard_text(&self, range: CaretRange) -> Option<String> {
        if range.anchor().revision() != self.revision {
            return None;
        }
        Some(Self::copy_rows(range, |row| self.document_row(row)))
    }
}

pub(super) struct DocumentRow<'a> {
    revision: DocumentRevision,
    index: usize,
    kind: LineTableRow,
    content: LineTableContent<'a>,
}

impl DocumentRow<'_> {
    pub(super) fn revision(&self) -> DocumentRevision {
        self.revision
    }

    pub(super) fn index(&self) -> usize {
        self.index
    }

    pub(super) fn kind(&self) -> LineTableRow {
        self.kind
    }

    pub(super) fn content(&self) -> &LineTableContent<'_> {
        &self.content
    }

    pub(super) fn cells(&self) -> impl DoubleEndedIterator<Item = (CellCarets, &RowCell<'_>)> {
        self.content.cells.indexed().map(|(cell, spec)| {
            let carets = CellCarets {
                start: Caret {
                    revision: self.revision,
                    row: self.index,
                    cell,
                    character: CharIndex::ZERO,
                },
                end: CharIndex(spec.clipboard_text().chars().count()),
            };
            (carets, spec)
        })
    }

    pub(super) fn range(&self) -> Option<CaretRange> {
        let mut cells = self.cells();
        let first = cells.next()?.0;
        let last = cells.next_back().map_or(first, |(carets, _)| carets);
        Some(CaretRange::point(first.start).with_endpoint(last.at(last.end)))
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellCarets {
    start: Caret,
    end: CharIndex,
}

impl CellCarets {
    pub(super) fn at(self, character: CharIndex) -> Caret {
        Caret {
            character: character.min(self.end),
            ..self.start
        }
    }

    pub(super) fn start(self) -> Caret {
        self.start
    }

    pub(super) fn text_end(self) -> CharIndex {
        self.end
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Caret {
    revision: DocumentRevision,
    row: usize,
    cell: CellIndex,
    character: CharIndex,
}

impl Caret {
    pub(super) fn revision(self) -> DocumentRevision {
        self.revision
    }

    pub(super) fn row(self) -> usize {
        self.row
    }

    pub(super) fn cell(self) -> CellIndex {
        self.cell
    }

    pub(super) fn character(self) -> CharIndex {
        self.character
    }
}

static NEXT_DOCUMENT_REVISION: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gt_ui_types::LoadedLogId;
    use rstest::rstest;

    use super::super::row_cells::RowCells;
    use super::super::tests as line_table_tests;
    use super::super::{ICON_DEVICE_CLOCK, LogViewerWindow, RowPresentation, TimestampText};
    use super::{
        CaretRange, CharIndex, DocumentRevision, DocumentRow, LineTableContent, LineTableRow,
        LineTableRows, RowCell,
    };

    fn source_row(
        revision: DocumentRevision,
        index: usize,
        cells: RowCells<'_>,
    ) -> DocumentRow<'_> {
        DocumentRow {
            revision,
            index,
            kind: LineTableRow::RenderRawSource { line_number: 1 },
            content: LineTableContent {
                presentation: RowPresentation::Source,
                cells,
            },
        }
    }

    #[test]
    fn copy_uses_cell_order_across_rows_with_optional_cells() {
        let revision = DocumentRevision::next();
        let row_at = |row: usize| {
            source_row(
                revision,
                row,
                RowCells([
                    Some(RowCell::Source {
                        text: "αβ",
                        boot: false,
                    }),
                    None,
                    Some(RowCell::Number(row.to_string())),
                ]),
            )
        };
        let anchor = row_at(102).cells().next_back().unwrap().0.at(CharIndex(2));
        let endpoint = row_at(100).cells().next().unwrap().0.at(CharIndex(1));
        let selection = CaretRange::point(anchor).with_endpoint(endpoint);
        let requested_rows = RefCell::new(Vec::new());

        let copied = LineTableRows::copy_rows(selection, |row| {
            requested_rows.borrow_mut().push(row);
            Some(row_at(row))
        });
        assert_eq!(copied, "β 100\nαβ 101\nαβ 10");
        assert_eq!(requested_rows.into_inner(), [100, 101, 102]);
    }

    #[rstest]
    #[case::device_clock(0, " ")]
    #[case::date_after_clock(1, "2")]
    fn copy_projects_device_clock_at_display_character_boundaries(
        #[case] character: usize,
        #[case] expected: &str,
    ) {
        let revision = DocumentRevision::next();
        let row_at = || {
            source_row(
                revision,
                0,
                RowCells([
                    Some(RowCell::Timestamp(TimestampText {
                        prefix: ICON_DEVICE_CLOCK,
                        date: "2026-01-01 ".to_owned(),
                        hour_minute: "14:02".to_owned(),
                        seconds: ":11".to_owned(),
                    })),
                    None,
                    None,
                ]),
            )
        };
        let (carets, _) = row_at().cells().next().unwrap();
        let selection = CaretRange::point(carets.at(CharIndex(character)))
            .with_endpoint(carets.at(CharIndex(character + 1)));

        assert_eq!(
            LineTableRows::copy_rows(selection, |_| Some(row_at())),
            expected
        );
    }

    #[test]
    fn replacing_cached_content_rejects_carets_from_the_previous_document() {
        let first = line_table_tests::source_log("2026-01-01 14:02:11 αβ\n");
        let second = line_table_tests::source_log("2026-01-01 14:02:11 different\n");
        let mut viewer = LogViewerWindow::new();
        let id = LoadedLogId::new(0);
        let first_rows = viewer.table_rows(&first, id);
        let second_rows = viewer.table_rows(&second, id);
        let row = first_rows.document_row(first_rows.len() - 1).unwrap();
        let (message, _) = row.cells().next_back().unwrap();
        let end = message.at(CharIndex(usize::MAX));
        let selected = CaretRange::point(message.start()).with_endpoint(end);

        assert_eq!(end.character(), CharIndex(2));
        assert_eq!(first_rows.clipboard_text(selected).as_deref(), Some("αβ"));
        assert!(second_rows.clipboard_text(selected).is_none());
        let foreign = second_rows.full_range().unwrap().endpoint();
        assert_eq!(selected.with_endpoint(foreign).endpoint(), end);
    }
}
