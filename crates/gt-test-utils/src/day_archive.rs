//! Reading and marking a day archive file directly, beside the store that
//! owns it, and the rules every day archive follows.

use std::fmt::Display;
use std::path::Path;

use chrono::{DateTime, Utc};
use gt_hdf5_archive::prune::DeleteState;
use gt_hdf5_archive::{ReadOnlyDayArchive as _, WritableDayArchive};
use tempfile::TempDir;

pub mod conformance;

/// Path of a group in an archive file, from its root: `"days"`, or `"kp/days"`
/// where the archive holds one index per group.
#[derive(Debug, Clone, Copy)]
pub struct GroupPath<'a>(pub &'a str);

/// Name of a column in that group.
#[derive(Debug, Clone, Copy)]
pub struct ColumnName<'a>(pub &'a str);

/// Leave the day index as a delete interrupted while the rows were moving does.
pub fn mark_delete_in_flight(path: &Path, GroupPath(days): GroupPath<'_>) -> Result<(), String> {
    let file = hdf5::File::open_rw(path).map_err(|err| format!("open {path:?}: {err}"))?;
    let index = file.group(days).map_err(|err| format!("{days}: {err}"))?;
    DeleteState::InFlight
        .write(&index)
        .map_err(|err| format!("mark the delete: {err}"))
}

pub fn delete_state(path: &Path, GroupPath(days): GroupPath<'_>) -> Result<DeleteState, String> {
    let file = hdf5::File::open(path).map_err(|err| format!("open {path:?}: {err}"))?;
    let index = file.group(days).map_err(|err| format!("{days}: {err}"))?;
    Ok(DeleteState::of(&index))
}

pub fn column_rows(
    path: &Path,
    GroupPath(group): GroupPath<'_>,
    ColumnName(column): ColumnName<'_>,
) -> Result<usize, String> {
    let file = hdf5::File::open(path).map_err(|err| format!("open {}: {err}", path.display()))?;
    let held = file.group(group).map_err(|err| format!("{group}: {err}"))?;
    gt_hdf5_archive::Column::new(&held, column)
        .rows()
        .map_err(|err| format!("{group}/{column}: {err}"))
}

/// The instant every day a store fixture archives was fetched at.
pub fn fetched_at() -> DateTime<Utc> {
    DateTime::from_timestamp(FETCH_INSTANT_UNIX_SECS, 0).unwrap_or(DateTime::UNIX_EPOCH)
}

/// A day archive of `A` in a temp directory of its own, under the file name
/// the archive declares. The file lives for as long as the caller holds the
/// [`TempDir`].
pub fn store_in_a_temp_dir<A>() -> Result<(TempDir, A), String>
where
    A: WritableDayArchive,
    A::Error: Display,
{
    let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
    let store = A::open_or_create(&dir.path().join(A::ReadOnly::FILE_NAME))
        .map_err(|err| format!("open archive: {err}"))?;
    Ok((dir, store))
}

/// 2026-07-20 00:00:00 UTC.
const FETCH_INSTANT_UNIX_SECS: i64 = 1_784_505_600;
