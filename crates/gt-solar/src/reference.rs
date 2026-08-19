//! The reference material on geomagnetic activity, as data the reference
//! window renders and a snapshot test pins.
//!
//! Every statement is either quoted from the source it cites or drawn from
//! what GeoTrace itself archives. Prose marks its abbreviations up as `[Kp]`
//! and its citations as `[^gfz-kp]`, resolved against the abbreviations and
//! sources the document declares.

use gt_ui_types::reference::{
    Abbreviation, ColumnWidth, IllustrationFrame, ReferenceBlock, ReferenceDocument,
    ReferenceIllustration, ReferenceTable, Source, SourceLink, TableCell, TableColumn,
};

pub const GEOMAGNETIC_ACTIVITY: ReferenceDocument = ReferenceDocument {
    title: "Geomagnetic activity and GNSS",
    link_question: "How does geomagnetic activity affect GNSS?",
    blocks: BLOCKS,
    abbreviations: ABBREVIATIONS,
    sources: SOURCES,
};

const ABBREVIATIONS: &[Abbreviation] = &[
    Abbreviation {
        short_form: "GNSS",
        full_form: "Global Navigation Satellite System",
    },
    Abbreviation {
        short_form: "CME",
        full_form: "coronal mass ejection",
    },
    Abbreviation {
        short_form: "Kp",
        full_form: "planetary K index (Bartels, 1949)",
    },
];

const BLOCKS: &[ReferenceBlock] = &[
    ReferenceBlock::Paragraph(
        "A geomagnetic storm is a major disturbance of Earth's magnetosphere that occurs when the \
         solar wind transfers energy very efficiently into the space environment around Earth. \
         The largest storms follow coronal mass ejections ([CME]s), where a billion tons of \
         plasma from the Sun, with its embedded magnetic field, arrives at Earth. [CME]s \
         typically take several days to arrive, but have been observed to arrive in as little as \
         18 hours for the most intense storms.[^noaa-storms]",
    ),
    ReferenceBlock::Paragraph(
        "Storm heating creates strong horizontal variations in ionospheric density that modify \
         the path of radio signals and create errors in the positioning information provided by \
         [GNSS].[^noaa-storms]",
    ),
    ReferenceBlock::Paragraph(
        "[Kp] summarizes this disturbance. It is the mean standardized K index of 13 geomagnetic \
         observatories between 44 and 60 degrees geomagnetic latitude,[^noaa-kp] published every \
         3 hours since 1932 on a quasi-logarithmic 0 to 9 scale in steps of \
         thirds.[^matzka-2021] Hp30 is its half-hour counterpart and is not capped at 9.",
    ),
    ReferenceBlock::Table(G_SCALE_TABLE),
    ReferenceBlock::Paragraph(
        "The strongest storm in two decades, the G5 storm of 10 to 12 May 2024 (the Gannon \
         storm), reached [Kp] 9.0[^gfz-kp] and Hp30 11.333[^gfz-hpo], the values archived by \
         GeoTrace for those days. GPS-guided agricultural equipment was disrupted during the US \
         planting season, and foregone corn revenue across 12 Midwestern states was estimated at \
         69.6 million to 1.7 billion dollars.[^griffin-2025] JPL's GUARDIAN system publishes the \
         slant total electron content measured at [GNSS] ground stations through those \
         days.[^jpl-guardian]",
    ),
    ReferenceBlock::Illustration(THERMOSPHERE_ILLUSTRATION),
    ReferenceBlock::QueryExample {
        intro: "The fixes a recording holds from the half hours at storm level:",
        query: "points | where hp30 > 5",
    },
    ReferenceBlock::QueryExample {
        intro: "The same fixes narrowed to those that also show more than two cycle slips per \
                minute:",
        query: "points | with mask 15 deg, snr_drop 10, slip_window 5 min | where hp30 > 5 and \
                slip_all > 2 per min",
    },
];

const G_SCALE_TABLE: ReferenceTable = ReferenceTable {
    title: "NOAA G-scale: navigation effects[^noaa-scales]",
    columns: &[
        TableColumn {
            header: "Scale",
            width: ColumnWidth::Fits,
        },
        TableColumn {
            header: "Kp",
            width: ColumnWidth::Fits,
        },
        TableColumn {
            header: "Navigation effects (NOAA wording)",
            width: ColumnWidth::Wraps,
        },
        TableColumn {
            header: "Occurrences per solar cycle",
            width: ColumnWidth::Fits,
        },
    ],
    rows: &[
        &[
            TableCell::Prose("G1 minor"),
            TableCell::Prose("5"),
            TableCell::Empty,
            TableCell::Prose("1700"),
        ],
        &[
            TableCell::Prose("G2 moderate"),
            TableCell::Prose("6"),
            TableCell::Quotation("HF radio propagation can fade at higher latitudes"),
            TableCell::Prose("600"),
        ],
        &[
            TableCell::Prose("G3 strong"),
            TableCell::Prose("7"),
            TableCell::Quotation(
                "Intermittent satellite navigation and low-frequency radio navigation problems \
                 may occur",
            ),
            TableCell::Prose("200"),
        ],
        &[
            TableCell::Prose("G4 severe"),
            TableCell::Prose("8"),
            TableCell::Quotation("Satellite navigation degraded for hours"),
            TableCell::Prose("100"),
        ],
        &[
            TableCell::Prose("G5 extreme"),
            TableCell::Prose("9"),
            TableCell::Quotation("Satellite navigation may be degraded for days"),
            TableCell::Prose("4"),
        ],
    ],
};

const THERMOSPHERE_ILLUSTRATION: ReferenceIllustration = ReferenceIllustration {
    frames: &[
        IllustrationFrame {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/nasa_svs_thermosphere_2024_05_10_quiet.jpg"
            )),
            asset_name: "nasa_svs_thermosphere_2024_05_10_quiet",
            label: "Quiet",
        },
        IllustrationFrame {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/nasa_svs_thermosphere_2024_05_11_storm_peak.jpg"
            )),
            asset_name: "nasa_svs_thermosphere_2024_05_11_storm_peak",
            label: "Storm peak",
        },
    ],
    caption: "Simulated thermosphere temperature from a TIEGCM run of the storm by the NASA DRIVE \
              Science Center for Geospace Storms. [Kp] was 2.667 for the quiet frame's 3-hour \
              interval and 9.0 for the storm frame's.[^gfz-kp]",
    credit: SourceLink {
        name: "NASA Scientific Visualization Studio (NASA/AJ Christensen)",
        url: "https://svs.gsfc.nasa.gov/14835/",
    },
};

const SOURCES: &[Source] = &[
    Source {
        citation_key: "noaa-storms",
        name: "NOAA SWPC Geomagnetic Storms",
        url: "https://www.spaceweather.gov/phenomena/geomagnetic-storms",
    },
    Source {
        citation_key: "noaa-scales",
        name: "NOAA Space Weather Scales",
        url: "https://www.spaceweather.gov/noaa-scales-explanation",
    },
    Source {
        citation_key: "noaa-kp",
        name: "NOAA Planetary K-index",
        url: "https://www.spaceweather.gov/products/planetary-k-index",
    },
    Source {
        citation_key: "gfz-kp",
        name: "GFZ Kp",
        url: "https://kp.gfz.de/en/",
    },
    Source {
        citation_key: "gfz-hpo",
        name: "GFZ Hpo",
        url: "https://kp.gfz.de/en/hp30-hp60/",
    },
    Source {
        citation_key: "matzka-2021",
        name: "Matzka et al. 2021, Space Weather, doi:10.1029/2020SW002641",
        url: "https://doi.org/10.1029/2020SW002641",
    },
    Source {
        citation_key: "griffin-2025",
        name: "Griffin et al. 2025, K-State Department of Agricultural Economics, \
               doi:10.5281/zenodo.14976490",
        url: "https://doi.org/10.5281/zenodo.14976490",
    },
    Source {
        citation_key: "jpl-guardian",
        name: "JPL GUARDIAN Gannon example",
        url: "https://guardian.jpl.nasa.gov/examples/20240513_gannon/",
    },
];

#[cfg(test)]
mod tests {
    use gt_ui_types::reference::ProseSpan;

    use super::*;

    /// The window's wording, in one place.
    #[test]
    fn geomagnetic_wording() {
        insta::assert_snapshot!(
            "geomagnetic_reference_document",
            GEOMAGNETIC_ACTIVITY.to_string()
        );
    }

    /// Prose is written without em-dashes and semicolons. A quotation keeps
    /// its source's punctuation and is exempt, which is why quotations are
    /// data of their own.
    #[test]
    fn prose_avoids_em_dashes_and_semicolons() {
        for text in GEOMAGNETIC_ACTIVITY.prose_texts() {
            assert!(!text.contains('—'), "em-dash in {text:?}");
            assert!(!text.contains(';'), "semicolon in {text:?}");
        }
    }

    /// A marker naming an abbreviation or a citation key the document does not
    /// declare would reach the window with its brackets showing.
    #[test]
    fn every_prose_marker_resolves() {
        for text in GEOMAGNETIC_ACTIVITY.prose_texts() {
            for span in GEOMAGNETIC_ACTIVITY.prose_spans(text) {
                if let ProseSpan::Text(plain) = span {
                    assert!(!plain.contains('['), "unresolved marker in {text:?}");
                }
            }
        }
    }

    /// Two sources sharing a citation key would make the marker resolve to
    /// whichever is listed first.
    #[test]
    fn every_citation_key_is_unique() {
        let mut keys: Vec<&str> = SOURCES.iter().map(|source| source.citation_key).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "duplicate citation key in {keys:?}");
    }

    /// A source no prose cites would stand in the footer under a number
    /// nothing points at.
    #[test]
    fn every_source_is_cited() {
        let cited: Vec<&str> = GEOMAGNETIC_ACTIVITY
            .prose_texts()
            .into_iter()
            .flat_map(|text| GEOMAGNETIC_ACTIVITY.prose_spans(text))
            .filter_map(|span| match span {
                ProseSpan::Citation(citation) => Some(citation.source.citation_key),
                ProseSpan::Text(_) | ProseSpan::Abbreviation(_) => None,
            })
            .collect();
        for source in SOURCES {
            assert!(
                cited.contains(&source.citation_key),
                "{} is never cited",
                source.citation_key
            );
        }
    }

    /// An abbreviation no prose marks up defines a term the window never
    /// shows.
    #[test]
    fn every_abbreviation_is_used() {
        let marked_up: Vec<&str> = GEOMAGNETIC_ACTIVITY
            .prose_texts()
            .into_iter()
            .flat_map(|text| GEOMAGNETIC_ACTIVITY.prose_spans(text))
            .filter_map(|span| match span {
                ProseSpan::Abbreviation(abbreviation) => Some(abbreviation.short_form),
                ProseSpan::Text(_) | ProseSpan::Citation(_) => None,
            })
            .collect();
        for abbreviation in ABBREVIATIONS {
            assert!(
                marked_up.contains(&abbreviation.short_form),
                "{} is never marked up",
                abbreviation.short_form
            );
        }
    }

    #[test]
    fn every_table_row_has_one_cell_per_column() {
        for row in G_SCALE_TABLE.rows {
            assert_eq!(row.len(), G_SCALE_TABLE.columns.len());
        }
    }
}
