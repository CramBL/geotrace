//! Round-trip days of maps through a real archive file in a temp directory.

use std::path::Path;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use rstest::rstest;
use tempfile::TempDir;

use gt_hdf5_archive::day_index;
use gt_hdf5_archive::prune::{
    DeclinedRecovery, DeleteState, InterruptedDelete, InterruptedDeleteRecovery,
};
use gt_hdf5_archive::{ReadOnlyDayArchive as _, WritableDayArchive as _};
use gt_ionex::IonexProduct;
use gt_ionex::grid::{AxisDeclaration, GridAxis, GridPoint, LatitudeAxis, LongitudeAxis, MapGrid};
use gt_ionex::maps::{GlobalIonosphereMaps, TecMap};
use gt_ionex::tec::TotalElectronContent;
use gt_ionex_store::{FILE_NAME, IonexStore, IonexStoreError, ReadOnlyIonexStore, schema};
use gt_test_utils::day_archive::{self, ColumnName, GroupPath};
use gt_types::{Latitude, Longitude};

const HOST: &str = "https://sideshow.jpl.nasa.gov/pub/iono_daily";

fn store() -> Result<(TempDir, IonexStore), String> {
    let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
    let store = IonexStore::open_or_create(&dir.path().join(FILE_NAME))
        .map_err(|err| format!("open archive: {err}"))?;
    Ok((dir, store))
}

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 5, 10).unwrap_or_default() + TimeDelta::days(offset)
}

fn fetched_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_784_505_600, 0).unwrap_or_default()
}

fn epoch(day: NaiveDate, hour: i64) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc() + TimeDelta::hours(hour)
}

fn axis(first: f64, last: f64, step: f64) -> Result<GridAxis, String> {
    GridAxis::new(AxisDeclaration {
        first_degrees: first,
        last_degrees: last,
        step_degrees: step,
    })
    .map_err(|err| format!("axis {first} to {last} by {step}: {err}"))
}

fn grid(latitudes: (f64, f64, f64), longitudes: (f64, f64, f64)) -> Result<MapGrid, String> {
    Ok(MapGrid {
        latitudes: LatitudeAxis::new(axis(latitudes.0, latitudes.1, latitudes.2)?),
        longitudes: LongitudeAxis::new(axis(longitudes.0, longitudes.1, longitudes.2)?),
        shell_height_km: 450.0,
    })
}

/// The axes JPL publishes global maps on: 71 latitudes descending, 73
/// longitudes with the meridian repeated at both ends.
fn published_grid() -> Result<MapGrid, String> {
    grid((87.5, -87.5, -2.5), (-180.0, 180.0, 5.0))
}

/// A grid small enough to write out node by node in an assertion.
fn small_grid() -> Result<MapGrid, String> {
    grid((10.0, 0.0, -10.0), (-180.0, 180.0, 180.0))
}

fn tecu(value: f64) -> Option<TotalElectronContent> {
    Some(TotalElectronContent::from_tecu(value))
}

/// Every node valued from its position, so a transposed read shows up.
fn map_over(grid: MapGrid, at: DateTime<Utc>, offset: f64) -> TecMap {
    let bands = (0..grid.latitudes.node_count())
        .map(|latitude_index| {
            (0..grid.longitudes.node_count())
                .map(|longitude_index| {
                    tecu(offset + latitude_index as f64 * 100.0 + longitude_index as f64)
                })
                .collect()
        })
        .collect();
    TecMap::new(at, bands)
}

/// A day of maps two hours apart on the published grid, as a fetched file
/// holds one.
fn published_day(day: NaiveDate) -> Result<GlobalIonosphereMaps, String> {
    let grid = published_grid()?;
    let maps = (0..13)
        .map(|index| map_over(grid, epoch(day, index * 2), f64::from(index as i32)))
        .collect();
    Ok(GlobalIonosphereMaps::new(grid, TimeDelta::hours(2), maps))
}

/// Two maps on the small grid, the second with a gap where the producer
/// published no value.
fn day_with_a_gap(day: NaiveDate) -> Result<GlobalIonosphereMaps, String> {
    let grid = small_grid()?;
    // The middle node of the northern band is a gap.
    let second = vec![
        vec![tecu(20.0), None, tecu(20.0)],
        vec![tecu(40.0), tecu(50.0), tecu(40.0)],
    ];
    Ok(GlobalIonosphereMaps::new(
        grid,
        TimeDelta::hours(2),
        vec![
            map_over(grid, epoch(day, 0), 0.0),
            TecMap::new(epoch(day, 2), second),
        ],
    ))
}

#[test]
fn a_day_of_published_maps_reads_back_exactly_as_it_was_stored() {
    let (_dir, store) = store().expect("archive");
    let stored = published_day(day(0)).expect("a day of maps");

    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &stored)
        .expect("insert");

    let read = store.day_maps(day(0)).expect("read");
    assert_eq!(read.as_ref(), Some(&stored));
}

/// A gap must come back a gap, never the fill the column holds in its place.
#[test]
fn a_gap_reads_back_as_a_gap_and_not_as_a_value() {
    let (_dir, store) = store().expect("archive");
    let stored = day_with_a_gap(day(0)).expect("a day with a gap");

    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Rapid, &stored)
        .expect("insert");

    let read = store
        .day_maps(day(0))
        .expect("read")
        .expect("the day is archived");
    assert_eq!(read, stored);

    let second = read.maps().get(1).expect("the day holds two maps");
    assert_eq!(
        second.value_at(GridPoint {
            latitude_index: 0,
            longitude_index: 1
        }),
        None
    );
    assert_eq!(
        second.value_at(GridPoint {
            latitude_index: 0,
            longitude_index: 0
        }),
        tecu(20.0)
    );
}

/// The grid the header declared reaches a reader unchanged, so an
/// interpolated value from the archive matches one from the file.
#[test]
fn the_grid_and_interval_survive_the_round_trip() {
    let (_dir, store) = store().expect("archive");
    let stored = published_day(day(0)).expect("a day of maps");

    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &stored)
        .expect("insert");
    let read = store
        .day_maps(day(0))
        .expect("read")
        .expect("the day is archived");

    assert_eq!(read.grid(), published_grid().expect("the grid"));
    assert_eq!(read.interval(), TimeDelta::hours(2));
    assert_eq!(read.epoch_of_first_map(), stored.epoch_of_first_map());
    assert_eq!(read.epoch_of_last_map(), stored.epoch_of_last_map());

    let at = epoch(day(0), 1) + TimeDelta::minutes(30);
    assert_eq!(
        read.total_electron_content_at(Latitude::new(12.5), Longitude::new(7.5), at),
        stored.total_electron_content_at(Latitude::new(12.5), Longitude::new(7.5), at)
    );
}

/// A day never stored has no maps and no product, which is what puts it on the
/// fetch queue.
#[test]
fn an_unarchived_day_reads_back_as_nothing() {
    let (_dir, store) = store().expect("archive");
    assert_eq!(store.day_maps(day(0)).expect("archive read"), None);
    assert!(!store.contains(day(0)).expect("archive read"));
    assert_eq!(store.archived_product(day(0)).expect("archive read"), None);
}

/// The product determines whether the day is fetched again, so it must be read
/// back for the day it belongs to.
#[rstest]
#[case::a_settled_day(IonexProduct::Final)]
#[case::an_earlier_estimate(IonexProduct::Rapid)]
fn the_product_a_day_came_from_is_recorded(#[case] product: IonexProduct) {
    let (_dir, store) = store().expect("archive");
    store
        .insert_or_replace_day(
            day(0),
            HOST,
            fetched_at(),
            product,
            &day_with_a_gap(day(0)).expect("a day with a gap"),
        )
        .expect("insert");

    assert_eq!(
        store.archived_product(day(0)).expect("archive read"),
        Some(product)
    );
    assert!(store.contains(day(0)).expect("archive read"));
}

/// The rapid maps of a day are replaced by the final ones, not appended to.
#[test]
fn storing_a_day_again_replaces_what_was_archived() {
    let (_dir, store) = store().expect("archive");
    store
        .insert_or_replace_day(
            day(0),
            HOST,
            fetched_at(),
            IonexProduct::Rapid,
            &day_with_a_gap(day(0)).expect("a day with a gap"),
        )
        .expect("insert rapid");

    let settled = published_day(day(0)).expect("a day of maps");
    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &settled)
        .expect("insert final");

    assert_eq!(
        store.day_maps(day(0)).expect("archive read").as_ref(),
        Some(&settled)
    );
    assert_eq!(
        store.archived_product(day(0)).expect("archive read"),
        Some(IonexProduct::Final)
    );
    let archived = store.archived_days().expect("archive read");
    assert_eq!(archived.len(), 1, "the day is indexed once");
    assert_eq!(archived.first().map(|entry| entry.map_count), Some(13));
}

/// Days are listed oldest first with what each was fetched from and by whom.
#[test]
fn every_archived_day_is_listed_oldest_first() {
    let (_dir, store) = store().expect("archive");
    for offset in [2, 0, 1] {
        let product = if offset == 1 {
            IonexProduct::Rapid
        } else {
            IonexProduct::Final
        };
        store
            .insert_or_replace_day(
                day(offset),
                HOST,
                fetched_at(),
                product,
                &day_with_a_gap(day(offset)).expect("a day with a gap"),
            )
            .unwrap_or_else(|err| panic!("insert {offset}: {err}"));
    }

    let archived = store.archived_days().expect("archive read");
    assert_eq!(
        archived
            .iter()
            .map(|entry| (entry.day, entry.product, entry.map_count))
            .collect::<Vec<_>>(),
        [
            (day(0), IonexProduct::Final, 2),
            (day(1), IonexProduct::Rapid, 2),
            (day(2), IonexProduct::Final, 2),
        ]
    );
    assert!(archived.iter().all(|entry| entry.host == HOST));
    assert!(
        archived
            .iter()
            .all(|entry| entry.fetched_at == fetched_at())
    );
}

/// Two days share the value columns, so each must read only its own rows.
#[test]
fn two_days_in_one_archive_keep_their_own_maps() {
    let (_dir, store) = store().expect("archive");
    let first = published_day(day(0)).expect("a day of maps");
    let second = day_with_a_gap(day(1)).expect("a day with a gap");
    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &first)
        .expect("insert first");
    store
        .insert_or_replace_day(day(1), HOST, fetched_at(), IonexProduct::Rapid, &second)
        .expect("insert second");

    assert_eq!(store.day_maps(day(0)).expect("archive read"), Some(first));
    assert_eq!(store.day_maps(day(1)).expect("archive read"), Some(second));
}

/// The archive survives being closed and opened again, which is the whole
/// point of keeping it.
#[test]
fn a_reopened_archive_still_holds_its_days() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let stored = published_day(day(0)).expect("a day of maps");
    {
        let store = IonexStore::open_or_create(&path).expect("open");
        store
            .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &stored)
            .expect("insert");
    }

    let store = IonexStore::open_or_create(&path).expect("reopen");
    assert_eq!(store.day_maps(day(0)).expect("archive read"), Some(stored));
}

/// An archive written by a newer build is rejected rather than misread.
#[test]
fn an_archive_from_a_newer_schema_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    {
        IonexStore::open_or_create(&path).expect("open");
    }
    {
        let file = hdf5::File::open_rw(&path).expect("raw open");
        file.attr(schema::SCHEMA_VERSION_ATTR)
            .and_then(|attr| attr.write_scalar(&(schema::CURRENT_SCHEMA_VERSION + 1)))
            .expect("bump the schema version");
    }

    match IonexStore::open_or_create(&path) {
        Err(IonexStoreError::SchemaTooNew { found, supported }) => {
            assert_eq!(found, schema::CURRENT_SCHEMA_VERSION + 1);
            assert_eq!(supported, schema::CURRENT_SCHEMA_VERSION);
        }
        other => panic!("the newer archive opened: {other:?}"),
    }
}

/// A file with no maps at all still records its grid, so reading it back does
/// not fail on a day the producer published an empty file for.
#[test]
fn a_day_without_maps_still_records_its_grid() {
    let (_dir, store) = store().expect("archive");
    let stored = GlobalIonosphereMaps::new(
        published_grid().expect("the grid"),
        TimeDelta::hours(2),
        Vec::new(),
    );

    store
        .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &stored)
        .expect("insert");

    assert_eq!(store.day_maps(day(0)).expect("archive read"), Some(stored));
}

/// Grow every column an interrupted store would have appended to before it
/// indexed the day, at all three levels at once.
fn append_unindexed_rows(path: &Path, rows: usize) -> Result<(), String> {
    let file = hdf5::File::open_rw(path).map_err(|err| format!("raw open: {err}"))?;
    let groups = [
        (schema::DAYS_GROUP, schema::DAY_COLUMNS.as_slice()),
        (schema::MAPS_GROUP, schema::MAP_COLUMNS.as_slice()),
        (schema::VALUES_GROUP, schema::VALUE_COLUMNS.as_slice()),
    ];
    for (group_path, columns) in groups {
        for &name in columns {
            let dataset = file
                .group(group_path)
                .and_then(|group| group.dataset(name))
                .map_err(|err| format!("{group_path}/{name}: {err}"))?;
            let held = dataset.shape().first().copied().unwrap_or_default();
            dataset
                .resize([held + rows])
                .map_err(|err| format!("{group_path}/{name}: {err}"))?;
        }
    }
    Ok(())
}

/// Rows appended without a day entry, which is what an interrupted store
/// leaves at every level, are cut when the archive is reopened.
#[test]
fn unindexed_rows_are_dropped_on_open() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let stored = day_with_a_gap(day(0)).expect("a day with a gap");
    {
        let store = IonexStore::open_or_create(&path).expect("open");
        store
            .insert_or_replace_day(day(0), HOST, fetched_at(), IonexProduct::Final, &stored)
            .expect("insert");
    }
    append_unindexed_rows(&path, 5).expect("grow the columns");

    let reopened = IonexStore::open_or_create(&path).expect("reopen");
    assert_eq!(
        reopened.day_maps(day(0)).expect("archive read"),
        Some(stored),
        "the archived day survives"
    );
    drop(reopened);

    assert_eq!(
        day_archive::column_rows(&path, DAYS, ColumnName(schema::DAY_PRODUCT))
            .expect("column rows"),
        1,
        "one row per indexed day"
    );
    assert_eq!(
        day_archive::column_rows(
            &path,
            GroupPath(schema::MAPS_GROUP),
            ColumnName(schema::MAP_EPOCH)
        )
        .expect("column rows"),
        2,
        "one row per map of the indexed day"
    );
    assert_eq!(
        day_archive::column_rows(
            &path,
            GroupPath(schema::VALUES_GROUP),
            ColumnName(schema::VALUE_TECU)
        )
        .expect("column rows"),
        12,
        "two maps of six nodes"
    );
}

/// A column shorter than what refers to it means archived rows are missing,
/// which no recovery can invent.
#[rstest]
#[case::a_day_column(schema::DAYS_GROUP, schema::DAY_PRODUCT)]
#[case::a_map_column(schema::MAPS_GROUP, schema::MAP_EPOCH)]
#[case::a_value_column(schema::VALUES_GROUP, schema::VALUE_TECU)]
fn a_column_shorter_than_what_refers_to_it_is_reported(
    #[case] group_path: &str,
    #[case] name: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    {
        let store = IonexStore::open_or_create(&path).expect("open");
        store
            .insert_or_replace_day(
                day(0),
                HOST,
                fetched_at(),
                IonexProduct::Final,
                &day_with_a_gap(day(0)).expect("a day with a gap"),
            )
            .expect("insert");
    }
    {
        let file = hdf5::File::open_rw(&path).expect("raw open");
        file.group(group_path)
            .and_then(|group| group.dataset(name))
            .and_then(|dataset| dataset.resize([0]))
            .map_err(|err| format!("{group_path}/{name}: {err}"))
            .expect("truncate the column");
    }

    match IonexStore::open_or_create(&path) {
        Err(IonexStoreError::Corrupt(detail)) => assert!(detail.contains(name), "{detail}"),
        other => panic!("the truncated column opened: {other:?}"),
    }
}

/// Days go from the front and the maps of the rest read back through the
/// offsets the delete rebased, at both levels: the day names its maps, and
/// each map names its values.
#[test]
fn deleting_days_before_a_cutoff_keeps_the_maps_of_the_rest() {
    let (_dir, store) = store().expect("archive");
    let deleted = published_day(day(0)).expect("a day of maps");
    let kept = day_with_a_gap(day(1)).expect("a day with a gap");
    let newest = published_day(day(2)).expect("a day of maps");
    for (day, maps) in [(day(0), &deleted), (day(1), &kept), (day(2), &newest)] {
        store
            .insert_or_replace_day(day, HOST, fetched_at(), IonexProduct::Final, maps)
            .expect("insert");
    }

    let removed = store.delete_days_before(day(1), None).expect("delete days");

    assert_eq!(removed, 1);
    assert_eq!(store.day_maps(day(0)).expect("archive read"), None);
    assert_eq!(store.day_maps(day(1)).expect("archive read"), Some(kept));
    assert_eq!(store.day_maps(day(2)).expect("archive read"), Some(newest));
    assert_eq!(
        store
            .archived_days()
            .expect("days")
            .into_iter()
            .map(|archived| archived.day)
            .collect::<Vec<NaiveDate>>(),
        [day(1), day(2)]
    );
}

#[test]
fn deleting_every_day_empties_the_archive() {
    let (_dir, store) = store().expect("archive");
    for offset in 0..2 {
        store
            .insert_or_replace_day(
                day(offset),
                HOST,
                fetched_at(),
                IonexProduct::Final,
                &published_day(day(offset)).expect("a day of maps"),
            )
            .expect("insert");
    }

    let removed = store.delete_all_days(None).expect("delete all");

    assert_eq!(removed, 2);
    assert!(store.archived_days().expect("days").is_empty());
    assert_eq!(store.day_maps(day(0)).expect("archive read"), None);
}

/// The product and grid a day was archived from sit in columns beside the day
/// index, and move with it.
#[test]
fn the_columns_beside_the_day_index_move_with_the_days_that_stay() {
    let (_dir, store) = store().expect("archive");
    store
        .insert_or_replace_day(
            day(0),
            HOST,
            fetched_at(),
            IonexProduct::Final,
            &published_day(day(0)).expect("a day of maps"),
        )
        .expect("insert");
    let kept = day_with_a_gap(day(1)).expect("a day with a gap");
    store
        .insert_or_replace_day(day(1), HOST, fetched_at(), IonexProduct::Rapid, &kept)
        .expect("insert");

    store.delete_days_before(day(1), None).expect("delete days");

    assert_eq!(
        store.archived_product(day(1)).expect("product"),
        Some(IonexProduct::Rapid)
    );
    assert_eq!(store.day_maps(day(1)).expect("archive read"), Some(kept));
}

/// The archive's day index, which is where a delete records that it is
/// part-way through.
const DAYS: GroupPath<'static> = GroupPath(schema::DAYS_GROUP);

/// Write access taken from an instance part-way through a delete must not
/// discard its days behind the user's back. The archive reports what
/// recovering costs, a declined recovery leaves every day where it is, and an
/// open that accepts the recovery still discards them.
#[test]
fn declining_recovery_leaves_the_interrupted_archive_as_it_was() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    let store = IonexStore::open_or_create(&path).expect("open");
    for offset in 0..2 {
        store
            .insert_or_replace_day(
                day(offset),
                HOST,
                fetched_at(),
                IonexProduct::Final,
                &day_with_a_gap(day(offset)).expect("a day with a gap"),
            )
            .expect("insert");
    }
    drop(store);
    day_archive::mark_delete_in_flight(&path, DAYS).expect("mark the delete");

    let interrupted = ReadOnlyIonexStore::interrupted_delete_at(&path).expect("inspect");
    let declined =
        IonexStore::open_or_create_with_recovery_choice(&path, InterruptedDeleteRecovery::Decline)
            .expect_err("the archive is unavailable until the recovery is accepted");

    assert_eq!(interrupted, Some(InterruptedDelete { archived_days: 2 }));
    assert!(
        matches!(
            declined,
            IonexStoreError::DeclinedRecovery(DeclinedRecovery(InterruptedDelete {
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
            GroupPath(schema::MAPS_GROUP),
            ColumnName(schema::MAP_EPOCH)
        )
        .expect("maps"),
        4
    );

    let store = IonexStore::open_or_create(&path).expect("open accepting the recovery");
    assert!(store.archived_days().expect("days").is_empty());
}

/// Nothing to recover, so the choice does not matter and inspection reports
/// none.
#[test]
fn a_settled_archive_reports_no_interrupted_delete() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);

    assert_eq!(
        ReadOnlyIonexStore::interrupted_delete_at(&path).expect("before the archive exists"),
        None
    );
    IonexStore::open_or_create(&path).expect("create");

    assert_eq!(
        ReadOnlyIonexStore::interrupted_delete_at(&path).expect("a settled archive"),
        None
    );
    IonexStore::open_or_create_with_recovery_choice(&path, InterruptedDeleteRecovery::Decline)
        .expect("a settled archive opens whatever the choice");
}
