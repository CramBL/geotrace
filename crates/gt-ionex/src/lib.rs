//! Global ionosphere maps, as published in the IONEX format.
//!
//! One file holds a day of vertical total electron content maps on a fixed
//! latitude/longitude grid, two hours apart in the products GeoTrace reads
//! (JPL's `JPLG`/`JPLR` and CODE's `CODG`). TEC is what the ionosphere adds
//! to a GNSS pseudorange, so a recording can be read against the ionosphere
//! it was made under.
//!
//! [`parse::global_ionosphere_maps`] reads a decompressed file into
//! [`maps::GlobalIonosphereMaps`], whose
//! [`total_electron_content_at`](maps::GlobalIonosphereMaps::total_electron_content_at)
//! interpolates one position and time out of it.
//! [`tec::L1_DELAY_METERS_PER_TECU`] turns the value into the range delay a
//! receiver saw on L1.

use std::path::PathBuf;

pub mod grid;
pub mod maps;
pub mod parse;
pub mod tec;

/// Suffix the archives serve their files under. The parser reads the
/// decompressed text.
pub const COMPRESSED_SUFFIX: &str = ".gz";

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

/// The captured files, in the order the manifest lists them.
pub const FIXTURE_FILES: [FixtureFile; 2] = [
    FixtureFile {
        name: "jpl-final-storm",
        url: "https://sideshow.jpl.nasa.gov/pub/iono_daily/IONEX_final/y2024/JPLG1310.24I.gz",
        file_name: "JPLG1310.24I",
        purpose: "10 May 2024, the day of the G5 storm, where TEC peaks far above a normal day",
    },
    FixtureFile {
        name: "jpl-final-quiet",
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

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
