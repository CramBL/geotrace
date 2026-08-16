//! Validate the committed IONEX captures.
//!
//! Guards [`gt_ionex::FIXTURE_FILES`], the capture harness
//! (`examples/fetch_ionex_fixtures.rs`), and the files under `tests/fixtures/`
//! against each other, and checks the captures are still the shape the parser
//! is written for.

mod support;

use std::collections::BTreeSet;

use chrono::TimeDelta;
use serde_json::Value;

use gt_ionex::FIXTURE_FILES;
use gt_ionex::grid::GridPoint;
use gt_ionex::maps::{GlobalIonosphereMaps, TecMap};
use gt_ionex::tec::TotalElectronContent;
use gt_types::{Latitude, Longitude};

/// How far an interpolated value may stand from the mean of the nodes it
/// falls between, which the two arithmetics do not round alike.
const TECU_TOLERANCE: f64 = 1e-9;

/// How far a captured shell height may stand from the one JPL declares.
const HEIGHT_TOLERANCE_KM: f64 = 1e-9;

const HTTP_OK: u64 = 200;

/// The grid JPL publishes its global maps on.
const LATITUDE_NODES: usize = 71;
const LONGITUDE_NODES: usize = 73;
const SHELL_HEIGHT_KM: f64 = 450.0;

/// A capture covers one UTC day, so its times are named by how far they
/// stand from its first map.
fn value_at(
    maps: &GlobalIonosphereMaps,
    latitude: f64,
    longitude: f64,
    after_the_first_epoch: TimeDelta,
) -> Option<TotalElectronContent> {
    let time = maps.epoch_of_first_map()? + after_the_first_epoch;
    maps.total_electron_content_at(Latitude::new(latitude), Longitude::new(longitude), time)
}

fn assert_tecu_near(value: Option<TotalElectronContent>, expected_tecu: f64) {
    let tecu = value.map(TotalElectronContent::tecu);
    assert!(
        tecu.is_some_and(|tecu| (tecu - expected_tecu).abs() < TECU_TOLERANCE),
        "{tecu:?} is not {expected_tecu} TECU"
    );
}

/// The manifest agrees with what each fixture declares.
#[test]
fn every_declared_fixture_has_a_matching_manifest_entry() {
    for fixture in FIXTURE_FILES {
        let entry = support::manifest_entry(fixture.name).unwrap();
        assert_eq!(
            entry.get("url").and_then(Value::as_str),
            Some(fixture.url),
            "{}: the capture was taken from another URL than FIXTURE_FILES declares",
            fixture.name
        );
        assert_eq!(
            entry.get("file_name").and_then(Value::as_str),
            Some(fixture.file_name),
            "{}: the capture was stored under another name",
            fixture.name
        );
        assert_eq!(
            entry.get("http_status").and_then(Value::as_u64),
            Some(HTTP_OK),
            "{}: the archive did not serve the file",
            fixture.name
        );
        assert!(
            entry
                .get("captured_at")
                .and_then(Value::as_str)
                .is_some_and(|captured_at| !captured_at.is_empty()),
            "{} has no capture date",
            fixture.name
        );
    }
}

/// No entry survives a dropped fixture, and no file is captured undeclared.
#[test]
fn the_manifest_lists_exactly_the_declared_fixtures() {
    let declared: BTreeSet<&str> = FIXTURE_FILES.iter().map(|fixture| fixture.name).collect();
    let recorded: Vec<String> = support::manifest_entries()
        .unwrap()
        .iter()
        .filter_map(|entry| Some(entry.get("name")?.as_str()?.to_owned()))
        .collect();
    let recorded: BTreeSet<&str> = recorded.iter().map(String::as_str).collect();
    assert_eq!(declared, recorded);
}

#[test]
fn every_capture_parses_into_what_the_manifest_records() {
    for fixture in FIXTURE_FILES {
        let maps = support::captured_maps(fixture.name).unwrap();
        let entry = support::manifest_entry(fixture.name).unwrap();
        let recorded = |field: &str| entry.get(field).and_then(Value::as_u64);
        assert_eq!(
            recorded("maps"),
            u64::try_from(maps.maps().len()).ok(),
            "{}: the file on disk is not the one the manifest describes",
            fixture.name
        );
        assert_eq!(
            recorded("latitude_nodes"),
            u64::try_from(maps.grid().latitudes.node_count()).ok(),
            "{}: the grid on disk is not the one the manifest describes",
            fixture.name
        );
        assert_eq!(
            recorded("longitude_nodes"),
            u64::try_from(maps.grid().longitudes.node_count()).ok(),
            "{}: the grid on disk is not the one the manifest describes",
            fixture.name
        );
    }
}

/// Both captures are published on the grid JPL declares: 2.5 deg by 5 deg,
/// pole caps left out, a shell at 450 km, and a map every two hours from
/// midnight to midnight.
#[test]
fn every_capture_holds_a_day_of_maps_on_the_published_grid() {
    for fixture in FIXTURE_FILES {
        let maps = support::captured_maps(fixture.name).unwrap();
        let grid = maps.grid();
        assert_eq!(
            grid.latitudes.node_count(),
            LATITUDE_NODES,
            "{}",
            fixture.name
        );
        assert_eq!(
            grid.longitudes.node_count(),
            LONGITUDE_NODES,
            "{}",
            fixture.name
        );
        assert_eq!(grid.latitudes.degrees_at(0), Some(87.5), "{}", fixture.name);
        assert_eq!(
            grid.latitudes.degrees_at(LATITUDE_NODES - 1),
            Some(-87.5),
            "{}",
            fixture.name
        );
        assert_eq!(
            grid.longitudes.degrees_at(0),
            Some(-180.0),
            "{}",
            fixture.name
        );
        assert_eq!(
            grid.longitudes.degrees_at(LONGITUDE_NODES - 1),
            Some(180.0),
            "{}",
            fixture.name
        );
        assert!(
            (grid.shell_height_km - SHELL_HEIGHT_KM).abs() < HEIGHT_TOLERANCE_KM,
            "{}: a shell at {} km",
            fixture.name,
            grid.shell_height_km
        );
        assert_eq!(maps.interval(), TimeDelta::hours(2), "{}", fixture.name);
        assert_eq!(maps.maps().len(), 13, "{}", fixture.name);

        let first = maps.epoch_of_first_map().unwrap();
        let last = maps.epoch_of_last_map().unwrap();
        assert_eq!(last - first, TimeDelta::days(1), "{}", fixture.name);
    }
}

/// JPL publishes a value at every node, so the captures exercise the parser's
/// full grid and none of its gap handling.
#[test]
fn every_captured_node_holds_a_published_value() {
    for fixture in FIXTURE_FILES {
        let maps = support::captured_maps(fixture.name).unwrap();
        let gaps = maps
            .maps()
            .iter()
            .flat_map(TecMap::values)
            .filter(Option::is_none)
            .count();
        assert_eq!(gaps, 0, "{}", fixture.name);
        assert_eq!(
            support::manifest_entry(fixture.name)
                .unwrap()
                .get("gaps")
                .and_then(Value::as_u64),
            Some(0),
            "{}",
            fixture.name
        );
        assert_eq!(
            maps.maps().iter().flat_map(TecMap::values).count(),
            13 * LATITUDE_NODES * LONGITUDE_NODES,
            "{}",
            fixture.name
        );
    }
}

/// The values the storm capture writes in its first map, scaled by the
/// exponent of -1 its header declares.
#[test]
fn the_stored_integers_read_back_as_the_tec_units_they_stand_for() {
    let maps = support::captured_maps(support::STORM_CAPTURE).unwrap();
    let map = maps.maps().first().unwrap();
    assert_eq!(
        map.value_at(GridPoint {
            latitude_index: 0,
            longitude_index: 0
        }),
        Some(TotalElectronContent::from_tecu(26.3)),
        "the northwestern node, stored as 263"
    );
    assert_eq!(
        map.value_at(GridPoint {
            latitude_index: 35,
            longitude_index: 36
        }),
        Some(TotalElectronContent::from_tecu(39.5)),
        "the equator at the prime meridian, stored as 395"
    );
}

#[test]
fn the_storm_day_reaches_far_higher_than_the_quiet_day() {
    let storm = support::captured_maps(support::STORM_CAPTURE).unwrap();
    let quiet = support::captured_maps(support::QUIET_CAPTURE).unwrap();
    assert_eq!(
        storm.peak_total_electron_content(),
        Some(TotalElectronContent::from_tecu(175.2))
    );
    assert_eq!(
        quiet.peak_total_electron_content(),
        Some(TotalElectronContent::from_tecu(135.8))
    );
}

/// The peak of the storm capture stands where the file writes it, so a query
/// on a node and an epoch reads that node back.
#[test]
fn a_query_on_a_node_and_an_epoch_reads_the_stored_value() {
    let maps = support::captured_maps(support::STORM_CAPTURE).unwrap();
    assert_eq!(
        value_at(&maps, 15.0, -105.0, TimeDelta::hours(22)),
        Some(TotalElectronContent::from_tecu(175.2))
    );
    assert_eq!(
        value_at(&maps, 87.5, -180.0, TimeDelta::zero()),
        Some(TotalElectronContent::from_tecu(26.3))
    );
    assert_eq!(
        value_at(&maps, 0.0, 0.0, TimeDelta::zero()),
        Some(TotalElectronContent::from_tecu(39.5))
    );
}

/// Both ends of the grid name the same meridian, and the file writes the same
/// value at each.
#[test]
fn the_repeated_meridian_answers_the_same_from_both_ends() {
    let maps = support::captured_maps(support::STORM_CAPTURE).unwrap();
    assert_eq!(
        value_at(&maps, 87.5, 180.0, TimeDelta::zero()),
        value_at(&maps, 87.5, -180.0, TimeDelta::zero())
    );
    assert_eq!(
        value_at(&maps, 87.5, 180.0, TimeDelta::zero()),
        Some(TotalElectronContent::from_tecu(26.3))
    );
}

/// Halfway between two nodes, or two epochs, is the mean of them.
#[test]
fn a_query_between_nodes_and_epochs_interpolates_between_them() {
    let maps = support::captured_maps(support::STORM_CAPTURE).unwrap();
    assert_tecu_near(
        value_at(&maps, 0.0, 2.5, TimeDelta::zero()),
        (39.5 + 41.5) / 2.0,
    );
    assert_tecu_near(
        value_at(&maps, 1.25, 0.0, TimeDelta::zero()),
        (39.5 + 34.9) / 2.0,
    );
    assert_tecu_near(
        value_at(&maps, 87.5, -180.0, TimeDelta::hours(1)),
        (26.3 + 27.6) / 2.0,
    );
}

#[test]
fn a_query_outside_the_captured_day_or_grid_has_no_value() {
    let maps = support::captured_maps(support::STORM_CAPTURE).unwrap();
    assert_eq!(value_at(&maps, 0.0, 0.0, TimeDelta::seconds(-1)), None);
    assert_eq!(
        value_at(&maps, 0.0, 0.0, TimeDelta::days(1) + TimeDelta::seconds(1)),
        None
    );
    assert_eq!(value_at(&maps, 89.0, 0.0, TimeDelta::hours(12)), None);
    assert_eq!(value_at(&maps, -89.0, 0.0, TimeDelta::hours(12)), None);
    assert_eq!(
        value_at(&maps, 87.5, 0.0, TimeDelta::days(1)),
        maps.maps().last().unwrap().value_at(GridPoint {
            latitude_index: 0,
            longitude_index: 36
        }),
        "the last epoch itself is still covered"
    );
}

/// One TEC unit is the delay every hover shows beside the value.
#[test]
fn the_storm_peak_delays_l1_by_the_published_relation() {
    let maps = support::captured_maps(support::STORM_CAPTURE).unwrap();
    let peak = maps.peak_total_electron_content().unwrap();
    let delay = peak.l1_delay_meters();
    assert!(
        (28.0..29.0).contains(&delay),
        "{delay} m of L1 delay at {peak:?}"
    );
}
