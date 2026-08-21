//! The TEC map archive: fetched global ionosphere maps, accumulated on disk.
//!
//! A day already archived costs no request: one HDF5 file holds every day of
//! maps ever fetched, queried per day. See [`schema`] for the layout.
//!
//! What is stored is the parsed maps, not the file they came from: the grid,
//! the epochs, and one TEC unit value per node with the file's exponent
//! already applied. A day reads back as the
//! [`GlobalIonosphereMaps`] it was stored from.
//!
//! An archived day can be stored again: JPL publishes a rapid map about a day
//! after the day ends and replaces it with a final one about two days later.

use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use gt_hdf5_archive::day_index::{self, DayIndex, RowPlacement};
use gt_hdf5_archive::prune::{ArchiveLayout, ExtentColumns, RowLevel};
use gt_hdf5_archive::{
    ArchiveError, ArchiveFile, Column, OpenArchive, StoredPresence, attributes, dates,
};
use gt_ionex::IonexProduct;
use gt_ionex::grid::{AxisDeclaration, GridAxis, LatitudeAxis, LongitudeAxis, MapGrid};
use gt_ionex::maps::{GlobalIonosphereMaps, TecMap};
use gt_ionex::tec::TotalElectronContent;
use hdf5::Group;
use parking_lot::Mutex;

use crate::schema::StoredProduct;

pub mod schema;

/// Name of the archive file, joined to the data directory by the caller.
pub const FILE_NAME: &str = "tec.h5";

/// The archive's name in messages about its columns.
const ARCHIVE_NAME: &str = "TEC map archive";

#[derive(Debug, thiserror::Error)]
pub enum IonexStoreError {
    #[error("archive error: {0}")]
    Backend(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("archive schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },

    #[error("archive is inconsistent: {0}")]
    Corrupt(String),
}

impl From<ArchiveError> for IonexStoreError {
    fn from(err: ArchiveError) -> Self {
        match err {
            ArchiveError::Backend(message) => Self::Backend(message),
            ArchiveError::Io(err) => Self::Io(err),
            ArchiveError::SchemaTooNew { found, supported } => {
                Self::SchemaTooNew { found, supported }
            }
            ArchiveError::Corrupt(message) => Self::Corrupt(message),
        }
    }
}

impl From<hdf5::Error> for IonexStoreError {
    fn from(err: hdf5::Error) -> Self {
        ArchiveError::from(err).into()
    }
}

/// One day's entry in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedMapDay {
    pub day: NaiveDate,
    /// How many maps are stored for the day.
    pub map_count: u32,
    /// Which product they came from. A day archived from
    /// [`IonexProduct::Rapid`] is fetched again once the final one appears.
    pub product: IonexProduct,
    pub fetched_at: DateTime<Utc>,
    /// Host that served it.
    pub host: String,
}

/// The TEC map archive.
#[derive(Debug)]
pub struct IonexStore {
    /// Every operation holds the lock for its whole sequence:
    /// [`Self::insert_or_replace_day`] appends values, appends maps, writes
    /// the day's own columns and indexes the day last, and a caller reading
    /// between those steps sees rows that no day entry names.
    archive: Mutex<ArchiveFile>,
    /// Held beside the lock: a caller reading the archive's path never waits
    /// for a delete rewriting it.
    path: PathBuf,
}

impl IonexStore {
    /// Open the archive at `path`, creating it if it does not exist.
    ///
    /// An archive created before archives recorded their free space in pages
    /// is rebuilt first, see [`ArchiveFile::migrate_file_space_if_needed`].
    ///
    /// Rows left behind by an interrupted store are dropped here, and so are
    /// the days an interrupted [`Self::delete_days_before`] left in an unknown
    /// layout.
    pub fn open_or_create(path: &Path) -> Result<Self, IonexStoreError> {
        let mut archive = ArchiveFile::new(path);
        if archive.exists() {
            archive.migrate_file_space_if_needed()?;
            archive.validate_schema_version(
                schema::SCHEMA_VERSION_ATTR,
                schema::CURRENT_SCHEMA_VERSION,
            )?;
            Self::recover_interrupted_delete(&mut archive)?;
            Self::drop_unindexed_rows(&mut archive)?;
        } else {
            Self::create(&mut archive)?;
        }
        Ok(Self {
            archive: Mutex::new(archive),
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn create(archive: &mut ArchiveFile) -> Result<(), IonexStoreError> {
        let file = archive.create()?;
        attributes::write_i64(
            &file,
            schema::SCHEMA_VERSION_ATTR,
            schema::CURRENT_SCHEMA_VERSION,
        )?;

        let days = file.create_group(schema::DAYS_GROUP)?;
        DayIndex::create_columns(&days, schema::DAY_FORMAT)?;
        Column::create::<u8>(&days, schema::DAY_PRODUCT, schema::DAY_FORMAT)?;
        Column::create::<i64>(&days, schema::DAY_INTERVAL_SECONDS, schema::DAY_FORMAT)?;
        for name in [
            schema::DAY_SHELL_HEIGHT_KM,
            schema::DAY_LATITUDE_FIRST_DEGREES,
            schema::DAY_LATITUDE_LAST_DEGREES,
            schema::DAY_LATITUDE_STEP_DEGREES,
            schema::DAY_LONGITUDE_FIRST_DEGREES,
            schema::DAY_LONGITUDE_LAST_DEGREES,
            schema::DAY_LONGITUDE_STEP_DEGREES,
        ] {
            Column::create::<f64>(&days, name, schema::DAY_FORMAT)?;
        }

        let maps = file.create_group(schema::MAPS_GROUP)?;
        Column::create::<i64>(&maps, schema::MAP_EPOCH, schema::MAP_FORMAT)?;
        Column::create::<u64>(&maps, schema::MAP_VALUE_OFFSET, schema::MAP_FORMAT)?;
        Column::create::<u64>(&maps, schema::MAP_VALUE_COUNT, schema::MAP_FORMAT)?;

        let values = file.create_group(schema::VALUES_GROUP)?;
        Column::create::<f64>(&values, schema::VALUE_TECU, schema::VALUE_FORMAT)?;
        Column::create::<u8>(&values, schema::VALUE_PRESENCE, schema::VALUE_FORMAT)?;
        Ok(())
    }

    fn recover_interrupted_delete(archive: &mut ArchiveFile) -> Result<(), IonexStoreError> {
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| {
            layout.recover_interrupted_delete(ARCHIVE_NAME)
        })
    }

    /// Cut the rows an interrupted store left behind, outermost group first:
    /// map rows no day names, then value rows no surviving map names, then the
    /// per-day columns down to the day index.
    fn drop_unindexed_rows(archive: &mut ArchiveFile) -> Result<(), IonexStoreError> {
        let file = archive.open_read_write()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let maps = file.group(schema::MAPS_GROUP)?;
        let values = file.group(schema::VALUES_GROUP)?;

        DayIndex::new(&days).drop_unindexed_rows(&maps, &schema::MAP_COLUMNS, ARCHIVE_NAME)?;

        let reached = values_reached(&maps)?;
        for name in schema::VALUE_COLUMNS {
            let column = Column::new(&values, name);
            let rows = column.rows()?;
            if rows < reached {
                return Err(IonexStoreError::Corrupt(format!(
                    "TEC map archive column {name} holds {rows} rows but the maps reach {reached}"
                )));
            }
            if rows > reached {
                log::warn!(
                    "Dropping {} unindexed rows from TEC column {name:?}",
                    rows - reached
                );
                column.truncate(reached)?;
            }
        }

        let indexed_days = Column::new(&days, day_index::DAY).rows()?;
        for name in schema::DAY_COLUMNS {
            let column = Column::new(&days, name);
            let rows = column.rows()?;
            if rows < indexed_days {
                return Err(IonexStoreError::Corrupt(format!(
                    "TEC map archive column {name} holds {rows} rows but {indexed_days} days are indexed"
                )));
            }
            if rows > indexed_days {
                column.truncate(indexed_days)?;
            }
        }
        Ok(())
    }

    /// Remove every day before `cutoff`, reporting how many days went.
    ///
    /// The maps and values the remaining days hold move down to close the
    /// gap. The file itself rarely shrinks: the space is what the days stored
    /// after the delete are written into.
    pub fn delete_days_before(&self, cutoff: NaiveDate) -> Result<usize, IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| layout.delete_days_before(cutoff))
    }

    /// Remove every archived day, reporting how many went.
    pub fn delete_all_days(&self) -> Result<usize, IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_write()?;
        with_layout(&file, |layout| layout.delete_all_days())
    }

    /// Every day archived, oldest first.
    pub fn archived_days(&self) -> Result<Vec<ArchivedMapDay>, IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let index = DayIndex::new(&days);
        let products: Vec<u8> = Column::new(&days, schema::DAY_PRODUCT).read()?;

        index
            .entries()?
            .into_iter()
            .map(|entry| {
                let row = index.row_of(entry.day)?.ok_or_else(|| {
                    IonexStoreError::Corrupt(format!("{} left the day index", entry.day))
                })?;
                let &code = products.get(row).ok_or_else(|| {
                    IonexStoreError::Corrupt(format!("{} has no product", entry.day))
                })?;
                let product = StoredProduct::from_code(code).ok_or_else(|| {
                    IonexStoreError::Corrupt(format!("{} has product code {code}", entry.day))
                })?;
                Ok(ArchivedMapDay {
                    day: entry.day,
                    map_count: entry.rows,
                    product: product.into(),
                    fetched_at: entry.fetched_at,
                    host: entry.host,
                })
            })
            .collect()
    }

    /// Whether `day` is archived.
    pub fn contains(&self, day: NaiveDate) -> Result<bool, IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        Ok(DayIndex::new(&days).row_of(day)?.is_some())
    }

    /// Which product `day` was archived from, or [`None`] if it is not
    /// archived.
    pub fn archived_product(
        &self,
        day: NaiveDate,
    ) -> Result<Option<IonexProduct>, IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let Some(row) = DayIndex::new(&days).row_of(day)? else {
            return Ok(None);
        };
        let products: Vec<u8> = Column::new(&days, schema::DAY_PRODUCT).read()?;
        let &code = products
            .get(row)
            .ok_or_else(|| IonexStoreError::Corrupt(format!("{day} has no product")))?;
        StoredProduct::from_code(code)
            .map(|product| Some(product.into()))
            .ok_or_else(|| IonexStoreError::Corrupt(format!("{day} has product code {code}")))
    }

    /// Store `maps` as the maps of `day`, served by `host` from `product`,
    /// replacing whatever was archived for that day.
    pub fn insert_or_replace_day(
        &self,
        day: NaiveDate,
        host: &str,
        fetched_at: DateTime<Utc>,
        product: IonexProduct,
        maps: &GlobalIonosphereMaps,
    ) -> Result<(), IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_write()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let map_group = file.group(schema::MAPS_GROUP)?;
        let value_group = file.group(schema::VALUES_GROUP)?;

        let grid = maps.grid();
        let nodes_per_map = grid.latitudes.node_count() * grid.longitudes.node_count();
        let first_map_row = Column::new(&map_group, schema::MAP_EPOCH).rows()?;

        let mut epochs: Vec<i64> = Vec::with_capacity(maps.maps().len());
        let mut value_offsets: Vec<u64> = Vec::with_capacity(maps.maps().len());
        let mut value_counts: Vec<u64> = Vec::with_capacity(maps.maps().len());
        let mut tecu: Vec<f64> = Vec::with_capacity(maps.maps().len() * nodes_per_map);
        let mut presence: Vec<u8> = Vec::with_capacity(maps.maps().len() * nodes_per_map);

        let mut next_value_row = Column::new(&value_group, schema::VALUE_TECU).rows()?;
        for map in maps.maps() {
            epochs.push(map.epoch().timestamp());
            value_offsets.push(row_index(next_value_row, "value offset")?);
            value_counts.push(row_index(nodes_per_map, "node count")?);
            for latitude_index in 0..grid.latitudes.node_count() {
                for longitude_index in 0..grid.longitudes.node_count() {
                    let value = map.value_at(gt_ionex::grid::GridPoint {
                        latitude_index,
                        longitude_index,
                    });
                    tecu.push(
                        value.map_or(schema::UNPUBLISHED_TECU_FILL, TotalElectronContent::tecu),
                    );
                    presence.push(StoredPresence::of(&value).code());
                }
            }
            next_value_row += nodes_per_map;
        }

        Column::new(&value_group, schema::VALUE_TECU).append(&tecu)?;
        Column::new(&value_group, schema::VALUE_PRESENCE).append(&presence)?;
        Column::new(&map_group, schema::MAP_EPOCH).append(&epochs)?;
        Column::new(&map_group, schema::MAP_VALUE_OFFSET).append(&value_offsets)?;
        Column::new(&map_group, schema::MAP_VALUE_COUNT).append(&value_counts)?;

        let index = DayIndex::new(&days);
        let row = match index.row_of(day)? {
            Some(row) => row,
            None => Column::new(&days, day_index::DAY).rows()?,
        };
        write_day_row(
            &days,
            row,
            schema::DAY_PRODUCT,
            StoredProduct::from(product).code(),
        )?;
        write_day_row(
            &days,
            row,
            schema::DAY_INTERVAL_SECONDS,
            maps.interval().num_seconds(),
        )?;
        for (name, degrees) in day_grid_columns(grid) {
            write_day_row(&days, row, name, degrees)?;
        }

        let placement = RowPlacement {
            offset: row_index(first_map_row, "map offset")?,
            rows: u32::try_from(maps.maps().len()).map_err(|err| {
                IonexStoreError::Corrupt(format!("{day} has too many maps: {err}"))
            })?,
        };
        // The day index goes last: rows an interrupted store leaves behind
        // stay unindexed, and the next open cuts them.
        index.insert_or_replace(day, placement, fetched_at, host)?;
        Ok(())
    }

    /// The maps archived for `day`, or [`None`] if the day is not archived.
    pub fn day_maps(
        &self,
        day: NaiveDate,
    ) -> Result<Option<GlobalIonosphereMaps>, IonexStoreError> {
        let mut archive = self.archive.lock();
        let file = archive.open_read_only()?;
        let days = file.group(schema::DAYS_GROUP)?;
        let index = DayIndex::new(&days);
        let (Some(map_rows), Some(row)) = (index.extent_of(day)?, index.row_of(day)?) else {
            return Ok(None);
        };

        let grid = read_day_grid(&days, row, day)?;
        let interval = read_day_scalar::<i64>(&days, row, schema::DAY_INTERVAL_SECONDS, day)?;
        let map_group = file.group(schema::MAPS_GROUP)?;
        let value_group = file.group(schema::VALUES_GROUP)?;

        let epochs: Vec<i64> =
            Column::new(&map_group, schema::MAP_EPOCH).read_slice(map_rows.clone())?;
        let offsets: Vec<u64> =
            Column::new(&map_group, schema::MAP_VALUE_OFFSET).read_slice(map_rows.clone())?;
        let counts: Vec<u64> =
            Column::new(&map_group, schema::MAP_VALUE_COUNT).read_slice(map_rows)?;

        let mut maps = Vec::with_capacity(epochs.len());
        for (position, &epoch) in epochs.iter().enumerate() {
            let (Some(&offset), Some(&count)) = (offsets.get(position), counts.get(position))
            else {
                return Err(IonexStoreError::Corrupt(format!(
                    "{day} map {position} has no values"
                )));
            };
            let rows = StoredValueExtent { offset, count }.rows(day, position)?;
            maps.push(TecMap::new(
                dates::timestamp_from_seconds(epoch)?,
                read_latitude_bands(&value_group, rows, grid, day, position)?,
            ));
        }
        Ok(Some(GlobalIonosphereMaps::new(
            grid,
            TimeDelta::seconds(interval),
            maps,
        )))
    }
}

/// Run `act` against the archive's layout: a day index over the map columns
/// beside the day's own, and the value columns each map names.
fn with_layout<T>(
    file: &OpenArchive<'_>,
    act: impl FnOnce(&ArchiveLayout<'_>) -> Result<T, ArchiveError>,
) -> Result<T, IonexStoreError> {
    let maps = file.group(schema::MAPS_GROUP)?;
    let values = file.group(schema::VALUES_GROUP)?;
    let levels = [
        RowLevel {
            group: &maps,
            columns: &[schema::MAP_EPOCH],
            extent: Some(ExtentColumns {
                offset: schema::MAP_VALUE_OFFSET,
                count: schema::MAP_VALUE_COUNT,
            }),
        },
        RowLevel {
            group: &values,
            columns: &schema::VALUE_COLUMNS,
            extent: None,
        },
    ];
    Ok(act(&ArchiveLayout {
        parent: file,
        index_name: schema::DAYS_GROUP,
        day_columns: &schema::DAY_COLUMNS,
        levels: &levels,
    })?)
}

/// The grid columns of one day, paired with the value each holds.
fn day_grid_columns(grid: MapGrid) -> [(&'static str, f64); 7] {
    let latitudes = grid.latitudes.axis();
    let longitudes = grid.longitudes.axis();
    [
        (schema::DAY_SHELL_HEIGHT_KM, grid.shell_height_km),
        (
            schema::DAY_LATITUDE_FIRST_DEGREES,
            latitudes.first_degrees(),
        ),
        (
            schema::DAY_LATITUDE_LAST_DEGREES,
            latitudes
                .last_degrees()
                .unwrap_or(latitudes.first_degrees()),
        ),
        (schema::DAY_LATITUDE_STEP_DEGREES, latitudes.step_degrees()),
        (
            schema::DAY_LONGITUDE_FIRST_DEGREES,
            longitudes.first_degrees(),
        ),
        (
            schema::DAY_LONGITUDE_LAST_DEGREES,
            longitudes
                .last_degrees()
                .unwrap_or(longitudes.first_degrees()),
        ),
        (
            schema::DAY_LONGITUDE_STEP_DEGREES,
            longitudes.step_degrees(),
        ),
    ]
}

/// Write one day's value of a per-day column, appending when the day is new
/// and overwriting when it is being replaced.
fn write_day_row(
    group: &Group,
    row: usize,
    name: &str,
    value: impl hdf5::H5Type,
) -> Result<(), IonexStoreError> {
    let column = Column::new(group, name);
    if row < column.rows()? {
        column.write_row(row, value)?;
    } else {
        column.append(&[value])?;
    }
    Ok(())
}

fn read_day_scalar<T: hdf5::H5Type + Clone>(
    group: &Group,
    row: usize,
    name: &str,
    day: NaiveDate,
) -> Result<T, IonexStoreError> {
    Column::new(group, name)
        .read_slice::<T>(row..row + 1)?
        .into_iter()
        .next()
        .ok_or_else(|| IonexStoreError::Corrupt(format!("{day} has no {name}")))
}

fn read_day_grid(group: &Group, row: usize, day: NaiveDate) -> Result<MapGrid, IonexStoreError> {
    let axis = |first: &str, last: &str, step: &str| -> Result<GridAxis, IonexStoreError> {
        let declaration = AxisDeclaration {
            first_degrees: read_day_scalar(group, row, first, day)?,
            last_degrees: read_day_scalar(group, row, last, day)?,
            step_degrees: read_day_scalar(group, row, step, day)?,
        };
        GridAxis::new(declaration)
            .map_err(|err| IonexStoreError::Corrupt(format!("{day} grid axis {first}: {err}")))
    };
    Ok(MapGrid {
        latitudes: LatitudeAxis::new(axis(
            schema::DAY_LATITUDE_FIRST_DEGREES,
            schema::DAY_LATITUDE_LAST_DEGREES,
            schema::DAY_LATITUDE_STEP_DEGREES,
        )?),
        longitudes: LongitudeAxis::new(axis(
            schema::DAY_LONGITUDE_FIRST_DEGREES,
            schema::DAY_LONGITUDE_LAST_DEGREES,
            schema::DAY_LONGITUDE_STEP_DEGREES,
        )?),
        shell_height_km: read_day_scalar(group, row, schema::DAY_SHELL_HEIGHT_KM, day)?,
    })
}

/// One map's nodes, cut into the grid's latitude bands.
fn read_latitude_bands(
    group: &Group,
    rows: Range<usize>,
    grid: MapGrid,
    day: NaiveDate,
    position: usize,
) -> Result<Vec<Vec<Option<TotalElectronContent>>>, IonexStoreError> {
    let longitude_nodes = grid.longitudes.node_count();
    let expected = grid.latitudes.node_count() * longitude_nodes;
    if rows.len() != expected {
        return Err(IonexStoreError::Corrupt(format!(
            "{day} map {position} holds {} values on a grid of {expected} nodes",
            rows.len()
        )));
    }

    let tecu: Vec<f64> = Column::new(group, schema::VALUE_TECU).read_slice(rows.clone())?;
    let presence: Vec<u8> = Column::new(group, schema::VALUE_PRESENCE).read_slice(rows)?;

    let mut bands = Vec::with_capacity(grid.latitudes.node_count());
    for band_index in 0..grid.latitudes.node_count() {
        let mut band = Vec::with_capacity(longitude_nodes);
        for node in 0..longitude_nodes {
            let at = band_index * longitude_nodes + node;
            let (Some(&value), Some(&code)) = (tecu.get(at), presence.get(at)) else {
                return Err(IonexStoreError::Corrupt(format!(
                    "{day} map {position} node {at} is missing"
                )));
            };
            let presence = StoredPresence::from_code(code).ok_or_else(|| {
                IonexStoreError::Corrupt(format!(
                    "{day} map {position} node {at} has presence code {code}"
                ))
            })?;
            band.push(match presence {
                StoredPresence::Unpublished => None,
                StoredPresence::Published => Some(TotalElectronContent::from_tecu(value)),
            });
        }
        bands.push(band);
    }
    Ok(bands)
}

/// How far into the value columns the map rows reach.
fn values_reached(maps: &Group) -> Result<usize, IonexStoreError> {
    let offsets: Vec<u64> = Column::new(maps, schema::MAP_VALUE_OFFSET).read()?;
    let counts: Vec<u64> = Column::new(maps, schema::MAP_VALUE_COUNT).read()?;
    let mut reached: usize = 0;
    for (&offset, &count) in offsets.iter().zip(&counts) {
        let end = usize::try_from(offset.saturating_add(count))
            .map_err(|err| IonexStoreError::Corrupt(format!("map extent {offset}: {err}")))?;
        reached = reached.max(end);
    }
    Ok(reached)
}

/// Where one map's nodes sit in the value columns, as the map columns record
/// it.
#[derive(Debug, Clone, Copy)]
struct StoredValueExtent {
    offset: u64,
    count: u64,
}

impl StoredValueExtent {
    fn rows(self, day: NaiveDate, position: usize) -> Result<Range<usize>, IonexStoreError> {
        let start = usize::try_from(self.offset).map_err(|err| {
            IonexStoreError::Corrupt(format!(
                "{day} map {position} offset {}: {err}",
                self.offset
            ))
        })?;
        let count = usize::try_from(self.count).map_err(|err| {
            IonexStoreError::Corrupt(format!("{day} map {position} count {}: {err}", self.count))
        })?;
        Ok(start..start + count)
    }
}

fn row_index(rows: usize, what: &str) -> Result<u64, IonexStoreError> {
    u64::try_from(rows).map_err(|err| IonexStoreError::Corrupt(format!("{what} {rows}: {err}")))
}
