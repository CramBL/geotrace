//! Removing days from an archive in place, and recovering from a removal that
//! was interrupted.
//!
//! An [`ArchiveLayout`] names everything one day index owns: the columns
//! holding one value per indexed day, and the levels of row columns its
//! entries reach into. Removing days cuts the index entries and moves the
//! surviving rows of every level down to close the gaps they leave.
//!
//! # What a delete gives back
//!
//! An archive file is created tracking its free space in pages
//! ([`crate::ArchiveFile::create`]), so the space a delete frees is what the
//! days stored after it are written into. The file itself rarely gets shorter:
//! libhdf5 only hands back what sits at the very end of it, and the rows and
//! headers a delete rewrites are placed wherever its free space manager puts
//! them. Measured over ten stored days of a published interference day, of
//! which five were deleted and five stored again: 729 KB filled, 659 KB after
//! the delete, 729 KB again after the five new days.
//!
//! # Recovering an interrupted delete
//!
//! [`DELETE_IN_FLIGHT_ATTR`] on the day index group is set for as long as the
//! rows are moving, and [`ArchiveLayout::recover_interrupted_delete`] reads it
//! on open. While it is set, nothing in the archive can be read: the data
//! columns hold rows of two layouts and the index may name either. Recovery
//! therefore drops every day of that index, which is downloaded again as it is
//! needed. What an entry never does is name rows that are not its own.

use std::ops::Range;

use chrono::NaiveDate;
use hdf5::filters::Filter;
use hdf5::types::VarLenUnicode;
use hdf5::{Dataset, Extents, Group, SimpleExtents};

use crate::day_index;
use crate::{ArchiveError, Column, attributes, dates};

/// Attribute of a day index group recording whether a delete is part-way
/// through it.
///
/// The index holds it from the moment it is created: an attribute added
/// later costs the file a header block past the rows a delete is about to
/// free, which pins the file's length.
pub const DELETE_IN_FLIGHT_ATTR: &str = "delete_in_flight";

/// Whether a delete is part-way through the index it is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteState {
    Settled,
    InFlight,
}

impl DeleteState {
    const fn code(self) -> i64 {
        match self {
            Self::Settled => 0,
            Self::InFlight => 1,
        }
    }

    /// What `index` records. An index written before the archive carried the
    /// attribute reads as settled, and so does one carrying a code the
    /// attribute does not define.
    pub fn of(index: &Group) -> Self {
        match attributes::read_i64(index, DELETE_IN_FLIGHT_ATTR) {
            Some(code) if code == Self::InFlight.code() => Self::InFlight,
            Some(_) | None => Self::Settled,
        }
    }

    /// Write the state and push the file to storage.
    ///
    /// The attribute has to reach the file before the rows a delete moves
    /// after it: the recovery reads one against the other. The flush covers a
    /// process that dies. A machine that loses power can still lose what the
    /// operating system had not written.
    pub fn write(self, index: &Group) -> Result<(), ArchiveError> {
        attributes::set_i64(index, DELETE_IN_FLIGHT_ATTR, self.code())?;
        Ok(index.file()?.flush()?)
    }
}

/// The columns of one level naming a range of rows of the level below it.
#[derive(Debug, Clone, Copy)]
pub struct ExtentColumns<'a> {
    /// Column holding the first row of the range.
    pub offset: &'a str,
    /// Column holding how many rows the range covers.
    pub count: &'a str,
}

/// One group of parallel columns the day entries reach into.
#[derive(Clone, Copy)]
pub struct RowLevel<'a> {
    pub group: &'a Group,
    /// Columns holding one value per row of this level, the extent columns
    /// aside.
    pub columns: &'a [&'a str],
    /// Set where every row of this level names a range of the next level's
    /// rows. Only the innermost level names none.
    pub extent: Option<ExtentColumns<'a>>,
}

impl<'a> RowLevel<'a> {
    /// Every column of the level, the extent columns included.
    fn all_columns(&self) -> Vec<&'a str> {
        let mut names = self.columns.to_vec();
        if let Some(extent) = self.extent {
            names.push(extent.count);
            names.push(extent.offset);
        }
        names
    }
}

/// Everything one day index owns, as the archive that keeps it lays it out.
#[derive(Clone, Copy)]
pub struct ArchiveLayout<'a> {
    /// Group the day index sits in.
    pub parent: &'a Group,
    /// Name of the day index group under [`Self::parent`].
    pub index_name: &'a str,
    /// Columns beside the day index holding one value per indexed day.
    pub day_columns: &'a [&'a str],
    /// The row levels the day entries reach, outermost first.
    pub levels: &'a [RowLevel<'a>],
}

impl ArchiveLayout<'_> {
    /// Remove every day before `cutoff`, reporting how many days went.
    pub fn delete_days_before(&self, cutoff: NaiveDate) -> Result<usize, ArchiveError> {
        self.retain_days(|day| day >= cutoff)
    }

    /// Remove every archived day, reporting how many went.
    pub fn delete_all_days(&self) -> Result<usize, ArchiveError> {
        self.retain_days(|_| false)
    }

    /// Bring an archive back to a state its index and rows agree on, after a
    /// removal that was interrupted.
    pub fn recover_interrupted_delete(&self, archive_name: &str) -> Result<(), ArchiveError> {
        let index = self.parent.group(self.index_name)?;
        if DeleteState::of(&index) == DeleteState::Settled {
            return Ok(());
        }
        let dropped = self.drop_every_day(&index)?;
        log::error!(
            "{archive_name}: a delete was interrupted while the rows were moving. Dropped the \
             {dropped} archived days it left behind, which are downloaded again as they are \
             needed."
        );
        DeleteState::Settled.write(&index)
    }

    /// Keep the days `keep` accepts and remove the rest, reporting how many
    /// went.
    fn retain_days(&self, keep: impl Fn(NaiveDate) -> bool) -> Result<usize, ArchiveError> {
        let index = self.parent.group(self.index_name)?;
        let stored: Vec<i32> = Column::new(&index, day_index::DAY).read()?;
        let mut kept_rows: Vec<Range<usize>> = Vec::with_capacity(stored.len());
        for (row, &day) in stored.iter().enumerate() {
            if keep(dates::date_from_epoch_days(day)?) {
                kept_rows.push(row..row + 1);
            }
        }
        let removed = stored.len() - kept_rows.len();
        if removed == 0 {
            return Ok(0);
        }

        // The surviving entries, read before the rows they name start moving.
        let mut entries: Vec<(&str, ColumnValues)> = Vec::new();
        for name in self.index_columns() {
            let kept = ColumnValues::read_rows(&index.dataset(name)?, &kept_rows)?;
            entries.push((name, kept));
        }

        DeleteState::InFlight.write(&index)?;
        let offsets = self.compact_rows(&index, &kept_rows)?;
        for (name, kept) in entries {
            let surviving = if name == day_index::OFFSET {
                kept.row_numbers_like(&offsets)?
            } else {
                kept
            };
            surviving.overwrite(&Column::new(&index, name))?;
        }
        DeleteState::Settled.write(&index)?;
        Ok(removed)
    }

    /// Every column holding one value per indexed day.
    fn index_columns(&self) -> Vec<&str> {
        let mut names = vec![
            day_index::DAY,
            day_index::OFFSET,
            day_index::COUNT,
            day_index::FETCHED_AT,
            day_index::HOST,
        ];
        names.extend_from_slice(self.day_columns);
        names
    }

    /// Rewrite every data column with the rows `kept_rows` of the day index
    /// name, reporting where each of those days ends up.
    ///
    /// Every level is planned from its extent columns first: they hold one
    /// value per row of the level above, so reading them costs little. The
    /// rows are then moved one column at a time, which bounds what a delete
    /// holds in memory to a single column's surviving rows.
    fn compact_rows(
        &self,
        index: &Group,
        kept_rows: &[Range<usize>],
    ) -> Result<Vec<u64>, ArchiveError> {
        let mut plans: Vec<RowPlan> = Vec::with_capacity(self.levels.len());
        let mut parent_group = index;
        let mut parent_extent = ExtentColumns {
            offset: day_index::OFFSET,
            count: day_index::COUNT,
        };
        let mut parent_rows = kept_rows.to_vec();
        for level in self.levels {
            let plan = RowPlan::of(&read_extents(parent_group, parent_extent, &parent_rows)?)?;
            parent_group = level.group;
            parent_rows.clone_from(&plan.kept);
            plans.push(plan);
            let Some(extent) = level.extent else {
                break;
            };
            parent_extent = extent;
        }

        for (position, level) in self.levels.iter().enumerate() {
            let Some(plan) = plans.get(position) else {
                break;
            };
            for name in level.columns {
                rewrite_column(level.group, name, &plan.kept, None)?;
            }
            if let Some(extent) = level.extent {
                rewrite_column(level.group, extent.count, &plan.kept, None)?;
                let below = plans
                    .get(position + 1)
                    .map(|below| below.offsets.as_slice());
                rewrite_column(level.group, extent.offset, &plan.kept, below)?;
            }
        }

        Ok(plans
            .first()
            .map(|plan| plan.offsets.clone())
            .unwrap_or_default())
    }

    /// Cut every column to nothing, the day index first so an interrupted
    /// clear leaves rows no entry names. Reports how many days went.
    fn drop_every_day(&self, index: &Group) -> Result<usize, ArchiveError> {
        let dropped = Column::new(index, day_index::DAY).rows()?;
        for name in self.index_columns() {
            Column::new(index, name).truncate(0)?;
        }
        for level in self.levels {
            for name in level.all_columns() {
                Column::new(level.group, name).truncate(0)?;
            }
        }
        Ok(dropped)
    }
}

/// How a column is chunked and filtered, read off the column itself so a
/// rewrite stores it the way the archive that created it declared.
struct ColumnLayout {
    chunk: Vec<usize>,
    filters: Vec<Filter>,
}

impl ColumnLayout {
    fn of(dataset: &Dataset) -> Result<Self, ArchiveError> {
        let chunk = dataset.chunk().ok_or_else(|| {
            ArchiveError::Corrupt(format!("column {} is not chunked", dataset.name()))
        })?;
        Ok(Self {
            chunk,
            filters: dataset.filters(),
        })
    }
}

/// Where the rows one level keeps end up once the gaps between them close.
struct RowPlan {
    /// First row each parent row's range moves to, in parent row order.
    offsets: Vec<u64>,
    /// The ranges to keep, in the order they are written back.
    kept: Vec<Range<usize>>,
}

impl RowPlan {
    /// Lay `extents` out end to end in ascending order of where they sit now,
    /// which keeps the rows in the order the archive stored them.
    fn of(extents: &[Range<usize>]) -> Result<Self, ArchiveError> {
        let mut order: Vec<usize> = (0..extents.len()).collect();
        order.sort_by_key(|&at| extents.get(at).map(|range| range.start));

        let mut offsets = vec![0_u64; extents.len()];
        let mut kept: Vec<Range<usize>> = Vec::with_capacity(extents.len());
        let mut reached = 0_usize;
        for at in order {
            let (Some(range), Some(offset)) = (extents.get(at), offsets.get_mut(at)) else {
                return Err(ArchiveError::Corrupt(format!("row {at} has no extent")));
            };
            if range.start < kept.last().map_or(0, |last| last.end) {
                return Err(ArchiveError::Corrupt(format!(
                    "rows {}..{} overlap the rows before them",
                    range.start, range.end
                )));
            }
            *offset = u64::try_from(reached)
                .map_err(|err| ArchiveError::Corrupt(format!("row offset {reached}: {err}")))?;
            reached += range.len();
            kept.push(range.clone());
        }
        Ok(Self { offsets, kept })
    }
}

/// The ranges `rows` of `group` name in the level below it.
fn read_extents(
    group: &Group,
    extent: ExtentColumns<'_>,
    rows: &[Range<usize>],
) -> Result<Vec<Range<usize>>, ArchiveError> {
    let offsets =
        ColumnValues::read_rows(&group.dataset(extent.offset)?, rows)?.into_row_numbers()?;
    let counts =
        ColumnValues::read_rows(&group.dataset(extent.count)?, rows)?.into_row_numbers()?;
    if offsets.len() != counts.len() {
        return Err(ArchiveError::Corrupt(format!(
            "{} holds {} rows against {}'s {}",
            extent.offset,
            offsets.len(),
            extent.count,
            counts.len()
        )));
    }

    let mut extents = Vec::with_capacity(offsets.len());
    for (at, (&offset, &count)) in offsets.iter().zip(&counts).enumerate() {
        let start = usize::try_from(offset)
            .map_err(|err| ArchiveError::Corrupt(format!("row {at} offset {offset}: {err}")))?;
        let rows = usize::try_from(count)
            .map_err(|err| ArchiveError::Corrupt(format!("row {at} count {count}: {err}")))?;
        extents.push(start..start + rows);
    }
    Ok(extents)
}

/// Rows lifted out of a column, in the type the column stores them as.
enum ColumnValues {
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    F64(Vec<f64>),
    Text(Vec<VarLenUnicode>),
}

impl ColumnValues {
    /// Read `rows` out of `dataset`, in the order they are given.
    fn read_rows(dataset: &Dataset, rows: &[Range<usize>]) -> Result<Self, ArchiveError> {
        let stored = dataset.dtype()?;
        if stored.is::<i32>() {
            Ok(Self::I32(read_ranges(dataset, rows)?))
        } else if stored.is::<i64>() {
            Ok(Self::I64(read_ranges(dataset, rows)?))
        } else if stored.is::<u8>() {
            Ok(Self::U8(read_ranges(dataset, rows)?))
        } else if stored.is::<u32>() {
            Ok(Self::U32(read_ranges(dataset, rows)?))
        } else if stored.is::<u64>() {
            Ok(Self::U64(read_ranges(dataset, rows)?))
        } else if stored.is::<f64>() {
            Ok(Self::F64(read_ranges(dataset, rows)?))
        } else if stored.is::<VarLenUnicode>() {
            Ok(Self::Text(read_ranges(dataset, rows)?))
        } else {
            Err(ArchiveError::Corrupt(format!(
                "column {} holds a type an archive cannot store",
                dataset.name()
            )))
        }
    }

    /// Create the column under `name` and write these rows into it.
    fn create(&self, group: &Group, name: &str, layout: &ColumnLayout) -> Result<(), ArchiveError> {
        match self {
            Self::I32(values) => create_column(group, name, layout, values),
            Self::I64(values) => create_column(group, name, layout, values),
            Self::U8(values) => create_column(group, name, layout, values),
            Self::U32(values) => create_column(group, name, layout, values),
            Self::U64(values) => create_column(group, name, layout, values),
            Self::F64(values) => create_column(group, name, layout, values),
            Self::Text(values) => create_column(group, name, layout, values),
        }
    }

    /// Write these rows over the front of `column` and cut what was past them.
    fn overwrite(&self, column: &Column<'_>) -> Result<(), ArchiveError> {
        match self {
            Self::I32(values) => overwrite_column(column, values),
            Self::I64(values) => overwrite_column(column, values),
            Self::U8(values) => overwrite_column(column, values),
            Self::U32(values) => overwrite_column(column, values),
            Self::U64(values) => overwrite_column(column, values),
            Self::F64(values) => overwrite_column(column, values),
            Self::Text(values) => overwrite_column(column, values),
        }
    }

    /// These rows as the row numbers an extent column addresses rows with.
    fn into_row_numbers(self) -> Result<Vec<u64>, ArchiveError> {
        match self {
            Self::U64(values) => Ok(values),
            Self::U32(values) => Ok(values.into_iter().map(u64::from).collect()),
            Self::I32(_) | Self::I64(_) | Self::U8(_) | Self::F64(_) | Self::Text(_) => {
                Err(ArchiveError::Corrupt(
                    "an extent column holds a type that cannot number rows".to_owned(),
                ))
            }
        }
    }

    /// `numbers` in the type this extent column stores row numbers as.
    fn row_numbers_like(&self, numbers: &[u64]) -> Result<Self, ArchiveError> {
        match self {
            Self::U64(_) => Ok(Self::U64(numbers.to_vec())),
            Self::U32(_) => numbers
                .iter()
                .map(|&number| {
                    u32::try_from(number)
                        .map_err(|err| ArchiveError::Corrupt(format!("row number {number}: {err}")))
                })
                .collect::<Result<Vec<u32>, ArchiveError>>()
                .map(Self::U32),
            Self::I32(_) | Self::I64(_) | Self::U8(_) | Self::F64(_) | Self::Text(_) => {
                Err(ArchiveError::Corrupt(
                    "an extent column holds a type that cannot number rows".to_owned(),
                ))
            }
        }
    }
}

/// Read the rows `ranges` cover, refusing a range past the end of `dataset`.
fn read_ranges<T: hdf5::H5Type + Clone>(
    dataset: &Dataset,
    ranges: &[Range<usize>],
) -> Result<Vec<T>, ArchiveError> {
    let available =
        dataset.shape().first().copied().ok_or_else(|| {
            ArchiveError::Corrupt(format!("{} has no dimensions", dataset.name()))
        })?;
    let mut values = Vec::new();
    for range in ranges {
        if range.end > available {
            return Err(ArchiveError::Corrupt(format!(
                "{} holds {available} rows, requested {}..{}",
                dataset.name(),
                range.start,
                range.end
            )));
        }
        values.extend(dataset.read_slice_1d::<T, _>(range.clone())?.to_vec());
    }
    Ok(values)
}

/// Write `name` again holding only the rows `kept` covers, in the layout it
/// was created with. `rebased_offsets` replaces the values of an extent
/// column, whose rows have moved with the level below it.
///
/// The column is freed first: the surviving rows are then written into the
/// space it held.
fn rewrite_column(
    group: &Group,
    name: &str,
    kept: &[Range<usize>],
    rebased_offsets: Option<&[u64]>,
) -> Result<(), ArchiveError> {
    let dataset = group.dataset(name)?;
    let layout = ColumnLayout::of(&dataset)?;
    let read = ColumnValues::read_rows(&dataset, kept)?;
    let surviving = match rebased_offsets {
        Some(offsets) => read.row_numbers_like(offsets)?,
        None => read,
    };
    drop(dataset);
    group.unlink(name)?;
    surviving.create(group, name, &layout)
}

fn create_column<T: hdf5::H5Type>(
    group: &Group,
    name: &str,
    layout: &ColumnLayout,
    values: &[T],
) -> Result<(), ArchiveError> {
    let dataset = group
        .new_dataset::<T>()
        .shape(Extents::Simple(SimpleExtents::resizable([0])))
        .chunk(layout.chunk.clone())
        .set_filters(&layout.filters)
        .create(name)?;
    dataset.resize([values.len()])?;
    if !values.is_empty() {
        dataset.write_slice(values, 0..values.len())?;
    }
    Ok(())
}

fn overwrite_column<T: hdf5::H5Type>(
    column: &Column<'_>,
    values: &[T],
) -> Result<(), ArchiveError> {
    column.write_rows(0, values)?;
    column.truncate(values.len())
}
