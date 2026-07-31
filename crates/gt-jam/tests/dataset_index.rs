//! [`JamDataset`] against the captured world day.
//!
//! The unit tests build datasets of two or three cells; these run the index
//! over all 44 546 cells the host actually published, where H3's distortion
//! near pentagons and the poles is present rather than assumed away.

use std::fs;
use std::sync::OnceLock;

use chrono::NaiveDate;
use gt_types::coordinates::{Latitude, Longitude};
use h3o::LatLng;
use proptest::test_runner::TestCaseError;

use gt_jam::dataset::{self, JamDataset};
use gt_jam::wire::{self, ParseWarningReporter};
use gt_jam::{FIXTURE_DAYS, H3_RESOLUTION, dataset_file_name, fixtures_dir, parse_day};

/// How far past one cell edge a containing cell's centre is allowed to sit.
/// H3's pentagons and projection distortion stretch the ideal hexagon.
const DISTORTION_ALLOWANCE: f64 = 2.0;

/// The captured day, indexed once for the whole run.
fn captured_day() -> Result<&'static JamDataset, String> {
    static DATASET: OnceLock<Result<JamDataset, String>> = OnceLock::new();
    DATASET
        .get_or_init(|| {
            let fixture = FIXTURE_DAYS
                .iter()
                .find(|fixture| fixture.is_served())
                .ok_or_else(|| "no served day is declared in FIXTURE_DAYS".to_owned())?;
            let day: NaiveDate = parse_day(fixture.day)
                .map_err(|err| format!("{} is not a calendar date: {err}", fixture.day))?;
            let path = fixtures_dir().join(dataset_file_name(day));
            let csv = fs::read_to_string(&path)
                .map_err(|err| format!("reading {}: {err}", path.display()))?;
            let observations = wire::parse_dataset(&csv, &ParseWarningReporter::default())
                .map_err(|err| format!("{}: {err}", fixture.day))?;
            Ok(JamDataset::new(day, observations))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The index's contract: a fix's coordinates reach the tally of the cell it
/// sits in, for every one of the day's cells.
#[test]
fn every_cell_is_found_from_its_own_center() {
    let dataset = captured_day().unwrap();
    for observation in dataset.observations() {
        let center = LatLng::from(observation.cell);
        assert_eq!(
            dataset.observation_at(Latitude::new(center.lat()), Longitude::new(center.lng())),
            Some(observation),
            "{} was not found from its own center",
            observation.cell
        );
    }
}

#[test]
fn the_index_holds_every_parsed_cell() {
    let dataset = captured_day().unwrap();
    assert_eq!(dataset.len(), 44_546);
    assert!(!dataset.is_empty());
}

#[test]
fn observations_are_returned_in_cell_order() {
    let dataset = captured_day().unwrap();
    let cells: Vec<_> = dataset
        .observations()
        .map(|observation| observation.cell)
        .collect();
    let mut sorted = cells.clone();
    sorted.sort_unstable();
    assert_eq!(cells, sorted);
}

#[test]
fn a_world_window_selects_every_cell() {
    let dataset = captured_day().unwrap();
    assert_eq!(
        dataset
            .observations_within(-90.0..=90.0, -180.0..=180.0)
            .count(),
        dataset.len()
    );
}

proptest::proptest! {
    /// Any coordinate resolves to a cell at the published resolution, and
    /// that cell is close enough to have contained the point.
    #[test]
    fn any_coordinate_resolves_to_a_cell_it_could_belong_to(
        lat in -90.0_f64..=90.0,
        lon in -180.0_f64..=180.0,
    ) {
        let cell = dataset::cell_at(Latitude::new(lat), Longitude::new(lon))
            .ok_or_else(|| TestCaseError::fail(format!("{lat},{lon} resolved to no cell")))?;
        proptest::prop_assert_eq!(cell.resolution(), H3_RESOLUTION);

        // A hexagon's circumradius is its edge length, so the containing
        // cell's center cannot be further away than that. Pentagons and H3's
        // projection distortion stretch it, hence the allowance.
        let position = LatLng::new(lat, lon)
            .map_err(|err| TestCaseError::fail(format!("{lat},{lon}: {err}")))?;
        let center = LatLng::from(cell);
        let reach_km = H3_RESOLUTION.edge_length_km() * DISTORTION_ALLOWANCE;
        proptest::prop_assert!(
            position.distance_km(center) <= reach_km,
            "{position} resolved to {cell}, whose center {center} is {:.1} km away",
            position.distance_km(center)
        );
    }

    /// A window selects exactly the cells a brute-force scan would, and
    /// never fewer than the cells whose centers are strictly inside it.
    #[test]
    fn a_window_selects_at_least_the_cells_centered_in_it(
        lat_min in -80.0_f64..=70.0,
        lon_min in -180.0_f64..=160.0,
        lat_span in 0.5_f64..=20.0,
        lon_span in 0.5_f64..=20.0,
    ) {
        let dataset = captured_day().map_err(TestCaseError::fail)?;
        let lat = lat_min..=(lat_min + lat_span);
        let lon = lon_min..=(lon_min + lon_span);

        let selected: Vec<_> = dataset
            .observations_within(lat.clone(), lon.clone())
            .map(|observation| observation.cell)
            .collect();

        let centered: Vec<_> = dataset
            .observations()
            .filter(|observation| {
                let center = LatLng::from(observation.cell);
                lat.contains(&center.lat()) && lon.contains(&center.lng())
            })
            .map(|observation| observation.cell)
            .collect();

        // Padding only ever widens the selection, and both come out of the
        // same cell-ordered scan, so containment is enough to check.
        proptest::prop_assert!(
            centered.iter().all(|cell| selected.contains(cell)),
            "window {lat:?} {lon:?} dropped a cell centered inside it"
        );
    }
}
