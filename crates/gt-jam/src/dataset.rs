//! One published UTC day, indexed for lookup and for drawing.
//!
//! Only resolution 4 is stored. Coarser parents for low zoom were dropped:
//! summing 49 children into one resolution 2 cell is aircraft-weighted, and
//! on the captured day that paints 51 % of the regions holding a cell above
//! the 10 % breakpoint as though they were below it.

use std::ops::RangeInclusive;

use chrono::NaiveDate;
use gt_types::coordinates::{Latitude, Longitude};
use h3o::{CellIndex, LatLng};

use crate::H3_RESOLUTION;
use crate::wire::HexObservation;

/// Longitude degrees spanning the whole world, the widest any padding can
/// usefully be.
const FULL_LON_SPAN: f64 = 360.0;

/// Cosine of a latitude close enough to a pole that longitude padding stops
/// being meaningful (about 89.9 degrees). Below it, the whole longitude
/// range is taken instead of dividing by a vanishing cosine.
const MIN_COS_LAT: f64 = 0.001;

/// The cell containing a position, at the published resolution.
///
/// [`None`] only for a coordinate outside the globe. [`Latitude`] and
/// [`Longitude`] debug-assert their ranges but do not enforce them in
/// release, so this is a release-only path and has no test.
pub fn cell_at(lat: Latitude, lon: Longitude) -> Option<CellIndex> {
    let position = LatLng::new(lat.as_degrees(), lon.as_degrees()).ok()?;
    Some(position.to_cell(H3_RESOLUTION))
}

/// Latitude degrees from a cell's centre to its furthest vertex.
///
/// A hexagon's circumradius equals its edge length, so a cell whose centre
/// is this far outside a window can still overlap it.
const fn cell_radius_deg() -> f64 {
    H3_RESOLUTION.edge_length_rads().to_degrees()
}

/// An observation with its cell centre, precomputed so viewport selection
/// does not redo the H3 centre math for 44 000 cells every frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct IndexedCell {
    observation: HexObservation,
    center_lat: f64,
    center_lon: f64,
}

impl IndexedCell {
    fn new(observation: HexObservation) -> Self {
        let center = LatLng::from(observation.cell);
        Self {
            observation,
            center_lat: center.lat(),
            center_lon: center.lng(),
        }
    }
}

/// One published UTC day of [`HexObservation`]s.
#[derive(Debug, Clone)]
pub struct JamDataset {
    day: NaiveDate,
    /// Sorted by cell index, one entry per cell.
    cells: Vec<IndexedCell>,
}

impl JamDataset {
    /// Index `observations` for `day`.
    ///
    /// Repeated cells keep their first observation, matching
    /// [`crate::wire::parse_dataset`], whose output this normally is.
    pub fn new(day: NaiveDate, observations: Vec<HexObservation>) -> Self {
        let mut cells: Vec<IndexedCell> = observations.into_iter().map(IndexedCell::new).collect();
        cells.sort_by_key(|indexed| indexed.observation.cell);
        cells.dedup_by_key(|indexed| indexed.observation.cell);
        Self { day, cells }
    }

    /// The UTC day these observations cover.
    pub const fn day(&self) -> NaiveDate {
        self.day
    }

    /// How many cells the day published.
    pub const fn len(&self) -> usize {
        self.cells.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The observation for `cell`.
    ///
    /// An absent cell had no aircraft reported, which is not the same as
    /// having only good ones.
    pub fn observation(&self, cell: CellIndex) -> Option<&HexObservation> {
        let position = self
            .cells
            .binary_search_by_key(&cell, |indexed| indexed.observation.cell)
            .ok()?;
        self.cells.get(position).map(|indexed| &indexed.observation)
    }

    /// The observation for the cell containing a position.
    pub fn observation_at(&self, lat: Latitude, lon: Longitude) -> Option<&HexObservation> {
        self.observation(cell_at(lat, lon)?)
    }

    /// Every observation, in cell-index order.
    pub fn observations(&self) -> impl Iterator<Item = &HexObservation> {
        self.cells.iter().map(|indexed| &indexed.observation)
    }

    /// The observations to draw for a window of the map, in cell-index
    /// order.
    ///
    /// Selects on cell centres, widened by one cell radius so a cell whose
    /// polygon reaches into the window is still drawn.
    ///
    /// Degrees, not [`Latitude`] and [`Longitude`]: a viewport derived from
    /// map corners can reach past the poles, and a window wider than the
    /// world is harmless. A window crossing the antimeridian must be passed
    /// as the wider range containing it, which over-selects.
    pub fn observations_within(
        &self,
        lat: RangeInclusive<f64>,
        lon: RangeInclusive<f64>,
    ) -> impl Iterator<Item = &HexObservation> {
        let radius = cell_radius_deg();
        let lat_window = (lat.start() - radius)..=(lat.end() + radius);

        // A degree of longitude covers less ground the closer to a pole the
        // window reaches, so the same radius spans more of them there.
        let cos_lat = lat_window
            .start()
            .abs()
            .max(lat_window.end().abs())
            .to_radians()
            .cos();
        let lon_radius = if cos_lat <= MIN_COS_LAT {
            FULL_LON_SPAN
        } else {
            (radius / cos_lat).min(FULL_LON_SPAN)
        };
        let lon_window = (lon.start() - lon_radius)..=(lon.end() + lon_radius);

        self.cells
            .iter()
            .filter(move |indexed| {
                lat_window.contains(&indexed.center_lat) && lon_window.contains(&indexed.center_lon)
            })
            .map(|indexed| &indexed.observation)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use rstest::rstest;

    use super::*;

    // Cells from the captured fixture day, far enough apart to sit in
    // different windows. The comment is where each one's centre is.
    const BALTIC: &str = "841f0c9ffffffff"; // 55.016 N, 15.413 E
    const WYOMING: &str = "8426b45ffffffff"; // 43.818 N, 109.957 W

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()
    }

    fn cell(hex: &str) -> CellIndex {
        CellIndex::from_str(hex).unwrap()
    }

    fn observation(cell: CellIndex, good: u32, bad: u32) -> HexObservation {
        HexObservation { cell, good, bad }
    }

    fn center(cell: CellIndex) -> LatLng {
        LatLng::from(cell)
    }

    /// A cell's centre as the coordinate pair the public API takes.
    fn center_coords(cell: CellIndex) -> (Latitude, Longitude) {
        let center = center(cell);
        (Latitude::new(center.lat()), Longitude::new(center.lng()))
    }

    fn dataset(cells: &[&str]) -> JamDataset {
        JamDataset::new(
            day(),
            cells
                .iter()
                .enumerate()
                .map(|(index, hex)| {
                    let count = u32::try_from(index).unwrap();
                    observation(cell(hex), count + 1, count)
                })
                .collect(),
        )
    }

    #[test]
    fn an_empty_day_is_empty() {
        let dataset = JamDataset::new(day(), Vec::new());
        assert!(dataset.is_empty());
        assert_eq!(dataset.len(), 0);
        assert_eq!(dataset.day(), day());
        assert_eq!(dataset.observations().count(), 0);
    }

    #[test]
    fn a_cell_is_found_by_index_and_by_position() {
        let dataset = dataset(&[BALTIC, WYOMING]);
        let found = dataset.observation(cell(BALTIC));
        assert_eq!(
            found.map(|observation| observation.cell),
            Some(cell(BALTIC))
        );
        let (lat, lon) = center_coords(cell(BALTIC));
        assert_eq!(dataset.observation_at(lat, lon), found);
    }

    #[test]
    fn an_unpublished_cell_has_no_observation() {
        let dataset = dataset(&[BALTIC]);
        assert_eq!(dataset.observation(cell(WYOMING)), None);
    }

    #[test]
    fn observations_come_back_in_cell_order() {
        let dataset = dataset(&[WYOMING, BALTIC]);
        let cells: Vec<CellIndex> = dataset
            .observations()
            .map(|observation| observation.cell)
            .collect();
        let mut expected = cells.clone();
        expected.sort_unstable();
        assert_eq!(cells, expected);
    }

    #[test]
    fn a_repeated_cell_keeps_its_first_observation() {
        let dataset = JamDataset::new(
            day(),
            vec![
                observation(cell(BALTIC), 412, 3),
                observation(cell(BALTIC), 1, 1),
            ],
        );
        assert_eq!(dataset.len(), 1);
        assert_eq!(
            dataset.observation(cell(BALTIC)).map(|found| found.good),
            Some(412)
        );
    }

    #[test]
    fn a_world_window_selects_every_cell() {
        let dataset = dataset(&[BALTIC, WYOMING]);
        assert_eq!(
            dataset
                .observations_within(-90.0..=90.0, -180.0..=180.0)
                .count(),
            dataset.len()
        );
    }

    #[test]
    fn a_window_elsewhere_selects_nothing() {
        let dataset = dataset(&[BALTIC]);
        assert_eq!(
            dataset
                .observations_within(-40.0..=-30.0, -70.0..=-60.0)
                .count(),
            0
        );
    }

    /// Windows half a cell past the centre, then four radii past it.
    #[test]
    fn a_window_includes_cells_whose_polygon_reaches_into_it() {
        let dataset = dataset(&[BALTIC]);
        let center = center(cell(BALTIC));
        let just_past = center.lat() + cell_radius_deg() / 2.0;

        let excluding_center = dataset
            .observations_within(
                just_past..=(just_past + 1.0),
                (center.lng() - 1.0)..=(center.lng() + 1.0),
            )
            .count();
        assert_eq!(excluding_center, 1, "the cell reaches into the window");

        let far_away = center.lat() + cell_radius_deg() * 4.0;
        let beyond_reach = dataset
            .observations_within(
                far_away..=(far_away + 1.0),
                (center.lng() - 1.0)..=(center.lng() + 1.0),
            )
            .count();
        assert_eq!(beyond_reach, 0, "the cell cannot reach this window");
    }

    /// The pole cases are where the longitude pad's `1/cos(lat)` blows up.
    #[rstest]
    #[case::equator(0.0)]
    #[case::mid_latitude(55.0)]
    #[case::high_latitude(85.0)]
    #[case::at_the_pole(90.0)]
    fn a_zero_width_window_still_finds_the_cell_it_sits_in(#[case] latitude: f64) {
        let cell = cell_at(Latitude::new(latitude), Longitude::new(10.0)).unwrap();
        let dataset = JamDataset::new(day(), vec![observation(cell, 5, 1)]);
        let center = center(cell);

        assert_eq!(
            dataset
                .observations_within(center.lat()..=center.lat(), center.lng()..=center.lng())
                .count(),
            1
        );
    }

    #[test]
    fn cell_at_answers_at_the_published_resolution() {
        let cell = cell_at(Latitude::new(55.68), Longitude::new(12.57)).unwrap();
        assert_eq!(cell.resolution(), H3_RESOLUTION);
    }
}
