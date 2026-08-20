//! The reference material on solar flares, as data the reference window
//! renders and a snapshot test pins.
//!
//! Every statement is either quoted from the source it cites or drawn from
//! what GeoTrace itself archives. Prose marks its abbreviations up as `[EUV]`
//! and its citations as `[^noaa-flares]`, resolved against the abbreviations
//! and sources the document declares.

use gt_ui_types::reference::{
    Abbreviation, ColumnWidth, IllustrationFrame, ReferenceBlock, ReferenceDocument,
    ReferenceIllustration, ReferenceImage, ReferenceTable, Source, SourceLink, TableCell,
    TableColumn,
};

pub const SOLAR_FLARES: ReferenceDocument = ReferenceDocument {
    title: "Solar flares and GNSS",
    link_question: "How do solar flares affect GNSS?",
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
        short_form: "EUV",
        full_form: "extreme ultraviolet",
    },
    Abbreviation {
        short_form: "HF",
        full_form: "high frequency, 3 to 30 MHz",
    },
    Abbreviation {
        short_form: "D-layer",
        full_form: "the lowest ionospheric layer, about 60 to 90 km",
    },
    Abbreviation {
        short_form: "L-band",
        full_form: "1 to 2 GHz, the GNSS transmission band",
    },
    Abbreviation {
        short_form: "L1",
        full_form: "the GPS carrier at 1575.42 MHz",
    },
    Abbreviation {
        short_form: "L2",
        full_form: "the GPS carrier at 1227.60 MHz",
    },
];

const BLOCKS: &[ReferenceBlock] = &[
    ReferenceBlock::Paragraph(
        "Solar flares are large eruptions of electromagnetic radiation from the Sun lasting from \
         minutes to hours.[^noaa-flares] Because the radiation travels at light speed, any effect \
         on the sunlit side of Earth's atmosphere occurs at the same time the event is observed. A \
         [CME]'s geomagnetic storm, by contrast, arrives days later.[^noaa-flares]",
    ),
    ReferenceBlock::Illustration(FLARE_ILLUSTRATION),
    ReferenceBlock::Paragraph(
        "The increased X-ray and extreme ultraviolet ([EUV]) radiation ionizes the lower layers of \
         the ionosphere on the sunlit side.[^noaa-flares] Radio waves crossing the denser \
         [D-layer] lose energy to more frequent electron collisions. [HF] signals degrade or are \
         completely absorbed, and low-frequency navigation signals degrade with \
         them.[^noaa-flares]",
    ),
    ReferenceBlock::Paragraph(
        "Strong flares can also emit radio noise directly at [GNSS] frequencies. During the flare \
         of 6 December 2006, the [L-band] radio burst reduced the carrier-to-noise density of GPS \
         receivers by 17 dB at [L1] and 18 to 20 dB at [L2], and many sunlit receivers of the \
         International GNSS Service tracked fewer than four satellites.[^swsc-bursts][^cerruti] \
         This mechanism is receiver interference rather than propagation delay, and no frequency \
         combination removes it.",
    ),
    ReferenceBlock::Quotation(
        "The X-ray flux levels start with the \"A\" level (nominally starting at 10^-8 W/m^2). The \
         next level, ten times higher, is the \"B\" level (>= 10^-7 W/m^2); followed by \"C\" \
         flares (10^-6 W/m^2), \"M\" flares (10^-5 W/m^2), and finally \"X\" flares (10^-4 \
         W/m^2).[^noaa-flares]",
    ),
    ReferenceBlock::Paragraph(
        "The number suffix scales within the class, so an X6 flare peaks at 6 x 10^-4 W/m^2.",
    ),
    ReferenceBlock::Table(R_SCALE_TABLE),
    ReferenceBlock::Paragraph(
        "The plot marks each archived flare at its peak.[^donki] Whether a flare could have \
         affected a recording depends on whether the receiver was on the sunlit side at that \
         instant, and the marker's hover states which side the receiver was on.",
    ),
];

const R_SCALE_TABLE: ReferenceTable = ReferenceTable {
    title: "NOAA R-scale: navigation effects[^noaa-scales]",
    columns: &[
        TableColumn {
            header: "Scale",
            width: ColumnWidth::Fits,
        },
        TableColumn {
            header: "Flare class",
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
            TableCell::Prose("R1 minor"),
            TableCell::Prose("M1"),
            TableCell::Quotation("Low-frequency navigation signals degraded for brief intervals"),
            TableCell::Prose("2000"),
        ],
        &[
            TableCell::Prose("R2 moderate"),
            TableCell::Prose("M5"),
            TableCell::Quotation(
                "Degradation of low-frequency navigation signals for tens of minutes",
            ),
            TableCell::Prose("350"),
        ],
        &[
            TableCell::Prose("R3 strong"),
            TableCell::Prose("X1"),
            TableCell::Quotation("Low-frequency navigation signals degraded for about an hour"),
            TableCell::Prose("175"),
        ],
        &[
            TableCell::Prose("R4 severe"),
            TableCell::Prose("X10"),
            TableCell::Quotation(
                "Outages of low-frequency navigation signals cause increased error in positioning \
                 for one to two hours. Minor disruptions of satellite navigation possible on \
                 sunlit side",
            ),
            TableCell::Prose("8"),
        ],
        &[
            TableCell::Prose("R5 extreme"),
            TableCell::Prose("X20"),
            TableCell::Quotation(
                "Low-frequency navigation signals experience outages on sunlit side for many \
                 hours. Increased satellite navigation errors in positioning for several hours on \
                 sunlit side, which may spread into night side",
            ),
            TableCell::Prose("fewer than 1"),
        ],
    ],
};

const FLARE_ILLUSTRATION: ReferenceIllustration = ReferenceIllustration {
    frames: &[IllustrationFrame {
        image: ReferenceImage {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/nasa_solar_flare_swpc.jpg"
            )),
            asset_name: "nasa_solar_flare_swpc",
        },
        label: "A flare on the solar disc",
    }],
    caption: "The bright point on the disc is the flare. NOAA SWPC publishes this image with its \
              description of radio blackouts.[^noaa-flares]",
    credit: Some(SourceLink {
        name: "Image courtesy of NASA, published by NOAA SWPC",
        url: "https://www.spaceweather.gov/phenomena/solar-flares-radio-blackouts",
    }),
};

const SOURCES: &[Source] = &[
    Source {
        citation_key: "noaa-flares",
        name: "NOAA SWPC Solar Flares (Radio Blackouts)",
        url: "https://www.spaceweather.gov/phenomena/solar-flares-radio-blackouts",
    },
    Source {
        citation_key: "swsc-bursts",
        name: "Solar radio bursts impact on the IGS network during Solar Cycle 24, J. Space \
               Weather Space Clim. 2024",
        url: "https://www.swsc-journal.org/articles/swsc/full_html/2024/01/swsc240021/swsc240021.html",
    },
    Source {
        citation_key: "cerruti",
        name: "Cerruti et al. 2008, Space Weather, doi:10.1029/2007SW000375",
        url: "https://doi.org/10.1029/2007SW000375",
    },
    Source {
        citation_key: "noaa-scales",
        name: "NOAA Space Weather Scales",
        url: "https://www.spaceweather.gov/noaa-scales-explanation",
    },
    Source {
        citation_key: "donki",
        name: "NASA DONKI",
        url: "https://ccmc.gsfc.nasa.gov/tools/DONKI/",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The window's wording, in one place.
    #[test]
    fn solar_flare_wording() {
        insta::assert_snapshot!("flare_reference_document", SOLAR_FLARES.to_string());
    }

    #[test]
    fn the_document_is_well_formed() {
        let defects = SOLAR_FLARES.defects();
        assert!(defects.is_empty(), "{defects:?}");
    }

    /// The scale ladder the table lists, against the level the app reads a
    /// flare of each listed class as.
    #[test]
    fn every_table_row_names_the_level_its_class_is_classified_as() {
        for row in R_SCALE_TABLE.rows {
            let (Some(TableCell::Prose(scale)), Some(TableCell::Prose(class_type))) =
                (row.first(), row.get(1))
            else {
                panic!("every row leads with its scale and its flare class");
            };
            let classification: crate::FlareClassification =
                class_type.parse().expect("a published class");
            let level = classification
                .radio_blackout_class()
                .expect("a class on the blackout scale");
            assert!(
                scale.starts_with(level.scale_name()),
                "{scale} lists {class_type}, which the app reads as {}",
                level.scale_name()
            );
        }
    }
}
