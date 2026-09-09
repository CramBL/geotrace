//! The reference material on aircraft-reported interference, as data the
//! reference window renders and a snapshot test pins.
//!
//! Every statement is either quoted from the source it cites or drawn from
//! what GeoTrace itself draws. Prose marks its abbreviations up as `[ADS-B]`
//! and its citations as `[^gpsjam-faq]`, resolved against the abbreviations
//! and sources the document declares.

use gt_ui_types::reference::{
    Abbreviation, IllustrationFrame, ReferenceBlock, ReferenceDocument, ReferenceIllustration,
    ReferenceImage, Source,
};

pub const AIRCRAFT_INTERFERENCE: ReferenceDocument = ReferenceDocument {
    title: "Aircraft interference and GNSS",
    link_question: "How does aircraft interference data relate to GNSS?",
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
        short_form: "GPS",
        full_form: "Global Positioning System",
    },
    Abbreviation {
        short_form: "ADS-B",
        full_form: "Automatic Dependent Surveillance - Broadcast",
    },
    Abbreviation {
        short_form: "UTC",
        full_form: "Coordinated Universal Time",
    },
];

const BLOCKS: &[ReferenceBlock] = &[
    ReferenceBlock::Paragraph(
        "The map uses data provided by [ADS-B] Exchange to generate maps of likely [GPS] \
         interference, based on aircraft reports of their navigation system \
         accuracy.[^gpsjam-about][^adsbx]",
    ),
    ReferenceBlock::Paragraph(
        "The accuracy reports are digital instrument telemetry, not observations by pilots. \
         Aircraft avionics broadcast [ADS-B] messages automatically, and these include the \
         navigation accuracy the avionics compute from their own [GNSS] receiver.[^gpsjam-faq] \
         Broadcasting is not voluntary for most traffic: [ADS-B] Out equipage has been required in \
         most US controlled airspace since 1 January 2020,[^cfr-adsb] and in Europe for aircraft \
         above 5 700 kg or 250 knots.[^eu-adsb]",
    ),
    ReferenceBlock::Paragraph(
        "The voluntary part is on the ground. The data reaches the map through [ADS-B] Exchange, \
         \"a network of thousands of enthusiasts who receive those signals\".[^gpsjam-faq] \
         Coverage therefore depends on where receivers exist, which is one reason blank cells \
         appear.",
    ),
    ReferenceBlock::Illustration(WORLD_DAY_ILLUSTRATION),
    ReferenceBlock::Paragraph(
        "Each cell aggregates one [UTC] day. GeoTrace colours a cell by the share of aircraft that \
         reported low navigation accuracy there. The dataset's author colours his own map from a \
         share with one bad aircraft subtracted:[^gpsjam-faq]",
    ),
    ReferenceBlock::Quotation(
        "percent_bad_aircraft = 100 * (num_bad_aircraft - 1) / (num_good_aircraft + \
         num_bad_aircraft)[^gpsjam-faq]",
    ),
    ReferenceBlock::Paragraph(
        "The subtraction of one is the author's deliberate denoising of single-aircraft cells. \
         GeoTrace draws the share as counted. Cells with fewer than five aircraft are hatched, \
         drawn with thin diagonal lines in place of a solid fill. gpsjam publishes cells with as \
         few as two aircraft, where one bad report would read as 50 percent.",
    ),
    ReferenceBlock::Quotation(
        "Green hexagons show where more than 98% of all aircraft who flew through that area \
         reported good navigation accuracy. Yellow hexagons show where between 2% and 10% of \
         aircraft reported low navigation accuracy. Red hexagons show where more than 10% of \
         aircraft reported low navigation accuracy.[^gpsjam-faq]",
    ),
    ReferenceBlock::Paragraph(
        "GeoTrace colours cells on the same 2 % and 10 % breakpoints, shading continuously between \
         them.",
    ),
    ReferenceBlock::Paragraph(
        "The most common reason for aircraft [GPS] systems to have degraded accuracy is jamming by \
         military systems.[^gpsjam-faq] The dataset's author also names jamming-system testing \
         outside conflict zones and drone-defence jamming among the causes.[^gpsjam-faq]",
    ),
    ReferenceBlock::Paragraph(
        "The dataset records the effect, not the cause: the author notes the data does not show \
         what caused the low accuracy.[^gpsjam-faq] The author's denoising biases his own map \
         against showing potential interference where there is very little data.[^gpsjam-faq] A \
         short interference episode can colour a whole day's cell, since the aggregation window is \
         24 hours.[^gpsjam-faq] Blank cells mean no aircraft or no receivers, for example in an \
         active war zone.[^gpsjam-faq]",
    ),
    ReferenceBlock::Paragraph(
        "At the receiver, jamming and spoofing differ. Jamming is radio frequency interference \
         that prevents the receiver from tracking satellite signals, degrading or denying \
         positioning: the same carrier-to-noise mechanism as a solar radio burst. Spoofing instead \
         broadcasts counterfeit satellite signals, and the receiver computes incorrect \
         positioning, navigation and timing data, including time and date shifts.[^easa-sib] \
         Spoofing is reported as common in some regions since 2022.[^gpsjam-faq]",
    ),
    ReferenceBlock::QueryExample {
        intro: "The fixes recorded inside cells with reported interference:",
        query: "points | where jamming > 10 %",
    },
];

const WORLD_DAY_ILLUSTRATION: ReferenceIllustration = ReferenceIllustration {
    frames: &[IllustrationFrame {
        image: ReferenceImage {
            image_bytes: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/interference_map_2026_07_20.png"
            )),
            asset_name: "interference_map_2026_07_20",
        },
        label: "20 July 2026 (UTC)",
    }],
    caption: "Every cell published for 20 July 2026, on the colour ramp the map layer uses. Where \
              a cell is missing, no aircraft flew over it or no receiver heard one.[^gpsjam-about]",
    credit: None,
};

const SOURCES: &[Source] = &[
    Source {
        citation_key: "gpsjam-about",
        name: "gpsjam.org: About",
        url: "https://gpsjam.org/about",
    },
    Source {
        citation_key: "adsbx",
        name: "ADS-B Exchange",
        url: "https://adsbexchange.com",
    },
    Source {
        citation_key: "gpsjam-faq",
        name: "gpsjam.org: FAQ (John Wiseman)",
        url: "https://gpsjam.org/faq",
    },
    Source {
        citation_key: "cfr-adsb",
        name: "14 CFR 91.225, ADS-B Out equipment and use",
        url: "https://www.ecfr.gov/current/title-14/chapter-I/subchapter-F/part-91/subpart-C/section-91.225",
    },
    Source {
        citation_key: "eu-adsb",
        name: "Commission Implementing Regulation (EU) No 1207/2011, Article 5 (as amended)",
        url: "https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX%3A32011R1207",
    },
    Source {
        citation_key: "easa-sib",
        name: "EASA Safety Information Bulletin 2022-02, GNSS outages and alterations",
        url: "https://ad.easa.europa.eu/blob/EASA_SIB_2022_02R2.pdf/SIB_2022-02R2_1",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The window's wording, in one place.
    #[test]
    fn aircraft_interference_wording() {
        insta::assert_snapshot!(
            "interference_reference_document",
            AIRCRAFT_INTERFERENCE.to_string()
        );
    }

    #[test]
    fn the_document_is_well_formed() {
        let defects = AIRCRAFT_INTERFERENCE.defects();
        assert!(defects.is_empty(), "{defects:?}");
    }
}
