//! The endpoint's JSON response, parsed into flare events.
//!
//! One response is an array of events, each holding the three times, the
//! classification, and where on the Sun the flare came from:
//!
//! ```text
//! [{"flrID":"2024-05-09T00:58:00-FLR-001","beginTime":"2024-05-09T00:58Z",
//!   "peakTime":"2024-05-09T01:15Z","endTime":"2024-05-09T01:57Z",
//!   "classType":"M1.8","sourceLocation":"S20W19","activeRegionNum":13664}]
//! ```
//!
//! The times are written to the minute, which is not RFC 3339, so
//! [`parse_flare_time`] reads that one format. `endTime`, `sourceLocation`
//! and `activeRegionNum` arrive as `null` on events the catalog left them off,
//! and parse as absent. The parser rejects the whole response where one event
//! cannot be read: an event silently dropped is a flare missing from the plot
//! with nothing saying so.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::class::{ClassificationParseError, FlareClassification};
use crate::flare::SolarFlare;

/// Format the three event times are written in, to the minute.
const EVENT_TIME_FORMAT: &str = "%Y-%m-%dT%H:%MZ";

/// Why a response could not be read as flare events.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("response is not JSON in the published shape: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{flare}: {field} is {value:?}, expected a time like 2024-05-09T00:58Z")]
    EventTime {
        flare: String,
        field: &'static str,
        value: String,
    },

    #[error("{flare}: classType is unreadable: {source}")]
    Classification {
        flare: String,
        #[source]
        source: ClassificationParseError,
    },

    #[error("{flare}: activeRegionNum is {number}, expected an active region number")]
    ActiveRegion { flare: String, number: i64 },
}

/// The response fields this parser reads.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireFlare {
    #[serde(rename = "flrID")]
    flr_id: String,
    begin_time: String,
    peak_time: String,
    #[serde(default)]
    end_time: Option<String>,
    class_type: String,
    #[serde(default)]
    source_location: Option<String>,
    #[serde(default)]
    active_region_num: Option<i64>,
}

/// Parse a response into its events, ordered by peak time.
///
/// The order is the one the plot places markers in, and it is not the order
/// the catalog returns: a long flare beginning before a short one can peak
/// after it.
pub fn parse_flares(json: &str) -> Result<Vec<SolarFlare>, ParseError> {
    let mut flares: Vec<SolarFlare> = serde_json::from_str::<Vec<WireFlare>>(json)?
        .into_iter()
        .map(read_flare)
        .collect::<Result<_, _>>()?;
    flares.sort_by_key(|flare| flare.peak);
    Ok(flares)
}

/// Read a time as the catalog writes them, `2024-05-09T00:58Z`.
pub fn parse_flare_time(time: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    Ok(NaiveDateTime::parse_from_str(time, EVENT_TIME_FORMAT)?.and_utc())
}

fn read_flare(wire: WireFlare) -> Result<SolarFlare, ParseError> {
    let event_time = |field, value: &str| {
        parse_flare_time(value).map_err(|_unparsed| ParseError::EventTime {
            flare: wire.flr_id.clone(),
            field,
            value: value.to_owned(),
        })
    };
    Ok(SolarFlare {
        begin: event_time("beginTime", &wire.begin_time)?,
        peak: event_time("peakTime", &wire.peak_time)?,
        end: wire
            .end_time
            .as_deref()
            .map(|end| event_time("endTime", end))
            .transpose()?,
        classification: wire
            .class_type
            .parse::<FlareClassification>()
            .map_err(|source| ParseError::Classification {
                flare: wire.flr_id.clone(),
                source,
            })?,
        source_location: wire
            .source_location
            .filter(|location| !location.trim().is_empty()),
        active_region: wire
            .active_region_num
            .map(|number| {
                u32::try_from(number).map_err(|_out_of_range| ParseError::ActiveRegion {
                    flare: wire.flr_id.clone(),
                    number,
                })
            })
            .transpose()?,
        id: wire.flr_id,
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::class::{FlareClass, RadioBlackoutClass};

    /// The published shape, with every field the parser reads.
    const ONE_FLARE: &str = r#"[{"flrID":"2024-05-09T08:45:00-FLR-001",
        "catalog":"M2M_CATALOG",
        "instruments":[{"displayName":"GOES-P: EXIS 1.0-8.0"}],
        "beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
        "endTime":"2024-05-09T09:36Z","classType":"X2.2",
        "sourceLocation":"S20W25","activeRegionNum":13664,
        "note":"","submissionTime":"2024-05-09T16:18Z","versionId":1,
        "linkedEvents":null}]"#;

    fn only_flare(json: &str) -> SolarFlare {
        let mut flares = parse_flares(json).expect("the published shape");
        assert_eq!(flares.len(), 1);
        flares.remove(0)
    }

    fn time(text: &str) -> DateTime<Utc> {
        parse_flare_time(text).expect("a catalog time")
    }

    #[test]
    fn parses_the_published_shape() {
        assert_eq!(
            only_flare(ONE_FLARE),
            SolarFlare {
                id: "2024-05-09T08:45:00-FLR-001".to_owned(),
                begin: time("2024-05-09T08:45Z"),
                peak: time("2024-05-09T09:13Z"),
                end: Some(time("2024-05-09T09:36Z")),
                classification: "X2.2".parse().expect("a published class"),
                source_location: Some("S20W25".to_owned()),
                active_region: Some(13664),
            }
        );
    }

    /// A window the catalog lists nothing for.
    #[test]
    fn an_empty_array_parses_as_no_flares() {
        assert!(parse_flares("[]").expect("an empty window").is_empty());
    }

    /// The three optional fields are absent on real events, and an event
    /// missing all of them still places a marker.
    #[rstest]
    #[case::as_nulls(
        r#"[{"flrID":"f","beginTime":"2019-01-26T13:13Z","peakTime":"2019-01-26T13:22Z",
            "endTime":null,"classType":"C5.0","sourceLocation":null,"activeRegionNum":null}]"#
    )]
    #[case::left_out(
        r#"[{"flrID":"f","beginTime":"2019-01-26T13:13Z","peakTime":"2019-01-26T13:22Z",
            "classType":"C5.0"}]"#
    )]
    #[case::as_an_empty_location(
        r#"[{"flrID":"f","beginTime":"2019-01-26T13:13Z","peakTime":"2019-01-26T13:22Z",
            "endTime":null,"classType":"C5.0","sourceLocation":"  ","activeRegionNum":null}]"#
    )]
    fn an_event_without_its_optional_fields_still_parses(#[case] json: &str) {
        let flare = only_flare(json);
        assert_eq!(flare.end, None);
        assert_eq!(flare.source_location, None);
        assert_eq!(flare.active_region, None);
        assert_eq!(flare.classification.class(), FlareClass::C);
    }

    /// The catalog returns events in beginning order, and the plot marks peaks.
    #[test]
    fn events_come_back_ordered_by_peak() {
        let json = r#"[
            {"flrID":"long","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
             "classType":"X2.2"},
            {"flrID":"short","beginTime":"2024-05-09T08:50Z","peakTime":"2024-05-09T08:52Z",
             "classType":"M1.0"}]"#;
        let flares = parse_flares(json).expect("two events");
        assert_eq!(
            flares
                .iter()
                .map(|flare| flare.id.as_str())
                .collect::<Vec<_>>(),
            ["short", "long"]
        );
    }

    /// The strongest flare of a response is the one the day is remembered by.
    #[test]
    fn the_strongest_class_of_a_response_is_the_x_class_one() {
        let json = r#"[
            {"flrID":"a","beginTime":"2024-05-09T03:07Z","peakTime":"2024-05-09T03:17Z",
             "classType":"M4.0"},
            {"flrID":"b","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
             "classType":"X2.2"}]"#;
        let strongest = parse_flares(json)
            .expect("two events")
            .into_iter()
            .map(|flare| flare.classification)
            .max()
            .expect("a strongest flare");
        assert_eq!(strongest.to_string(), "X2.2");
        assert_eq!(
            strongest.radio_blackout_class(),
            Some(RadioBlackoutClass::Strong)
        );
    }

    #[rstest]
    #[case::a_seconds_precision_time(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45:00Z","peakTime":"2024-05-09T09:13Z",
            "classType":"X2.2"}]"#,
        "f: beginTime is \"2024-05-09T08:45:00Z\", expected a time like 2024-05-09T00:58Z"
    )]
    #[case::a_worded_peak_time(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45Z","peakTime":"noon","classType":"X2.2"}]"#,
        "f: peakTime is \"noon\", expected a time like 2024-05-09T00:58Z"
    )]
    #[case::a_worded_end_time(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
            "endTime":"later","classType":"X2.2"}]"#,
        "f: endTime is \"later\", expected a time like 2024-05-09T00:58Z"
    )]
    #[case::an_unknown_class(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
            "classType":"Z9.9"}]"#,
        "f: classType is unreadable: \"Z9.9\" names no flare class, which is one of A, B, C, M or X"
    )]
    #[case::a_negative_active_region(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
            "classType":"X2.2","activeRegionNum":-1}]"#,
        "f: activeRegionNum is -1, expected an active region number"
    )]
    fn a_malformed_event_names_what_is_wrong(#[case] json: &str, #[case] expected: &str) {
        assert_eq!(
            parse_flares(json).expect_err("rejected").to_string(),
            expected
        );
    }

    #[rstest]
    #[case::not_json("not json at all")]
    #[case::an_object(r#"{"flrID":"f"}"#)]
    #[case::without_a_begin_time(
        r#"[{"flrID":"f","peakTime":"2024-05-09T09:13Z","classType":"X2.2"}]"#
    )]
    #[case::without_a_class(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z"}]"#
    )]
    #[case::a_numeric_class(
        r#"[{"flrID":"f","beginTime":"2024-05-09T08:45Z","peakTime":"2024-05-09T09:13Z",
            "classType":2.2}]"#
    )]
    fn a_response_that_is_not_the_published_shape_is_rejected(#[case] json: &str) {
        let error = parse_flares(json).expect_err("rejected");
        assert!(
            matches!(error, ParseError::Json(_)),
            "{json:?} produced {error:?}"
        );
        assert!(error.to_string().starts_with("response is not JSON"));
    }
}
