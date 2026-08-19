//! The reference window: the material behind one data source, rendered from
//! the document the source's crate declares.
//!
//! One document is shown at a time. Opening another replaces it, so the
//! reference material never stacks up in windows the reader has to close.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use egui::{CursorIcon, Grid, Label, RichText, ScrollArea, Window};
use gt_ui_types::reference::{
    Abbreviation, Citation, ColumnWidth, IllustrationFrame, ProseSpan, ReferenceBlock,
    ReferenceDocument, ReferenceIllustration, ReferenceTable, Source, TableCell,
};

use crate::app::query;

/// Wide enough to lay an illustration frame out at its own pixel width, where
/// the temperature scale drawn into it stays legible.
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1080.0, 720.0);

/// The narrowest the window goes, wide enough for the tables to render
/// without clipping: their columns have a width of their own and the window
/// scrolls vertically only.
const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(680.0, 320.0);

const BLOCK_SPACING: f32 = 10.0;

/// Width a wrapping table column lays its cells out in, wide enough for the
/// longest quotation to take two lines.
const WRAPPING_COLUMN_WIDTH: f32 = 380.0;

/// Width a paragraph wraps at, short of the width an illustration frame
/// renders at, past which a line of prose is hard to follow back to the next.
const PROSE_MAX_WIDTH: f32 = 760.0;

/// Padding around the query text inside its code background.
const QUERY_BLOCK_MARGIN: egui::Margin = egui::Margin::symmetric(6, 3);

/// The dots of an abbreviation's underline: how far apart they sit, how big
/// they are, and how far above the bottom of the text row they run.
const UNDERLINE_DOT_SPACING: f32 = 3.0;
const UNDERLINE_DOT_RADIUS: f32 = 0.6;
const UNDERLINE_RISE_FROM_ROW_BOTTOM: f32 = 3.0;

/// How a run of prose is styled: body text, a heading over the block under it,
/// a line labelling the block under it, or the small print beneath an
/// illustration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProseStyle {
    Body,
    Heading,
    Label,
    Caption,
}

impl ProseStyle {
    fn rich_text(self, text: &str) -> RichText {
        match self {
            Self::Body => RichText::new(text),
            Self::Heading => RichText::new(text).heading(),
            Self::Label => RichText::new(text).strong(),
            Self::Caption => RichText::new(text).weak().small(),
        }
    }
}

pub(super) struct ReferenceWindow {
    document: Option<ReferenceDocument>,

    /// One entry per illustration frame the window has shown. [`None`] records
    /// an image that did not decode, so the window neither retries it every
    /// frame nor reports it more than once.
    frame_textures: HashMap<&'static str, Option<egui::TextureHandle>>,
}

impl ReferenceWindow {
    pub(super) fn new() -> Self {
        Self {
            document: None,
            frame_textures: HashMap::new(),
        }
    }

    pub(super) fn open(&mut self, document: ReferenceDocument) {
        self.document = Some(document);
    }

    #[cfg(test)]
    pub(super) fn is_open(&self) -> bool {
        self.document.is_some()
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        let Some(document) = self.document else {
            return;
        };
        let mut open = true;
        Window::new(document.title)
            .open(&mut open)
            .default_size(DEFAULT_WINDOW_SIZE)
            .min_size(MIN_WINDOW_SIZE)
            .resizable(true)
            .show(ctx, |ui| self.document_ui(ui, document));
        if !open {
            self.document = None;
        }
    }

    fn document_ui(&mut self, ui: &mut egui::Ui, document: ReferenceDocument) {
        ui.style_mut().interaction.selectable_labels = false;
        // Hovers here open without egui's default delay: the abbreviations and
        // the citations are what the reader points at.
        ui.style_mut().interaction.tooltip_delay = 0.0;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for block in document.blocks {
                    self.block_ui(ui, document, block);
                    ui.add_space(BLOCK_SPACING);
                }
                ui.separator();
                sources_ui(ui, document.sources);
            });
    }

    fn block_ui(&mut self, ui: &mut egui::Ui, document: ReferenceDocument, block: &ReferenceBlock) {
        match block {
            ReferenceBlock::Paragraph(prose) => {
                paragraph_ui(ui, document, prose, ProseStyle::Body);
            }
            ReferenceBlock::QueryExample { intro, query } => {
                paragraph_ui(ui, document, intro, ProseStyle::Body);
                query_example_ui(ui, query);
            }
            ReferenceBlock::Table(table) => table_ui(ui, document, table),
            ReferenceBlock::Illustration(illustration) => {
                self.illustration_ui(ui, document, illustration);
            }
        }
    }

    fn illustration_ui(
        &mut self,
        ui: &mut egui::Ui,
        document: ReferenceDocument,
        illustration: &ReferenceIllustration,
    ) {
        for frame in illustration.frames {
            paragraph_ui(ui, document, frame.label, ProseStyle::Heading);
            self.frame_image_ui(ui, frame);
        }
        paragraph_ui(ui, document, illustration.caption, ProseStyle::Caption);
        ui.hyperlink_to(
            RichText::new(illustration.credit.name).weak().small(),
            illustration.credit.url,
        );
    }

    /// One frame at the width the window has left, never blown up past the
    /// pixel width of the image itself.
    fn frame_image_ui(&mut self, ui: &mut egui::Ui, frame: &IllustrationFrame) {
        let available_width = ui.available_width();
        let Some(texture) = self.frame_texture(ui.ctx(), frame) else {
            return;
        };
        let sized = egui::load::SizedTexture::from_handle(texture);
        let width = available_width.min(sized.size.x);
        ui.add(egui::Image::from_texture(sized).max_width(width));
    }

    fn frame_texture(
        &mut self,
        ctx: &egui::Context,
        frame: &IllustrationFrame,
    ) -> Option<&egui::TextureHandle> {
        self.frame_textures
            .entry(frame.asset_name)
            .or_insert_with(|| {
                decode_frame(frame).map(|image| {
                    ctx.load_texture(frame.asset_name, image, egui::TextureOptions::LINEAR)
                })
            })
            .as_ref()
    }
}

/// A query on the code background, coloured by the same highlighter the query
/// editor lays its text out with, and selectable so the reader can copy it.
/// It lays out at the full width the window has: a query that fits renders on
/// one line.
fn query_example_ui(ui: &mut egui::Ui, query_text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().code_bg_color)
        .inner_margin(QUERY_BLOCK_MARGIN)
        .show(ui, |ui| {
            ui.add(Label::new(query::query_syntax_layout(ui, query_text)).selectable(true));
        });
}

/// One paragraph, laid out as a run of spans so an abbreviation carries its
/// own hover while the text around it wraps as one paragraph.
fn paragraph_ui(
    ui: &mut egui::Ui,
    document: ReferenceDocument,
    prose: &'static str,
    style: ProseStyle,
) {
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(PROSE_MAX_WIDTH));
        ui.horizontal_wrapped(|ui| prose_spans_ui(ui, document, prose, style));
    });
}

fn prose_spans_ui(
    ui: &mut egui::Ui,
    document: ReferenceDocument,
    prose: &'static str,
    style: ProseStyle,
) {
    ui.spacing_mut().item_spacing.x = 0.0;
    for span in document.prose_spans(prose) {
        match span {
            ProseSpan::Text(text) => {
                ui.label(style.rich_text(text));
            }
            ProseSpan::Abbreviation(abbreviation) => abbreviation_ui(ui, abbreviation, style),
            ProseSpan::Citation(citation) => citation_ui(ui, citation),
        }
    }
}

/// An abbreviation under a dotted underline, the conventional mark for a word
/// whose definition is a hover away.
fn abbreviation_ui(ui: &mut egui::Ui, abbreviation: Abbreviation, style: ProseStyle) {
    let response = ui.label(style.rich_text(abbreviation.short_form));
    paint_dotted_underline(ui.painter(), response.rect, ui.visuals().weak_text_color());
    response
        .on_hover_cursor(CursorIcon::Help)
        .on_hover_text(abbreviation.full_form);
}

/// The raised number the source carries in the sources footer, linking to the
/// source itself.
fn citation_ui(ui: &mut egui::Ui, citation: Citation) {
    ui.hyperlink_to(
        RichText::new(citation.number.to_string()).small_raised(),
        citation.source.url,
    )
    .on_hover_text(citation.source.name);
}

fn paint_dotted_underline(painter: &egui::Painter, text_rect: egui::Rect, color: egui::Color32) {
    let y = text_rect.bottom() - UNDERLINE_RISE_FROM_ROW_BOTTOM;
    let mut x = text_rect.left();
    while x < text_rect.right() {
        painter.circle_filled(egui::pos2(x, y), UNDERLINE_DOT_RADIUS, color);
        x += UNDERLINE_DOT_SPACING;
    }
}

/// The table. A column declared [`ColumnWidth::Wraps`] gets a width of its
/// own and its cells wrap inside it. Cells of the other columns never wrap: a
/// cell that wrapped at the width the grid measured for it would feed that
/// width back into the measurement and the layout would never settle.
fn table_ui(ui: &mut egui::Ui, document: ReferenceDocument, table: &ReferenceTable) {
    ui.horizontal_wrapped(|ui| prose_spans_ui(ui, document, table.title, ProseStyle::Label));
    Grid::new((document.title, table.title))
        .num_columns(table.columns.len())
        .striped(true)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for column in table.columns {
                ui.label(RichText::new(column.header).strong());
            }
            ui.end_row();
            for row in table.rows {
                for (cell, column) in row.iter().zip(table.columns) {
                    match column.width {
                        ColumnWidth::Fits => cell_ui(ui, document, cell, column.width),
                        ColumnWidth::Wraps => {
                            ui.scope(|ui| {
                                ui.set_max_width(WRAPPING_COLUMN_WIDTH);
                                cell_ui(ui, document, cell, column.width);
                            });
                        }
                    }
                }
                ui.end_row();
            }
        });
}

fn cell_ui(ui: &mut egui::Ui, document: ReferenceDocument, cell: &TableCell, width: ColumnWidth) {
    match cell {
        TableCell::Prose(prose) => match width {
            ColumnWidth::Fits => {
                ui.horizontal(|ui| prose_spans_ui(ui, document, prose, ProseStyle::Body));
            }
            ColumnWidth::Wraps => {
                ui.horizontal_wrapped(|ui| prose_spans_ui(ui, document, prose, ProseStyle::Body));
            }
        },
        TableCell::Quotation(quotation) => {
            ui.add(Label::new(RichText::new(*quotation).italics()).wrap());
        }
        TableCell::Empty => {}
    }
}

/// The sources under the numbers the prose cites them by, each hovering the
/// url it opens.
fn sources_ui(ui: &mut egui::Ui, sources: &[Source]) {
    ui.style_mut().url_in_tooltip = true;
    for (index, source) in sources.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{}.", index + 1));
            ui.hyperlink_to(source.name, source.url);
        });
    }
}

fn decode_frame(frame: &IllustrationFrame) -> Option<egui::ColorImage> {
    let decoded = match image::load_from_memory(frame.image_bytes) {
        Ok(decoded) => decoded.to_rgba8(),
        Err(error) => {
            log::error!(
                "Reference illustration {:?} did not decode: {error}",
                frame.asset_name
            );
            return None;
        }
    };
    let size = [
        usize::try_from(decoded.width()).ok()?,
        usize::try_from(decoded.height()).ok()?,
    ];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        decoded.as_raw(),
    ))
}
