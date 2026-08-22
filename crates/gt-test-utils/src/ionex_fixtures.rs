//! Ionospheric maps for tests that archive or read a TEC day.

use chrono::{NaiveDate, NaiveTime, TimeDelta};
use gt_ionex::grid::{AxisDeclaration, GridAxis, LatitudeAxis, LongitudeAxis, MapGrid};
use gt_ionex::maps::{GlobalIonosphereMaps, TecMap};
use gt_ionex::tec::TotalElectronContent;

/// Maps of `day` whose every node carries one value, at the given whole hours
/// from that day's midnight. Hour 24 is the map at the end of the day, which a
/// published file dates to the next day's midnight.
#[expect(
    clippy::expect_used,
    reason = "The grid axis parameters here are fixed constants, so the maps cannot fail to build"
)]
pub fn uniform_maps(day: NaiveDate, samples: &[(i64, f64)]) -> GlobalIonosphereMaps {
    let midnight = day.and_time(NaiveTime::MIN).and_utc();
    let axis = |first_degrees, last_degrees, step_degrees| {
        GridAxis::new(AxisDeclaration {
            first_degrees,
            last_degrees,
            step_degrees,
        })
        .expect("axis")
    };
    let grid = MapGrid {
        latitudes: LatitudeAxis::new(axis(57.5, 52.5, -2.5)),
        longitudes: LongitudeAxis::new(axis(10.0, 15.0, 5.0)),
        shell_height_km: 450.0,
    };
    let maps = samples
        .iter()
        .map(|&(hours, tecu)| {
            TecMap::new(
                midnight + TimeDelta::hours(hours),
                vec![vec![Some(TotalElectronContent::from_tecu(tecu)); 2]; 3],
            )
        })
        .collect();
    GlobalIonosphereMaps::new(grid, TimeDelta::hours(2), maps)
}
