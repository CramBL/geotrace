//! The column primitives, against a real HDF5 file in a temp directory.

use chrono::{DateTime, NaiveDate, Utc};
use hdf5::{File, Group};
use rstest::rstest;
use tempfile::TempDir;

use gt_hdf5_archive::day_index::{DayIndex, RowPlacement};
use gt_hdf5_archive::{Column, ColumnFormat, attributes, dates};

const FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 8,
    deflate_level: 6,
};

const VALUES: &str = "values";
const HOST: &str = "https://example.invalid";

fn archive() -> Result<(TempDir, File), String> {
    let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
    let file =
        File::create(dir.path().join("archive.h5")).map_err(|err| format!("create: {err}"))?;
    Ok((dir, file))
}

fn value_column(file: &File) -> Result<Group, String> {
    let group = file
        .create_group("data")
        .map_err(|err| format!("group: {err}"))?;
    Column::create::<u32>(&group, VALUES, FORMAT).map_err(|err| format!("column: {err}"))?;
    Ok(group)
}

fn day_index(file: &File) -> Result<Group, String> {
    let group = file
        .create_group("days")
        .map_err(|err| format!("group: {err}"))?;
    DayIndex::create_columns(&group, FORMAT).map_err(|err| format!("day index: {err}"))?;
    Ok(group)
}

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 16).unwrap_or_default() + chrono::TimeDelta::days(offset)
}

#[test]
fn a_column_holds_everything_appended_to_it() {
    let (_dir, file) = archive().unwrap();
    let group = value_column(&file).unwrap();
    let column = Column::new(&group, VALUES);

    column.append(&[1_u32, 2, 3]).expect("first append");
    column.append(&[4_u32]).expect("second append");

    assert_eq!(column.rows().expect("rows"), 4);
    assert_eq!(column.read::<u32>().expect("read"), [1, 2, 3, 4]);
    assert_eq!(column.read_slice::<u32>(1..3).expect("slice"), [2, 3]);
}

/// HDF5 returns fewer values than requested for an out-of-range slice, which
/// would silently shorten a day.
#[test]
fn reading_past_the_end_of_a_column_is_rejected() {
    let (_dir, file) = archive().unwrap();
    let group = value_column(&file).unwrap();
    let column = Column::new(&group, VALUES);
    column.append(&[1_u32, 2]).expect("append");

    let err = column.read_slice::<u32>(1..3).expect_err("past the end");
    assert_eq!(
        err.to_string(),
        "archive is inconsistent: values holds 2 rows, requested 1..3"
    );
}

#[test]
fn a_row_written_again_replaces_what_it_held() {
    let (_dir, file) = archive().unwrap();
    let group = value_column(&file).unwrap();
    let column = Column::new(&group, VALUES);
    column.append(&[1_u32, 2, 3]).expect("append");

    column.write_row(1, 9_u32).expect("write row");
    assert_eq!(column.read::<u32>().expect("read"), [1, 9, 3]);
}

#[test]
fn an_attribute_reads_back_and_an_absent_one_is_none() {
    let (_dir, file) = archive().unwrap();
    assert_eq!(attributes::read_i64(&file, "schema_version"), None);

    attributes::write_i64(&file, "schema_version", 3).expect("write");
    assert_eq!(attributes::read_i64(&file, "schema_version"), Some(3));
}

#[test]
fn a_day_is_indexed_with_its_extent_and_provenance() {
    let (_dir, file) = archive().unwrap();
    let group = day_index(&file).unwrap();
    let index = DayIndex::new(&group);
    let fetched_at = DateTime::<Utc>::default();

    index
        .insert_or_replace(
            day(0),
            RowPlacement { offset: 0, rows: 3 },
            fetched_at,
            HOST,
        )
        .expect("insert");

    assert_eq!(index.row_of(day(0)).expect("row"), Some(0));
    assert_eq!(index.extent_of(day(0)).expect("extent"), Some(0..3));
    assert_eq!(index.extent_of(day(1)).expect("extent"), None);
    let entries = index.entries().expect("entries");
    assert_eq!(entries.len(), 1);
    let entry = entries.first().expect("one entry");
    assert_eq!(entry.day, day(0));
    assert_eq!(entry.rows, 3);
    assert_eq!(entry.fetched_at, fetched_at);
    assert_eq!(entry.host, HOST);
}

/// An entry stored again points at the rows appended for it, and the index
/// keeps one row for the day.
#[test]
fn a_day_stored_again_is_repointed_in_place() {
    let (_dir, file) = archive().unwrap();
    let group = day_index(&file).unwrap();
    let index = DayIndex::new(&group);

    for placement in [
        RowPlacement { offset: 0, rows: 3 },
        RowPlacement { offset: 3, rows: 2 },
    ] {
        index
            .insert_or_replace(day(0), placement, DateTime::<Utc>::default(), HOST)
            .expect("store");
    }

    assert_eq!(index.extent_of(day(0)).expect("extent"), Some(3..5));
    assert_eq!(index.entries().expect("entries").len(), 1);
}

#[test]
fn entries_come_back_oldest_first() {
    let (_dir, file) = archive().unwrap();
    let group = day_index(&file).unwrap();
    let index = DayIndex::new(&group);

    for offset in [2, 0, 1] {
        index
            .insert_or_replace(
                day(offset),
                RowPlacement { offset: 0, rows: 1 },
                DateTime::<Utc>::default(),
                HOST,
            )
            .expect("store");
    }

    assert_eq!(
        index
            .entries()
            .expect("entries")
            .into_iter()
            .map(|entry| entry.day)
            .collect::<Vec<NaiveDate>>(),
        [day(0), day(1), day(2)]
    );
}

#[rstest]
#[case::epoch(0, NaiveDate::from_ymd_opt(1970, 1, 1))]
#[case::before_epoch(-1, NaiveDate::from_ymd_opt(1969, 12, 31))]
#[case::past_the_calendar(i32::MAX, None)]
fn a_stored_day_reads_back_as_its_date(#[case] days: i32, #[case] expected: Option<NaiveDate>) {
    assert_eq!(dates::date_from_epoch_days(days).ok(), expected);
}

#[rstest]
#[case::epoch(0, true)]
#[case::before_epoch(-1, true)]
#[case::past_the_representable_range(i64::MAX, false)]
fn a_stored_timestamp_reads_back_as_an_instant(#[case] seconds: i64, #[case] readable: bool) {
    assert_eq!(
        dates::timestamp_from_seconds(seconds)
            .map(|timestamp| timestamp.timestamp())
            .ok(),
        readable.then_some(seconds)
    );
}
