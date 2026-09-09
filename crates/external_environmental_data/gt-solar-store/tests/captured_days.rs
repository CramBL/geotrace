//! The archive against the captured GFZ responses.

use chrono::{DateTime, NaiveDate, Utc};

use gt_solar::series::{Hp30Series, IndexSample as _, KpSeries};
use gt_solar::{test_util, wire};
use gt_solar_store::SolarStore;
use gt_test_utils::day_archive;

/// The May 2024 storm, at both cadences.
const KP_STORM_CAPTURE: &str = "kp-storm";
const HP30_STORM_CAPTURE: &str = "hp30-storm";

/// The captured response for `name`, and the first UTC day of the window it
/// was requested over, which is the day the samples are archived under.
fn captured_response(name: &str) -> Result<(NaiveDate, String), String> {
    let capture = test_util::declared_window(name)?;
    let day = capture
        .window()
        .map_err(|err| format!("{name} window: {err}"))?
        .start
        .date_naive();
    Ok((day, test_util::captured_response(capture)?))
}

#[test]
fn a_captured_kp_day_round_trips() {
    let (day, json) = captured_response(KP_STORM_CAPTURE).unwrap();
    let published = KpSeries {
        samples: wire::parse_kp_series(&json)
            .expect("the capture parses")
            .samples
            .into_iter()
            .filter(|sample| sample.period_start().date_naive() == day)
            .collect(),
    };
    assert_eq!(published.samples.len(), 8, "a full day of Kp");

    let (_dir, store) = day_archive::store_in_a_temp_dir::<SolarStore>().unwrap();
    store
        .insert_or_replace_kp_day(
            day,
            gt_solar::DEFAULT_BASE_URL,
            DateTime::<Utc>::default(),
            &published,
        )
        .expect("store");
    assert_eq!(store.kp_series(day).expect("read back"), Some(published));
}

#[test]
fn a_captured_hp30_day_round_trips() {
    let (day, json) = captured_response(HP30_STORM_CAPTURE).unwrap();
    let published = Hp30Series {
        samples: wire::parse_hp30_series(&json)
            .expect("the capture parses")
            .samples
            .into_iter()
            .filter(|sample| sample.period_start().date_naive() == day)
            .collect(),
    };
    assert_eq!(published.samples.len(), 48, "a full day of Hp30");

    let (_dir, store) = day_archive::store_in_a_temp_dir::<SolarStore>().unwrap();
    store
        .insert_or_replace_hp30_day(
            day,
            gt_solar::DEFAULT_BASE_URL,
            DateTime::<Utc>::default(),
            &published,
        )
        .expect("store");
    assert_eq!(store.hp30_series(day).expect("read back"), Some(published));
}
