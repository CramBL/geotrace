//! The maps one IONEX file holds, and interpolation over them.

use chrono::{DateTime, TimeDelta, Utc};
use gt_types::{Latitude, Longitude};

use crate::grid::{AxisPosition, GridPoint, MapGrid};
use crate::tec::TotalElectronContent;

/// The grid's values at one epoch, one row per latitude band in the order the
/// grid declares them.
#[derive(Debug, Clone, PartialEq)]
pub struct TecMap {
    epoch: DateTime<Utc>,
    latitude_bands: Vec<Vec<Option<TotalElectronContent>>>,
}

impl TecMap {
    pub(crate) const fn new(
        epoch: DateTime<Utc>,
        latitude_bands: Vec<Vec<Option<TotalElectronContent>>>,
    ) -> Self {
        Self {
            epoch,
            latitude_bands,
        }
    }

    pub const fn epoch(&self) -> DateTime<Utc> {
        self.epoch
    }

    /// The value at one grid node, or [`None`] where the node is off the grid
    /// or the producer published no value for it.
    pub fn value_at(&self, point: GridPoint) -> Option<TotalElectronContent> {
        self.latitude_bands
            .get(point.latitude_index)?
            .get(point.longitude_index)
            .copied()
            .flatten()
    }

    /// Every node of the map, northernmost band first, [`None`] where the
    /// producer published no value.
    pub fn values(&self) -> impl Iterator<Item = Option<TotalElectronContent>> {
        self.latitude_bands.iter().flatten().copied()
    }

    fn interpolated_tecu(&self, latitude: AxisPosition, longitude: AxisPosition) -> Option<f64> {
        let lower = self.interpolated_band_tecu(latitude.lower_index, longitude)?;
        let upper = self.interpolated_band_tecu(latitude.upper_index, longitude)?;
        Some(lower + (upper - lower) * latitude.fraction)
    }

    /// The value along one latitude band, between its two neighbouring
    /// longitude nodes.
    fn interpolated_band_tecu(
        &self,
        latitude_index: usize,
        longitude: AxisPosition,
    ) -> Option<f64> {
        let band = self.latitude_bands.get(latitude_index)?;
        let lower = band.get(longitude.lower_index).copied().flatten()?.tecu();
        let upper = band.get(longitude.upper_index).copied().flatten()?.tecu();
        Some(lower + (upper - lower) * longitude.fraction)
    }
}

/// The two maps a queried time falls between, and how far it stands from the
/// earlier one. Both are the same map at the file's last epoch, which nothing
/// follows.
struct TimeBracket<'a> {
    earlier: &'a TecMap,
    later: &'a TecMap,
    fraction_from_earlier: f64,
}

/// One IONEX file: the grid it declares and its maps in epoch order, which
/// only [`crate::parse`] builds.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalIonosphereMaps {
    grid: MapGrid,
    interval: TimeDelta,
    maps: Vec<TecMap>,
}

impl GlobalIonosphereMaps {
    pub(crate) const fn new(grid: MapGrid, interval: TimeDelta, maps: Vec<TecMap>) -> Self {
        Self {
            grid,
            interval,
            maps,
        }
    }

    pub const fn grid(&self) -> MapGrid {
        self.grid
    }

    /// The time between maps the header declares in its `INTERVAL` record.
    pub const fn interval(&self) -> TimeDelta {
        self.interval
    }

    pub fn maps(&self) -> &[TecMap] {
        &self.maps
    }

    pub fn epoch_of_first_map(&self) -> Option<DateTime<Utc>> {
        Some(self.maps.first()?.epoch())
    }

    pub fn epoch_of_last_map(&self) -> Option<DateTime<Utc>> {
        Some(self.maps.last()?.epoch())
    }

    /// The highest value in any map, [`None`] for a file whose nodes are all
    /// gaps.
    pub fn peak_total_electron_content(&self) -> Option<TotalElectronContent> {
        self.maps
            .iter()
            .flat_map(TecMap::values)
            .flatten()
            .max_by(|left, right| left.tecu().total_cmp(&right.tecu()))
    }

    /// The value at a position and time: bilinear between the four
    /// surrounding grid nodes, linear between the two maps bracketing the
    /// time.
    ///
    /// [`None`] where the time falls outside the file's epochs, the latitude
    /// outside the grid, or any contributing node is a gap, which keeps a
    /// published value from being mixed with an unpublished one. A longitude
    /// the grid does not hold directly is wrapped a full turn, so the
    /// meridian both ends of a global grid name is reached either way round.
    pub fn total_electron_content_at(
        &self,
        latitude: Latitude,
        longitude: Longitude,
        time: DateTime<Utc>,
    ) -> Option<TotalElectronContent> {
        let latitude_position = self.grid.latitudes.position_of(latitude)?;
        let longitude_position = self.grid.longitudes.position_of(longitude)?;
        let bracket = self.bracketing_maps(time)?;
        let earlier = bracket
            .earlier
            .interpolated_tecu(latitude_position, longitude_position)?;
        let later = bracket
            .later
            .interpolated_tecu(latitude_position, longitude_position)?;
        Some(TotalElectronContent::from_tecu(
            earlier + (later - earlier) * bracket.fraction_from_earlier,
        ))
    }

    fn bracketing_maps(&self, time: DateTime<Utc>) -> Option<TimeBracket<'_>> {
        let later_index = self.maps.partition_point(|map| map.epoch() <= time);
        let earlier = self.maps.get(later_index.checked_sub(1)?)?;
        let Some(later) = self.maps.get(later_index) else {
            return (earlier.epoch() == time).then_some(TimeBracket {
                earlier,
                later: earlier,
                fraction_from_earlier: 0.0,
            });
        };

        let span_ms = later
            .epoch()
            .signed_duration_since(earlier.epoch())
            .num_milliseconds();
        let elapsed_ms = time
            .signed_duration_since(earlier.epoch())
            .num_milliseconds();
        Some(TimeBracket {
            earlier,
            later,
            fraction_from_earlier: if span_ms == 0 {
                0.0
            } else {
                elapsed_ms as f64 / span_ms as f64
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::grid::{AxisDeclaration, GridAxis, LatitudeAxis, LongitudeAxis};

    use super::*;

    fn epoch(hour: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(2024, 5, 10)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .unwrap()
            .and_utc()
    }

    fn tecu(value: f64) -> Option<TotalElectronContent> {
        Some(TotalElectronContent::from_tecu(value))
    }

    /// A grid of two latitudes and three longitudes, the last of which
    /// repeats the first meridian a full turn on.
    fn grid() -> MapGrid {
        MapGrid {
            latitudes: LatitudeAxis::new(
                GridAxis::new(AxisDeclaration {
                    first_degrees: 10.0,
                    last_degrees: 0.0,
                    step_degrees: -10.0,
                })
                .unwrap(),
            ),
            longitudes: LongitudeAxis::new(
                GridAxis::new(AxisDeclaration {
                    first_degrees: -180.0,
                    last_degrees: 180.0,
                    step_degrees: 180.0,
                })
                .unwrap(),
            ),
            shell_height_km: 450.0,
        }
    }

    fn map_at(hour: u32, latitude_bands: Vec<Vec<Option<TotalElectronContent>>>) -> TecMap {
        TecMap::new(epoch(hour), latitude_bands)
    }

    fn first_map_bands() -> Vec<Vec<Option<TotalElectronContent>>> {
        vec![
            vec![tecu(10.0), tecu(20.0), tecu(10.0)],
            vec![tecu(30.0), tecu(40.0), tecu(30.0)],
        ]
    }

    /// The same bands with the northwestern node unpublished.
    fn first_map_bands_with_a_gap() -> Vec<Vec<Option<TotalElectronContent>>> {
        vec![
            vec![None, tecu(20.0), tecu(10.0)],
            vec![tecu(30.0), tecu(40.0), tecu(30.0)],
        ]
    }

    fn second_map_bands() -> Vec<Vec<Option<TotalElectronContent>>> {
        vec![
            vec![tecu(20.0), tecu(30.0), tecu(20.0)],
            vec![tecu(40.0), tecu(50.0), tecu(40.0)],
        ]
    }

    fn maps_from(first_bands: Vec<Vec<Option<TotalElectronContent>>>) -> GlobalIonosphereMaps {
        GlobalIonosphereMaps::new(
            grid(),
            TimeDelta::hours(2),
            vec![map_at(0, first_bands), map_at(2, second_map_bands())],
        )
    }

    /// Two maps two hours apart, every node of the second ten TECU above the
    /// first.
    fn two_maps() -> GlobalIonosphereMaps {
        maps_from(first_map_bands())
    }

    fn value_at(
        maps: &GlobalIonosphereMaps,
        latitude: f64,
        longitude: f64,
        time: DateTime<Utc>,
    ) -> Option<f64> {
        maps.total_electron_content_at(Latitude::new(latitude), Longitude::new(longitude), time)
            .map(TotalElectronContent::tecu)
    }

    #[rstest]
    #[case::a_grid_node(10.0, -180.0, 0, 10.0)]
    #[case::halfway_along_a_longitude_row(10.0, -90.0, 0, 15.0)]
    #[case::halfway_between_two_latitudes(5.0, -180.0, 0, 20.0)]
    #[case::the_center_of_a_cell(5.0, -90.0, 0, 25.0)]
    #[case::the_repeated_meridian(10.0, 180.0, 0, 10.0)]
    #[case::the_last_epoch(10.0, -180.0, 2, 20.0)]
    fn a_value_interpolates_between_the_surrounding_nodes(
        #[case] latitude: f64,
        #[case] longitude: f64,
        #[case] hour: u32,
        #[case] expected: f64,
    ) {
        assert_eq!(
            value_at(&two_maps(), latitude, longitude, epoch(hour)),
            Some(expected)
        );
    }

    #[test]
    fn a_time_between_two_maps_interpolates_between_them() {
        let maps = two_maps();
        assert_eq!(
            value_at(&maps, 10.0, -180.0, epoch(0) + TimeDelta::minutes(30)),
            Some(12.5)
        );
        assert_eq!(value_at(&maps, 5.0, -90.0, epoch(1)), Some(30.0));
    }

    #[rstest]
    #[case::before_the_first_map(-1)]
    #[case::after_the_last_map(3)]
    fn a_time_outside_the_file_has_no_value(#[case] hours_from_the_first_epoch: i64) {
        let time = epoch(0) + TimeDelta::hours(hours_from_the_first_epoch);
        assert_eq!(value_at(&two_maps(), 10.0, -180.0, time), None);
    }

    #[test]
    fn a_latitude_outside_the_grid_has_no_value() {
        assert_eq!(value_at(&two_maps(), 20.0, -180.0, epoch(0)), None);
        assert_eq!(value_at(&two_maps(), -0.5, -180.0, epoch(0)), None);
    }

    /// The gap sits at the northwestern node of the first map, so it
    /// withholds every query whose cell touches that node and whose time
    /// reaches that map.
    #[rstest]
    #[case::the_gap_itself(-180.0, 0)]
    #[case::another_node_of_the_same_cell(-170.0, 0)]
    #[case::a_time_between_the_two_maps(-180.0, 1)]
    fn a_gap_withholds_every_value_it_contributes_to(#[case] longitude: f64, #[case] hour: u32) {
        let maps = maps_from(first_map_bands_with_a_gap());
        assert_eq!(value_at(&maps, 10.0, longitude, epoch(hour)), None);
    }

    /// At the last epoch only the last map contributes, and it has no gap.
    #[test]
    fn the_map_after_a_gap_answers_on_its_own_epoch() {
        let maps = maps_from(first_map_bands_with_a_gap());
        assert_eq!(value_at(&maps, 10.0, -180.0, epoch(2)), Some(20.0));
    }

    #[test]
    fn a_cell_away_from_a_gap_still_has_a_value() {
        let maps = maps_from(first_map_bands_with_a_gap());
        assert_eq!(value_at(&maps, 5.0, 90.0, epoch(0)), Some(25.0));
    }

    #[test]
    fn a_file_without_maps_has_no_values_and_no_epochs() {
        let maps = GlobalIonosphereMaps::new(grid(), TimeDelta::hours(2), Vec::new());
        assert_eq!(maps.epoch_of_first_map(), None);
        assert_eq!(maps.epoch_of_last_map(), None);
        assert_eq!(maps.peak_total_electron_content(), None);
        assert_eq!(value_at(&maps, 10.0, -180.0, epoch(0)), None);
    }

    #[test]
    fn a_file_of_one_map_covers_only_that_epoch() {
        let maps = GlobalIonosphereMaps::new(
            grid(),
            TimeDelta::hours(2),
            vec![map_at(0, first_map_bands())],
        );
        assert_eq!(value_at(&maps, 10.0, -180.0, epoch(0)), Some(10.0));
        assert_eq!(
            value_at(&maps, 10.0, -180.0, epoch(0) + TimeDelta::seconds(1)),
            None
        );
    }

    #[test]
    fn the_peak_is_the_highest_value_in_any_map() {
        let maps = two_maps();
        assert_eq!(maps.peak_total_electron_content(), tecu(50.0));
        assert_eq!(maps.epoch_of_first_map(), Some(epoch(0)));
        assert_eq!(maps.epoch_of_last_map(), Some(epoch(2)));
        assert_eq!(maps.interval(), TimeDelta::hours(2));
    }

    #[test]
    fn a_node_reads_back_by_its_grid_position() {
        let maps = two_maps();
        let map = maps.maps().first().unwrap();
        assert_eq!(
            map.value_at(GridPoint {
                latitude_index: 1,
                longitude_index: 1
            }),
            tecu(40.0)
        );
        assert_eq!(
            map.value_at(GridPoint {
                latitude_index: 2,
                longitude_index: 0
            }),
            None
        );
        assert_eq!(map.values().count(), 6);
    }
}
