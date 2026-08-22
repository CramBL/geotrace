//! Round-trip flare days through a real archive file in a temp directory.

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use rstest::rstest;
use tempfile::TempDir;

use gt_flare::SolarFlare;
use gt_flare::class::FlareClassification;
use gt_flare_store::{FILE_NAME, FlareStore, FlareStoreError, schema};

/// The base URL the archive records. The API key is never part of it.
const HOST: &str = "https://api.nasa.gov";

fn store() -> Result<(TempDir, FlareStore), String> {
    let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
    let store = FlareStore::open_or_create(&dir.path().join(FILE_NAME))
        .map_err(|err| format!("open archive: {err}"))?;
    Ok((dir, store))
}

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 5, 9).unwrap_or_default() + TimeDelta::days(offset)
}

fn fetched_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_784_505_600, 0).unwrap_or_default()
}

fn at(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    day.and_hms_opt(hour, minute, 0)
        .unwrap_or_default()
        .and_utc()
}

fn classification(class_type: &str) -> Option<FlareClassification> {
    class_type.parse().ok()
}

/// A day of flares: one fully described, and one with everything the catalog
/// leaves off left off.
fn flare_day(day: NaiveDate) -> Option<Vec<SolarFlare>> {
    Some(vec![
        SolarFlare {
            id: format!("{day}T08:45:00-FLR-001"),
            begin: at(day, 8, 45),
            peak: at(day, 9, 13),
            end: Some(at(day, 9, 36)),
            classification: classification("X2.2")?,
            source_location: Some("S20W25".to_owned()),
            active_region: Some(13664),
        },
        SolarFlare {
            id: format!("{day}T23:04:00-FLR-001"),
            begin: at(day, 23, 4),
            peak: at(day, 23, 8),
            end: None,
            classification: classification("M1.2")?,
            source_location: None,
            active_region: None,
        },
    ])
}

#[test]
fn a_new_archive_holds_no_day() {
    let (_dir, store) = store().unwrap();
    assert!(store.archived_days().expect("days").is_empty());
    assert!(!store.contains(day(0)).expect("contains"));
    assert_eq!(store.flares(day(0)).expect("flares"), None);
}

/// Every field survives the round trip, the ones the catalog left off
/// included.
#[test]
fn a_day_round_trips_with_the_fields_the_catalog_left_off() {
    let (_dir, store) = store().unwrap();
    let written = flare_day(day(0)).expect("a day of flares");
    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), &written)
        .expect("store");

    let read = store.flares(day(0)).expect("flares").expect("archived");
    assert_eq!(read, written);
    assert!(store.contains(day(0)).expect("contains"));
}

/// A day the catalog lists no flare for is still an archived day, distinct
/// from one never fetched.
#[test]
fn a_day_without_flares_is_still_archived() {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), &[])
        .expect("store");

    assert!(store.contains(day(0)).expect("contains"));
    assert_eq!(store.flares(day(0)).expect("flares"), Some(vec![]));
    assert_eq!(
        store
            .archived_days()
            .expect("days")
            .first()
            .map(|entry| entry.flares),
        Some(0)
    );
}

/// A day already archived is stored again when the catalog revises it.
#[test]
fn storing_a_day_again_replaces_what_was_archived() {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_day(
            day(0),
            HOST,
            fetched_at(),
            &flare_day(day(0)).expect("a day of flares"),
        )
        .expect("store");

    let revised: Vec<SolarFlare> = flare_day(day(0))
        .expect("a day of flares")
        .into_iter()
        .take(1)
        .map(|flare| SolarFlare {
            classification: classification("X5.8").expect("a published class"),
            ..flare
        })
        .collect();
    let revised_at = fetched_at() + TimeDelta::days(1);
    store
        .insert_or_replace_day(day(0), HOST, revised_at, &revised)
        .expect("store revision");

    assert_eq!(store.flares(day(0)).expect("flares"), Some(revised));
    let days = store.archived_days().expect("days");
    assert_eq!(days.len(), 1, "the day is indexed once");
    assert_eq!(days.first().map(|entry| entry.fetched_at), Some(revised_at));
}

/// A replacement of a different length must not spill into the days around
/// it.
#[test]
fn replacing_a_day_leaves_the_days_around_it_alone() {
    let (_dir, store) = store().unwrap();
    for offset in [0, 1, 2] {
        store
            .insert_or_replace_day(
                day(offset),
                HOST,
                fetched_at(),
                &flare_day(day(offset)).expect("a day of flares"),
            )
            .expect("store");
    }

    let partial: Vec<SolarFlare> = flare_day(day(1))
        .expect("a day of flares")
        .into_iter()
        .take(1)
        .collect();
    store
        .insert_or_replace_day(day(1), HOST, fetched_at(), &partial)
        .expect("store partial");

    assert_eq!(
        store.flares(day(0)).expect("flares"),
        Some(flare_day(day(0)).expect("a day of flares"))
    );
    assert_eq!(store.flares(day(1)).expect("flares"), Some(partial));
    assert_eq!(
        store.flares(day(2)).expect("flares"),
        Some(flare_day(day(2)).expect("a day of flares"))
    );
}

/// Store order does not determine read order.
#[test]
fn archived_days_come_back_oldest_first_with_their_provenance() {
    let (_dir, store) = store().unwrap();
    for offset in [2, 0, 1] {
        store
            .insert_or_replace_day(
                day(offset),
                HOST,
                fetched_at(),
                &flare_day(day(offset)).expect("a day of flares"),
            )
            .expect("store");
    }

    let archived = store.archived_days().expect("days");
    assert_eq!(
        archived
            .iter()
            .map(|entry| entry.day)
            .collect::<Vec<NaiveDate>>(),
        [day(0), day(1), day(2)]
    );
    let first = archived.first().expect("one day");
    assert_eq!(first.flares, 2);
    assert_eq!(first.host, HOST);
    assert_eq!(first.fetched_at, fetched_at());
}

/// Both threads write through the one archive: the lock serializes each
/// store's read-append-index sequence.
#[test]
fn days_stored_from_two_threads_both_reach_the_archive() {
    let (_dir, store) = store().unwrap();
    let store = &store;
    std::thread::scope(|scope| {
        for offset in [0, 1] {
            scope.spawn(move || {
                store
                    .insert_or_replace_day(
                        day(offset),
                        HOST,
                        fetched_at(),
                        &flare_day(day(offset)).expect("a day of flares"),
                    )
                    .expect("store");
            });
        }
    });

    let days: Vec<NaiveDate> = store
        .archived_days()
        .expect("days")
        .into_iter()
        .map(|entry| entry.day)
        .collect();
    assert_eq!(days, [day(0), day(1)]);
    assert_eq!(
        store.flares(day(1)).expect("second day"),
        Some(flare_day(day(1)).expect("a day of flares"))
    );
}

#[test]
fn an_archive_reopens_with_its_days() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    {
        let store = FlareStore::open_or_create(&path).expect("create");
        store
            .insert_or_replace_day(
                day(0),
                HOST,
                fetched_at(),
                &flare_day(day(0)).expect("a day of flares"),
            )
            .expect("store");
    }
    let reopened = FlareStore::open_or_create(&path).expect("reopen");
    assert_eq!(
        reopened.flares(day(0)).expect("flares"),
        Some(flare_day(day(0)).expect("a day of flares"))
    );
}

#[rstest]
#[case::the_catalog_coverage_start(NaiveDate::from_ymd_opt(2010, 4, 3))]
#[case::before_epoch(NaiveDate::from_ymd_opt(1969, 12, 31))]
#[case::far_future(NaiveDate::from_ymd_opt(2999, 1, 1))]
fn any_date_round_trips_through_the_day_index(#[case] date: Option<NaiveDate>) {
    let date = date.expect("date");
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_day(
            date,
            HOST,
            fetched_at(),
            &flare_day(date).expect("a day of flares"),
        )
        .expect("store");
    assert_eq!(
        store
            .archived_days()
            .expect("days")
            .first()
            .map(|entry| entry.day),
        Some(date)
    );
}

/// An archive written by a newer build is refused.
#[test]
fn a_newer_schema_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    FlareStore::open_or_create(&path).expect("create");
    {
        let file = hdf5::File::open_rw(&path).expect("reopen");
        let attr = file.attr(schema::SCHEMA_VERSION_ATTR).expect("attr");
        attr.write_scalar(&(schema::CURRENT_SCHEMA_VERSION + 1))
            .expect("bump");
    }
    let err = FlareStore::open_or_create(&path).expect_err("refuse");
    assert!(
        matches!(err, FlareStoreError::SchemaTooNew { found, supported }
            if found == schema::CURRENT_SCHEMA_VERSION + 1
                && supported == schema::CURRENT_SCHEMA_VERSION),
        "{err}"
    );
}

/// Events appended without an index entry, which is what an interrupted store
/// leaves, are cut when the archive is reopened.
#[test]
fn unindexed_events_are_dropped_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    {
        let store = FlareStore::open_or_create(&path).unwrap();
        store
            .insert_or_replace_day(
                day(0),
                HOST,
                fetched_at(),
                &flare_day(day(0)).expect("a day of flares"),
            )
            .unwrap();
    }
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        let group = file.group(schema::EVENTS_GROUP).unwrap();
        for name in schema::EVENT_COLUMNS {
            let dataset = group.dataset(name).unwrap();
            let rows = dataset.shape().first().copied().unwrap_or_default();
            dataset.resize([rows + 5]).unwrap();
        }
    }

    let reopened = FlareStore::open_or_create(&path).unwrap();
    assert_eq!(
        reopened.flares(day(0)).unwrap(),
        Some(flare_day(day(0)).expect("a day of flares")),
        "the archived day survives"
    );
    let file = hdf5::File::open(&path).unwrap();
    let rows = file
        .group(schema::EVENTS_GROUP)
        .unwrap()
        .dataset(schema::EVENT_PEAK)
        .unwrap()
        .shape()
        .first()
        .copied()
        .unwrap_or_default();
    assert_eq!(rows, 2, "the unindexed events are gone");
}

/// A column shorter than the index means archived events are missing, which
/// no recovery can invent.
#[test]
fn a_column_shorter_than_the_index_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    {
        let store = FlareStore::open_or_create(&path).unwrap();
        store
            .insert_or_replace_day(
                day(0),
                HOST,
                fetched_at(),
                &flare_day(day(0)).expect("a day of flares"),
            )
            .unwrap();
    }
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        file.group(schema::EVENTS_GROUP)
            .unwrap()
            .dataset(schema::EVENT_MAGNITUDE)
            .unwrap()
            .resize([1])
            .unwrap();
    }

    let err = FlareStore::open_or_create(&path).expect_err("truncated column");
    assert!(matches!(err, FlareStoreError::Corrupt(_)), "{err}");
}

/// A code the schema does not define is reported as an inconsistent archive,
/// never read as a flare with an invented field.
#[rstest]
#[case::end_presence(schema::EVENT_END_PRESENCE, 9, "flare row 0 has end presence code 9")]
#[case::source_location_presence(
    schema::EVENT_SOURCE_LOCATION_PRESENCE,
    8,
    "flare row 0 has source location presence code 8"
)]
#[case::active_region_presence(
    schema::EVENT_ACTIVE_REGION_PRESENCE,
    7,
    "flare row 0 has active region presence code 7"
)]
#[case::class(schema::EVENT_CLASS, 6, "flare row 0 has class code 6")]
fn an_undecodable_event_is_reported(
    #[case] column: &str,
    #[case] code: u8,
    #[case] expected: &str,
) {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_day(
            day(0),
            HOST,
            fetched_at(),
            &flare_day(day(0)).expect("a day of flares"),
        )
        .unwrap();
    {
        let file = hdf5::File::open_rw(store.path()).unwrap();
        file.group(schema::EVENTS_GROUP)
            .unwrap()
            .dataset(column)
            .unwrap()
            .write_slice(&[code], 0..1)
            .unwrap();
    }

    let err = store.flares(day(0)).expect_err("refuse");
    assert_eq!(
        err.to_string(),
        format!("archive is inconsistent: {expected}")
    );
}

/// Days go from the front and the flares of the rest read back through the
/// offsets the delete rebased, text columns included.
#[test]
fn deleting_days_before_a_cutoff_keeps_the_flares_of_the_rest() {
    let (_dir, store) = store().unwrap();
    let kept = flare_day(day(1)).expect("a day of flares");
    for (day, flares) in [
        (day(0), flare_day(day(0)).expect("a day of flares")),
        (day(1), kept.clone()),
    ] {
        store
            .insert_or_replace_day(day, HOST, fetched_at(), &flares)
            .expect("store");
    }

    let removed = store.delete_days_before(day(1), None).expect("delete days");

    assert_eq!(removed, 1);
    assert_eq!(store.flares(day(0)).expect("flares"), None);
    assert_eq!(store.flares(day(1)).expect("flares"), Some(kept));
}

#[test]
fn deleting_every_day_empties_the_archive() {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_day(
            day(0),
            HOST,
            fetched_at(),
            &flare_day(day(0)).expect("a day of flares"),
        )
        .expect("store");

    let removed = store.delete_all_days(None).expect("delete all");

    assert_eq!(removed, 1);
    assert!(store.archived_days().expect("days").is_empty());
    assert_eq!(store.flares(day(0)).expect("flares"), None);
}
