//! The parser against uncurated input.
//!
//! [`gt_jam::wire::parse_dataset`] is the crate's only consumer of untrusted
//! network bytes, which the unit tests' hand-built rows cannot cover.
//!
//! All three properties assert the same thing: the parser never panics, and
//! what survives is at [`H3_RESOLUTION`], counted at least one aircraft, and
//! is the only row for its cell. Mirrors how `gt-snap` fuzzes its shape
//! decoder and response classifier.

use std::collections::HashSet;
use std::fs;
use std::sync::OnceLock;

use proptest::test_runner::TestCaseError;

use gt_jam::wire::{self, HexObservation, ParseWarningReporter};
use gt_jam::{FIXTURE_DAYS, H3_RESOLUTION, dataset_file_name, fixtures_dir, parse_day};

/// How far into the captured day the truncation property cuts: enough for
/// the header and the first rows, without re-parsing 900 KiB per case.
const MAX_TRUNCATION_BYTES: usize = 4096;

/// The captured world day, read once for the whole run.
fn captured_world_day() -> Result<&'static str, String> {
    static CSV: OnceLock<Result<String, String>> = OnceLock::new();
    CSV.get_or_init(|| {
        let fixture = FIXTURE_DAYS
            .iter()
            .find(|fixture| fixture.is_served())
            .ok_or_else(|| "no served day is declared in FIXTURE_DAYS".to_owned())?;
        let day = parse_day(fixture.day)
            .map_err(|err| format!("{} is not a calendar date: {err}", fixture.day))?;
        let path = fixtures_dir().join(dataset_file_name(day));
        fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
    })
    .as_deref()
    .map_err(Clone::clone)
}

/// What [`wire::parse_dataset`] promises about its output.
fn check_observations(observations: &[HexObservation]) -> Result<(), TestCaseError> {
    let mut seen: HashSet<_> = HashSet::new();
    for observation in observations {
        if observation.cell.resolution() != H3_RESOLUTION {
            return Err(TestCaseError::fail(format!(
                "{observation:?} is not at the published resolution"
            )));
        }
        if observation.rate().is_none() {
            return Err(TestCaseError::fail(format!(
                "{observation:?} counted no aircraft, so it has no share to show"
            )));
        }
        if !seen.insert(observation.cell) {
            return Err(TestCaseError::fail(format!(
                "{observation:?} is a second row for one cell"
            )));
        }
    }
    Ok(())
}

proptest::proptest! {
    /// Any input at all. Most cases are rejected at the header.
    #[test]
    fn arbitrary_input_yields_only_usable_observations(csv in ".{0,2048}") {
        let reporter = ParseWarningReporter::default();
        if let Ok(observations) = wire::parse_dataset(&csv, &reporter) {
            check_observations(&observations)?;
        }
    }

    /// A real header, so cases land past the header check.
    #[test]
    fn arbitrary_rows_under_a_real_header_yield_only_usable_observations(rows in ".{0,2048}") {
        let csv = format!("hex,count_good_aircraft,count_bad_aircraft\n{rows}");
        let reporter = ParseWarningReporter::default();
        let observations =
            wire::parse_dataset(&csv, &reporter).expect("the published header is present");
        check_observations(&observations)?;
    }

    /// A dropped connection or a truncated cache entry.
    #[test]
    fn a_truncated_real_dataset_yields_only_usable_observations(cut in 0..MAX_TRUNCATION_BYTES) {
        let csv = captured_world_day().map_err(TestCaseError::fail)?;
        // The dataset is ASCII, so every byte is a character boundary.
        // `get` checks it.
        if let Some(truncated) = csv.get(..cut) {
            let reporter = ParseWarningReporter::default();
            if let Ok(observations) = wire::parse_dataset(truncated, &reporter) {
                check_observations(&observations)?;
            }
        }
    }
}
