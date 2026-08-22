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

/// Days stored for the delete measurement, half of which are then deleted and
/// half of those stored again.
const STORED_DAYS: i64 = 10;

/// The day every measurement here is made against, and its observations.
fn captured_day() -> Result<(chrono::NaiveDate, Vec<gt_jam::wire::HexObservation>), String> {
    let fixture = FIXTURE_DAYS
        .iter()
        .find(|fixture| fixture.is_served())
        .ok_or("no served fixture day")?;
    let day = parse_day(fixture.day).map_err(|err| format!("fixture day: {err}"))?;
    let csv = fs::read_to_string(fixtures_dir().join(dataset_file_name(day)))
        .map_err(|err| format!("captured day: {err}"))?;
    let parsed = wire::parse_dataset(&csv, &ParseWarningReporter::default())
        .map_err(|err| format!("parse: {err}"))?;
    Ok((day, parsed))
}

#[test]
fn a_published_day_round_trips_and_stays_small() {
    let (day, parsed) = captured_day().expect("a captured day");

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
    let (day, parsed) = captured_day().expect("a captured day");

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

/// On the columns a published day fills, the file ends where it started: the
/// days stored after a delete are written into the space it freed.
#[test]
fn days_stored_after_a_delete_reuse_the_space_it_freed() {
    let (first, parsed) = captured_day().expect("a captured day");
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let store = JamStore::open_or_create(&path).expect("create");
    let store_days = |offsets: std::ops::Range<i64>| {
        for offset in offsets {
            store
                .insert_day(
                    first + chrono::TimeDelta::days(offset),
                    HOST,
                    DateTime::<Utc>::default(),
                    &parsed,
                )
                .expect("insert");
        }
    };
    store_days(0..STORED_DAYS);
    let filled = fs::metadata(&path).expect("archive metadata").len();

    let cutoff = first + chrono::TimeDelta::days(STORED_DAYS / 2);
    let removed = store.delete_days_before(cutoff, None).expect("delete days");
    let pruned = fs::metadata(&path).expect("archive metadata").len();
    store_days(STORED_DAYS..STORED_DAYS + STORED_DAYS / 2);
    let refilled = fs::metadata(&path).expect("archive metadata").len();

    assert_eq!(
        removed,
        usize::try_from(STORED_DAYS / 2).unwrap_or_default()
    );
    assert_eq!(store.observations(cutoff).expect("read back"), Some(parsed));
    assert!(
        pruned <= filled,
        "deleting half the days grew the file from {filled} to {pruned}"
    );
    // The space five days take is about half of what the file holds here.
    assert!(
        refilled <= filled,
        "the days stored after the delete grew the file from {filled} to {refilled}"
    );
}
