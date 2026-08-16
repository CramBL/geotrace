//! Round-trip index days through a real archive file in a temp directory.

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use rstest::rstest;
use tempfile::TempDir;

use gt_solar::GeomagneticIndex;
use gt_solar::activity::GeomagneticActivity;
use gt_solar::series::{Hp30Sample, Hp30Series, KpSample, KpSeries, KpStatus};
use gt_solar_store::schema::IndexArchiveLayout as _;
use gt_solar_store::{FILE_NAME, SolarStore, SolarStoreError, schema};

const HOST: &str = "https://kp.gfz.de";

fn store() -> Result<(TempDir, SolarStore), String> {
    let dir = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
    let store = SolarStore::open_or_create(&dir.path().join(FILE_NAME))
        .map_err(|err| format!("open archive: {err}"))?;
    Ok((dir, store))
}

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 5, 10).unwrap_or_default() + TimeDelta::days(offset)
}

fn fetched_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_784_505_600, 0).unwrap_or_default()
}

fn period_start(day: NaiveDate, period: i32, index: GeomagneticIndex) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc() + index.period_length() * period
}

fn activity(index: GeomagneticIndex, value: f64) -> Option<GeomagneticActivity> {
    GeomagneticActivity::from_published_value(index, value)
}

/// A day of Kp: eight three-hourly periods, the last two still nowcast and one
/// of them a period the service published no value for.
fn kp_day(day: NaiveDate) -> KpSeries {
    let samples = (0..8)
        .map(|period| KpSample {
            period_start: period_start(day, period, GeomagneticIndex::Kp),
            activity: if period == 6 {
                None
            } else {
                activity(GeomagneticIndex::Kp, 2.0 + f64::from(period) / 3.0)
            },
            status: if period < 6 {
                KpStatus::Definitive
            } else {
                KpStatus::Nowcast
            },
        })
        .collect();
    KpSeries { samples }
}

/// A day of Hp30: 48 half-hourly periods, one of them without a value and one
/// above the ceiling Kp is capped at.
fn hp30_day(day: NaiveDate) -> Hp30Series {
    let samples = (0..48)
        .map(|period| Hp30Sample {
            period_start: period_start(day, period, GeomagneticIndex::Hp30),
            activity: match period {
                17 => None,
                20 => activity(GeomagneticIndex::Hp30, 11.333),
                _ => activity(GeomagneticIndex::Hp30, f64::from(period) / 8.0),
            },
        })
        .collect();
    Hp30Series { samples }
}

#[test]
fn a_new_archive_holds_neither_index() {
    let (_dir, store) = store().unwrap();
    for index in [GeomagneticIndex::Kp, GeomagneticIndex::Hp30] {
        assert!(store.archived_days(index).expect("days").is_empty());
        assert!(!store.contains(index, day(0)).expect("contains"));
    }
    assert_eq!(store.kp_series(day(0)).expect("kp"), None);
    assert_eq!(store.hp30_series(day(0)).expect("hp30"), None);
}

/// Values, gaps and per-value statuses all survive the round trip.
#[test]
fn a_kp_day_round_trips_with_its_statuses_and_gaps() {
    let (_dir, store) = store().unwrap();
    let written = kp_day(day(0));
    store
        .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &written)
        .expect("store");

    let read = store.kp_series(day(0)).expect("kp").expect("archived");
    assert_eq!(read, written);
    assert_eq!(
        read.samples.iter().filter(|s| s.activity.is_none()).count(),
        1
    );
    assert_eq!(
        read.samples
            .iter()
            .filter(|s| s.status == KpStatus::Nowcast)
            .count(),
        2
    );
}

#[test]
fn an_hp30_day_round_trips_with_its_gaps() {
    let (_dir, store) = store().unwrap();
    let written = hp30_day(day(0));
    store
        .insert_or_replace_hp30_day(day(0), HOST, fetched_at(), &written)
        .expect("store");

    let read = store.hp30_series(day(0)).expect("hp30").expect("archived");
    assert_eq!(read, written);
    assert_eq!(
        read.peak_activity().map(GeomagneticActivity::value),
        Some(11.333),
        "an Hp30 value above the Kp ceiling is stored as published"
    );
}

/// One day can hold both indices, which are archived apart.
#[test]
fn one_day_holds_both_indices_independently() {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
        .expect("store kp");

    assert!(store.contains(GeomagneticIndex::Kp, day(0)).expect("kp"));
    assert!(
        !store
            .contains(GeomagneticIndex::Hp30, day(0))
            .expect("hp30")
    );
    assert_eq!(store.hp30_series(day(0)).expect("hp30"), None);

    store
        .insert_or_replace_hp30_day(day(0), HOST, fetched_at(), &hp30_day(day(0)))
        .expect("store hp30");
    assert_eq!(
        store.kp_series(day(0)).expect("kp"),
        Some(kp_day(day(0))),
        "storing Hp30 leaves the day's Kp untouched"
    );
}

/// A day already archived is stored again when GFZ revises its nowcast
/// values.
#[test]
fn storing_a_day_again_replaces_what_was_archived() {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
        .expect("store nowcast");

    let revised = KpSeries {
        samples: kp_day(day(0))
            .samples
            .into_iter()
            .map(|sample| KpSample {
                status: KpStatus::Definitive,
                ..sample
            })
            .collect(),
    };
    let revised_at = fetched_at() + TimeDelta::days(1);
    store
        .insert_or_replace_kp_day(day(0), HOST, revised_at, &revised)
        .expect("store revision");

    assert_eq!(store.kp_series(day(0)).expect("kp"), Some(revised));
    let days = store
        .archived_days(GeomagneticIndex::Kp)
        .expect("archived days");
    assert_eq!(days.len(), 1, "the day is indexed once");
    assert_eq!(days.first().map(|entry| entry.fetched_at), Some(revised_at));
}

/// A replacement of a different length must not spill into the days around it.
#[test]
fn replacing_a_day_leaves_the_days_around_it_alone() {
    let (_dir, store) = store().unwrap();
    for offset in [0, 1, 2] {
        store
            .insert_or_replace_kp_day(day(offset), HOST, fetched_at(), &kp_day(day(offset)))
            .expect("store");
    }

    let partial = KpSeries {
        samples: kp_day(day(1)).samples.into_iter().take(3).collect(),
    };
    store
        .insert_or_replace_kp_day(day(1), HOST, fetched_at(), &partial)
        .expect("store partial");

    assert_eq!(store.kp_series(day(0)).expect("kp"), Some(kp_day(day(0))));
    assert_eq!(store.kp_series(day(1)).expect("kp"), Some(partial));
    assert_eq!(store.kp_series(day(2)).expect("kp"), Some(kp_day(day(2))));
}

/// Store order does not decide read order.
#[test]
fn archived_days_come_back_oldest_first_with_their_provenance() {
    let (_dir, store) = store().unwrap();
    for offset in [2, 0, 1] {
        store
            .insert_or_replace_hp30_day(day(offset), HOST, fetched_at(), &hp30_day(day(offset)))
            .expect("store");
    }

    let archived = store
        .archived_days(GeomagneticIndex::Hp30)
        .expect("archived days");
    assert_eq!(
        archived
            .iter()
            .map(|entry| entry.day)
            .collect::<Vec<NaiveDate>>(),
        [day(0), day(1), day(2)]
    );
    let first = archived.first().expect("one day");
    assert_eq!(first.samples, 48);
    assert_eq!(first.host, HOST);
    assert_eq!(first.fetched_at, fetched_at());
}

/// A window the service answered with no periods is still an archived day,
/// distinct from one never fetched.
#[test]
fn a_day_with_no_samples_is_still_archived() {
    let (_dir, store) = store().unwrap();
    let empty = Hp30Series { samples: vec![] };
    store
        .insert_or_replace_hp30_day(day(0), HOST, fetched_at(), &empty)
        .expect("store");

    assert!(
        store
            .contains(GeomagneticIndex::Hp30, day(0))
            .expect("contains")
    );
    assert_eq!(store.hp30_series(day(0)).expect("hp30"), Some(empty));
}

/// Both threads write through the one archive: the lock serializes each
/// store's read-append-index sequence.
#[test]
fn days_stored_from_two_threads_both_reach_the_archive() {
    let (_dir, store) = store().unwrap();
    let store = &store;
    std::thread::scope(|scope| {
        for offset in [0, 1] {
            scope.spawn(move || {
                store
                    .insert_or_replace_kp_day(day(offset), HOST, fetched_at(), &kp_day(day(offset)))
                    .expect("store");
            });
        }
    });

    let days: Vec<NaiveDate> = store
        .archived_days(GeomagneticIndex::Kp)
        .expect("archived days")
        .into_iter()
        .map(|entry| entry.day)
        .collect();
    assert_eq!(days, [day(0), day(1)]);
    assert_eq!(
        store.kp_series(day(1)).expect("second day"),
        Some(kp_day(day(1)))
    );
}

#[test]
fn an_archive_reopens_with_its_days() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    {
        let store = SolarStore::open_or_create(&path).expect("create");
        store
            .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
            .expect("store");
    }
    let reopened = SolarStore::open_or_create(&path).expect("reopen");
    assert_eq!(
        reopened.kp_series(day(0)).expect("kp"),
        Some(kp_day(day(0)))
    );
}

#[rstest]
#[case::before_epoch(NaiveDate::from_ymd_opt(1969, 12, 31))]
#[case::hp30_coverage_start(NaiveDate::from_ymd_opt(1985, 1, 1))]
#[case::far_future(NaiveDate::from_ymd_opt(2999, 1, 1))]
fn any_date_round_trips_through_the_day_index(#[case] date: Option<NaiveDate>) {
    let date = date.expect("date");
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_hp30_day(date, HOST, fetched_at(), &hp30_day(date))
        .expect("store");
    assert_eq!(
        store
            .archived_days(GeomagneticIndex::Hp30)
            .expect("archived days")
            .first()
            .map(|entry| entry.day),
        Some(date)
    );
}

/// An archive written by a newer build is refused.
#[test]
fn a_newer_schema_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    SolarStore::open_or_create(&path).expect("create");
    {
        let file = hdf5::File::open_rw(&path).expect("reopen");
        let attr = file.attr(schema::SCHEMA_VERSION_ATTR).expect("attr");
        attr.write_scalar(&(schema::CURRENT_SCHEMA_VERSION + 1))
            .expect("bump");
    }
    let err = SolarStore::open_or_create(&path).expect_err("refuse");
    assert!(
        matches!(err, SolarStoreError::SchemaTooNew { found, supported }
            if found == schema::CURRENT_SCHEMA_VERSION + 1
                && supported == schema::CURRENT_SCHEMA_VERSION),
        "{err}"
    );
}

/// Samples appended without an index entry, which is what an interrupted
/// store leaves, are cut when the archive is reopened.
#[test]
fn unindexed_samples_are_dropped_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    {
        let store = SolarStore::open_or_create(&path).unwrap();
        store
            .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
            .unwrap();
    }
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        let group = file
            .group(&GeomagneticIndex::Kp.samples_group_path())
            .unwrap();
        for &name in GeomagneticIndex::Kp.sample_columns() {
            let dataset = group.dataset(name).unwrap();
            let rows = dataset.shape().first().copied().unwrap_or_default();
            dataset.resize([rows + 5]).unwrap();
        }
    }

    let reopened = SolarStore::open_or_create(&path).unwrap();
    assert_eq!(
        reopened.kp_series(day(0)).unwrap(),
        Some(kp_day(day(0))),
        "the archived day survives"
    );
    let file = hdf5::File::open(&path).unwrap();
    let rows = file
        .group(&GeomagneticIndex::Kp.samples_group_path())
        .unwrap()
        .dataset(schema::SAMPLE_PERIOD_START)
        .unwrap()
        .shape()
        .first()
        .copied()
        .unwrap_or_default();
    assert_eq!(rows, 8, "the unindexed samples are gone");
}

/// Columns shorter than the index means archived samples are missing, which no
/// recovery can invent.
#[test]
fn a_column_shorter_than_the_index_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    {
        let store = SolarStore::open_or_create(&path).unwrap();
        store
            .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
            .unwrap();
    }
    {
        let file = hdf5::File::open_rw(&path).unwrap();
        file.group(&GeomagneticIndex::Kp.samples_group_path())
            .unwrap()
            .dataset(schema::SAMPLE_ACTIVITY)
            .unwrap()
            .resize([1])
            .unwrap();
    }

    let err = SolarStore::open_or_create(&path).expect_err("truncated column");
    assert!(matches!(err, SolarStoreError::Corrupt(_)), "{err}");
}

/// A code the schema does not define is reported as an inconsistent archive.
#[rstest]
#[case::activity_presence(
    schema::SAMPLE_ACTIVITY_PRESENCE,
    8,
    "Kp sample 0 has activity presence code 8"
)]
#[case::status(schema::SAMPLE_KP_STATUS, 5, "Kp sample 0 has status code 5")]
fn an_undecodable_sample_is_reported(
    #[case] column: &str,
    #[case] code: u8,
    #[case] expected: &str,
) {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
        .unwrap();
    {
        let file = hdf5::File::open_rw(store.path()).unwrap();
        file.group(&GeomagneticIndex::Kp.samples_group_path())
            .unwrap()
            .dataset(column)
            .unwrap()
            .write_slice(&[code], 0..1)
            .unwrap();
    }

    let err = store.kp_series(day(0)).expect_err("undecodable sample");
    assert_eq!(
        err.to_string(),
        format!("archive is inconsistent: {expected}")
    );
}

/// Kp is defined up to 9: a stored value above it reads back as an
/// inconsistent archive.
#[test]
fn an_activity_outside_the_published_range_is_reported() {
    let (_dir, store) = store().unwrap();
    store
        .insert_or_replace_kp_day(day(0), HOST, fetched_at(), &kp_day(day(0)))
        .unwrap();
    {
        let file = hdf5::File::open_rw(store.path()).unwrap();
        file.group(&GeomagneticIndex::Kp.samples_group_path())
            .unwrap()
            .dataset(schema::SAMPLE_ACTIVITY)
            .unwrap()
            .write_slice(&[11.333_f64], 0..1)
            .unwrap();
    }

    let err = store.kp_series(day(0)).expect_err("out of range");
    assert_eq!(
        err.to_string(),
        "archive is inconsistent: Kp sample 0 is 11.333, outside the range Kp is published in"
    );
}
