//! The document model behind the reference windows: prose carrying inline
//! abbreviations and citations, tables, query examples, display equations,
//! illustrations, and the numbered sources the text is written from.
//!
//! Prose marks an abbreviation up as `[GNSS]` and a citation as `[^gfz-kp]`,
//! resolved against the document's own [`ReferenceDocument::abbreviations`]
//! and [`ReferenceDocument::sources`] when the window walks the text with
//! [`ReferenceDocument::prose_spans`]. A marker no entry matches stays in the
//! text brackets and all, so the window shows what the author wrote.
//! [`ReferenceDocument::defects`] finds those, and the rest of what makes a
//! document ill formed, for each document's own test to assert on.
//!
//! [`std::fmt::Display`] writes the whole document as text, for the snapshot
//! that pins the wording.

use std::fmt;
use std::mem;

const MARKER_OPEN: char = '[';

const MARKER_CLOSE: char = ']';

/// Prefixes a marker body that names a citation key.
const CITATION_SIGIL: char = '^';

/// An abbreviation the prose marks up, shown with its full form on hover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Abbreviation {
    pub short_form: &'static str,
    pub full_form: &'static str,
}

/// A named link to material a document draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLink {
    pub name: &'static str,
    pub url: &'static str,
}

/// One entry of a document's numbered sources, cited from prose as
/// `[^citation_key]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub citation_key: &'static str,
    pub name: &'static str,
    pub url: &'static str,
}

/// One window's worth of reference material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceDocument {
    pub title: &'static str,
    /// Label of the link that opens this window, phrased as the question
    /// "How does {topic} affect GNSS?". A settings page declares the same
    /// `&'static str` searchable, which requires it to be a compile-time
    /// constant equal to the label the page renders.
    pub link_question: &'static str,
    pub blocks: &'static [ReferenceBlock],
    pub abbreviations: &'static [Abbreviation],
    pub sources: &'static [Source],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceBlock {
    Paragraph(&'static str),
    /// A query the reader can run as written, under a line introducing it.
    QueryExample {
        intro: &'static str,
        query: &'static str,
    },
    /// Words as the source published them, set off from the prose around the
    /// block and keeping the source's own punctuation.
    Quotation(&'static str),
    Table(ReferenceTable),
    Equation(ReferenceEquation),
    Illustration(ReferenceIllustration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceTable {
    pub title: &'static str,
    pub columns: &'static [TableColumn],
    pub rows: &'static [&'static [TableCell]],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableColumn {
    pub header: &'static str,
    pub width: ColumnWidth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnWidth {
    /// Sized to the widest of its header and cells.
    Fits,
    /// Laid out at the width the window gives prose, wrapping its cells.
    Wraps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCell {
    Prose(&'static str),
    /// Words as the source published them, punctuation included.
    Quotation(&'static str),
    /// A cell the source leaves blank for that row.
    Empty,
}

/// An image committed alongside the text it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceImage {
    pub image_bytes: &'static [u8],
    /// Names the image wherever it is addressed by name, such as the texture
    /// the window uploads it to.
    pub asset_name: &'static str,
}

/// An equation set on a line of its own, pre-rendered from its typst source.
///
/// The asset holds black glyphs whose anti-aliased coverage lives in the alpha
/// channel, which is what lets the window tint them to the theme's text
/// colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceEquation {
    pub image: ReferenceImage,
    /// The equation written as one line of text, which the window offers to a
    /// reader who cannot see the image.
    pub alt_text: &'static str,
}

/// The images committed alongside the text they illustrate, shown in the order
/// they are declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceIllustration {
    pub frames: &'static [IllustrationFrame],
    pub caption: &'static str,
    /// Who made the image, for an illustration taken from elsewhere. An
    /// illustration GeoTrace rendered itself has none, and its caption cites
    /// the source of the data instead.
    pub credit: Option<SourceLink>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllustrationFrame {
    pub image: ReferenceImage,
    pub label: &'static str,
}

/// One piece of a prose string: text as written, an abbreviation the window
/// renders with its full form on hover, or a citation the window renders as a
/// raised source number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProseSpan {
    Text(&'static str),
    Abbreviation(Abbreviation),
    Citation(Citation),
}

/// A cited source together with the number it carries in the document's
/// sources, counting from one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Citation {
    pub number: usize,
    pub source: Source,
}

/// One way a document is not well formed. Each document's own test asserts
/// [`ReferenceDocument::defects`] finds none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentDefect {
    /// A marker naming an abbreviation or a citation key the document does not
    /// declare, which the window shows with its brackets.
    UnresolvedMarker { prose: &'static str },
    /// Prose is written without em-dashes. A quotation keeps its source's
    /// punctuation and is exempt, which is why quotations are data of their
    /// own.
    EmDashInProse { prose: &'static str },
    /// Prose is written without semicolons, quotations again exempt.
    SemicolonInProse { prose: &'static str },
    /// Two sources under one key, which makes the marker resolve to whichever
    /// is listed first.
    DuplicateCitationKey { citation_key: &'static str },
    /// A source standing in the footer under a number no prose points at.
    UncitedSource { citation_key: &'static str },
    /// An abbreviation defining a term the window never shows.
    UnmarkedAbbreviation { short_form: &'static str },
    /// A table row with a cell count other than the column count.
    TableRowLength { table_title: &'static str },
}

impl ReferenceDocument {
    /// Walks one prose string, resolving its abbreviation and citation
    /// markers.
    pub fn prose_spans(&self, prose: &'static str) -> ProseSpans<'_> {
        ProseSpans {
            remaining: prose,
            abbreviations: self.abbreviations,
            sources: self.sources,
        }
    }

    /// Every string the document renders as prose, in document order.
    /// Quotations, query text, equations, and URLs are not prose: they are
    /// reproduced as their source wrote them.
    pub fn prose_texts(&self) -> Vec<&'static str> {
        let mut texts = vec![self.title];
        for block in self.blocks {
            match block {
                ReferenceBlock::Paragraph(prose) => texts.push(prose),
                ReferenceBlock::QueryExample { intro, query: _ } => texts.push(intro),
                ReferenceBlock::Table(table) => {
                    texts.push(table.title);
                    texts.extend(table.columns.iter().map(|column| column.header));
                    texts.extend(table.rows.iter().flat_map(|row| {
                        row.iter().filter_map(|cell| match cell {
                            TableCell::Prose(prose) => Some(*prose),
                            TableCell::Quotation(_) | TableCell::Empty => None,
                        })
                    }));
                }
                ReferenceBlock::Quotation(_) | ReferenceBlock::Equation(_) => {}
                ReferenceBlock::Illustration(illustration) => {
                    texts.extend(illustration.frames.iter().map(|frame| frame.label));
                    texts.push(illustration.caption);
                    texts.extend(illustration.credit.map(|credit| credit.name));
                }
            }
        }
        texts.extend(self.sources.iter().map(|source| source.name));
        texts
    }

    /// Every quotation the document sets off from its prose, in document
    /// order. The window resolves their markers like prose.
    pub fn quoted_texts(&self) -> Vec<&'static str> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ReferenceBlock::Quotation(quotation) => Some(*quotation),
                ReferenceBlock::Paragraph(_)
                | ReferenceBlock::QueryExample { .. }
                | ReferenceBlock::Table(_)
                | ReferenceBlock::Equation(_)
                | ReferenceBlock::Illustration(_) => None,
            })
            .collect()
    }

    /// The queries the document offers to run, in document order.
    pub fn query_examples(&self) -> Vec<&'static str> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ReferenceBlock::QueryExample { intro: _, query } => Some(*query),
                ReferenceBlock::Paragraph(_)
                | ReferenceBlock::Quotation(_)
                | ReferenceBlock::Table(_)
                | ReferenceBlock::Equation(_)
                | ReferenceBlock::Illustration(_) => None,
            })
            .collect()
    }

    /// Every image the document shows, in document order.
    pub fn images(&self) -> Vec<ReferenceImage> {
        self.blocks
            .iter()
            .flat_map(|block| match block {
                ReferenceBlock::Equation(equation) => vec![equation.image],
                ReferenceBlock::Illustration(illustration) => illustration
                    .frames
                    .iter()
                    .map(|frame| frame.image)
                    .collect(),
                ReferenceBlock::Paragraph(_)
                | ReferenceBlock::QueryExample { .. }
                | ReferenceBlock::Quotation(_)
                | ReferenceBlock::Table(_) => Vec::new(),
            })
            .collect()
    }

    /// Every way this document is not well formed.
    pub fn defects(&self) -> Vec<DocumentDefect> {
        let mut defects = Vec::new();
        let mut cited: Vec<&str> = Vec::new();
        let mut marked_up: Vec<&str> = Vec::new();
        for prose in self.prose_texts() {
            if prose.contains('—') {
                defects.push(DocumentDefect::EmDashInProse { prose });
            }
            if prose.contains(';') {
                defects.push(DocumentDefect::SemicolonInProse { prose });
            }
        }
        for prose in self.prose_texts().into_iter().chain(self.quoted_texts()) {
            let mut unresolved = false;
            for span in self.prose_spans(prose) {
                match span {
                    ProseSpan::Text(text) => unresolved |= text.contains(MARKER_OPEN),
                    ProseSpan::Abbreviation(abbreviation) => {
                        marked_up.push(abbreviation.short_form)
                    }
                    ProseSpan::Citation(citation) => cited.push(citation.source.citation_key),
                }
            }
            if unresolved {
                defects.push(DocumentDefect::UnresolvedMarker { prose });
            }
        }
        for (index, source) in self.sources.iter().enumerate() {
            if self
                .sources
                .iter()
                .take(index)
                .any(|earlier| earlier.citation_key == source.citation_key)
            {
                defects.push(DocumentDefect::DuplicateCitationKey {
                    citation_key: source.citation_key,
                });
            }
            if !cited.contains(&source.citation_key) {
                defects.push(DocumentDefect::UncitedSource {
                    citation_key: source.citation_key,
                });
            }
        }
        for abbreviation in self.abbreviations {
            if !marked_up.contains(&abbreviation.short_form) {
                defects.push(DocumentDefect::UnmarkedAbbreviation {
                    short_form: abbreviation.short_form,
                });
            }
        }
        for block in self.blocks {
            if let ReferenceBlock::Table(table) = block
                && table
                    .rows
                    .iter()
                    .any(|row| row.len() != table.columns.len())
            {
                defects.push(DocumentDefect::TableRowLength {
                    table_title: table.title,
                });
            }
        }
        defects
    }
}

impl fmt::Display for ReferenceDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.title)?;
        for block in self.blocks {
            writeln!(f)?;
            write!(f, "{block}")?;
        }
        writeln!(f, "\nSources")?;
        for (index, source) in self.sources.iter().enumerate() {
            writeln!(f, "{}. {} ({})", index + 1, source.name, source.url)?;
        }
        Ok(())
    }
}

impl fmt::Display for ReferenceBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paragraph(prose) => writeln!(f, "{prose}"),
            Self::QueryExample { intro, query } => writeln!(f, "{intro}\n{query}"),
            Self::Quotation(quotation) => writeln!(f, "\"{quotation}\""),
            Self::Table(table) => write!(f, "{table}"),
            Self::Equation(equation) => write!(f, "{equation}"),
            Self::Illustration(illustration) => write!(f, "{illustration}"),
        }
    }
}

impl fmt::Display for ReferenceTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.title)?;
        let headers: Vec<&str> = self.columns.iter().map(|column| column.header).collect();
        writeln!(f, "{}", headers.join(" | "))?;
        for row in self.rows {
            let cells: Vec<String> = row.iter().map(TableCell::to_string).collect();
            writeln!(f, "{}", cells.join(" | "))?;
        }
        Ok(())
    }
}

impl fmt::Display for TableCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prose(prose) => write!(f, "{prose}"),
            Self::Quotation(quotation) => write!(f, "\"{quotation}\""),
            Self::Empty => Ok(()),
        }
    }
}

impl fmt::Display for ReferenceEquation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Equation {}", self.image.asset_name)?;
        writeln!(f, "{}", self.alt_text)
    }
}

impl fmt::Display for ReferenceIllustration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for frame in self.frames {
            writeln!(
                f,
                "Illustration {} ({})",
                frame.image.asset_name, frame.label
            )?;
        }
        writeln!(f, "{}", self.caption)?;
        match self.credit {
            Some(credit) => writeln!(f, "{} ({})", credit.name, credit.url),
            None => Ok(()),
        }
    }
}

impl fmt::Display for DocumentDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnresolvedMarker { prose } => write!(f, "unresolved marker in {prose:?}"),
            Self::EmDashInProse { prose } => write!(f, "em-dash in {prose:?}"),
            Self::SemicolonInProse { prose } => write!(f, "semicolon in {prose:?}"),
            Self::DuplicateCitationKey { citation_key } => {
                write!(f, "two sources under the key {citation_key:?}")
            }
            Self::UncitedSource { citation_key } => write!(f, "{citation_key:?} is never cited"),
            Self::UnmarkedAbbreviation { short_form } => {
                write!(f, "{short_form:?} is never marked up")
            }
            Self::TableRowLength { table_title } => {
                write!(
                    f,
                    "a row of {table_title:?} has one cell too few or too many"
                )
            }
        }
    }
}

/// Iterator over one prose string's [`ProseSpan`]s, borrowing the document's
/// abbreviations and sources to resolve the markers against.
pub struct ProseSpans<'a> {
    remaining: &'static str,
    abbreviations: &'a [Abbreviation],
    sources: &'a [Source],
}

impl ProseSpans<'_> {
    fn resolve_marker_body(&self, body: &'static str) -> Option<ProseSpan> {
        match body.strip_prefix(CITATION_SIGIL) {
            Some(citation_key) => self
                .sources
                .iter()
                .enumerate()
                .find(|(_, source)| source.citation_key == citation_key)
                .map(|(index, source)| {
                    ProseSpan::Citation(Citation {
                        number: index + 1,
                        source: *source,
                    })
                }),
            None => self
                .abbreviations
                .iter()
                .find(|abbreviation| abbreviation.short_form == body)
                .map(|abbreviation| ProseSpan::Abbreviation(*abbreviation)),
        }
    }
}

impl Iterator for ProseSpans<'_> {
    type Item = ProseSpan;

    fn next(&mut self) -> Option<ProseSpan> {
        if self.remaining.is_empty() {
            return None;
        }
        let Some(open) = self.remaining.find(MARKER_OPEN) else {
            return Some(ProseSpan::Text(mem::take(&mut self.remaining)));
        };
        if open > 0 {
            let (text, rest) = self.remaining.split_at(open);
            self.remaining = rest;
            return Some(ProseSpan::Text(text));
        }
        let Some(close) = self.remaining.find(MARKER_CLOSE) else {
            return Some(ProseSpan::Text(mem::take(&mut self.remaining)));
        };
        let marker = self.remaining.get(..=close)?;
        let body = self.remaining.get(MARKER_OPEN.len_utf8()..close)?;
        self.remaining = self.remaining.get(close + MARKER_CLOSE.len_utf8()..)?;
        Some(
            self.resolve_marker_body(body)
                .unwrap_or(ProseSpan::Text(marker)),
        )
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const GNSS: Abbreviation = Abbreviation {
        short_form: "GNSS",
        full_form: "Global Navigation Satellite System",
    };

    const ABBREVIATIONS: &[Abbreviation] = &[GNSS];

    const GFZ_KP: Source = Source {
        citation_key: "gfz-kp",
        name: "GFZ Kp",
        url: "https://kp.gfz.de/en/",
    };

    const MATZKA: Source = Source {
        citation_key: "matzka-2021",
        name: "Matzka et al. 2021",
        url: "https://doi.org/10.1029/2020SW002641",
    };

    const SOURCES: &[Source] = &[GFZ_KP, MATZKA];

    const DOCUMENT: ReferenceDocument = ReferenceDocument {
        title: "Test document",
        link_question: "How does the test topic affect GNSS?",
        blocks: &[],
        abbreviations: ABBREVIATIONS,
        sources: SOURCES,
    };

    fn spans(prose: &'static str) -> Vec<ProseSpan> {
        DOCUMENT.prose_spans(prose).collect()
    }

    #[test]
    fn prose_without_a_marker_is_one_span() {
        assert_eq!(spans("Plain prose"), vec![ProseSpan::Text("Plain prose")]);
    }

    #[test]
    fn a_marker_splits_the_text_around_it() {
        assert_eq!(
            spans("errors in [GNSS] positions"),
            vec![
                ProseSpan::Text("errors in "),
                ProseSpan::Abbreviation(GNSS),
                ProseSpan::Text(" positions"),
            ]
        );
    }

    #[test]
    fn a_marker_at_the_start_yields_the_abbreviation_first() {
        assert_eq!(
            spans("[GNSS] positions"),
            vec![ProseSpan::Abbreviation(GNSS), ProseSpan::Text(" positions")]
        );
    }

    #[test]
    fn a_marker_at_the_end_closes_the_prose() {
        assert_eq!(
            spans("positions from [GNSS]"),
            vec![
                ProseSpan::Text("positions from "),
                ProseSpan::Abbreviation(GNSS)
            ]
        );
    }

    #[test]
    fn adjacent_markers_stay_separate_spans() {
        assert_eq!(
            spans("[GNSS][GNSS]"),
            vec![ProseSpan::Abbreviation(GNSS), ProseSpan::Abbreviation(GNSS),]
        );
    }

    /// A citation carries the position of its source in the document's
    /// sources, which is the number the footer prints it under.
    #[rstest]
    #[case("published every 3 hours[^gfz-kp]", GFZ_KP, 1)]
    #[case("in steps of thirds.[^matzka-2021]", MATZKA, 2)]
    fn a_citation_marker_numbers_its_source(
        #[case] prose: &'static str,
        #[case] source: Source,
        #[case] number: usize,
    ) {
        assert_eq!(
            spans(prose).last(),
            Some(&ProseSpan::Citation(Citation { number, source }))
        );
    }

    /// An unresolved marker and an unclosed bracket both keep their brackets
    /// in the text the window renders.
    #[rstest]
    #[case("errors in [TEC] positions", "[TEC]")]
    #[case("errors in [GNSS positions", "[GNSS positions")]
    #[case("in steps of thirds.[^bartels-1949]", "[^bartels-1949]")]
    fn an_unresolved_marker_keeps_its_brackets(
        #[case] prose: &'static str,
        #[case] expected: &str,
    ) {
        let texts: Vec<&str> = spans(prose)
            .into_iter()
            .filter_map(|span| match span {
                ProseSpan::Text(text) => Some(text),
                ProseSpan::Abbreviation(_) | ProseSpan::Citation(_) => None,
            })
            .collect();
        assert!(
            texts.contains(&expected),
            "expected {expected:?} among {texts:?}"
        );
    }

    #[test]
    fn multi_byte_prose_splits_on_character_boundaries() {
        assert_eq!(
            spans("60° north and [GNSS]"),
            vec![
                ProseSpan::Text("60° north and "),
                ProseSpan::Abbreviation(GNSS),
            ]
        );
    }

    #[test]
    fn prose_texts_lead_with_the_title() {
        assert_eq!(
            DOCUMENT.prose_texts(),
            vec!["Test document", "GFZ Kp", "Matzka et al. 2021"]
        );
    }

    const EQUATION_IMAGE: ReferenceImage = ReferenceImage {
        image_bytes: &[],
        asset_name: "test_equation",
    };

    const ILLUSTRATION_IMAGE: ReferenceImage = ReferenceImage {
        image_bytes: &[],
        asset_name: "test_illustration",
    };

    const IMAGE_BLOCKS: &[ReferenceBlock] = &[
        ReferenceBlock::Equation(ReferenceEquation {
            image: EQUATION_IMAGE,
            alt_text: "STEC = integral of N_e along the signal path",
        }),
        ReferenceBlock::Illustration(ReferenceIllustration {
            frames: &[IllustrationFrame {
                image: ILLUSTRATION_IMAGE,
                label: "Storm peak",
            }],
            caption: "A caption citing[^gfz-kp]",
            credit: None,
        }),
    ];

    /// This list holds an equation's asset as well as every illustration
    /// frame's, which the test that decodes what the window uploads walks.
    #[test]
    fn images_lists_every_equation_and_frame() {
        let document = ReferenceDocument {
            blocks: IMAGE_BLOCKS,
            ..DOCUMENT
        };
        assert_eq!(document.images(), vec![EQUATION_IMAGE, ILLUSTRATION_IMAGE]);
    }

    #[test]
    fn a_document_meeting_every_rule_has_no_defects() {
        const BLOCKS: &[ReferenceBlock] = &[ReferenceBlock::Paragraph(
            "Storms disturb [GNSS][^gfz-kp] and are indexed in thirds.[^matzka-2021]",
        )];
        let document = ReferenceDocument {
            blocks: BLOCKS,
            ..DOCUMENT
        };
        assert_eq!(document.defects(), vec![]);
    }

    /// A quotation is reproduced with its source's punctuation, and the
    /// citation it carries numbers that source like a citation from prose.
    #[test]
    fn a_quotation_keeps_its_punctuation_and_cites_its_source() {
        const BLOCKS: &[ReferenceBlock] = &[
            ReferenceBlock::Paragraph("Storms are indexed in thirds.[^matzka-2021]"),
            ReferenceBlock::Quotation(
                "The \"B\" level; followed by \"C\" flares, and [GNSS] with them.[^gfz-kp]",
            ),
        ];
        let document = ReferenceDocument {
            blocks: BLOCKS,
            ..DOCUMENT
        };
        assert_eq!(document.defects(), vec![]);
    }

    const UNRESOLVED_MARKER_QUOTATION: &str = "Storms disturb [TEC].[^gfz-kp][^matzka-2021]";

    #[test]
    fn an_unresolved_marker_in_a_quotation_is_a_defect() {
        const BLOCKS: &[ReferenceBlock] = &[ReferenceBlock::Quotation(UNRESOLVED_MARKER_QUOTATION)];
        let document = ReferenceDocument {
            blocks: BLOCKS,
            ..DOCUMENT
        };
        assert_eq!(
            document.defects(),
            vec![
                DocumentDefect::UnresolvedMarker {
                    prose: UNRESOLVED_MARKER_QUOTATION
                },
                DocumentDefect::UnmarkedAbbreviation { short_form: "GNSS" },
            ]
        );
    }

    const UNRESOLVED_MARKER_PROSE: &str = "Storms disturb [TEC] and [GNSS].[^gfz-kp][^matzka-2021]";

    const EM_DASH_PROSE: &str = "Storms — the largest ones — disturb [GNSS].[^gfz-kp]\
                                 [^matzka-2021]";

    const SEMICOLON_PROSE: &str = "Storms disturb [GNSS]; badly.[^gfz-kp][^matzka-2021]";

    const UNCITED_SOURCE_PROSE: &str = "Storms disturb [GNSS].[^gfz-kp]";

    const UNMARKED_ABBREVIATION_PROSE: &str = "Storms disturb GNSS.[^gfz-kp][^matzka-2021]";

    #[rstest]
    #[case(
        &[ReferenceBlock::Paragraph(UNRESOLVED_MARKER_PROSE)],
        DocumentDefect::UnresolvedMarker { prose: UNRESOLVED_MARKER_PROSE }
    )]
    #[case(
        &[ReferenceBlock::Paragraph(EM_DASH_PROSE)],
        DocumentDefect::EmDashInProse { prose: EM_DASH_PROSE }
    )]
    #[case(
        &[ReferenceBlock::Paragraph(SEMICOLON_PROSE)],
        DocumentDefect::SemicolonInProse { prose: SEMICOLON_PROSE }
    )]
    #[case(
        &[ReferenceBlock::Paragraph(UNCITED_SOURCE_PROSE)],
        DocumentDefect::UncitedSource { citation_key: "matzka-2021" }
    )]
    #[case(
        &[ReferenceBlock::Paragraph(UNMARKED_ABBREVIATION_PROSE)],
        DocumentDefect::UnmarkedAbbreviation { short_form: "GNSS" }
    )]
    fn a_paragraph_breaking_one_rule_yields_its_defect(
        #[case] blocks: &'static [ReferenceBlock],
        #[case] expected: DocumentDefect,
    ) {
        let document = ReferenceDocument { blocks, ..DOCUMENT };
        let defects = document.defects();
        assert!(
            defects.contains(&expected),
            "expected {expected} among {defects:?}"
        );
    }

    #[test]
    fn two_sources_under_one_key_are_a_defect() {
        const DUPLICATE_SOURCES: &[Source] = &[GFZ_KP, GFZ_KP];
        const BLOCKS: &[ReferenceBlock] =
            &[ReferenceBlock::Paragraph("Storms disturb [GNSS].[^gfz-kp]")];
        let document = ReferenceDocument {
            blocks: BLOCKS,
            sources: DUPLICATE_SOURCES,
            ..DOCUMENT
        };
        assert_eq!(
            document.defects(),
            vec![DocumentDefect::DuplicateCitationKey {
                citation_key: "gfz-kp"
            }]
        );
    }

    #[test]
    fn a_row_shorter_than_the_columns_is_a_defect() {
        const BLOCKS: &[ReferenceBlock] = &[
            ReferenceBlock::Paragraph("Storms disturb [GNSS].[^gfz-kp][^matzka-2021]"),
            ReferenceBlock::Table(ReferenceTable {
                title: "Scales",
                columns: &[
                    TableColumn {
                        header: "Scale",
                        width: ColumnWidth::Fits,
                    },
                    TableColumn {
                        header: "Effects",
                        width: ColumnWidth::Wraps,
                    },
                ],
                rows: &[&[TableCell::Prose("G1 minor")]],
            }),
        ];
        let document = ReferenceDocument {
            blocks: BLOCKS,
            ..DOCUMENT
        };
        assert_eq!(
            document.defects(),
            vec![DocumentDefect::TableRowLength {
                table_title: "Scales"
            }]
        );
    }
}
