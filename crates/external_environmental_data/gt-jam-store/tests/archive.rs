//! Round-trip days through a real archive file in a temp directory.

use std::fs;
use std::str::FromStr as _;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use h3o::CellIndex;
use rstest::rstest;

use gt_hdf5_archive::day_index;
use gt_hdf5_archive::prune::{
    DeclinedRecovery, DeleteState, InterruptedDelete, InterruptedDeleteRecovery,
};
use gt_hdf5_archive::{ReadOnlyDayArchive as _, WritableDayArchive as _};
use gt_jam::wire::HexObservation;
use gt_jam_store::{FILE_NAME, JamStore, JamStoreError, ReadOnlyJamStore, schema};
use gt_test_utils::day_archive::conformance::{self, StoredDayOperations};
use gt_test_utils::day_archive::{self, ColumnName, GroupPath};

/// Cells copied from the captured day.
const CELLS: [&str; 4] = [
    "84005c7ffffffff",
    "840104bffffffff",
    "8401221ffffffff",
    "8401255ffffffff",
];

const HOST: &str = "https://gpsjam.org";

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or_default() + TimeDelta::days(offset)
}

fn fetched_at() -> DateTime<Utc> {
    day_archive::fetched_at()
}

/// `count` observations with distinct cells and recognisable counts. A cell
/// that fails to parse is dropped, which the callers' count assertions catch.
fn observations(count: usize) -> Vec<HexObservation> {
    CELLS
        .iter()
        .take(count)
        .enumerate()
        .filter_map(|(index, hex)| {
            let index = u32::try_from(index).ok()?;
            Some(HexObservation {
                cell: CellIndex::from_str(hex).ok()?,
                good: 100 + index,
                bad: index,
            })
        })
        .collect()
}

/// The archive's day index, which is where a delete records that it is
/// part-way through.
const DAYS: GroupPath<'static> = GroupPath(schema::DAYS_GROUP);

const DAY_OPERATIONS: StoredDayOperations<JamStore, Vec<HexObservation>> = StoredDayOperations {
    insert_a_day,
    read_a_day,
    indexed_days,
};

fn insert_a_day(store: &JamStore, day: NaiveDate) -> Result<Vec<HexObservation>, String> {
    let observations = observations(4);
    store
        .insert_day(day, HOST, fetched_at(), &observations)
        .map_err(|err| format!("insert {day}: {err}"))?;
    Ok(observations)
}

fn read_a_day(store: &JamStore, day: NaiveDate) -> Result<Option<Vec<HexObservation>>, String> {
    store
        .observations(day)
        .map_err(|err| format!("read {day}: {err}"))
}

fn indexed_days(store: &JamStore) -> Result<Vec<NaiveDate>, String> {
    Ok(store
        .days()
        .map_err(|err| format!("days: {err}"))?
        .into_iter()
        .map(|stored| stored.day)
        .collect())
}

#[test]
fn a_new_archive_is_empty() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    assert!(store.days().expect("days").is_empty());
    assert!(!store.contains(day(0)).expect("contains"));
    assert_eq!(store.observations(day(0)).expect("observations"), None);
}

#[test]
fn a_stored_day_round_trips() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    let written = observations(4);
    store
        .insert_day(day(0), HOST, fetched_at(), &written)
        .expect("insert");

    assert!(store.contains(day(0)).expect("contains"));
    assert_eq!(
        store.observations(day(0)).expect("observations"),
        Some(written)
    );
}

#[test]
fn days_are_indexed_with_their_provenance() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(3))
        .expect("insert");

    let days = store.days().expect("days");
    assert_eq!(days.len(), 1);
    let stored = days.first().expect("one day");
    assert_eq!(stored.day, day(0));
    assert_eq!(stored.cells, 3);
    assert_eq!(stored.host, HOST);
    assert_eq!(stored.fetched_at, fetched_at());
}

/// Rows of one day must not leak into another's slice.
#[test]
fn days_stored_together_stay_separate() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(4))
        .expect("insert first");
    store
        .insert_day(day(1), HOST, fetched_at(), &observations(2))
        .expect("insert second");

    assert_eq!(
        store.observations(day(0)).expect("first").map(|o| o.len()),
        Some(4)
    );
    assert_eq!(
        store.observations(day(1)).expect("second"),
        Some(observations(2))
    );
}

/// Ingest order does not determine read order.
#[test]
fn days_come_back_oldest_first() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    for offset in [2, 0, 1] {
        store
            .insert_day(day(offset), HOST, fetched_at(), &observations(1))
            .expect("insert");
    }
    let days: Vec<NaiveDate> = store
        .days()
        .expect("days")
        .into_iter()
        .map(|stored| stored.day)
        .collect();
    assert_eq!(days, [day(0), day(1), day(2)]);
}

#[test]
fn a_day_cannot_be_stored_twice() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(2))
        .expect("insert");
    let err = store
        .insert_day(day(0), HOST, fetched_at(), &observations(2))
        .expect_err("second insert");
    assert!(
        matches!(err, JamStoreError::DayAlreadyStored { day: stored } if stored == day(0)),
        "{err}"
    );
}

/// A rejected insert must not leave rows behind.
#[test]
fn a_rejected_insert_leaves_the_archive_unchanged() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(3))
        .expect("insert");
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(1))
        .expect_err("duplicate");

    assert_eq!(store.days().expect("days").len(), 1);
    assert_eq!(
        store.observations(day(0)).expect("observations"),
        Some(observations(3))
    );
}

/// A day the host published as empty is still a stored day, distinct from
/// one never fetched.
#[test]
fn a_day_with_no_cells_is_still_stored() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &[])
        .expect("insert");
    assert!(store.contains(day(0)).expect("contains"));
    assert_eq!(
        store.observations(day(0)).expect("observations"),
        Some(vec![])
    );
}

/// Both threads write through the one archive: the lock serializes each
/// insert's read-append-index sequence.
#[test]
fn days_inserted_from_two_threads_both_reach_the_archive() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    let store = &store;
    std::thread::scope(|scope| {
        for offset in [0, 1] {
            scope.spawn(move || {
                store
                    .insert_day(day(offset), HOST, fetched_at(), &observations(2))
                    .expect("insert");
            });
        }
    });

    let days: Vec<NaiveDate> = store
        .days()
        .expect("days")
        .into_iter()
        .map(|stored| stored.day)
        .collect();
    assert_eq!(days, [day(0), day(1)]);
    assert_eq!(
        store.observations(day(1)).expect("second day"),
        Some(observations(2))
    );
}

#[test]
fn an_archive_reopens_with_its_days() {
    conformance::an_archive_reopens_with_its_days(&DAY_OPERATIONS, day(0));
}

#[test]
fn a_stored_day_indexes_for_lookup() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(4))
        .expect("insert");

    let dataset = store.dataset(day(0)).expect("dataset").expect("stored");
    assert_eq!(dataset.day(), day(0));
    assert_eq!(dataset.len(), 4);
    let cell = CellIndex::from_str(CELLS[0]).expect("cell index");
    assert_eq!(dataset.observation(cell).map(|found| found.good), Some(100));
}

#[rstest]
#[case::before_epoch(NaiveDate::from_ymd_opt(1969, 12, 31))]
#[case::coverage_start(NaiveDate::from_ymd_opt(2022, 2, 14))]
#[case::far_future(NaiveDate::from_ymd_opt(2999, 1, 1))]
fn any_date_round_trips_through_the_day_index(#[case] date: Option<NaiveDate>) {
    conformance::any_date_round_trips_through_the_day_index(&DAY_OPERATIONS, date.expect("date"));
}

#[test]
fn a_newer_schema_is_rejected() {
    conformance::a_newer_schema_is_rejected::<JamStore>();
}

/// Rows appended without an index entry, which is what an interrupted
/// insert leaves, are cut when the archive is reopened.
#[test]
fn unindexed_rows_are_dropped_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    {
        let store = JamStore::open_or_create(&path).unwrap();
        store
            .insert_day(day(0), HOST, fetched_at(), &observations(2))
            .unwrap();
    }
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        let group = file.group(schema::OBSERVATIONS_GROUP).unwrap();
        for name in schema::OBSERVATION_COLUMNS {
            let dataset = group.dataset(name).unwrap();
            let rows = dataset.shape().first().copied().unwrap_or_default();
            dataset.resize([rows + 5]).unwrap();
        }
    }

    let reopened = JamStore::open_or_create(&path).unwrap();
    assert_eq!(
        reopened.observations(day(0)).unwrap(),
        Some(observations(2)),
        "the stored day survives"
    );
    let file = hdf5::File::open(&path).unwrap();
    let rows = file
        .group(schema::OBSERVATIONS_GROUP)
        .unwrap()
        .dataset(schema::OBS_CELL)
        .unwrap()
        .shape()
        .first()
        .copied()
        .unwrap_or_default();
    assert_eq!(rows, 2, "the unindexed rows are gone");
}

/// Columns shorter than the index means indexed rows are missing, which no
/// recovery can invent.
#[test]
fn a_column_shorter_than_the_index_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    {
        let store = JamStore::open_or_create(&path).unwrap();
        store
            .insert_day(day(0), HOST, fetched_at(), &observations(4))
            .unwrap();
    }
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        let dataset = file
            .group(schema::OBSERVATIONS_GROUP)
            .unwrap()
            .dataset(schema::OBS_CELL)
            .unwrap();
        dataset.resize([1]).unwrap();
    }

    let err = JamStore::open_or_create(&path).expect_err("truncated column");
    assert!(matches!(err, JamStoreError::Corrupt(_)), "{err}");
}

/// A cell index the archive cannot decode is reported, not passed on.
#[test]
fn an_undecodable_cell_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let store = JamStore::open_or_create(&path).unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(2))
        .unwrap();
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        let dataset = file
            .group(schema::OBSERVATIONS_GROUP)
            .unwrap()
            .dataset(schema::OBS_CELL)
            .unwrap();
        dataset.write_slice(&[0_u64], 0..1).unwrap();
    }

    let err = store.observations(day(0)).expect_err("undecodable cell");
    assert!(matches!(err, JamStoreError::Corrupt(_)), "{err}");
}

/// Days go from the front and the rest keep their observations, read back
/// through the offsets the delete rebased.
#[rstest]
#[case::the_oldest(1, vec![day(1), day(2)])]
#[case::all_but_the_newest(2, vec![day(2)])]
#[case::none_of_them(0, vec![day(0), day(1), day(2)])]
fn deleting_days_before_a_cutoff_keeps_the_rest(
    #[case] cutoff_offset: i64,
    #[case] expected: Vec<NaiveDate>,
) {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    for offset in 0..3 {
        let count = usize::try_from(offset).unwrap_or_default() + 1;
        store
            .insert_day(day(offset), HOST, fetched_at(), &observations(count))
            .expect("insert");
    }

    let removed = store
        .delete_days_before(day(cutoff_offset), None)
        .expect("delete days");

    assert_eq!(removed, 3 - expected.len());
    assert_eq!(
        store
            .days()
            .expect("days")
            .into_iter()
            .map(|stored| stored.day)
            .collect::<Vec<NaiveDate>>(),
        expected
    );
    for day in expected {
        let count = usize::try_from((day - self::day(0)).num_days()).unwrap_or_default() + 1;
        assert_eq!(
            store.observations(day).expect("observations"),
            Some(observations(count)),
            "{day} lost its observations"
        );
    }
}

#[test]
fn deleting_every_day_empties_the_archive() {
    conformance::deleting_every_day_empties_the_archive(&DAY_OPERATIONS, &[day(0), day(1), day(2)]);
}

/// A day deleted and fetched again is stored where the delete freed room for
/// it, and reads back on its own.
#[test]
fn a_day_can_be_stored_again_after_it_was_deleted() {
    let (_dir, store) = day_archive::store_in_a_temp_dir::<JamStore>().unwrap();
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(4))
        .expect("insert");
    store
        .insert_day(day(1), HOST, fetched_at(), &observations(2))
        .expect("insert");

    store.delete_days_before(day(1), None).expect("delete days");
    store
        .insert_day(day(0), HOST, fetched_at(), &observations(3))
        .expect("insert again");

    assert_eq!(
        store.observations(day(0)).expect("observations"),
        Some(observations(3))
    );
    assert_eq!(
        store.observations(day(1)).expect("observations"),
        Some(observations(2))
    );
}

/// Write access taken from an instance part-way through a delete must not
/// discard its days behind the user's back. The archive reports what
/// recovering costs before either choice is made.
#[test]
fn a_declined_recovery_keeps_the_interrupted_days_and_an_accepted_one_discards_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let store = JamStore::open_or_create(&path).expect("open");
    for offset in 0..2 {
        store
            .insert_day(day(offset), HOST, fetched_at(), &observations(2))
            .expect("insert");
    }
    drop(store);
    day_archive::mark_delete_in_flight(&path, DAYS).expect("mark the delete");
    let interrupted_bytes = fs::read(&path).expect("archive bytes");

    let interrupted = ReadOnlyJamStore::interrupted_delete_at(&path).expect("inspect");
    let declined =
        JamStore::open_or_create_with_recovery_choice(&path, InterruptedDeleteRecovery::Decline)
            .expect_err("the archive is unavailable until the recovery is accepted");

    assert_eq!(interrupted, Some(InterruptedDelete { archived_days: 2 }));
    assert!(
        fs::read(&path).expect("archive bytes") == interrupted_bytes,
        "inspecting and declining wrote to the archive"
    );
    assert!(
        matches!(
            declined,
            JamStoreError::DeclinedRecovery(DeclinedRecovery(InterruptedDelete {
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
            GroupPath(schema::OBSERVATIONS_GROUP),
            ColumnName(schema::OBS_CELL)
        )
        .expect("observations"),
        4
    );

    let store = JamStore::open_or_create(&path).expect("open accepting the recovery");
    assert!(store.days().expect("days").is_empty());
    assert_eq!(store.observations(day(0)).expect("observations"), None);
}

#[test]
fn a_settled_archive_reports_no_interrupted_delete() {
    conformance::a_settled_archive_reports_no_interrupted_delete(&DAY_OPERATIONS, day(0));
}
