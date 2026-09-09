//! The reference material on ionospheric TEC, as data the reference window
//! renders and a snapshot test pins.
//!
//! Every statement is either quoted from the source it cites or drawn from
//! what GeoTrace itself archives. Prose marks its abbreviations up as `[TEC]`
//! and its citations as `[^navipedia-iono]`, resolved against the
//! abbreviations and sources the document declares. The display equations are
//! rendered from the typst sources beside their assets by
//! `just generate-reference-equations`.

use gt_ui_types::reference::{
    Abbreviation, ColumnWidth, IllustrationFrame, ReferenceBlock, ReferenceDocument,
    ReferenceEquation, ReferenceIllustration, ReferenceImage, ReferenceTable, Source, TableCell,
    TableColumn,
};

pub const IONOSPHERIC_TEC: ReferenceDocument = ReferenceDocument {
    title: "Ionospheric TEC and GNSS",
    link_question: "How does ionospheric TEC affect GNSS?",
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
        short_form: "TEC",
        full_form: "total electron content",
    },
    Abbreviation {
        short_form: "TECU",
        full_form: "TEC unit, 10¹⁶ electrons per square metre",
    },
    Abbreviation {
        short_form: "STEC",
        full_form: "slant total electron content",
    },
    Abbreviation {
        short_form: "L1",
        full_form: "the primary GPS carrier, 1575.42 MHz",
    },
];

const BLOCKS: &[ReferenceBlock] = &[
    ReferenceBlock::Paragraph(
        "The ionosphere is ionized by solar radiation: during the day, Sun radiation ionizes \
         neutral atoms, producing free electrons and ions, and at night the recombination process \
         prevails.[^navipedia-iono]",
    ),
    ReferenceBlock::Paragraph(
        "[TEC], the total electron content, counts those electrons along the signal path. Along \
         one satellite's ray that count is the slant total electron content, [STEC].\
         [^navipedia-iono]",
    ),
    ReferenceBlock::Equation(SLANT_TEC_EQUATION),
    ReferenceBlock::Paragraph(
        "One [TECU] is 10¹⁶ electrons per square metre.[^navipedia-iono] A [GNSS] code \
         measurement is delayed in proportion, at the coefficient below.[^navipedia-iono]",
    ),
    ReferenceBlock::Equation(DELAY_COEFFICIENT_EQUATION),
    ReferenceBlock::Paragraph(
        "The [TEC], and thence, the ionospheric refraction, depends on the geographical location \
         of the receiver, the hour of day and the solar activity.[^navipedia-iono] The delay can \
         reach 10 to 20 m.[^navipedia-iono]",
    ),
    ReferenceBlock::Paragraph(
        "Dual-frequency receivers remove almost all of this error by combining the two \
         frequencies. Single-frequency receivers carry whatever their broadcast correction model \
         misses.[^navipedia-iono]",
    ),
    ReferenceBlock::Paragraph(
        "A receiver never measures range alone. Each pseudorange is the geometric range plus the \
         receiver clock offset times c, plus the propagation terms. With four or more satellites, \
         the receiver solves jointly for four unknowns: the coordinates (x, y, z) and the \
         receiver clock offset.[^navipedia-code]",
    ),
    ReferenceBlock::Equation(PSEUDORANGE_EQUATION),
    ReferenceBlock::Paragraph(
        "A delay that is identical on every pseudorange is algebraically indistinguishable from a \
         larger clock offset. The position estimate is unchanged. Only the part of the \
         ionospheric delay that differs between satellites displaces the position: each ray \
         crosses a different slice of ionosphere, a low satellite through more of it than one \
         overhead. That differential error is scaled by the satellite geometry.[^navipedia-pos]",
    ),
    ReferenceBlock::Paragraph(
        "The delay component common to all satellites is attributed to the receiver clock offset \
         estimate. Positioning accuracy is therefore preserved, while the receiver's time \
         estimate carries the error.",
    ),
    ReferenceBlock::Paragraph(
        "During geomagnetic storms the ionospheric density develops strong horizontal variations \
         that modify the path of radio signals and create errors in the positioning information \
         provided by [GNSS].[^noaa-storms] The reference material on geomagnetic activity covers \
         the storm mechanism.",
    ),
    ReferenceBlock::Paragraph(
        "During the geomagnetic storm of 10 to 12 May 2024 the archived maps peak above 175 \
         [TECU]. At 0.16 m per [TECU] an [L1]-only receiver carried roughly 28 m of uncorrected \
         delay.[^guardian]",
    ),
    ReferenceBlock::Illustration(STORM_MAP_ILLUSTRATION),
    ReferenceBlock::Paragraph(
        "Storm studies quantify the ionospheric response as the deviation of [TEC] from its \
         quiet-time level, the median over the same location and time of day, and place typical \
         quiet-time variation at around 40 % either way from that median.[^storm-2017] The \
         planetary ionospheric storm index takes the quiet reference as the median of the 27 days \
         before the day observed and grades the logarithmic deviation from quiet through moderate \
         disturbance to moderate and intense storm.[^iono-storm-index] GeoTrace's environment \
         warning uses that index: it warns from the moderate-storm grade, a deviation of more \
         than 43 % above or 30 % below the 27-day \
         median.[^iono-storm-index][^w-index-thresholds]",
    ),
    ReferenceBlock::Paragraph(
        "The index compares each [TEC] value with the median of the same hour of day over the 27 \
         days before it, the quiet-time reference.[^iono-storm-index] The comparison is per hour \
         because [TEC] over one place follows a daily cycle: it rises after sunrise, peaks in the \
         afternoon and falls through the night. A storm is a departure from that median: [TEC] \
         above it in the positive phase, below it in the negative phase.[^iono-storm-index]",
    ),
    ReferenceBlock::Illustration(STORM_PLOT_ILLUSTRATION),
    ReferenceBlock::Paragraph(
        "What the index grades is DTEC, the base-10 logarithm of the value over its quiet-time \
         median, and its sign is what tells the two phases apart.[^iono-storm-index] Either phase \
         develops the horizontal variations that displace a [GNSS] position.[^noaa-storms] At the \
         node above, the deepest deviation of the whole event is the depletion through 11 May, \
         not the enhancement that preceded it.",
    ),
    ReferenceBlock::Table(STORM_INDEX_TABLE),
    ReferenceBlock::Paragraph(
        "A grade reports how far the ionosphere stood from its own recent level, not how high the \
         [TEC] was in absolute terms: the reference it is measured against is the median of the \
         days before. The published planetary index reduces each map by the solar zenith angle \
         and averages the extremes across latitudes into a single number for the \
         globe.[^iono-storm-index] GeoTrace applies the same thresholds to one grid node and one \
         time of day, which is what a recording was made under: it reads each fix's own node and \
         map epoch, takes the median of the 27 archived days before that day at the same time, \
         and warns from the moderate-storm grade. The warning states the deviation it found and \
         the share the grade begins at.",
    ),
    ReferenceBlock::QueryExample {
        intro: "The fixes recorded under elevated [TEC]:",
        query: "points | where tec > 100",
    },
    ReferenceBlock::QueryExample {
        intro: "The same fixes narrowed to those that also show more than two cycle slips per \
                minute:",
        query: "points | with mask 15 deg, snr_drop 10, slip_window 5 min | where tec > 100 and \
                slip_all > 2 per min",
    },
];

/// One committed equation asset, named by the stem the generation recipe
/// writes it under.
macro_rules! equation_image {
    ($asset_name:literal) => {
        ReferenceImage {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/equations/",
                $asset_name,
                ".png"
            )),
            asset_name: $asset_name,
        }
    };
}

const SLANT_TEC_EQUATION: ReferenceEquation = ReferenceEquation {
    image: equation_image!("slant_tec"),
    alt_text: "STEC = integral of N_e along the signal path",
};

const DELAY_COEFFICIENT_EQUATION: ReferenceEquation = ReferenceEquation {
    image: equation_image!("delay_coefficient"),
    alt_text: "alpha_f = 40.3e16 / f^2 metres per TECU",
};

const PSEUDORANGE_EQUATION: ReferenceEquation = ReferenceEquation {
    image: equation_image!("pseudorange"),
    alt_text: "R = rho + c (dt - dt^s) + T + alpha_f I + TGD + M + epsilon",
};

const STORM_MAP_ILLUSTRATION: ReferenceIllustration = ReferenceIllustration {
    frames: &[IllustrationFrame {
        image: ReferenceImage {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/tec_map_2024_05_10_gannon_storm.png"
            )),
            asset_name: "tec_map_2024_05_10_gannon_storm",
        },
        label: "10 May 2024, 22:00 UTC",
    }],
    caption: "Global [TEC] during the Gannon storm of May 2024, as archived by GeoTrace.[^jpl]",
    credit: None,
};

const STORM_PLOT_ILLUSTRATION: ReferenceIllustration = ReferenceIllustration {
    frames: &[IllustrationFrame {
        image: ReferenceImage {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/tec_plot_2024_05_gannon_storm.png"
            )),
            asset_name: "tec_plot_2024_05_gannon_storm",
        },
        // The days and the node are `DRAWN_DAYS` and `DRAWN_NODE` of
        // `gt-plot`'s `reference_illustration`, which renders the asset.
        label: "6 to 13 May 2024, grid node 40 N, 100 W",
    }],
    caption: "Vertical [TEC] over mid-latitude North America across the Gannon storm, from the \
              maps GeoTrace archived, with the quiet-time median of each epoch dashed.[^jpl] The \
              days before the storm track that median. Late on 10 May the [TEC] reaches 88 \
              [TECU] against a median of 48, and through 11 and 12 May it stays far below it, \
              down to 14 [TECU] against a median of 50.",
    credit: None,
};

/// The thresholds of Table 3 in the W index paper, which the planetary index
/// applies to DTEC unchanged. The shares are those bounds as a change from
/// the median, which `the_table_states_the_shares_its_bounds_come_to` pins
/// against [`crate::quiet_time`]'s own constants.
const STORM_INDEX_TABLE: ReferenceTable = ReferenceTable {
    title: "Planetary ionospheric storm index: the grade of one deviation[^w-index-thresholds]",
    columns: &[
        TableColumn {
            header: "Grade",
            width: ColumnWidth::Fits,
        },
        TableColumn {
            header: "DTEC",
            width: ColumnWidth::Fits,
        },
        TableColumn {
            header: "Share of the median",
            width: ColumnWidth::Wraps,
        },
        TableColumn {
            header: "W",
            width: ColumnWidth::Fits,
        },
        TableColumn {
            header: "GeoTrace warns",
            width: ColumnWidth::Fits,
        },
    ],
    rows: &[
        &[
            TableCell::Prose("Quiet"),
            TableCell::Prose("up to 0.046"),
            TableCell::Prose("up to 11 % above or 10 % below"),
            TableCell::Prose("±1"),
            TableCell::Prose("no"),
        ],
        &[
            TableCell::Prose("Moderate disturbance"),
            TableCell::Prose("over 0.046, up to 0.155"),
            TableCell::Prose("11 to 43 % above or 10 to 30 % below"),
            TableCell::Prose("±2"),
            TableCell::Prose("no"),
        ],
        &[
            TableCell::Prose("Moderate ionospheric storm"),
            TableCell::Prose("over 0.155, up to 0.301"),
            TableCell::Prose("43 to 100 % above or 30 to 50 % below"),
            TableCell::Prose("±3"),
            TableCell::Prose("yes"),
        ],
        &[
            TableCell::Prose("Intense ionospheric storm"),
            TableCell::Prose("over 0.301"),
            TableCell::Prose("over 100 % above or over 50 % below"),
            TableCell::Prose("±4"),
            TableCell::Prose("yes"),
        ],
    ],
};

const SOURCES: &[Source] = &[
    Source {
        citation_key: "navipedia-iono",
        name: "Navipedia: Ionospheric Delay (Sanz Subirana, Juan Zornoza and Hernandez-Pajares, \
               UPC/ESA)",
        url: "https://gssc.esa.int/navipedia/index.php/Ionospheric_Delay",
    },
    Source {
        citation_key: "navipedia-code",
        name: "Navipedia: Code Based Positioning (SPS)",
        url: "https://gssc.esa.int/navipedia/index.php/Code_Based_Positioning_(SPS)",
    },
    Source {
        citation_key: "navipedia-pos",
        name: "Navipedia: Positioning Error",
        url: "https://gssc.esa.int/navipedia/index.php/Positioning_Error",
    },
    Source {
        citation_key: "noaa-storms",
        name: "NOAA SWPC Geomagnetic Storms",
        url: "https://www.spaceweather.gov/phenomena/geomagnetic-storms",
    },
    Source {
        citation_key: "guardian",
        name: "JPL GUARDIAN Gannon example",
        url: "https://guardian.jpl.nasa.gov/examples/20240513_gannon/",
    },
    Source {
        citation_key: "jpl",
        name: "NASA JPL ionosphere products",
        url: "https://sideshow.jpl.nasa.gov",
    },
    Source {
        citation_key: "storm-2017",
        name: "TEC disturbances caused by CME-triggered geomagnetic storm of September 6 to 9, \
               2017 (Uga, Gautam and Seba, Heliyon 10, 2024)",
        url: "https://doi.org/10.1016/j.heliyon.2024.e30725",
    },
    Source {
        citation_key: "iono-storm-index",
        name: "Derivation of a planetary ionospheric storm index (Gulyaeva and Stanislawska, \
               Annales Geophysicae 26, 2008)",
        url: "https://doi.org/10.5194/angeo-26-2645-2008",
    },
    Source {
        citation_key: "w-index-thresholds",
        name: "Ionospheric weather: cloning missed foF2 observations for derivation of \
               variability index (Gulyaeva, Stanislawska and Tomasik, Annales Geophysicae 26, \
               2008)",
        url: "https://doi.org/10.5194/angeo-26-315-2008",
    },
];

#[cfg(test)]
mod tests {
    use crate::quiet_time::{
        INTENSE_STORM_LOG_RATIO, MODERATE_STORM_LOG_RATIO, QUIET_GRADE_LIMIT_LOG_RATIO,
        QuietTimeDeviation,
    };
    use crate::tec::L1_DELAY_METERS_PER_TECU;

    use super::*;

    /// Where the share of the median sits in a row of [`STORM_INDEX_TABLE`].
    const SHARE_COLUMN: usize = 2;

    /// The share of the median each threshold comes to, as
    /// [`crate::quiet_time`] computes it, rounded the way the table states it.
    fn share_of_the_median(log_ratio: f64) -> i64 {
        QuietTimeDeviation::from_log_ratio(log_ratio)
            .percent_from_median()
            .round() as i64
    }

    /// The shares the table states are the boundaries the index grades on,
    /// written out. A deviation of 0.301 in DTEC is twice the median, which
    /// is 100 % above it.
    #[test]
    fn the_table_states_the_shares_its_bounds_come_to() {
        let above = |log_ratio: f64| share_of_the_median(log_ratio);
        let below = |log_ratio: f64| -share_of_the_median(-log_ratio);
        let stated: Vec<&str> = STORM_INDEX_TABLE
            .rows
            .iter()
            .filter_map(|row| match row.get(SHARE_COLUMN) {
                Some(TableCell::Prose(share)) => Some(*share),
                _ => None,
            })
            .collect();

        assert_eq!(
            stated,
            [
                format!(
                    "up to {} % above or {} % below",
                    above(QUIET_GRADE_LIMIT_LOG_RATIO),
                    below(QUIET_GRADE_LIMIT_LOG_RATIO)
                ),
                format!(
                    "{} to {} % above or {} to {} % below",
                    above(QUIET_GRADE_LIMIT_LOG_RATIO),
                    above(MODERATE_STORM_LOG_RATIO),
                    below(QUIET_GRADE_LIMIT_LOG_RATIO),
                    below(MODERATE_STORM_LOG_RATIO)
                ),
                format!(
                    "{} to {} % above or {} to {} % below",
                    above(MODERATE_STORM_LOG_RATIO),
                    above(INTENSE_STORM_LOG_RATIO),
                    below(MODERATE_STORM_LOG_RATIO),
                    below(INTENSE_STORM_LOG_RATIO)
                ),
                format!(
                    "over {} % above or over {} % below",
                    above(INTENSE_STORM_LOG_RATIO),
                    below(INTENSE_STORM_LOG_RATIO)
                ),
            ]
        );
    }

    /// The window's wording, in one place.
    #[test]
    fn ionospheric_tec_wording() {
        insta::assert_snapshot!("tec_reference_document", IONOSPHERIC_TEC.to_string());
    }

    #[test]
    fn the_document_is_well_formed() {
        let defects = IONOSPHERIC_TEC.defects();
        assert!(defects.is_empty(), "{defects:?}");
    }

    /// The storm figures the material quotes, against the day the archive
    /// holds them for.
    #[test]
    fn the_storm_day_peaks_where_the_material_says_it_does() {
        let maps = crate::captured_maps(crate::STORM_CAPTURE).expect("the storm capture");
        let peak = maps
            .peak_total_electron_content()
            .expect("the day holds values");

        assert!(peak.tecu() > 175.0, "the day peaks at {} TECU", peak.tecu());
        let delay_meters = peak.tecu() * L1_DELAY_METERS_PER_TECU;
        assert!(
            (27.5..28.5).contains(&delay_meters),
            "the peak delays L1 by {delay_meters} m"
        );
    }
}
