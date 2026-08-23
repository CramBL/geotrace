//! Reading and marking a day archive file directly, beside the store that
//! owns it.

use std::path::Path;

use gt_hdf5_archive::prune::DeleteState;

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
