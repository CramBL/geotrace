//! Removing days from real archive files in a temp directory.

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, Utc};
use hdf5::types::VarLenUnicode;
use hdf5::{File, Group};
use rstest::rstest;
use tempfile::TempDir;

use gt_hdf5_archive::day_index::{self, DayIndex, RowPlacement};
use gt_hdf5_archive::prune::{ArchiveLayout, DeleteState, ExtentColumns, PruneProgress, RowLevel};
use gt_hdf5_archive::{ArchiveError, Column, ColumnFormat};

const FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 4_096,
    deflate_level: 6,
};

const DAYS: &str = "days";
const NOTE: &str = "note";
const ROWS: &str = "rows";
const VALUE: &str = "value";
const LABEL: &str = "label";
const HOST: &str = "https://example.invalid";

const DAY_COLUMNS: [&str; 1] = [NOTE];
const ROW_COLUMNS: [&str; 2] = [VALUE, LABEL];

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 10).unwrap_or_default() + chrono::TimeDelta::days(offset)
}

/// Values whose file size follows the rows they fill: they do not compress.
fn values_of(day: NaiveDate, rows: usize) -> Vec<u64> {
    let seed = u64::try_from(day.to_epoch_days()).unwrap_or_default();
    (0..rows)
        .map(|row| (seed * 1_000 + row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .collect()
}

fn labels_of(day: NaiveDate, rows: usize) -> Vec<String> {
    (0..rows).map(|row| format!("{day}-{row}")).collect()
}

fn stored_labels(labels: &[String]) -> Result<Vec<VarLenUnicode>, String> {
    labels
        .iter()
        .map(|label| {
            label
                .parse::<VarLenUnicode>()
                .map_err(|err| format!("label {label:?}: {err}"))
        })
        .collect()
}

/// One day's rows as they read back.
#[derive(Debug, PartialEq, Eq)]
struct StoredRows {
    values: Vec<u64>,
    labels: Vec<String>,
}

impl StoredRows {
    /// What [`TestArchive::insert_day`] stored for `day`.
    fn of(day: NaiveDate, rows: usize) -> Self {
        Self {
            values: values_of(day, rows),
            labels: labels_of(day, rows),
        }
    }
}

/// A day-keyed archive of one row level, the shape three of the four stores
/// have.
struct TestArchive {
    _dir: TempDir,
    path: PathBuf,
    /// Held open between operations, and closed to measure the file: libhdf5
    /// cuts the file back to what it needs when it closes it.
    file: Option<File>,
    row_columns: &'static [&'static str],
}

impl TestArchive {
    fn create() -> Result<Self, String> {
        Self::holding(&ROW_COLUMNS)
    }

    /// An archive of fixed-width rows alone, the shape the archives that grow
    /// large have. Only these can shrink: libhdf5 never hands back the heap a
    /// column of strings takes.
    fn create_without_text() -> Result<Self, String> {
        Self::holding(&[VALUE])
    }

    fn holding(row_columns: &'static [&'static str]) -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
        let path = dir.path().join("archive.h5");
        let file = File::create(&path).map_err(|err| format!("create: {err}"))?;

        let days = file
            .create_group(DAYS)
            .map_err(|err| format!("days group: {err}"))?;
        DayIndex::create_columns(&days, FORMAT).map_err(|err| format!("day index: {err}"))?;
        Column::create::<i64>(&days, NOTE, FORMAT).map_err(|err| format!("note: {err}"))?;

        let rows = file
            .create_group(ROWS)
            .map_err(|err| format!("rows group: {err}"))?;
        Column::create::<u64>(&rows, VALUE, FORMAT).map_err(|err| format!("value: {err}"))?;
        if row_columns.contains(&LABEL) {
            Column::create_strings(&rows, LABEL, FORMAT).map_err(|err| format!("label: {err}"))?;
        }
        Ok(Self {
            _dir: dir,
            path,
            file: Some(file),
            row_columns,
        })
    }

    fn file(&self) -> Result<&File, String> {
        self.file.as_ref().ok_or("the archive is closed".to_owned())
    }

    fn group(&self, name: &str) -> Result<Group, String> {
        self.file()?
            .group(name)
            .map_err(|err| format!("{name} group: {err}"))
    }

    /// Close the archive, measure it, and open it again.
    fn size_on_disk(&mut self) -> Result<u64, String> {
        self.file = None;
        let size = std::fs::metadata(&self.path)
            .map_err(|err| format!("metadata: {err}"))?
            .len();
        self.file = Some(File::open_rw(&self.path).map_err(|err| format!("reopen: {err}"))?);
        Ok(size)
    }

    /// Append the rows of each day and index it, in the order given.
    fn insert_days(&self, days: &[(NaiveDate, usize)]) -> Result<(), String> {
        for &(day, rows) in days {
            self.insert_day(day, rows)?;
        }
        Ok(())
    }

    fn insert_day(&self, day: NaiveDate, rows: usize) -> Result<(), String> {
        let group = self.group(ROWS)?;
        let value = Column::new(&group, VALUE);
        let offset = value.rows().map_err(|err| format!("row count: {err}"))?;
        value
            .append(&values_of(day, rows))
            .map_err(|err| format!("append values: {err}"))?;
        if self.row_columns.contains(&LABEL) {
            Column::new(&group, LABEL)
                .append(&stored_labels(&labels_of(day, rows))?)
                .map_err(|err| format!("append labels: {err}"))?;
        }

        let days = self.group(DAYS)?;
        Column::new(&days, NOTE)
            .append(&[i64::from(day.to_epoch_days())])
            .map_err(|err| format!("append note: {err}"))?;
        DayIndex::new(&days)
            .insert_or_replace(
                day,
                RowPlacement {
                    offset: u64::try_from(offset).map_err(|err| format!("offset: {err}"))?,
                    rows: u32::try_from(rows).map_err(|err| format!("rows: {err}"))?,
                },
                DateTime::<Utc>::default(),
                HOST,
            )
            .map_err(|err| format!("index {day}: {err}"))
    }

    /// Run `act` against the archive's layout.
    fn with_layout<T>(
        &self,
        act: impl FnOnce(&ArchiveLayout<'_>) -> Result<T, ArchiveError>,
    ) -> Result<T, String> {
        let rows = self.group(ROWS)?;
        let levels = [RowLevel {
            group: &rows,
            columns: self.row_columns,
            extent: None,
        }];
        act(&ArchiveLayout {
            parent: self.file()?,
            index_name: DAYS,
            day_columns: &DAY_COLUMNS,
            levels: &levels,
        })
        .map_err(|err| format!("layout: {err}"))
    }

    /// The days the archive holds, oldest first.
    fn archived_days(&self) -> Result<Vec<NaiveDate>, String> {
        let days = self.group(DAYS)?;
        Ok(DayIndex::new(&days)
            .entries()
            .map_err(|err| format!("entries: {err}"))?
            .into_iter()
            .map(|entry| entry.day)
            .collect())
    }

    /// The rows the index names for `day`, or [`None`] when it is not indexed.
    fn day_rows(&self, day: NaiveDate) -> Result<Option<StoredRows>, String> {
        let days = self.group(DAYS)?;
        let Some(extent) = DayIndex::new(&days)
            .extent_of(day)
            .map_err(|err| format!("extent of {day}: {err}"))?
        else {
            return Ok(None);
        };
        let rows = self.group(ROWS)?;
        let values: Vec<u64> = Column::new(&rows, VALUE)
            .read_slice(extent.clone())
            .map_err(|err| format!("values of {day}: {err}"))?;
        let labels: Vec<VarLenUnicode> = if self.row_columns.contains(&LABEL) {
            Column::new(&rows, LABEL)
                .read_slice(extent)
                .map_err(|err| format!("labels of {day}: {err}"))?
        } else {
            Vec::new()
        };
        Ok(Some(StoredRows {
            values,
            labels: labels
                .into_iter()
                .map(|label| label.as_str().to_owned())
                .collect(),
        }))
    }

    /// The note stored beside the day's index entry.
    fn day_note(&self, day: NaiveDate) -> Result<Option<i64>, String> {
        let days = self.group(DAYS)?;
        let Some(row) = DayIndex::new(&days)
            .row_of(day)
            .map_err(|err| format!("row of {day}: {err}"))?
        else {
            return Ok(None);
        };
        Ok(Column::new(&days, NOTE)
            .read_slice::<i64>(row..row + 1)
            .map_err(|err| format!("note of {day}: {err}"))?
            .first()
            .copied())
    }

    fn stored_rows(&self) -> Result<usize, String> {
        let rows = self.group(ROWS)?;
        Column::new(&rows, VALUE)
            .rows()
            .map_err(|err| format!("row count: {err}"))
    }

    fn delete_state(&self) -> Result<DeleteState, String> {
        Ok(DeleteState::of(&self.group(DAYS)?))
    }

    fn mark_delete_in_flight(&self) -> Result<(), String> {
        DeleteState::InFlight
            .write(&self.group(DAYS)?)
            .map_err(|err| format!("mark the delete: {err}"))
    }
}

/// Days stored out of the order they fall in, which is what a backfill does:
/// the index rows are not in day order.
const STORED_DAYS: [(NaiveDate, usize); 4] = [
    (day_at(2), 5),
    (day_at(0), 3),
    (day_at(3), 6),
    (day_at(1), 4),
];

const fn day_at(offset: u32) -> NaiveDate {
    match NaiveDate::from_ymd_opt(2026, 8, 10 + offset) {
        Some(day) => day,
        None => NaiveDate::MIN,
    }
}

fn rows_of(day: NaiveDate) -> usize {
    STORED_DAYS
        .iter()
        .find(|(stored, _)| *stored == day)
        .map_or(0, |(_, rows)| *rows)
}

/// The days an archive holds are exactly the ones at or after the cutoff, and
/// each of them still reads back what it was stored with.
#[rstest]
#[case::before_every_day(day(0), vec![day(0), day(1), day(2), day(3)])]
#[case::past_the_oldest(day(1), vec![day(1), day(2), day(3)])]
#[case::past_all_but_the_newest(day(3), vec![day(3)])]
#[case::past_every_day(day(4), vec![])]
fn deleting_days_before_a_cutoff_keeps_the_rest_whole(
    #[case] cutoff: NaiveDate,
    #[case] expected: Vec<NaiveDate>,
) {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");

    let removed = archive
        .with_layout(|layout| layout.delete_days_before(cutoff, None))
        .expect("delete");

    assert_eq!(removed, STORED_DAYS.len() - expected.len());
    assert_eq!(archive.archived_days().expect("days"), expected);
    for day in &expected {
        let rows = rows_of(*day);
        assert_eq!(
            archive.day_rows(*day).expect("rows"),
            Some(StoredRows::of(*day, rows)),
            "{day} lost its rows"
        );
        assert_eq!(
            archive.day_note(*day).expect("note"),
            Some(i64::from(day.to_epoch_days()))
        );
    }
    assert_eq!(
        archive.stored_rows().expect("row count"),
        expected.iter().copied().map(rows_of).sum::<usize>(),
        "the rows of the deleted days are still stored"
    );
}

/// A delete reports its progress in the columns it rewrites: it starts before
/// the first one, never goes backwards, and ends on the last.
#[test]
fn a_delete_reports_progress_up_to_every_column_it_rewrites() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    let reported = std::cell::RefCell::new(Vec::new());

    archive
        .with_layout(|layout| {
            let report = |progress: PruneProgress| reported.borrow_mut().push(progress);
            layout.delete_days_before(day(2), Some(&report))
        })
        .expect("delete");

    assert_progress_ran_to_completion(&reported.into_inner());
}

/// A delete reports before it rewrites anything, never goes backwards or past
/// what it counted, and ends on the last column.
#[track_caller]
fn assert_progress_ran_to_completion(reported: &[PruneProgress]) {
    assert!(
        matches!(reported, [first, ..] if first.columns_rewritten == 0),
        "a delete reports before it rewrites a column: {reported:?}"
    );
    assert!(
        reported.windows(2).all(|pair| matches!(
            pair,
            [before, after] if after.columns_rewritten >= before.columns_rewritten
        )),
        "progress went backwards: {reported:?}"
    );
    assert!(
        reported
            .iter()
            .all(|progress| progress.columns_rewritten <= progress.columns_total),
        "progress passed the columns it counts: {reported:?}"
    );
    assert!(
        matches!(reported, [.., last] if last.columns_rewritten == last.columns_total),
        "the delete ended short of the columns it counted: {reported:?}"
    );
    assert!(
        matches!(reported, [.., last] if (last.fraction() - 1.0).abs() < f32::EPSILON),
        "the delete ended on a bar short of full: {reported:?}"
    );
}

#[test]
fn deleting_every_day_empties_the_archive() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");

    let removed = archive
        .with_layout(|layout| layout.delete_all_days(None))
        .expect("delete");

    assert_eq!(removed, STORED_DAYS.len());
    assert_eq!(archive.archived_days().expect("days"), []);
    assert_eq!(archive.stored_rows().expect("row count"), 0);
}

/// Rows a replaced day left behind belong to no entry, and go with the next
/// delete.
#[test]
fn rows_no_day_names_go_with_the_delete() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_day(day(0), 5).expect("insert");
    archive.insert_day(day(1), 5).expect("insert");
    // Stored again: the five rows of the first copy are left unnamed.
    archive.insert_day(day(1), 7).expect("insert");

    archive
        .with_layout(|layout| layout.delete_days_before(day(1), None))
        .expect("delete");

    assert_eq!(archive.archived_days().expect("days"), [day(1)]);
    assert_eq!(archive.stored_rows().expect("row count"), 7);
    assert_eq!(
        archive.day_rows(day(1)).expect("rows"),
        Some(StoredRows::of(day(1), 7))
    );
}

/// An archive pruned and refilled ends about where it started: the rows
/// stored after a delete are written into the space it freed.
#[test]
fn rows_stored_after_a_delete_reuse_the_space_it_freed() {
    let mut archive = TestArchive::create_without_text().expect("archive");
    for offset in 0..12 {
        archive.insert_day(day(offset), 8_192).expect("insert");
    }
    let filled = archive.size_on_disk().expect("size");

    archive
        .with_layout(|layout| layout.delete_days_before(day(6), None))
        .expect("delete");
    let pruned = archive.size_on_disk().expect("size");
    for offset in 12..18 {
        archive.insert_day(day(offset), 8_192).expect("insert");
    }
    let refilled = archive.size_on_disk().expect("size");

    assert!(
        pruned <= filled,
        "deleting half the days grew the file from {filled} to {pruned}"
    );
    // The space six days take is about half of what the file holds here.
    assert!(
        refilled < filled + filled / 10,
        "the days stored after the delete grew the file from {filled} to {refilled}"
    );
}

/// A delete that ran to the end leaves nothing for the next open to repair.
#[test]
fn a_delete_that_finished_leaves_the_index_settled() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");

    archive
        .with_layout(|layout| layout.delete_days_before(day(2), None))
        .expect("delete");
    assert_eq!(archive.delete_state().expect("state"), DeleteState::Settled);
    archive
        .with_layout(|layout| layout.recover_interrupted_delete("test archive"))
        .expect("recover");

    assert_eq!(archive.archived_days().expect("days"), [day(2), day(3)]);
    assert_eq!(
        archive.day_rows(day(2)).expect("rows"),
        Some(StoredRows::of(day(2), rows_of(day(2))))
    );
}

/// A delete that fails part-way leaves the index marked, and the next open
/// drops every day it holds.
#[test]
fn a_delete_that_fails_leaves_the_index_marked() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    let rows = archive.group(ROWS).expect("rows group");
    let levels = [RowLevel {
        group: &rows,
        columns: &[VALUE, "a column the archive never held"],
        extent: None,
    }];
    let layout = ArchiveLayout {
        parent: archive.file().expect("the archive is open"),
        index_name: DAYS,
        day_columns: &DAY_COLUMNS,
        levels: &levels,
    };

    layout
        .delete_days_before(day(2), None)
        .expect_err("a column that is not there");

    assert_eq!(
        archive.delete_state().expect("state"),
        DeleteState::InFlight
    );
    archive
        .with_layout(|layout| layout.recover_interrupted_delete("test archive"))
        .expect("recover");
    assert_eq!(archive.archived_days().expect("days"), []);
}

/// A delete interrupted while the rows were moving leaves rows of two layouts
/// mixed, which no index can be trusted against: every day goes.
#[test]
fn an_interrupted_delete_drops_every_day() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    archive.mark_delete_in_flight().expect("mark");

    archive
        .with_layout(|layout| layout.recover_interrupted_delete("test archive"))
        .expect("recover");

    assert_eq!(archive.archived_days().expect("days"), []);
    assert_eq!(archive.stored_rows().expect("row count"), 0);
    assert_eq!(archive.delete_state().expect("state"), DeleteState::Settled);
}

/// Two days claiming the same rows is a state no archive writes, and a delete
/// that moved them would hand one day the other's rows.
#[test]
fn overlapping_days_are_refused() {
    let archive = TestArchive::create().expect("archive");
    for offset in 0..3 {
        archive.insert_day(day(offset), 5).expect("insert");
    }
    let days = archive.group(DAYS).expect("days group");
    Column::new(&days, day_index::OFFSET)
        .write_row(2, 7_u64)
        .expect("overlap the surviving days");
    drop(days);

    let err = archive
        .with_layout(|layout| layout.delete_days_before(day(1), None))
        .expect_err("overlapping days");

    assert!(err.contains("overlap the rows before them"), "{err}");
}

/// A day index cut short of the days it holds cannot say where they sit.
#[test]
fn an_index_column_that_lost_rows_is_refused() {
    let archive = TestArchive::create().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    let days = archive.group(DAYS).expect("days group");
    Column::new(&days, day_index::COUNT)
        .truncate(2)
        .expect("shorten the counts");
    drop(days);

    let err = archive
        .with_layout(|layout| layout.delete_days_before(day(1), None))
        .expect_err("an index that disagrees with itself");

    assert!(err.contains("count holds 2 rows, requested"), "{err}");
}

/// The TEC archive's shape: a day names maps, and a map names the values it
/// holds. Both sets of offsets are rebased when days go.
mod three_levels {
    use super::*;

    const MAPS: &str = "maps";
    const VALUES: &str = "values";
    const EPOCH: &str = "epoch";
    const VALUE_OFFSET: &str = "value_offset";
    const VALUE_COUNT: &str = "value_count";
    const TECU: &str = "tecu";

    const MAP_COLUMNS: [&str; 1] = [EPOCH];
    const VALUE_COLUMNS: [&str; 1] = [TECU];

    /// Nodes per map, small enough to check every one of them.
    const NODES: usize = 3;

    struct NestedArchive {
        _dir: TempDir,
        file: File,
    }

    impl NestedArchive {
        fn create() -> Result<Self, String> {
            let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
            let file = File::create(dir.path().join("nested.h5"))
                .map_err(|err| format!("create: {err}"))?;

            let days = file
                .create_group(DAYS)
                .map_err(|err| format!("days group: {err}"))?;
            DayIndex::create_columns(&days, FORMAT).map_err(|err| format!("day index: {err}"))?;

            let maps = file
                .create_group(MAPS)
                .map_err(|err| format!("maps group: {err}"))?;
            Column::create::<i64>(&maps, EPOCH, FORMAT).map_err(|err| format!("epoch: {err}"))?;
            Column::create::<u64>(&maps, VALUE_OFFSET, FORMAT)
                .map_err(|err| format!("value offset: {err}"))?;
            Column::create::<u64>(&maps, VALUE_COUNT, FORMAT)
                .map_err(|err| format!("value count: {err}"))?;

            let values = file
                .create_group(VALUES)
                .map_err(|err| format!("values group: {err}"))?;
            Column::create::<f64>(&values, TECU, FORMAT).map_err(|err| format!("tecu: {err}"))?;
            Ok(Self { _dir: dir, file })
        }

        fn group(&self, name: &str) -> Result<Group, String> {
            self.file
                .group(name)
                .map_err(|err| format!("{name} group: {err}"))
        }

        /// Append `maps` maps of [`NODES`] nodes each, valued from `day`.
        fn insert_day(&self, day: NaiveDate, maps: usize) -> Result<(), String> {
            let value_group = self.group(VALUES)?;
            let map_group = self.group(MAPS)?;
            let first_map = Column::new(&map_group, EPOCH)
                .rows()
                .map_err(|err| format!("map count: {err}"))?;
            let mut next_value = Column::new(&value_group, TECU)
                .rows()
                .map_err(|err| format!("value count: {err}"))?;

            for map in 0..maps {
                Column::new(&value_group, TECU)
                    .append(&nodes_of(day, map))
                    .map_err(|err| format!("append nodes: {err}"))?;
                Column::new(&map_group, EPOCH)
                    .append(&[i64::from(day.to_epoch_days()) * 100 + map as i64])
                    .map_err(|err| format!("append epoch: {err}"))?;
                Column::new(&map_group, VALUE_OFFSET)
                    .append(&[next_value as u64])
                    .map_err(|err| format!("append value offset: {err}"))?;
                Column::new(&map_group, VALUE_COUNT)
                    .append(&[NODES as u64])
                    .map_err(|err| format!("append value count: {err}"))?;
                next_value += NODES;
            }

            let days = self.group(DAYS)?;
            DayIndex::new(&days)
                .insert_or_replace(
                    day,
                    RowPlacement {
                        offset: first_map as u64,
                        rows: u32::try_from(maps).map_err(|err| format!("maps: {err}"))?,
                    },
                    DateTime::<Utc>::default(),
                    HOST,
                )
                .map_err(|err| format!("index {day}: {err}"))
        }

        fn with_layout<T>(
            &self,
            act: impl FnOnce(&ArchiveLayout<'_>) -> Result<T, ArchiveError>,
        ) -> Result<T, String> {
            let maps = self.group(MAPS)?;
            let values = self.group(VALUES)?;
            let levels = [
                RowLevel {
                    group: &maps,
                    columns: &MAP_COLUMNS,
                    extent: Some(ExtentColumns {
                        offset: VALUE_OFFSET,
                        count: VALUE_COUNT,
                    }),
                },
                RowLevel {
                    group: &values,
                    columns: &VALUE_COLUMNS,
                    extent: None,
                },
            ];
            act(&ArchiveLayout {
                parent: &self.file,
                index_name: DAYS,
                day_columns: &[],
                levels: &levels,
            })
            .map_err(|err| format!("layout: {err}"))
        }

        /// The nodes of every map of `day`, read the way a store reads them:
        /// through the day entry, then through each map's own extent.
        fn day_nodes(&self, day: NaiveDate) -> Result<Option<Vec<Vec<f64>>>, String> {
            let days = self.group(DAYS)?;
            let Some(map_rows) = DayIndex::new(&days)
                .extent_of(day)
                .map_err(|err| format!("extent of {day}: {err}"))?
            else {
                return Ok(None);
            };
            let maps = self.group(MAPS)?;
            let values = self.group(VALUES)?;
            let offsets: Vec<u64> = Column::new(&maps, VALUE_OFFSET)
                .read_slice(map_rows.clone())
                .map_err(|err| format!("value offsets: {err}"))?;
            let counts: Vec<u64> = Column::new(&maps, VALUE_COUNT)
                .read_slice(map_rows)
                .map_err(|err| format!("value counts: {err}"))?;

            let mut nodes = Vec::with_capacity(offsets.len());
            for (&offset, &count) in offsets.iter().zip(&counts) {
                let start = usize::try_from(offset).map_err(|err| format!("offset: {err}"))?;
                let rows = usize::try_from(count).map_err(|err| format!("count: {err}"))?;
                nodes.push(
                    Column::new(&values, TECU)
                        .read_slice::<f64>(start..start + rows)
                        .map_err(|err| format!("nodes of {day}: {err}"))?,
                );
            }
            Ok(Some(nodes))
        }

        fn stored_values(&self) -> Result<usize, String> {
            let values = self.group(VALUES)?;
            Column::new(&values, TECU)
                .rows()
                .map_err(|err| format!("value count: {err}"))
        }
    }

    fn nodes_of(day: NaiveDate, map: usize) -> Vec<f64> {
        (0..NODES)
            .map(|node| f64::from(day.to_epoch_days()) + map as f64 / 10.0 + node as f64 / 100.0)
            .collect()
    }

    #[test]
    fn deleting_days_rebases_the_maps_and_the_values_they_name() {
        let archive = NestedArchive::create().expect("archive");
        archive.insert_day(day(0), 2).expect("insert");
        archive.insert_day(day(1), 3).expect("insert");
        archive.insert_day(day(2), 1).expect("insert");
        let reported = std::cell::RefCell::new(Vec::new());

        let removed = archive
            .with_layout(|layout| {
                let report = |progress: PruneProgress| reported.borrow_mut().push(progress);
                layout.delete_days_before(day(1), Some(&report))
            })
            .expect("delete");

        assert_eq!(removed, 1);
        assert_progress_ran_to_completion(&reported.into_inner());
        assert_eq!(
            archive.day_nodes(day(1)).expect("nodes"),
            Some((0..3).map(|map| nodes_of(day(1), map)).collect::<Vec<_>>())
        );
        assert_eq!(
            archive.day_nodes(day(2)).expect("nodes"),
            Some(vec![nodes_of(day(2), 0)]),
            "the newest day reads through its rebased offsets"
        );
        assert_eq!(archive.day_nodes(day(0)).expect("nodes"), None);
        assert_eq!(
            archive.stored_values().expect("value count"),
            4 * NODES,
            "the deleted day's values are still stored"
        );
    }
}
