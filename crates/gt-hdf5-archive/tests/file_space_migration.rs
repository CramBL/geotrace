//! Rebuilding an archive created before archives recorded their free space in
//! pages.

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use hdf5::filters::Filter;
use hdf5::plist::file_create::FileSpaceStrategy;
use hdf5::{Dataset, File, Group};
use rstest::rstest;
use tempfile::TempDir;

use gt_hdf5_archive::day_index::{DayIndex, RowPlacement};
use gt_hdf5_archive::prune::{ArchiveLayout, RowLevel};
use gt_hdf5_archive::{ArchiveFile, Column, ColumnFormat, FileSpaceMigration, attributes};

const FORMAT: ColumnFormat = ColumnFormat {
    chunk_rows: 4_096,
    deflate_level: 6,
};

const DAYS: &str = "days";
const ROWS: &str = "rows";
const VALUE: &str = "value";
const HOST: &str = "https://example.invalid";
const SCHEMA_VERSION_ATTR: &str = "schema_version";
const SCHEMA_VERSION: i64 = 3;

/// What [`ArchiveFile::create`] creates an archive with, and what a migrated
/// archive is expected to carry.
const PAGED_FILE_SPACE: FileSpaceStrategy = FileSpaceStrategy::FreeSpaceManager {
    paged: true,
    persist: true,
    threshold: 1,
};

/// The days the tests that read an archive back store, out of the order they
/// fall in.
const STORED_DAYS: [(NaiveDate, usize); 3] = [(day_at(2), 5), (day_at(0), 3), (day_at(1), 4)];

const fn day_at(offset: u32) -> NaiveDate {
    match NaiveDate::from_ymd_opt(2026, 8, 10 + offset) {
        Some(day) => day,
        None => NaiveDate::MIN,
    }
}

fn day(offset: i64) -> NaiveDate {
    day_at(0) + TimeDelta::days(offset)
}

/// Values whose file size follows the rows they fill: they do not compress.
fn values_of(day: NaiveDate, rows: usize) -> Vec<u64> {
    let seed = u64::try_from(day.to_epoch_days()).unwrap_or_default();
    (0..rows)
        .map(|row| (seed * 1_000 + row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .collect()
}

/// A day-keyed archive of one row column, left closed between operations the
/// way a store leaves it.
struct TestArchive {
    _dir: TempDir,
    path: PathBuf,
}

impl TestArchive {
    /// An archive as a version before the paged strategy created it: the file
    /// space strategy libhdf5 defaults to.
    fn create_unpaged() -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
        let path = dir.path().join("archive.h5");
        let file = File::create(&path).map_err(|err| format!("create: {err}"))?;
        write_schema(&file)?;
        drop(file);
        Ok(Self { _dir: dir, path })
    }

    /// An archive [`ArchiveFile::create`] created.
    fn create_paged() -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
        let path = dir.path().join("archive.h5");
        let mut archive = ArchiveFile::new(&path);
        let file = archive.create().map_err(|err| format!("create: {err}"))?;
        write_schema(&file)?;
        drop(file);
        Ok(Self { _dir: dir, path })
    }

    fn migrate(&self) -> Result<FileSpaceMigration, String> {
        ArchiveFile::new(&self.path)
            .migrate_file_space_if_needed()
            .map_err(|err| format!("migrate: {err}"))
    }

    fn file_space_strategy(&self) -> Result<FileSpaceStrategy, String> {
        let file = File::open(&self.path).map_err(|err| format!("open: {err}"))?;
        Ok(file
            .fcpl()
            .map_err(|err| format!("fcpl: {err}"))?
            .file_space_strategy())
    }

    /// Path of the file an interrupted rebuild leaves beside the archive.
    fn rebuilding_path(&self) -> PathBuf {
        let mut path = self.path.clone().into_os_string();
        path.push(".rebuilding");
        PathBuf::from(path)
    }

    fn size_on_disk(&self) -> Result<u64, String> {
        Ok(std::fs::metadata(&self.path)
            .map_err(|err| format!("metadata: {err}"))?
            .len())
    }

    fn insert_days(&self, days: &[(NaiveDate, usize)]) -> Result<(), String> {
        let file = File::open_rw(&self.path).map_err(|err| format!("open: {err}"))?;
        for &(day, rows) in days {
            let group = group_of(&file, ROWS)?;
            let value = Column::new(&group, VALUE);
            let offset = value.rows().map_err(|err| format!("row count: {err}"))?;
            value
                .append(&values_of(day, rows))
                .map_err(|err| format!("append values: {err}"))?;

            let days = group_of(&file, DAYS)?;
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
                .map_err(|err| format!("index {day}: {err}"))?;
        }
        Ok(())
    }

    fn delete_days_before(&self, cutoff: NaiveDate) -> Result<usize, String> {
        let file = File::open_rw(&self.path).map_err(|err| format!("open: {err}"))?;
        let rows = group_of(&file, ROWS)?;
        let levels = [RowLevel {
            group: &rows,
            columns: &[VALUE],
            extent: None,
        }];
        ArchiveLayout {
            parent: &file,
            index_name: DAYS,
            day_columns: &[],
            levels: &levels,
        }
        .delete_days_before(cutoff)
        .map_err(|err| format!("delete: {err}"))
    }

    /// The days the archive holds, oldest first.
    fn archived_days(&self) -> Result<Vec<NaiveDate>, String> {
        let file = File::open(&self.path).map_err(|err| format!("open: {err}"))?;
        let days = group_of(&file, DAYS)?;
        Ok(DayIndex::new(&days)
            .entries()
            .map_err(|err| format!("entries: {err}"))?
            .into_iter()
            .map(|entry| entry.day)
            .collect())
    }

    /// The rows the index names for `day`, or [`None`] when it is not indexed.
    fn day_rows(&self, day: NaiveDate) -> Result<Option<Vec<u64>>, String> {
        let file = File::open(&self.path).map_err(|err| format!("open: {err}"))?;
        let days = group_of(&file, DAYS)?;
        let Some(extent) = DayIndex::new(&days)
            .extent_of(day)
            .map_err(|err| format!("extent of {day}: {err}"))?
        else {
            return Ok(None);
        };
        let rows = group_of(&file, ROWS)?;
        Column::new(&rows, VALUE)
            .read_slice(extent)
            .map(Some)
            .map_err(|err| format!("values of {day}: {err}"))
    }

    fn schema_version(&self) -> Result<Option<i64>, String> {
        let file = File::open(&self.path).map_err(|err| format!("open: {err}"))?;
        Ok(attributes::read_i64(&file, SCHEMA_VERSION_ATTR))
    }

    fn value_column(&self) -> Result<Dataset, String> {
        let file = File::open(&self.path).map_err(|err| format!("open: {err}"))?;
        group_of(&file, ROWS)?
            .dataset(VALUE)
            .map_err(|err| format!("value column: {err}"))
    }
}

fn group_of(file: &File, name: &str) -> Result<Group, String> {
    file.group(name)
        .map_err(|err| format!("{name} group: {err}"))
}

/// The schema every archive here holds: a day index, one row column, and a
/// schema version on the file itself.
fn write_schema(file: &File) -> Result<(), String> {
    attributes::write_i64(file, SCHEMA_VERSION_ATTR, SCHEMA_VERSION)
        .map_err(|err| format!("schema version: {err}"))?;
    let days = file
        .create_group(DAYS)
        .map_err(|err| format!("days group: {err}"))?;
    DayIndex::create_columns(&days, FORMAT).map_err(|err| format!("day index: {err}"))?;
    let rows = file
        .create_group(ROWS)
        .map_err(|err| format!("rows group: {err}"))?;
    Column::create::<u64>(&rows, VALUE, FORMAT).map_err(|err| format!("value: {err}"))
}

/// An archive from before the paged strategy is rebuilt on open, and reads
/// back everything it held.
#[test]
fn an_unpaged_archive_is_rebuilt_and_keeps_what_it_held() {
    let archive = TestArchive::create_unpaged().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");

    assert_eq!(
        archive.migrate().expect("migrate"),
        FileSpaceMigration::Rebuilt
    );

    assert_eq!(
        archive.file_space_strategy().expect("strategy"),
        PAGED_FILE_SPACE
    );
    assert_eq!(
        archive.schema_version().expect("schema version"),
        Some(SCHEMA_VERSION)
    );
    assert_eq!(
        archive.archived_days().expect("days"),
        [day(0), day(1), day(2)]
    );
    for (stored, rows) in STORED_DAYS {
        assert_eq!(
            archive.day_rows(stored).expect("rows"),
            Some(values_of(stored, rows)),
            "{stored} lost its rows"
        );
    }
    let value = archive.value_column().expect("value column");
    assert_eq!(value.chunk(), Some(vec![FORMAT.chunk_rows]));
    assert_eq!(
        value.filters(),
        [Filter::Shuffle, Filter::Deflate(FORMAT.deflate_level)]
    );
    assert!(value.is_resizable());
}

/// The rebuild runs once: the archive it leaves is one every later open
/// accepts as it is.
#[test]
fn a_rebuilt_archive_is_not_rebuilt_again() {
    let archive = TestArchive::create_unpaged().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    archive.migrate().expect("migrate");
    let rebuilt = archive.size_on_disk().expect("size");

    assert_eq!(
        archive.migrate().expect("migrate again"),
        FileSpaceMigration::NotNeeded
    );
    assert_eq!(archive.size_on_disk().expect("size"), rebuilt);
}

/// An archive already recording its free space in pages is not touched.
#[test]
fn a_paged_archive_is_left_as_it_is() {
    let archive = TestArchive::create_paged().expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    let modified = std::fs::metadata(&archive.path)
        .and_then(|file| file.modified())
        .expect("modified time");
    let size = archive.size_on_disk().expect("size");

    assert_eq!(
        archive.migrate().expect("migrate"),
        FileSpaceMigration::NotNeeded
    );

    assert_eq!(archive.size_on_disk().expect("size"), size);
    assert_eq!(
        std::fs::metadata(&archive.path)
            .and_then(|file| file.modified())
            .expect("modified time"),
        modified
    );
}

/// A rebuild that was interrupted leaves a file beside the archive, which the
/// next open removes before it looks at the archive itself.
#[rstest]
#[case::unpaged(TestArchive::create_unpaged(), FileSpaceMigration::Rebuilt)]
#[case::paged(TestArchive::create_paged(), FileSpaceMigration::NotNeeded)]
fn a_file_an_interrupted_rebuild_left_is_removed(
    #[case] archive: Result<TestArchive, String>,
    #[case] expected: FileSpaceMigration,
) {
    let archive = archive.expect("archive");
    archive.insert_days(&STORED_DAYS).expect("insert");
    std::fs::write(archive.rebuilding_path(), b"half a rebuild").expect("write");

    assert_eq!(archive.migrate().expect("migrate"), expected);

    assert!(
        !archive.rebuilding_path().exists(),
        "the file is still there"
    );
    assert_eq!(
        archive.archived_days().expect("days"),
        [day(0), day(1), day(2)]
    );
}

/// What the rebuild is for: on a migrated archive the days stored after a
/// delete are written into the space it freed, which an archive left on the
/// strategy libhdf5 defaults to does not do.
#[test]
fn days_stored_after_a_delete_reuse_the_space_a_migrated_archive_freed() {
    let migrated = TestArchive::create_unpaged().expect("archive");
    migrated.migrate().expect("migrate");
    let unmigrated = TestArchive::create_unpaged().expect("archive");

    let migrated_bytes = fill_delete_and_refill(&migrated).expect("migrated archive");
    let unmigrated_bytes = fill_delete_and_refill(&unmigrated).expect("unmigrated archive");

    assert!(
        migrated_bytes < unmigrated_bytes,
        "the days stored after the delete took {migrated_bytes} bytes on the migrated archive \
         against {unmigrated_bytes} on the one left as it was"
    );
}

/// Store twelve days, delete the older half and store six more, reporting
/// what the archive ends up taking.
fn fill_delete_and_refill(archive: &TestArchive) -> Result<u64, String> {
    const ROWS_PER_DAY: usize = 8_192;
    for offset in 0..12 {
        archive.insert_days(&[(day(offset), ROWS_PER_DAY)])?;
    }
    archive.delete_days_before(day(6))?;
    for offset in 12..18 {
        archive.insert_days(&[(day(offset), ROWS_PER_DAY)])?;
    }
    archive.size_on_disk()
}
