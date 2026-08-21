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
    Abbreviation, IllustrationFrame, ReferenceBlock, ReferenceDocument, ReferenceEquation,
    ReferenceIllustration, ReferenceImage, Source,
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
         warning uses that index: it warns from the moderate-storm grade, a deviation of at least \
         43 % above or 30 % below the 27-day median.[^iono-storm-index][^w-index-thresholds]",
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
    use crate::tec::L1_DELAY_METERS_PER_TECU;

    use super::*;

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
