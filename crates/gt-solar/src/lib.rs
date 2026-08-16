//! Geomagnetic activity indices, as published by the GFZ Potsdam web service.
//!
//! Kp is the three-hourly planetary index every storm report quotes, and the
//! input to the NOAA G-scale. Hp30 is the same scale at 30-minute cadence,
//! and unlike Kp it is not capped at 9. GeoTrace resolves a value per fix
//! from the fix's own UTC time, so a recording can be read against what the
//! geomagnetic field was doing while it was made.
//!
//! One request covers one index over one UTC window, addressed by
//! [`index_url`]:
//!
//! ```text
//! https://kp.gfz.de/app/json/?start=2024-05-10T00:00:00Z&end=2024-05-11T00:00:00Z&index=Kp
//! ```
//!
//! The response holds parallel arrays: one value per period, the period start
//! times, and for Kp a status per value. [`wire`] parses them into
//! [`series::KpSeries`] and [`series::Hp30Series`]. A window the service has
//! no values for, before an index begins or beyond its last published period,
//! answers HTTP 200 with empty arrays.
//!
//! [`text`] holds the wording every surface showing a value uses.

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};

pub mod activity;
pub mod calendar;
pub mod series;
pub mod text;
pub mod transport;
pub mod wire;

/// Base URL of the default index host. Configurable in settings, for a
/// self-hosted mirror or an offline copy.
pub const DEFAULT_BASE_URL: &str = "https://kp.gfz.de";

/// Path of the index endpoint, appended to the base URL.
const INDEX_PATH: &str = "/app/json/";

/// Timestamp format the endpoint's `start` and `end` parameters take, and the
/// format the response's period start times arrive in.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// One of the indices GeoTrace requests.
///
/// The variant names are the endpoint's `index` parameter values.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumCount,
    strum::EnumIter,
)]
pub enum GeomagneticIndex {
    /// The three-hourly planetary index, published since 1932.
    Kp,
    /// The half-hourly index on the Kp scale, published since 1985.
    Hp30,
}

impl GeomagneticIndex {
    /// The `index` parameter value, which is also the key the response's
    /// value array is under.
    pub fn wire_name(self) -> &'static str {
        self.into()
    }

    /// How long one published value covers.
    pub const fn period_length(self) -> TimeDelta {
        match self {
            Self::Kp => TimeDelta::hours(3),
            Self::Hp30 => TimeDelta::minutes(30),
        }
    }

    /// [`period_length`](Self::period_length) as it is written in hover text.
    pub const fn period_length_words(self) -> &'static str {
        match self {
            Self::Kp => "3 hours",
            Self::Hp30 => "30 minutes",
        }
    }

    /// Start of the period covering `time`.
    ///
    /// Periods run from UTC midnight and both lengths divide the day evenly,
    /// so a period start is a whole multiple of the period length in Unix
    /// seconds. [`None`] for a time outside the representable range.
    pub fn period_start_covering(self, time: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let seconds_per_period = self.period_length().num_seconds();
        let periods = time.timestamp().div_euclid(seconds_per_period);
        DateTime::from_timestamp(periods.checked_mul(seconds_per_period)?, 0)
    }

    /// Whether the service publishes a status alongside each value.
    pub const fn publishes_status(self) -> bool {
        match self {
            Self::Kp => true,
            Self::Hp30 => false,
        }
    }

    /// First UTC day the index has values for. Kp reaches back to the
    /// International Polar Year of 1932, Hp30 to 1985.
    pub const fn coverage_start(self) -> NaiveDate {
        match self {
            Self::Kp => KP_COVERAGE_START,
            Self::Hp30 => HP30_COVERAGE_START,
        }
    }
}

/// First day of each index's coverage, as (year, month, day).
const KP_COVERAGE_START_YMD: (i32, u32, u32) = (1932, 1, 1);
const HP30_COVERAGE_START_YMD: (i32, u32, u32) = (1985, 1, 1);

const KP_COVERAGE_START: NaiveDate = coverage_start(KP_COVERAGE_START_YMD);
const HP30_COVERAGE_START: NaiveDate = coverage_start(HP30_COVERAGE_START_YMD);

const fn coverage_start((year, month, day): (i32, u32, u32)) -> NaiveDate {
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => date,
        // Dead arm: const evaluation cannot unwrap without panicking, and the
        // assertions below fail the build if it ever stops being dead.
        None => NaiveDate::MIN,
    }
}

const _: () = {
    let (year, month, day) = KP_COVERAGE_START_YMD;
    assert!(
        NaiveDate::from_ymd_opt(year, month, day).is_some(),
        "KP_COVERAGE_START_YMD must name a real calendar date"
    );
    let (year, month, day) = HP30_COVERAGE_START_YMD;
    assert!(
        NaiveDate::from_ymd_opt(year, month, day).is_some(),
        "HP30_COVERAGE_START_YMD must name a real calendar date"
    );
};

/// The UTC window one request covers, inclusive of both ends: a request from
/// midnight to midnight answers with the following day's first period too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeWindow {
    /// Midnight to the last second of `day`, so the response holds every
    /// period starting within the day and none of the next day's.
    pub fn covering_utc_day(day: NaiveDate) -> Self {
        Self {
            start: day.and_time(NaiveTime::MIN).and_utc(),
            end: day.and_time(LAST_SECOND_OF_DAY).and_utc(),
        }
    }
}

const LAST_SECOND_OF_DAY: NaiveTime = match NaiveTime::from_hms_opt(23, 59, 59) {
    Some(time) => time,
    // Dead arm: const evaluation cannot unwrap without panicking, and the
    // assertion below fails the build if it ever stops being dead.
    None => NaiveTime::MIN,
};

const _: () = assert!(
    NaiveTime::from_hms_opt(23, 59, 59).is_some(),
    "LAST_SECOND_OF_DAY must name a real time of day"
);

/// The URL of `index` over `window` on `base_url`, which must not end in a
/// slash.
pub fn index_url(base_url: &str, index: GeomagneticIndex, window: TimeWindow) -> String {
    format!(
        "{base_url}{INDEX_PATH}?start={start}&end={end}&index={index}",
        start = window.start.format(TIMESTAMP_FORMAT),
        end = window.end.format(TIMESTAMP_FORMAT),
    )
}

/// Read a timestamp as the endpoint writes them, `2024-05-10T00:00:00Z`, or
/// any other RFC 3339 time, converted to UTC.
pub fn parse_timestamp(timestamp: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(timestamp)?.with_timezone(&Utc))
}

/// One response captured under [`fixtures_dir`] by `just solar-fixtures`.
///
/// Captures are frozen once committed. A re-capture's diff is reviewed like
/// code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureWindow {
    /// Names the capture on disk and selects it on the capture command line.
    pub name: &'static str,
    pub index: GeomagneticIndex,
    /// Start of the requested window, as [`parse_timestamp`] reads it.
    pub start: &'static str,
    /// End of the requested window, as [`parse_timestamp`] reads it.
    pub end: &'static str,
    /// What this capture exists to exercise.
    pub purpose: &'static str,
}

impl FixtureWindow {
    /// The window that was requested, or the error in one of its declared
    /// timestamps.
    pub fn window(&self) -> Result<TimeWindow, chrono::ParseError> {
        Ok(TimeWindow {
            start: parse_timestamp(self.start)?,
            end: parse_timestamp(self.end)?,
        })
    }

    /// The file the response is captured to, under [`fixtures_dir`].
    pub fn file_name(&self) -> String {
        format!("{}.json", self.name)
    }
}

/// The captured windows, in the order the manifest lists them.
pub const FIXTURE_WINDOWS: [FixtureWindow; 4] = [
    FixtureWindow {
        name: "kp-quiet",
        index: GeomagneticIndex::Kp,
        start: "2024-04-01T00:00:00Z",
        end: "2024-04-02T00:00:00Z",
        purpose: "a day no storm reached, the shape most requests answer with",
    },
    FixtureWindow {
        name: "kp-storm",
        index: GeomagneticIndex::Kp,
        start: "2024-05-10T00:00:00Z",
        end: "2024-05-12T00:00:00Z",
        purpose: "the May 2024 storm, where Kp reaches its ceiling of 9 and the G5 class",
    },
    FixtureWindow {
        name: "hp30-storm",
        index: GeomagneticIndex::Hp30,
        start: "2024-05-10T00:00:00Z",
        end: "2024-05-12T00:00:00Z",
        purpose: "the same storm at 30-minute cadence, with values above 9 and no status array",
    },
    FixtureWindow {
        name: "hp30-before-coverage",
        index: GeomagneticIndex::Hp30,
        start: "1980-01-01T00:00:00Z",
        end: "1980-01-02T00:00:00Z",
        purpose: "a window before Hp30 begins in 1985, answered with empty arrays and HTTP 200",
    },
];

/// File name of the capture manifest written beside the fixtures, recording
/// when each window was captured and what the service answered.
pub const CAPTURE_MANIFEST: &str = "capture.json";

/// Directory holding the captured response fixtures.
///
/// Resolved from the crate manifest dir, so it is only meaningful to
/// development tooling running inside the workspace, never to the shipped
/// application.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    fn timestamp(text: &str) -> DateTime<Utc> {
        parse_timestamp(text).unwrap()
    }

    #[test]
    fn index_url_addresses_the_window_on_the_default_host() {
        let window = TimeWindow {
            start: timestamp("2024-05-10T00:00:00Z"),
            end: timestamp("2024-05-11T00:00:00Z"),
        };
        assert_eq!(
            index_url(DEFAULT_BASE_URL, GeomagneticIndex::Kp, window),
            "https://kp.gfz.de/app/json/?start=2024-05-10T00:00:00Z&end=2024-05-11T00:00:00Z&index=Kp"
        );
    }

    #[test]
    fn index_url_honors_the_configured_host_and_the_requested_index() {
        let window = TimeWindow {
            start: timestamp("1985-01-01T00:30:00Z"),
            end: timestamp("1985-01-01T06:00:00Z"),
        };
        assert_eq!(
            index_url("https://mirror.example", GeomagneticIndex::Hp30, window),
            "https://mirror.example/app/json/?start=1985-01-01T00:30:00Z&end=1985-01-01T06:00:00Z&index=Hp30"
        );
    }

    /// A window given in another offset is requested as the same instant in
    /// UTC.
    #[test]
    fn a_non_utc_timestamp_is_requested_in_utc() {
        let window = TimeWindow {
            start: timestamp("2024-05-10T02:00:00+02:00"),
            end: timestamp("2024-05-10T04:00:00+02:00"),
        };
        assert_eq!(
            index_url(DEFAULT_BASE_URL, GeomagneticIndex::Kp, window),
            "https://kp.gfz.de/app/json/?start=2024-05-10T00:00:00Z&end=2024-05-10T02:00:00Z&index=Kp"
        );
    }

    #[rstest]
    #[case(GeomagneticIndex::Kp, "Kp")]
    #[case(GeomagneticIndex::Hp30, "Hp30")]
    fn wire_names_are_the_endpoints_index_values(
        #[case] index: GeomagneticIndex,
        #[case] expected: &str,
    ) {
        assert_eq!(index.wire_name(), expected);
        assert_eq!(index.to_string(), expected);
    }

    #[rstest]
    #[case(GeomagneticIndex::Kp, TimeDelta::hours(3), "3 hours")]
    #[case(GeomagneticIndex::Hp30, TimeDelta::minutes(30), "30 minutes")]
    fn period_length_words_state_the_period_length(
        #[case] index: GeomagneticIndex,
        #[case] length: TimeDelta,
        #[case] words: &str,
    ) {
        assert_eq!(index.period_length(), length);
        assert_eq!(index.period_length_words(), words);
    }

    #[rstest]
    #[case::hp30_inside_a_period(
        GeomagneticIndex::Hp30,
        "2024-05-10T18:29:59Z",
        "2024-05-10T18:00:00Z"
    )]
    #[case::hp30_on_a_boundary(
        GeomagneticIndex::Hp30,
        "2024-05-10T18:30:00Z",
        "2024-05-10T18:30:00Z"
    )]
    #[case::kp_inside_a_period(
        GeomagneticIndex::Kp,
        "2024-05-10T20:59:59Z",
        "2024-05-10T18:00:00Z"
    )]
    #[case::kp_at_midnight(GeomagneticIndex::Kp, "2024-05-10T00:00:00Z", "2024-05-10T00:00:00Z")]
    #[case::kp_before_the_epoch(
        GeomagneticIndex::Kp,
        "1932-01-01T01:30:00Z",
        "1932-01-01T00:00:00Z"
    )]
    fn a_period_start_floors_to_the_published_grid(
        #[case] index: GeomagneticIndex,
        #[case] time: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(
            index.period_start_covering(timestamp(time)),
            Some(timestamp(expected))
        );
    }

    #[test]
    fn every_index_has_a_capture() {
        for index in GeomagneticIndex::iter() {
            assert!(
                FIXTURE_WINDOWS.iter().any(|fixture| fixture.index == index),
                "{index} has no captured window"
            );
        }
        assert_eq!(GeomagneticIndex::COUNT, 2);
    }

    #[test]
    fn a_fixture_window_names_its_capture_and_parses_its_timestamps() {
        for fixture in FIXTURE_WINDOWS {
            let window = fixture.window().unwrap();
            assert!(
                window.start <= window.end,
                "{}: {}",
                fixture.name,
                fixture.purpose
            );
            assert_eq!(fixture.file_name(), format!("{}.json", fixture.name));
        }
    }

    /// A captured window must be one the service has already published, so
    /// its capture cannot change meaning as time passes.
    #[test]
    fn no_captured_window_reaches_into_the_future() {
        for fixture in FIXTURE_WINDOWS {
            assert!(
                fixture.window().unwrap().end < Utc::now(),
                "{}: {}",
                fixture.name,
                fixture.purpose
            );
        }
    }
}
