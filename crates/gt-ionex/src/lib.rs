//! Global ionosphere maps, as published in the IONEX format.
//!
//! One file holds a day of vertical total electron content maps on a fixed
//! latitude/longitude grid, two hours apart in the products GeoTrace reads
//! (JPL's `JPLG` and `JPLR`). TEC is what the ionosphere adds to a GNSS
//! pseudorange, so a recording can be read against the ionosphere it was made
//! under.
//!
//! [`parse::global_ionosphere_maps`] reads a decompressed file into
//! [`maps::GlobalIonosphereMaps`], whose
//! [`total_electron_content_at`](maps::GlobalIonosphereMaps::total_electron_content_at)
//! interpolates one position and time out of it.
//! [`tec::L1_DELAY_METERS_PER_TECU`] turns the value into the range delay a
//! receiver saw on L1.
//!
//! [`IonexProduct::file_url`] addresses one day's file:
//!
//! ```text
//! https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz
//! ```
//!
//! [`calendar`] says which days and products are worth requesting,
//! [`mirrors`] holds the hosts to try in order, and [`transport`] fetches one
//! day and decompresses it. A mirror serves either that layout or the CDDIS
//! one ([`cddis`]), which files the same producer's day under a long IGS name
//! and a legacy [`unix_compress`] one and serves it to callers holding an
//! Earthdata token.

use std::path::PathBuf;

use chrono::{Datelike as _, NaiveDate};

use crate::maps::GlobalIonosphereMaps;

pub mod calendar;
pub mod cddis;
pub mod grid;
pub mod instant_selection;
pub mod maps;
pub mod mirrors;
pub mod parse;
pub mod quiet_time;
pub mod reference;
pub mod tec;
pub mod text;
pub mod transport;
pub mod unix_compress;

pub use instant_selection::{ShownInstant, TecEmptyReason, TecInstantSelection};
pub use mirrors::{Mirror, MirrorBaseUrl, MirrorLayout, MirrorList};

/// Suffix of the gzipped files the archives serve. The parser reads the
/// decompressed text.
pub const COMPRESSED_SUFFIX: &str = ".gz";

/// Base URL of the host that publishes the maps, and the sole entry of a
/// default [`MirrorList`].
pub const DEFAULT_BASE_URL: &str = "https://sideshow.jpl.nasa.gov/pub/iono_daily";

/// Which of JPL's two daily map products a file comes from.
///
/// The same day is published twice: once about a day after it ends, and again
/// two days later from the full station set. The second replaces the first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumCount, strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
pub enum IonexProduct {
    /// Published about 50 hours after the day ends, from every station that
    /// reported. JPL revises no further.
    Final,
    /// Published about a day after the day ends, from the stations available
    /// by then.
    Rapid,
}

impl IonexProduct {
    /// The order a day is requested in: the settled product first, so a day
    /// old enough to have one is never archived from the earlier estimate.
    pub const PREFERENCE_ORDER: [Self; 2] = [Self::Final, Self::Rapid];

    /// Directory the product's files sit in, under the base URL.
    const fn directory(self) -> &'static str {
        match self {
            Self::Final => "IONEX_final",
            Self::Rapid => "IONEX_rapid",
        }
    }

    /// The four letters a file name starts with.
    const fn file_name_prefix(self) -> &'static str {
        match self {
            Self::Final => "JPLG",
            Self::Rapid => "JPLR",
        }
    }

    /// The compressed file name of `day`, `JPLG1310.24I.gz` for 10 May 2024:
    /// the product prefix, the zero-padded day of the year, the file sequence
    /// digit, the last two digits of the year, and the ionosphere map type.
    pub fn file_name(self, day: NaiveDate) -> String {
        format!(
            "{prefix}{day_of_year:03}{FILE_SEQUENCE_DIGIT}.{year:02}{IONOSPHERE_MAPS_TYPE}{COMPRESSED_SUFFIX}",
            prefix = self.file_name_prefix(),
            day_of_year = day.ordinal(),
            year = day.year().rem_euclid(100),
        )
    }

    /// The URL of `day` on `base_url`, which must not end in a slash.
    ///
    /// Final maps are filed under a year directory. Rapid ones are not.
    pub fn file_url(self, base_url: &str, day: NaiveDate) -> String {
        let file_name = self.file_name(day);
        match self {
            Self::Final => format!(
                "{base_url}/{directory}/y{year}/{file_name}",
                directory = self.directory(),
                year = day.year(),
            ),
            Self::Rapid => format!(
                "{base_url}/{directory}/{file_name}",
                directory = self.directory()
            ),
        }
    }
}

/// Digit distinguishing several files for one day. Both daily products
/// publish a single file, numbered zero.
pub(crate) const FILE_SEQUENCE_DIGIT: char = '0';

/// File type letter of ionosphere maps, which ends a file name and which the
/// header's type record declares.
pub(crate) const IONOSPHERE_MAPS_TYPE: char = 'I';

/// One file captured under [`fixtures_dir`] by `just ionex-fixtures`.
///
/// Captures are frozen once committed. A re-capture's diff is reviewed like
/// code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureFile {
    /// Names the capture on the capture command line.
    pub name: &'static str,
    /// Where the file was captured from, compressed as the archive serves it.
    pub url: &'static str,
    /// What the decompressed capture is stored as, which is the name the
    /// archive publishes the file under.
    pub file_name: &'static str,
    /// What this capture exists to exercise.
    pub purpose: &'static str,
}

/// Name of the capture taken on the May 2024 storm day.
pub const STORM_CAPTURE: &str = "jpl-final-storm";

/// Name of the capture taken on the geomagnetically quiet day on the same
/// grid.
pub const QUIET_CAPTURE: &str = "jpl-final-quiet";

/// The captured files, in the order the manifest lists them.
pub const FIXTURE_FILES: [FixtureFile; 2] = [
    FixtureFile {
        name: STORM_CAPTURE,
        url: "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz",
        file_name: "JPLG1310.24I",
        purpose: "10 May 2024, the day of the G5 storm, where TEC peaks far above a normal day",
    },
    FixtureFile {
        name: QUIET_CAPTURE,
        url: "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG0920.24I.gz",
        file_name: "JPLG0920.24I",
        purpose: "1 April 2024, a geomagnetically quiet day on the same grid",
    },
];

/// File name of the capture manifest written beside the fixtures, recording
/// when each file was captured and what the archive served.
pub const CAPTURE_MANIFEST: &str = "capture.json";

/// Directory holding the captured files.
///
/// Resolved from the crate manifest dir, so it is only meaningful to
/// development tooling running inside the workspace, never to the shipped
/// application.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Why a captured file did not reach the caller as maps.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("{name} is not declared in FIXTURE_FILES")]
    Undeclared { name: String },
    #[error("reading {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{file_name}: {source}")]
    Parse {
        file_name: &'static str,
        source: parse::ParseError,
    },
}

/// The capture [`FIXTURE_FILES`] declares under `name`.
pub fn declared_fixture(name: &str) -> Option<&'static FixtureFile> {
    FIXTURE_FILES.iter().find(|fixture| fixture.name == name)
}

/// The decompressed text of one capture, as the archive published it.
pub fn captured_text(fixture: &FixtureFile) -> Result<String, CaptureError> {
    let path = fixtures_dir().join(fixture.file_name);
    std::fs::read_to_string(&path).map_err(|source| CaptureError::Read { path, source })
}

/// The maps of the capture declared under `name`, which the tests and the
/// asset generators of the workspace read their archived day from.
pub fn captured_maps(name: &str) -> Result<GlobalIonosphereMaps, CaptureError> {
    let fixture = declared_fixture(name).ok_or_else(|| CaptureError::Undeclared {
        name: name.to_owned(),
    })?;
    parse::global_ionosphere_maps(&captured_text(fixture)?).map_err(|source| CaptureError::Parse {
        file_name: fixture.file_name,
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    /// Final maps are filed by year, rapid ones sit in one directory, and both
    /// name the day by its ordinal in the year.
    #[rstest]
    #[case::a_final_file(
        IonexProduct::Final,
        date(2024, 5, 10),
        "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz"
    )]
    #[case::a_rapid_file(
        IonexProduct::Rapid,
        date(2026, 8, 15),
        "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_rapid/JPLR2270.26I.gz"
    )]
    #[case::the_first_day_of_a_year(
        IonexProduct::Final,
        date(2026, 1, 1),
        "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2026/JPLG0010.26I.gz"
    )]
    #[case::the_last_day_of_a_leap_year(
        IonexProduct::Final,
        date(2024, 12, 31),
        "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG3660.24I.gz"
    )]
    #[case::a_year_whose_last_two_digits_lead_with_a_zero(
        IonexProduct::Final,
        date(2008, 11, 19),
        "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2008/JPLG3240.08I.gz"
    )]
    fn a_file_url_names_the_day_by_its_ordinal_in_the_year(
        #[case] product: IonexProduct,
        #[case] day: NaiveDate,
        #[case] expected: &str,
    ) {
        assert_eq!(product.file_url(DEFAULT_BASE_URL, day), expected);
    }

    #[test]
    fn a_file_url_honors_the_configured_host() {
        assert_eq!(
            IonexProduct::Rapid.file_url("https://mirror.example", date(2024, 5, 10)),
            "https://mirror.example/IONEX_rapid/JPLR1310.24I.gz"
        );
    }

    /// The settled product is requested first, and adding a product cannot
    /// leave it out of the order.
    #[test]
    fn every_product_is_tried_in_preference_order() {
        assert_eq!(
            IonexProduct::PREFERENCE_ORDER,
            [IonexProduct::Final, IonexProduct::Rapid]
        );
        assert_eq!(IonexProduct::PREFERENCE_ORDER.len(), IonexProduct::COUNT);
        for product in IonexProduct::iter() {
            assert!(
                IonexProduct::PREFERENCE_ORDER.contains(&product),
                "{product}"
            );
        }
    }

    /// The captured fixtures were downloaded from URLs verified against the
    /// live archive, so addressing their days must reproduce them exactly.
    #[rstest]
    #[case::the_storm_day("jpl-final-storm", date(2024, 5, 10))]
    #[case::the_quiet_day("jpl-final-quiet", date(2024, 4, 1))]
    fn addressing_a_captured_day_reproduces_the_url_it_was_captured_from(
        #[case] name: &str,
        #[case] day: NaiveDate,
    ) {
        let fixture = FIXTURE_FILES
            .iter()
            .find(|fixture| fixture.name == name)
            .expect("the capture is declared");
        assert_eq!(
            IonexProduct::Final.file_url(DEFAULT_BASE_URL, day),
            fixture.url
        );
    }

    #[test]
    fn every_capture_is_named_after_the_file_its_url_serves() {
        for fixture in FIXTURE_FILES {
            assert_eq!(
                fixture.url.rsplit('/').next(),
                Some(format!("{}{COMPRESSED_SUFFIX}", fixture.file_name).as_str()),
                "{}: {}",
                fixture.name,
                fixture.purpose
            );
        }
    }

    #[test]
    fn every_capture_has_its_own_name_and_file_name() {
        let names: BTreeSet<&str> = FIXTURE_FILES.iter().map(|fixture| fixture.name).collect();
        let file_names: BTreeSet<&str> = FIXTURE_FILES
            .iter()
            .map(|fixture| fixture.file_name)
            .collect();
        assert_eq!(names.len(), FIXTURE_FILES.len());
        assert_eq!(file_names.len(), FIXTURE_FILES.len());
    }
}
