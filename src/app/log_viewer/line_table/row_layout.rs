use std::sync::Arc;

use egui::emath::GuiRounding as _;
use egui::epaint::text::FontsView;
use egui::text::LayoutJob;
use egui::{Align, FontSelection, Galley, RichText, TextStyle};
use gt_log_view::TimestampTick;

use super::row_cells::RowCell;
use super::selection_document::{CellCarets, DocumentRevision, DocumentRow};
use super::{BOOT_DIVIDER_SOURCE_WIDTH_FRACTION, EntryText, FindHighlight, RowPresentation};

pub(super) struct CellLayout {
    carets: CellCarets,
    pub(super) origin: egui::Pos2,
    pub(super) galley: Arc<Galley>,
    pub(super) bounds: egui::Rect,
}

impl CellLayout {
    pub(super) fn carets(&self) -> CellCarets {
        self.carets
    }
}

pub(super) struct RowLayout {
    revision: DocumentRevision,
    row: usize,
    cells: Vec<CellLayout>,
    pub(super) bounds: egui::Rect,
    pub(super) gutter: Option<egui::Rect>,
    pub(super) separator: Option<egui::Rect>,
}

impl RowLayout {
    pub(super) fn revision(&self) -> DocumentRevision {
        self.revision
    }

    pub(super) fn row(&self) -> usize {
        self.row
    }

    pub(super) fn cells(&self) -> &[CellLayout] {
        &self.cells
    }

    pub(super) fn into_cells(self) -> Vec<CellLayout> {
        self.cells
    }
}

pub(super) struct RowLayoutStyle<'a> {
    pub(super) style: &'a egui::Style,
    pub(super) line_number_width: f32,
    pub(super) gutter_width: f32,
}

pub(super) struct RowLayoutRequest<'a> {
    pub(super) row: &'a DocumentRow<'a>,
    pub(super) bounds: egui::Rect,
    pub(super) find_highlight: Option<&'a FindHighlight<'a>>,
    pub(super) entry_text: Option<EntryText>,
}

impl RowLayoutStyle<'_> {
    pub(super) fn layout(
        &self,
        fonts: &mut FontsView<'_>,
        row: &RowLayoutRequest<'_>,
    ) -> RowLayout {
        let style = self.style;
        let color = style.visuals.text_color();
        let font = TextStyle::Monospace.resolve(style);
        let spacing = style.spacing.item_spacing.x;
        let bounds = row.bounds.round_ui();
        let center_y = bounds.center().y;
        let is_divider = matches!(row.row.content().presentation, RowPresentation::Divider);
        let has_gutter = !is_divider
            || row
                .row
                .content()
                .cells
                .indexed()
                .any(|(_, cell)| !matches!(cell, RowCell::Divider { .. }));
        let gutter = has_gutter.then(|| {
            egui::Rect::from_center_size(
                egui::pos2(bounds.left() + self.gutter_width / 2.0, center_y),
                egui::vec2(self.gutter_width, fonts.row_height(&font)),
            )
            .round_ui()
        });
        let mut right = gutter.map_or(bounds.left(), |gutter| gutter.right());
        let mut x = right + if has_gutter { spacing } else { 0.0 };
        let mut cells = Vec::new();
        for (carets, cell) in row.row.cells() {
            let available = (bounds.right() - x).max(0.0);
            let mut job = match cell {
                RowCell::Number(number) => self.rich_job(RichText::new(number).monospace().weak()),
                RowCell::Timestamp(timestamp) => {
                    let tick = row
                        .entry_text
                        .as_ref()
                        .map_or(TimestampTick::Plain, |text| text.tick);
                    timestamp.layout_job(fonts, &font, &style.visuals, tick)
                }
                RowCell::Message(message) => row.entry_text.as_ref().map_or_else(
                    || LayoutJob::simple_singleline((*message).to_owned(), font.clone(), color),
                    |entry| entry.message_job(style, &font, message),
                ),
                RowCell::Source { text, .. } => {
                    LayoutJob::simple_singleline((*text).to_owned(), font.clone(), color)
                }
                RowCell::Divider { text, strong } => {
                    let label = RichText::new(text).monospace();
                    self.rich_job(if *strong { label.strong() } else { label })
                }
            };
            if let Some(find) = row.find_highlight
                && matches!(cell, RowCell::Message(_) | RowCell::Source { .. })
            {
                find.highlight_job(style.visuals.selection.bg_fill, &mut job);
            }
            let fixed_width = match cell {
                RowCell::Number(_) => {
                    job.halign = Align::RIGHT;
                    Some(self.line_number_width)
                }
                RowCell::Source { text, boot: true } => {
                    let width = fonts
                        .layout_no_wrap((*text).to_owned(), font.clone(), color)
                        .size()
                        .x
                        .min(available * BOOT_DIVIDER_SOURCE_WIDTH_FRACTION);
                    let width = ((x + width).round_ui() - x.round_ui()).max(0.0);
                    job.justify = true;
                    Some(width)
                }
                _ => None,
            };
            if !matches!(cell, RowCell::Number(_) | RowCell::Timestamp(_)) {
                job.wrap.max_width = fixed_width.unwrap_or(available);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
            }
            let galley = fonts.layout_job(job);
            let width = fixed_width.unwrap_or_else(|| galley.size().x);
            let left = x.round_ui();
            right = (x + width).round_ui();
            let origin_x = if galley.job.halign == Align::RIGHT {
                right
            } else {
                left
            };
            let origin = egui::pos2(origin_x, center_y - galley.size().y / 2.0).round_ui();
            let cell_bounds = if matches!(cell, RowCell::Source { boot: true, .. }) {
                egui::Rect::from_x_y_ranges(left..=right, bounds.y_range())
            } else {
                galley.rect.translate(origin.to_vec2()).round_ui()
            };
            cells.push(CellLayout {
                carets,
                origin,
                galley,
                bounds: cell_bounds,
            });
            x = right + spacing;
        }
        let separator = (is_divider && x < bounds.right())
            .then(|| egui::Rect::from_x_y_ranges(x.round_ui()..=bounds.right(), bounds.y_range()));
        let row_right = if separator.is_some() {
            bounds.right()
        } else {
            right
        };
        RowLayout {
            revision: row.row.revision(),
            row: row.row.index(),
            cells,
            bounds: egui::Rect::from_x_y_ranges(bounds.left()..=row_right, bounds.y_range()),
            gutter,
            separator,
        }
    }

    fn rich_job(&self, label: RichText) -> LayoutJob {
        let mut job = LayoutJob::default();
        label.append_to(
            &mut job,
            self.style,
            FontSelection::Default,
            self.style.override_text_valign.unwrap_or(Align::Center),
        );
        job
    }
}

#[cfg(test)]
mod tests {
    use super::super::LineTableRows;
    use super::super::tests as line_table_tests;
    use super::*;

    #[test]
    fn source_cell_layout_preserves_columns_across_scroll_positions() {
        let context = egui::Context::default();
        let output = context.run_ui(Default::default(), |ui| {
            let source = "--- Device reboot with a long export marker ---";
            let raw = format!(
                "2026-01-01 00:00:00 entry\n----------- Journal summary -----------\n{source}\n"
            );
            let log = line_table_tests::source_log(&raw);
            let rows = LineTableRows::with_overlay(&log, true, None);
            let row = rows.document_row(rows.len() - 1).unwrap();
            let metrics = RowLayoutStyle {
                style: ui.style(),
                line_number_width: 24.0,
                gutter_width: 11.0,
            };
            let bounds = egui::Rect::from_min_size(egui::pos2(8.0, 20.0), egui::vec2(160.0, 20.0));
            ui.fonts_mut(|fonts| {
                let mut layout = |bounds| {
                    metrics.layout(
                        fonts,
                        &RowLayoutRequest {
                            row: &row,
                            bounds,
                            find_highlight: None,
                            entry_text: None,
                        },
                    )
                };
                let scroll = egui::vec2(0.0, 300.0);
                let visible = layout(bounds);
                let offscreen = layout(bounds.translate(scroll));
                assert_eq!(offscreen.bounds, visible.bounds.translate(scroll));
                assert_eq!(
                    offscreen.gutter,
                    visible.gutter.map(|gutter| gutter.translate(scroll))
                );
                assert_eq!(visible.cells.len(), 2);
                assert_eq!(offscreen.cells.len(), visible.cells.len());
                for (visible, offscreen) in visible.cells.iter().zip(&offscreen.cells) {
                    assert_eq!(offscreen.carets.start(), visible.carets.start());
                    assert_eq!(offscreen.bounds, visible.bounds.translate(scroll));
                    assert_eq!(offscreen.origin, visible.origin + scroll);
                    assert_eq!(offscreen.galley, visible.galley);
                }
                let number = visible.cells.first().unwrap();
                let number_right = visible.gutter.unwrap().right()
                    + metrics.style.spacing.item_spacing.x
                    + metrics.line_number_width;
                assert_eq!(number.origin.x, number_right.round_ui());
                assert_eq!(number.galley.job.halign, Align::RIGHT);
                let source_cell = visible.cells.last().unwrap();
                assert!(source_cell.galley.elided);
                assert_eq!(source_cell.galley.text(), source);
            });
        });
        output.drop_without_applying_deltas();
    }
}
