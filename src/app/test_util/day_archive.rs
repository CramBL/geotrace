//! Dates for the environment schedulers, and days written into an archive.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use gt_store::{
    EnvironmentArchive, GeomagneticIndexArchive, InterferenceArchive, SolarFlareArchive,
    TecMapArchive,
};

use crate::app::environment_storage;

pub fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
}

pub fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, 0, 0))
        .map(|naive| naive.and_utc())
        .unwrap_or_default()
}

/// Archive `day` in `store` with no aircraft observations. The fetch worker
/// writes an empty day when the service reports none.
pub fn archive_an_empty_interference_day(store: &InterferenceArchive, day: NaiveDate) {
    environment_storage::archive_one_day(
        store,
        EnvironmentArchive::AircraftInterference.day_insert_registration(day),
        |archive| archive.insert_day(day, "host", Utc::now(), &[]),
    );
}

/// A Kp day archived as the fetch worker leaves one: eight three-hour periods,
/// each at `first_period_index` plus the period number modulo five.
pub fn archive_kp_day(store: &GeomagneticIndexArchive, day: NaiveDate, first_period_index: f64) {
    let midnight = day.and_time(NaiveTime::MIN).and_utc();
    let samples = (0..8_u32)
        .map(|period| gt_solar::series::KpSample {
            period_start: midnight + TimeDelta::hours(i64::from(period) * 3),
            activity: gt_solar::activity::GeomagneticActivity::from_published_value(
                gt_solar::GeomagneticIndex::Kp,
                first_period_index + f64::from(period % 5),
            ),
            status: gt_solar::series::KpStatus::Definitive,
        })
        .collect();
    environment_storage::archive_one_day(
        store,
        EnvironmentArchive::GeomagneticIndices.day_insert_registration(day),
        |archive| {
            archive.insert_or_replace_kp_day(
                day,
                "host",
                Utc::now(),
                &gt_solar::series::KpSeries { samples },
            )?;
            archive.insert_or_replace_hp30_day(
                day,
                "host",
                Utc::now(),
                &gt_solar::series::Hp30Series { samples: vec![] },
            )
        },
    );
}

/// Archive one UTC day of TEC maps over the recording's own position, every
/// node standing at `tecu`, two hours apart.
pub fn archive_tec_day(store: &TecMapArchive, day: NaiveDate, tecu: f64) {
    let axis = |first_degrees: f64, last_degrees: f64, step_degrees: f64| {
        gt_ionex::grid::GridAxis::new(gt_ionex::grid::AxisDeclaration {
            first_degrees,
            last_degrees,
            step_degrees,
        })
        .expect("axis")
    };
    let grid = gt_ionex::grid::MapGrid {
        latitudes: gt_ionex::grid::LatitudeAxis::new(axis(55.0, 50.0, -2.5)),
        longitudes: gt_ionex::grid::LongitudeAxis::new(axis(-5.0, 5.0, 5.0)),
        shell_height_km: 450.0,
    };
    let midnight = day.and_time(NaiveTime::MIN).and_utc();
    let maps = (0..=12)
        .map(|step| {
            gt_ionex::maps::TecMap::new(
                midnight + TimeDelta::hours(step * 2),
                vec![vec![Some(gt_ionex::tec::TotalElectronContent::from_tecu(tecu)); 3]; 3],
            )
        })
        .collect();
    environment_storage::archive_one_day(
        store,
        EnvironmentArchive::IonosphericTec.day_insert_registration(day),
        |archive| {
            archive.insert_or_replace_day(
                day,
                "host",
                Utc::now(),
                gt_ionex::IonexProduct::Final,
                &gt_ionex::maps::GlobalIonosphereMaps::new(grid, TimeDelta::hours(2), maps),
            )
        },
    );
}

/// The flares of the May 2024 storm, as the fetch worker archives a day:
/// classes spread across the scale so each marker colour is drawn.
pub fn archive_flare_day(store: &SolarFlareArchive, day: NaiveDate, peaks: &[(u32, &str)]) {
    let flares: Vec<gt_flare::SolarFlare> = peaks
        .iter()
        .map(|&(hour, class_type)| {
            let peak = day.and_hms_opt(hour, 13, 0).unwrap_or_default().and_utc();
            gt_flare::SolarFlare {
                id: format!("{peak}-FLR-001"),
                begin: peak - TimeDelta::minutes(28),
                peak,
                end: Some(peak + TimeDelta::minutes(23)),
                classification: class_type.parse().expect("a published class"),
                source_location: Some("S20W25".to_owned()),
                active_region: Some(13664),
            }
        })
        .collect();
    environment_storage::archive_one_day(
        store,
        EnvironmentArchive::SolarFlares.day_insert_registration(day),
        |archive| archive.insert_or_replace_day(day, "host", Utc::now(), &flares),
    );
}
