//! Solar flare events, as published by NASA's DONKI catalog.
//!
//! A flare raises the X-ray flux on the sunlit side of the Earth within
//! minutes, ionizing the lower ionosphere while it lasts. GeoTrace marks the
//! flares of the days a recording spans on the plot, so a stretch of degraded
//! reception can be read against what the Sun was doing.
//!
//! One request covers one range of UTC days, addressed by [`flare_url`]:
//!
//! ```text
//! https://api.nasa.gov/DONKI/FLR?startDate=2024-05-09&endDate=2024-05-09&api_key=KEY
//! ```
//!
//! The endpoint answers with a JSON array of events, which [`wire`] parses
//! into [`SolarFlare`]s, and with an empty array for a range it lists nothing
//! for. The key is the user's own: [`ApiKey`] keeps it out of everything the
//! app writes down.
//!
//! [`text`] holds the wording every surface showing a flare uses.

use std::path::PathBuf;

use chrono::NaiveDate;

pub mod calendar;
pub mod class;
pub mod flare;
pub mod text;
pub mod transport;
pub mod wire;

pub use class::{FlareClass, FlareClassification, RadioBlackoutClass};
pub use flare::SolarFlare;

/// Base URL of the default host. Configurable in settings, for a proxy or an
/// offline copy.
pub const DEFAULT_BASE_URL: &str = "https://api.nasa.gov";

/// Path of the flare endpoint, appended to the base URL.
const FLARE_PATH: &str = "/DONKI/FLR";

/// Date format the endpoint's `startDate` and `endDate` parameters take.
const DATE_FORMAT: &str = "%Y-%m-%d";

/// Stands in for the key wherever text that may hold it is written down.
pub const REDACTED_KEY: &str = "[redacted]";

/// The api.nasa.gov key a user registers for and enters in settings.
///
/// The key is a secret, so this type has no [`Debug`] or [`Display`](std::fmt::Display)
/// implementation: printing one does not compile. Text that may hold it, such
/// as a transport failure quoting the URL it tried, goes through
/// [`redact`](Self::redact) first.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// The key entered, or [`None`] for an entry holding nothing but
    /// whitespace, which is how an empty settings field reads.
    pub fn new(entered: &str) -> Option<Self> {
        let trimmed = entered.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    /// `text` with every occurrence of the key replaced by [`REDACTED_KEY`].
    pub fn redact(&self, text: &str) -> String {
        text.replace(&self.0, REDACTED_KEY)
    }

    /// The key as it goes into a request URL. Every other use is a leak.
    fn query_value(&self) -> &str {
        &self.0
    }
}

/// The UTC days one request covers, inclusive of both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateWindow {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl DateWindow {
    /// The window covering `day` alone.
    pub const fn covering_utc_day(day: NaiveDate) -> Self {
        Self {
            start: day,
            end: day,
        }
    }
}

/// The URL of `window` on `base_url`, which must not end in a slash.
pub fn flare_url(base_url: &str, window: DateWindow, key: &ApiKey) -> String {
    format!(
        "{base_url}{FLARE_PATH}?startDate={start}&endDate={end}&api_key={api_key}",
        start = window.start.format(DATE_FORMAT),
        end = window.end.format(DATE_FORMAT),
        api_key = key.query_value(),
    )
}

/// Read a date as the endpoint writes them, `2024-05-09`.
pub fn parse_date(date: &str) -> Result<NaiveDate, chrono::ParseError> {
    NaiveDate::parse_from_str(date, DATE_FORMAT)
}

/// One response captured under [`fixtures_dir`] by `just flare-fixtures`.
///
/// Captures are frozen once committed. A re-capture's diff is reviewed like
/// code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureWindow {
    /// Names the capture on disk and selects it on the capture command line.
    pub name: &'static str,
    /// First day of the requested window, as [`parse_date`] reads it.
    pub start: &'static str,
    /// Last day of the requested window, as [`parse_date`] reads it.
    pub end: &'static str,
    /// What this capture exists to exercise.
    pub purpose: &'static str,
}

impl FixtureWindow {
    /// The window that was requested, or the error in one of its declared
    /// dates.
    pub fn window(&self) -> Result<DateWindow, chrono::ParseError> {
        Ok(DateWindow {
            start: parse_date(self.start)?,
            end: parse_date(self.end)?,
        })
    }

    /// The file the response is captured to, under [`fixtures_dir`].
    pub fn file_name(&self) -> String {
        format!("{}.json", self.name)
    }
}

/// The captured windows, in the order the manifest lists them.
pub const FIXTURE_WINDOWS: [FixtureWindow; 3] = [
    FixtureWindow {
        name: "storm-may-2024",
        start: "2024-05-09",
        end: "2024-05-11",
        purpose: "the May 2024 storm, with X-class flares and an event whose active region is null",
    },
    FixtureWindow {
        name: "quiet-january-2019",
        start: "2019-01-01",
        end: "2019-01-31",
        purpose: "solar minimum, where the catalog lists two C-class flares and neither has an \
                  end time",
    },
    FixtureWindow {
        name: "before-coverage",
        start: "2009-01-01",
        end: "2009-12-31",
        purpose: "a year before the catalog begins, answered with an empty array and HTTP 200",
    },
];

/// File name of the capture manifest written beside the fixtures, recording
/// when each window was captured and what the endpoint returned.
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
    use chrono::Utc;
    use rstest::rstest;

    use super::*;

    fn key(entered: &str) -> ApiKey {
        ApiKey::new(entered).expect("a key")
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    #[test]
    fn flare_url_addresses_the_window_on_the_default_host() {
        let window = DateWindow {
            start: date(2024, 5, 9),
            end: date(2024, 5, 11),
        };
        assert_eq!(
            flare_url(DEFAULT_BASE_URL, window, &key("KEY")),
            "https://api.nasa.gov/DONKI/FLR?startDate=2024-05-09&endDate=2024-05-11&api_key=KEY"
        );
    }

    #[test]
    fn flare_url_honors_the_configured_host_and_covers_one_day() {
        let window = DateWindow::covering_utc_day(date(2024, 5, 9));
        assert_eq!(
            flare_url("https://proxy.example", window, &key("KEY")),
            "https://proxy.example/DONKI/FLR?startDate=2024-05-09&endDate=2024-05-09&api_key=KEY"
        );
    }

    #[rstest]
    #[case::entered("abc123", Some("abc123"))]
    #[case::padded("  abc123\n", Some("abc123"))]
    #[case::empty("", None)]
    #[case::whitespace("   ", None)]
    fn a_key_is_the_entry_without_its_padding(
        #[case] entered: &str,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            ApiKey::new(entered).map(|key| key.query_value().to_owned()),
            expected.map(str::to_owned)
        );
    }

    /// Every failure detail passes through this, so a URL quoted back by the
    /// HTTP client cannot carry the key into a log or the settings page.
    #[test]
    fn redacting_removes_the_key_from_a_quoted_url() {
        let key = key("abc123");
        let detail = format!(
            "error sending request for url ({})",
            flare_url(
                DEFAULT_BASE_URL,
                DateWindow::covering_utc_day(date(2024, 5, 9)),
                &key
            )
        );

        let redacted = key.redact(&detail);

        assert_eq!(
            redacted,
            "error sending request for url \
             (https://api.nasa.gov/DONKI/FLR?startDate=2024-05-09&endDate=2024-05-09\
             &api_key=[redacted])"
        );
    }

    #[test]
    fn redacting_leaves_text_without_the_key_alone() {
        assert_eq!(key("abc123").redact("HTTP 503"), "HTTP 503");
    }

    #[test]
    fn a_fixture_window_names_its_capture_and_parses_its_dates() {
        for fixture in FIXTURE_WINDOWS {
            let window = fixture.window().expect("declared dates");
            assert!(
                window.start <= window.end,
                "{}: {}",
                fixture.name,
                fixture.purpose
            );
            assert_eq!(fixture.file_name(), format!("{}.json", fixture.name));
        }
    }

    /// A captured window must be one the catalog has already settled, so its
    /// capture cannot change meaning as time passes.
    #[test]
    fn no_captured_window_reaches_into_the_future() {
        for fixture in FIXTURE_WINDOWS {
            assert!(
                fixture.window().expect("declared dates").end < Utc::now().date_naive(),
                "{}: {}",
                fixture.name,
                fixture.purpose
            );
        }
    }
}
