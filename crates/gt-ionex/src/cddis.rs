//! Addressing the IONEX files CDDIS serves.
//!
//! One directory per year and day of the year holds every producer's file for
//! that day, under the long IGS name of each solution. The older years hold
//! the legacy short name of the same file as well: 2024 serves the long name
//! alone, 2020 serves both. A day is looked for under the long name first and
//! the legacy one after it, which covers both eras of JPL's files.
//!
//! ```text
//! https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131/JPL0OPSFIN_20241310000_01D_02H_GIM.INX.gz
//! https://cddis.nasa.gov/archive/gnss/products/ionex/2020/131/jplg1310.20i.Z
//! ```
//!
//! The archive answers a request without an Earthdata bearer token with a
//! redirect to the login service, so every request here is authenticated.

use chrono::{Datelike as _, NaiveDate};

use crate::mirrors::FileCandidate;
use crate::{COMPRESSED_SUFFIX, FILE_SEQUENCE_DIGIT, IONOSPHERE_MAPS_TYPE, IonexProduct};

/// Base URL of the archive's IONEX tree, without a trailing slash.
pub const DEFAULT_BASE_URL: &str = "https://cddis.nasa.gov/archive/gnss/products/ionex";

/// Analysis centre, version and campaign fields of the long names GeoTrace
/// reads: JPL's operational solution.
const LONG_NAME_PRODUCER: &str = "JPL0OPS";

/// Content and format fields ending a long name: global ionosphere maps in
/// IONEX, over one day.
const LONG_NAME_CONTENT: &str = "01D";
const LONG_NAME_FORMAT: &str = "GIM.INX";

/// Suffix of the legacy short names, which are LZW compressed rather than
/// gzipped.
const LEGACY_SUFFIX: &str = ".Z";

/// The files `product` may sit under for `day`, in the order they are
/// requested.
///
/// A long name states the map spacing, which JPL publishes its rapid solution
/// at one hour today and published at two hours before, so both are requested
/// before the legacy name is.
pub fn file_candidates(
    base_url: &str,
    product: IonexProduct,
    day: NaiveDate,
) -> Vec<FileCandidate> {
    let directory = format!(
        "{base_url}/{year}/{day_of_year:03}",
        year = day.year(),
        day_of_year = day.ordinal(),
    );
    let mut candidates: Vec<FileCandidate> = map_spacings(product)
        .iter()
        .map(|spacing| {
            FileCandidate::gzipped(format!("{directory}/{}", long_name(product, day, spacing)))
        })
        .collect();
    candidates.push(FileCandidate::compressed(format!(
        "{directory}/{}",
        legacy_name(product, day)
    )));
    candidates
}

/// Solution field of the long name.
const fn solution(product: IonexProduct) -> &'static str {
    match product {
        IonexProduct::Final => "FIN",
        IonexProduct::Rapid => "RAP",
    }
}

/// Map spacings the long name of `product` may declare, in the order they are
/// requested.
const fn map_spacings(product: IonexProduct) -> &'static [&'static str] {
    match product {
        IonexProduct::Final => &["02H"],
        IonexProduct::Rapid => &["01H", "02H"],
    }
}

/// The legacy short name's four letters, lowercase as the archive files them.
const fn legacy_prefix(product: IonexProduct) -> &'static str {
    match product {
        IonexProduct::Final => "jplg",
        IonexProduct::Rapid => "jplr",
    }
}

/// `JPL0OPSFIN_20241310000_01D_02H_GIM.INX.gz` for the final maps of 10 May
/// 2024: the producer and solution, the day it starts on, its length, the map
/// spacing, and the format.
fn long_name(product: IonexProduct, day: NaiveDate, map_spacing: &str) -> String {
    format!(
        "{LONG_NAME_PRODUCER}{solution}_{year}{day_of_year:03}0000_{LONG_NAME_CONTENT}_\
         {map_spacing}_{LONG_NAME_FORMAT}{COMPRESSED_SUFFIX}",
        solution = solution(product),
        year = day.year(),
        day_of_year = day.ordinal(),
    )
}

/// `jplg1310.20i.Z` for the final maps of 10 May 2020, the name the archive
/// filed a day under before the long names replaced it.
fn legacy_name(product: IonexProduct, day: NaiveDate) -> String {
    format!(
        "{prefix}{day_of_year:03}{FILE_SEQUENCE_DIGIT}.{year:02}{maps_type}{LEGACY_SUFFIX}",
        prefix = legacy_prefix(product),
        day_of_year = day.ordinal(),
        year = day.year().rem_euclid(100),
        maps_type = IONOSPHERE_MAPS_TYPE.to_ascii_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::mirrors::FileCompression;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    /// The long names come first, and the legacy name closes the list.
    #[rstest]
    #[case::the_final_solution(
        IonexProduct::Final,
        date(2024, 5, 10),
        &[
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131/JPL0OPSFIN_20241310000_01D_02H_GIM.INX.gz",
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2024/131/jplg1310.24i.Z",
        ]
    )]
    #[case::the_rapid_solution(
        IonexProduct::Rapid,
        date(2026, 8, 15),
        &[
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2026/227/JPL0OPSRAP_20262270000_01D_01H_GIM.INX.gz",
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2026/227/JPL0OPSRAP_20262270000_01D_02H_GIM.INX.gz",
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2026/227/jplr2270.26i.Z",
        ]
    )]
    #[case::the_first_day_of_a_year(
        IonexProduct::Final,
        date(2026, 1, 1),
        &[
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2026/001/JPL0OPSFIN_20260010000_01D_02H_GIM.INX.gz",
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2026/001/jplg0010.26i.Z",
        ]
    )]
    #[case::a_year_whose_last_two_digits_lead_with_a_zero(
        IonexProduct::Final,
        date(2008, 11, 19),
        &[
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2008/324/JPL0OPSFIN_20083240000_01D_02H_GIM.INX.gz",
            "https://cddis.nasa.gov/archive/gnss/products/ionex/2008/324/jplg3240.08i.Z",
        ]
    )]
    fn a_day_is_requested_under_its_long_name_before_its_legacy_one(
        #[case] product: IonexProduct,
        #[case] day: NaiveDate,
        #[case] expected: &[&str],
    ) {
        assert_eq!(
            file_candidates(DEFAULT_BASE_URL, product, day)
                .iter()
                .map(|candidate| candidate.url.clone())
                .collect::<Vec<_>>(),
            expected
        );
    }

    /// The legacy name is LZW compressed and the long ones are gzipped, which
    /// is what decides how the served file is read.
    #[test]
    fn each_candidate_declares_how_its_file_is_compressed() {
        assert_eq!(
            file_candidates(DEFAULT_BASE_URL, IonexProduct::Final, date(2024, 5, 10))
                .iter()
                .map(|candidate| candidate.compression)
                .collect::<Vec<_>>(),
            [FileCompression::Gzip, FileCompression::UnixCompress]
        );
    }

    #[test]
    fn a_candidate_honors_the_configured_host() {
        assert_eq!(
            file_candidates(
                "https://mirror.example/ionex",
                IonexProduct::Final,
                date(2024, 5, 10)
            )
            .first()
            .map(|candidate| candidate.url.clone()),
            Some(
                "https://mirror.example/ionex/2024/131/JPL0OPSFIN_20241310000_01D_02H_GIM.INX.gz"
                    .to_owned()
            )
        );
    }
}
