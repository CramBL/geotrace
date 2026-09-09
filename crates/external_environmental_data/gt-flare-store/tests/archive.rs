//! Round-trip flare days through a real archive file in a temp directory.

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use rstest::rstest;

use gt_flare::SolarFlare;
use gt_flare::class::FlareClassification;
use gt_flare_store::{FILE_NAME, FlareStore, FlareStoreError, ReadOnlyFlareStore, schema};
use gt_hdf5_archive::day_index;
use gt_hdf5_archive::prune::{
    DeclinedRecovery, DeleteState, InterruptedDelete, InterruptedDeleteRecovery,
};
use gt_hdf5_archive::{ReadOnlyDayArchive as _, WritableDayArchive as _};
use gt_test_utils::day_archive::conformance::{self, StoredDayOperations};
use gt_test_utils::day_archive::{self, ColumnName, GroupPath};

/// The base URL the archive records. The API key is never part of it.
const HOST: &str = "https://api.nasa.gov";

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 5, 9).unwrap_or_default() + TimeDelta::days(offset)
}

fn fetched_at() -> DateTime<Utc> {
    day_archive::fetched_at()
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

const DAY_OPERATIONS: StoredDayOperations<FlareStore, Vec<SolarFlare>> = StoredDayOperations {
    insert_a_day,
    read_a_day,
    indexed_days,
};

fn insert_a_day(store: &FlareStore, day: NaiveDate) -> Result<Vec<SolarFlare>, String> {
    let flares = flare_day(day).ok_or_else(|| format!("a day of flares for {day}"))?;
    store
        .insert_or_replace_day(day, HOST, fetched_at(), &flares)
        .map_err(|err| format!("store {day}: {err}"))?;
    Ok(flares)
}

fn read_a_day(store: &FlareStore, day: NaiveDate) -> Result<Option<Vec<SolarFlare>>, String> {
    store
        .flares(day)
        .map_err(|err| format!("read {day}: {err}"))
}

fn indexed_days(store: &FlareStore) -> Result<Vec<NaiveDate>, String> {
    Ok(store
        .archived_days()
        .map_err(|err| format!("archived days: {err}"))?
        .into_iter()
        .map(|entry| entry.day)
        .collect())
}

#[test]
fn a_new_archive_holds_no_day() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
    assert!(store.archived_days().expect("days").is_empty());
    assert!(!store.contains(day(0)).expect("contains"));
    assert_eq!(store.flares(day(0)).expect("flares"), None);
}

/// Every field survives the round trip, the ones the catalog left off
/// included.
#[test]
fn a_day_round_trips_with_the_fields_the_catalog_left_off() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
    let written = flare_day(day(0)).expect("a day of flares");
    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), &written)
        .expect("store");

    let read = store.flares(day(0)).expect("flares").expect("archived");
    assert_eq!(read, written);
    assert!(store.contains(day(0)).expect("contains"));
}

/// A day without a flare in the catalog is still an archived day, distinct
/// from one never fetched.
#[test]
fn a_day_without_flares_is_still_archived() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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
    conformance::an_archive_reopens_with_its_days(&DAY_OPERATIONS, day(0));
}

#[rstest]
#[case::the_catalog_coverage_start(NaiveDate::from_ymd_opt(2010, 4, 3))]
#[case::before_epoch(NaiveDate::from_ymd_opt(1969, 12, 31))]
#[case::far_future(NaiveDate::from_ymd_opt(2999, 1, 1))]
fn any_date_round_trips_through_the_day_index(#[case] date: Option<NaiveDate>) {
    conformance::any_date_round_trips_through_the_day_index(&DAY_OPERATIONS, date.expect("date"));
}

#[test]
fn a_newer_schema_is_rejected() {
    conformance::a_newer_schema_is_rejected::<FlareStore>();
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
/// never decoded as a flare with an invented field.
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
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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

    let err = store.flares(day(0)).expect_err("reject");
    assert_eq!(
        err.to_string(),
        format!("archive is inconsistent: {expected}")
    );
}

/// Days go from the front and the flares of the rest read back through the
/// offsets the delete rebased, text columns included.
#[test]
fn deleting_days_before_a_cutoff_keeps_the_flares_of_the_rest() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<FlareStore>().unwrap();
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
    conformance::deleting_every_day_empties_the_archive(&DAY_OPERATIONS, &[day(0)]);
}

/// The archive's day index, which is where a delete records that it is
/// part-way through.
const DAYS: GroupPath<'static> = GroupPath(schema::DAYS_GROUP);

/// Write access taken from an instance part-way through a delete must not
/// discard its days behind the user's back. The archive reports what
/// recovering costs before either choice is made.
#[test]
fn a_declined_recovery_keeps_the_interrupted_days_and_an_accepted_one_discards_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let store = FlareStore::open_or_create(&path).expect("open");
    for offset in 0..2 {
        store
            .insert_or_replace_day(
                day(offset),
                HOST,
                fetched_at(),
                &flare_day(day(offset)).expect("a day of flares"),
            )
            .expect("store");
    }
    drop(store);
    day_archive::mark_delete_in_flight(&path, DAYS).expect("mark the delete");

    let interrupted = ReadOnlyFlareStore::interrupted_delete_at(&path).expect("inspect");
    let declined =
        FlareStore::open_or_create_with_recovery_choice(&path, InterruptedDeleteRecovery::Decline)
            .expect_err("the archive is unavailable until the recovery is accepted");

    assert_eq!(interrupted, Some(InterruptedDelete { archived_days: 2 }));
    assert!(
        matches!(
            declined,
            FlareStoreError::DeclinedRecovery(DeclinedRecovery(InterruptedDelete {
                archived_days: 2
            }))
        ),
        "{declined:#}"
    );
    assert_eq!(
        day_archive::delete_state(&path, DAYS).expect("state"),
        DeleteState::InFlight
    );
    assert_eq!(
        day_archive::column_rows(&path, DAYS, ColumnName(day_index::DAY)).expect("indexed days"),
        2
    );
    assert_eq!(
        day_archive::column_rows(
            &path,
            GroupPath(schema::EVENTS_GROUP),
            ColumnName(schema::EVENT_BEGIN)
        )
        .expect("events"),
        4
    );

    let store = FlareStore::open_or_create(&path).expect("open accepting the recovery");
    assert!(store.archived_days().expect("days").is_empty());
}

#[test]
fn a_settled_archive_reports_no_interrupted_delete() {
    conformance::a_settled_archive_reports_no_interrupted_delete(&DAY_OPERATIONS, day(0));
}
