//! The archive against a real published day.

use std::fs;

use chrono::{DateTime, Utc};

use gt_jam::wire::{self, ParseWarningReporter};
use gt_jam::{FIXTURE_DAYS, dataset_file_name, fixtures_dir, parse_day};
use gt_jam_store::{FILE_NAME, JamStore};

const HOST: &str = "https://gpsjam.org";

/// Ceiling for one stored day. It measures about 81 KiB. The headroom
/// absorbs HDF5 version differences without letting a lost filter through,
/// which would cost 891 KiB.
const MAX_ARCHIVE_BYTES: u64 = 200_000;

#[test]
fn a_published_day_round_trips_and_stays_small() {
    let fixture = FIXTURE_DAYS
        .iter()
        .find(|fixture| fixture.is_served())
        .expect("a served fixture day");
    let day = parse_day(fixture.day).expect("fixture day is a calendar date");
    let csv = fs::read_to_string(fixtures_dir().join(dataset_file_name(day)))
        .expect("captured day is readable");
    let parsed = wire::parse_dataset(&csv, &ParseWarningReporter::default()).expect("parses");

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let store = JamStore::open_or_create(&path).expect("create");
    store
        .insert_day(day, HOST, DateTime::<Utc>::default(), &parsed)
        .expect("insert");

    assert_eq!(store.observations(day).expect("read back"), Some(parsed));

    let bytes = fs::metadata(&path).expect("archive metadata").len();
    assert!(
        bytes < MAX_ARCHIVE_BYTES,
        "a day took {bytes} bytes, over the {MAX_ARCHIVE_BYTES} ceiling - \
         check the chunking and filters"
    );
}

/// Every cell of the day is queryable through the index the archive returns.
#[test]
fn a_published_day_is_queryable_after_storing() {
    let fixture = FIXTURE_DAYS
        .iter()
        .find(|fixture| fixture.is_served())
        .expect("a served fixture day");
    let day = parse_day(fixture.day).expect("fixture day is a calendar date");
    let csv = fs::read_to_string(fixtures_dir().join(dataset_file_name(day)))
        .expect("captured day is readable");
    let parsed = wire::parse_dataset(&csv, &ParseWarningReporter::default()).expect("parses");

    let dir = tempfile::tempdir().expect("temp dir");
    let store = JamStore::open_or_create(&dir.path().join(FILE_NAME)).expect("create");
    store
        .insert_day(day, HOST, DateTime::<Utc>::default(), &parsed)
        .expect("insert");

    let dataset = store.dataset(day).expect("dataset").expect("stored");
    assert_eq!(dataset.len(), parsed.len());
    for observation in &parsed {
        assert_eq!(dataset.observation(observation.cell), Some(observation));
    }
}
