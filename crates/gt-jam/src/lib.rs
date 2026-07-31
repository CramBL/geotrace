//! Aircraft-reported GNSS interference, as published daily by gpsjam.org.
//!
//! The dataset counts aircraft that reported good versus low navigation
//! integrity (ADS-B NIC) inside each H3 cell over one UTC day, aggregated by
//! adsbexchange.com from volunteer receivers. GeoTrace draws it as a map
//! overlay and as a per-track plot line.
//!
//! **This is airborne data**, averaged over a whole day and cells tens of
//! kilometers across, and a cell holding two aircraft carries no statistical
//! weight. [`text`] holds the wording every surface showing a value uses.
//!
//! One UTC day is one file, addressed by [`dataset_url`]:
//!
//! ```text
//! https://gpsjam.org/data/2026-07-20-h3_4.csv
//! ```
//!
//! Roughly 44 000 rows covering the whole world, about 300 KiB gzipped. A
//! request carries a date and nothing about the user's recordings.
//!
//! Shipped so far: the [`calendar`], the endpoint addressing, the shared UI
//! wording, and the [`wire`] parser. The dataset index, the transport, and
//! the disk cache land on top.

use std::path::PathBuf;

use chrono::NaiveDate;
use h3o::Resolution;

pub mod calendar;
pub mod dataset;
pub mod text;
pub mod wire;

/// Base URL of the default dataset host. Configurable in settings, for a
/// self-hosted mirror or an offline copy.
pub const DEFAULT_BASE_URL: &str = "https://gpsjam.org";

/// Path segment preceding a dataset's date, appended to the base URL.
const DATASET_PATH_PREFIX: &str = "/data/";

/// Separates a dataset's date from its H3 resolution in the file name.
const RESOLUTION_MARKER: &str = "-h3_";

/// Extension of a published dataset.
const DATASET_EXTENSION: &str = ".csv";

/// H3 resolution of the published cells: about 22 km edge, about 1770 km2
/// per cell.
///
/// [`dataset_file_name`] builds the file name from this, so the address and
/// the cells [`wire::parse_dataset`] accepts cannot drift apart.
pub const H3_RESOLUTION: Resolution = Resolution::Four;

/// Date format of the dataset filenames (ISO 8601 calendar date).
const DATE_FORMAT: &str = "%Y-%m-%d";

/// The URL of `day`'s dataset on `base_url`, which must not end in a slash.
///
/// Whether the day is worth requesting is [`calendar::day_outlook`]'s
/// answer, not this one's.
pub fn dataset_url(base_url: &str, day: NaiveDate) -> String {
    format!(
        "{base_url}{DATASET_PATH_PREFIX}{file_name}",
        file_name = dataset_file_name(day)
    )
}

/// The name the host serves `day`'s dataset under, and the name it is
/// captured and cached as.
pub fn dataset_file_name(day: NaiveDate) -> String {
    let date = day.format(DATE_FORMAT);
    let resolution = u8::from(H3_RESOLUTION);
    format!("{date}{RESOLUTION_MARKER}{resolution}{DATASET_EXTENSION}")
}

/// Read a day written as [`dataset_file_name`] and [`dataset_url`] write it.
pub fn parse_day(day: &str) -> Result<NaiveDate, chrono::ParseError> {
    NaiveDate::parse_from_str(day, DATE_FORMAT)
}

/// One day captured under [`fixtures_dir`] by `just jam-fixtures`.
///
/// Captures are frozen once committed; a re-capture's diff is reviewed like
/// code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureDay {
    /// The UTC day, as it appears in the dataset's file name.
    pub day: &'static str,
    /// The status the host answered when this day was captured. Pinned here
    /// as well as in the manifest, so a changed answer fails a test.
    pub http_status: u16,
    /// What this capture exists to exercise.
    pub purpose: &'static str,
}

/// The status the host answers for a day it serves.
const HTTP_OK: u16 = 200;

impl FixtureDay {
    /// Whether the host served this day, and so whether it has a dataset
    /// file. A refused day exists only in the capture manifest.
    pub const fn is_served(&self) -> bool {
        self.http_status == HTTP_OK
    }
}

/// The captured days, in the order the manifest lists them.
pub const FIXTURE_DAYS: [FixtureDay; 2] = [
    FixtureDay {
        day: "2026-07-20",
        http_status: 200,
        purpose: "a full world day, downloaded unchanged, for the parser, the index, and \
                  the renderers to run against",
    },
    FixtureDay {
        day: "2022-02-13",
        http_status: 404,
        purpose: "the day before coverage begins, so its 404 is permanent: a valid answer \
                  meaning 'never published', not a failed request",
    },
];

/// File name of the capture manifest written beside the fixtures, recording
/// when each day was captured and what the host answered.
pub const CAPTURE_MANIFEST: &str = "capture.json";

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

    #[rstest]
    #[case(2026, 7, 20, "2026-07-20-h3_4.csv")]
    #[case(2022, 2, 14, "2022-02-14-h3_4.csv")]
    fn dataset_file_name_is_the_tail_of_the_url(
        #[case] year: i32,
        #[case] month: u32,
        #[case] day: u32,
        #[case] expected: &str,
    ) {
        let day = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        assert_eq!(dataset_file_name(day), expected);
        assert!(dataset_url(DEFAULT_BASE_URL, day).ends_with(expected));
    }

    /// A captured day must never be one the host has not reached yet: such a
    /// day answers 404 now and 200 later, which would rot the fixture into
    /// meaning the opposite of what it was captured for.
    #[test]
    fn no_captured_day_is_still_ahead_of_the_host() {
        for fixture in FIXTURE_DAYS {
            let day = parse_day(fixture.day).unwrap();
            assert_ne!(
                calendar::day_outlook(day, calendar::today_utc()),
                calendar::DayOutlook::InFuture,
                "{}: {}",
                fixture.day,
                fixture.purpose
            );
        }
    }
}
