//! Aircraft-reported GNSS interference, as published daily by gpsjam.org.
//!
//! The dataset counts aircraft that reported good versus low navigation
//! integrity (ADS-B NIC) inside each H3 cell over one UTC day, aggregated by
//! adsbexchange.com from volunteer receivers. GeoTrace draws it as a map
//! overlay and plots it against a track so a recording can be read next to
//! the interference environment it was made in.
//!
//! **This is airborne data.** It says nothing directly about a receiver on
//! the ground, it averages a whole day over cells tens of kilometers across,
//! and a cell holding two aircraft carries no statistical weight at all.
//! Every surface that shows a value says so - see [`text`], which holds the
//! wording all of them share.
//!
//! One UTC day is one file, addressed by [`dataset_url`]:
//!
//! ```text
//! https://gpsjam.org/data/2026-07-20-h3_4.csv
//! ```
//!
//! about 170 KiB gzipped, roughly 44 000 rows, covering the whole world -
//! so a request discloses a date and nothing about the user's recordings.
//!
//! This crate currently ships the calendar ([`calendar`]), the endpoint
//! addressing, the shared UI wording, and a live-captured fixture day under
//! `tests/fixtures/`. Parsing, the dataset index, the transport, and the
//! disk cache land on top.

use std::path::PathBuf;

use chrono::NaiveDate;

pub mod calendar;
pub mod text;

/// Base URL of the default dataset host. Configurable in settings so a
/// self-hosted mirror or an offline copy can be pointed at instead.
pub const DEFAULT_BASE_URL: &str = "https://gpsjam.org";

/// Path segment preceding a dataset's date, appended to the base URL.
const DATASET_PATH_PREFIX: &str = "/data/";

/// Path segment following a dataset's date. Encodes the H3 resolution the
/// host publishes at, so it moves together with [`H3_RESOLUTION`].
const DATASET_PATH_SUFFIX: &str = "-h3_4.csv";

/// H3 resolution of the published cells: about 22 km edge, about 1770 km2
/// per cell. Coarse enough that a cell covers a good part of a drive, which
/// is why [`text::SOURCE_CAVEAT`] exists.
pub const H3_RESOLUTION: u8 = 4;

/// Date format of the dataset filenames (ISO 8601 calendar date).
const DATE_FORMAT: &str = "%Y-%m-%d";

/// The URL of `day`'s dataset on `base_url`, which must not end in a slash
/// (pass [`DEFAULT_BASE_URL`] or a configured mirror verbatim).
///
/// Addressing a day says nothing about whether it can be fetched; ask
/// [`calendar::day_outlook`] first.
pub fn dataset_url(base_url: &str, day: NaiveDate) -> String {
    let date = day.format(DATE_FORMAT);
    format!("{base_url}{DATASET_PATH_PREFIX}{date}{DATASET_PATH_SUFFIX}")
}

/// The day captured in [`fixtures_dir`]. The file is named for the day plus
/// the same suffix the host serves it under.
///
/// A full world day of real data, kept for the parser, the index, and the
/// renderers to develop against. It is the format's own artifact, downloaded
/// unchanged from the host.
pub const FIXTURE_DAY: &str = "2026-07-20";

/// Directory holding the captured dataset fixtures.
///
/// Resolved from the crate manifest dir, so it is only meaningful to
/// development tooling running inside the workspace, never to the shipped
/// application.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn dataset_url_addresses_the_day_on_the_default_host() {
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        assert_eq!(
            dataset_url(DEFAULT_BASE_URL, day),
            "https://gpsjam.org/data/2026-07-20-h3_4.csv"
        );
    }

    #[rstest]
    #[case(2026, 1, 2, "https://mirror.example/data/2026-01-02-h3_4.csv")]
    #[case(2022, 2, 1, "https://mirror.example/data/2022-02-01-h3_4.csv")]
    #[case(2026, 12, 31, "https://mirror.example/data/2026-12-31-h3_4.csv")]
    fn dataset_url_zero_pads_and_honors_the_configured_host(
        #[case] year: i32,
        #[case] month: u32,
        #[case] day: u32,
        #[case] expected: &str,
    ) {
        let day = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        assert_eq!(dataset_url("https://mirror.example", day), expected);
    }

    /// The captured fixture is the day [`FIXTURE_DAY`] names, and carries the
    /// header the parser is written against.
    #[test]
    fn fixture_day_is_present_and_has_the_published_header() {
        let path = fixtures_dir().join(format!("{FIXTURE_DAY}{DATASET_PATH_SUFFIX}"));
        let contents = std::fs::read_to_string(&path).unwrap();
        let mut lines = contents.lines();
        assert_eq!(
            lines.next(),
            Some("hex,count_good_aircraft,count_bad_aircraft")
        );
        // A full world day: enough rows that the renderers' culling and the
        // index's aggregation are exercised by real data, not a toy.
        assert!(lines.count() > 40_000);
    }

    /// The fixture's day is inside the coverage window, so the calendar
    /// agrees it is a day the host would serve.
    #[test]
    fn fixture_day_is_fetchable() {
        let day = NaiveDate::parse_from_str(FIXTURE_DAY, DATE_FORMAT).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 7, 29).unwrap();
        assert_eq!(
            calendar::day_outlook(day, today),
            calendar::DayOutlook::Fetchable
        );
    }
}
