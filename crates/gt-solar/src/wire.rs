//! The endpoint's JSON response, parsed into an index series.
//!
//! One response holds parallel arrays: the values under the index's own name,
//! their period start times under `datetime`, and for Kp a status per value.
//!
//! ```text
//! {"Kp":[2.667,9.0],
//!  "datetime":["2024-05-10T00:00:00Z","2024-05-10T03:00:00Z"],
//!  "meta":{"license":"CC BY 4.0","source":"GFZ Potsdam"},
//!  "status":["def","def"]}
//! ```
//!
//! Arrays that disagree in length leave every sample after the disagreement
//! unattributable, so any such response is refused whole. A `null` in the
//! value array is a period the service published no value for, and parses as
//! a sample without one.

use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use strum::VariantNames as _;

use crate::activity::GeomagneticActivity;
use crate::series::{Hp30Sample, Hp30Series, KpSample, KpSeries, KpStatus};
use crate::{GeomagneticIndex, parse_timestamp};

/// Why a response could not be read as a series.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("response is not JSON in the published shape: {0}")]
    Json(#[from] serde_json::Error),

    #[error("response has no {index} array")]
    MissingValues { index: GeomagneticIndex },

    #[error("{index} is {found}, expected an array of values")]
    ValuesNotAnArray {
        index: GeomagneticIndex,
        found: &'static str,
    },

    #[error("{index} has {values} values for {timestamps} timestamps")]
    ValueCountMismatch {
        index: GeomagneticIndex,
        values: usize,
        timestamps: usize,
    },

    #[error("{index} value {position} is {found}, expected a number or null")]
    ValueNotANumber {
        index: GeomagneticIndex,
        position: usize,
        found: &'static str,
    },

    #[error("{index} value {position} is {value}, outside the range {index} is published in")]
    ValueOutOfRange {
        index: GeomagneticIndex,
        position: usize,
        value: f64,
    },

    #[error("timestamp {position} ({timestamp:?}) is not an RFC 3339 time: {detail}")]
    Timestamp {
        position: usize,
        timestamp: String,
        detail: String,
    },

    #[error("response has no status array, which Kp publishes one entry of per value")]
    MissingKpStatus,

    #[error("Kp has {statuses} statuses for {timestamps} timestamps")]
    KpStatusCountMismatch { statuses: usize, timestamps: usize },

    #[error(
        "Kp status {position} is {status:?}, expected one of {:?}",
        KpStatus::VARIANTS
    )]
    UnrecognizedKpStatus { position: usize, status: String },
}

/// The response fields this parser reads. The value array arrives under the
/// requested index's name, so it is picked out of `columns` by key.
#[derive(Debug, Deserialize)]
struct ResponseBody {
    datetime: Vec<String>,
    #[serde(default)]
    status: Option<Vec<String>>,
    #[serde(flatten)]
    columns: BTreeMap<String, Value>,
}

/// One period's timestamp and value, before the per-index sample types are
/// built from them.
#[derive(Debug, Clone, Copy)]
struct Period {
    start: DateTime<Utc>,
    activity: Option<GeomagneticActivity>,
}

/// Parse a Kp response, whose values are three-hourly and each carry a
/// [`KpStatus`].
pub fn parse_kp_series(json: &str) -> Result<KpSeries, ParseError> {
    let body: ResponseBody = serde_json::from_str(json)?;
    let periods = parse_periods(&body, GeomagneticIndex::Kp)?;
    let statuses = parse_kp_statuses(&body)?;
    let samples = periods
        .into_iter()
        .zip(statuses)
        .map(|(period, status)| KpSample {
            period_start: period.start,
            activity: period.activity,
            status,
        })
        .collect();
    Ok(KpSeries { samples })
}

/// Parse an Hp30 response, whose values are half-hourly and carry no status.
pub fn parse_hp30_series(json: &str) -> Result<Hp30Series, ParseError> {
    let body: ResponseBody = serde_json::from_str(json)?;
    let samples = parse_periods(&body, GeomagneticIndex::Hp30)?
        .into_iter()
        .map(|period| Hp30Sample {
            period_start: period.start,
            activity: period.activity,
        })
        .collect();
    Ok(Hp30Series { samples })
}

fn parse_periods(body: &ResponseBody, index: GeomagneticIndex) -> Result<Vec<Period>, ParseError> {
    let column = body
        .columns
        .get(index.wire_name())
        .ok_or(ParseError::MissingValues { index })?;
    let values = column
        .as_array()
        .ok_or_else(|| ParseError::ValuesNotAnArray {
            index,
            found: json_type_name(column),
        })?;
    if values.len() != body.datetime.len() {
        return Err(ParseError::ValueCountMismatch {
            index,
            values: values.len(),
            timestamps: body.datetime.len(),
        });
    }

    body.datetime
        .iter()
        .zip(values)
        .enumerate()
        .map(|(position, (timestamp, value))| {
            Ok(Period {
                start: parse_timestamp(timestamp).map_err(|err| ParseError::Timestamp {
                    position,
                    timestamp: timestamp.clone(),
                    detail: err.to_string(),
                })?,
                activity: parse_activity(index, position, value)?,
            })
        })
        .collect()
}

fn parse_activity(
    index: GeomagneticIndex,
    position: usize,
    value: &Value,
) -> Result<Option<GeomagneticActivity>, ParseError> {
    if value.is_null() {
        return Ok(None);
    }
    let number = value.as_f64().ok_or(ParseError::ValueNotANumber {
        index,
        position,
        found: json_type_name(value),
    })?;
    GeomagneticActivity::from_published_value(index, number)
        .map(Some)
        .ok_or(ParseError::ValueOutOfRange {
            index,
            position,
            value: number,
        })
}

fn parse_kp_statuses(body: &ResponseBody) -> Result<Vec<KpStatus>, ParseError> {
    let statuses = body.status.as_ref().ok_or(ParseError::MissingKpStatus)?;
    if statuses.len() != body.datetime.len() {
        return Err(ParseError::KpStatusCountMismatch {
            statuses: statuses.len(),
            timestamps: body.datetime.len(),
        });
    }
    statuses
        .iter()
        .enumerate()
        .map(|(position, status)| {
            KpStatus::from_str(status).map_err(|_unmatched| ParseError::UnrecognizedKpStatus {
                position,
                status: status.clone(),
            })
        })
        .collect()
}

/// The JSON type of `value`, for an error naming what arrived instead.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::activity::GeomagneticStormClass;

    use super::*;

    /// The published Kp shape, with one definitive and one nowcast value.
    const KP_RESPONSE: &str = r#"{"Kp":[2.667,9.0],
        "datetime":["2024-05-10T00:00:00Z","2024-05-10T03:00:00Z"],
        "meta":{"license":"CC BY 4.0","source":"GFZ Potsdam"},
        "status":["def","pre"]}"#;

    /// The published Hp30 shape, which has no status array.
    const HP30_RESPONSE: &str = r#"{"Hp30":[1.667,11.333],
        "datetime":["2024-05-10T00:00:00Z","2024-05-10T00:30:00Z"],
        "meta":{"license":"CC BY 4.0","source":"GFZ Potsdam"}}"#;

    fn kp_value(value: f64) -> Option<GeomagneticActivity> {
        GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, value)
    }

    fn timestamp(text: &str) -> DateTime<Utc> {
        parse_timestamp(text).unwrap()
    }

    #[test]
    fn parses_the_published_kp_shape() {
        let series = parse_kp_series(KP_RESPONSE).unwrap();
        assert_eq!(
            series.samples.as_slice(),
            [
                KpSample {
                    period_start: timestamp("2024-05-10T00:00:00Z"),
                    activity: kp_value(2.667),
                    status: KpStatus::Definitive,
                },
                KpSample {
                    period_start: timestamp("2024-05-10T03:00:00Z"),
                    activity: kp_value(9.0),
                    status: KpStatus::Nowcast,
                },
            ]
        );
    }

    #[test]
    fn parses_the_published_hp30_shape() {
        let series = parse_hp30_series(HP30_RESPONSE).unwrap();
        assert_eq!(
            series.samples.as_slice(),
            [
                Hp30Sample {
                    period_start: timestamp("2024-05-10T00:00:00Z"),
                    activity: GeomagneticActivity::from_published_value(
                        GeomagneticIndex::Hp30,
                        1.667
                    ),
                },
                Hp30Sample {
                    period_start: timestamp("2024-05-10T00:30:00Z"),
                    activity: GeomagneticActivity::from_published_value(
                        GeomagneticIndex::Hp30,
                        11.333
                    ),
                },
            ]
        );
        assert_eq!(
            series
                .peak_activity()
                .and_then(GeomagneticActivity::storm_class),
            Some(GeomagneticStormClass::Extreme)
        );
    }

    /// A window the service has no values for.
    #[test]
    fn empty_arrays_parse_as_an_empty_series() {
        let json = r#"{"Hp30":[],"datetime":[],"meta":{"license":"CC BY 4.0"}}"#;
        assert!(parse_hp30_series(json).unwrap().is_empty());
    }

    #[test]
    fn a_null_value_parses_as_a_period_without_one() {
        let json = r#"{"Hp30":[null,3.0],
            "datetime":["2024-05-10T00:00:00Z","2024-05-10T00:30:00Z"]}"#;
        let series = parse_hp30_series(json).unwrap();
        assert_eq!(
            series
                .samples
                .iter()
                .map(|sample| sample.activity)
                .collect::<Vec<_>>(),
            vec![
                None,
                GeomagneticActivity::from_published_value(GeomagneticIndex::Hp30, 3.0)
            ]
        );
    }

    /// An Hp30 response that does carry a status array is read the same way.
    #[test]
    fn hp30_ignores_a_status_array() {
        let json = r#"{"Hp30":[3.0],"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#;
        assert_eq!(parse_hp30_series(json).unwrap().samples.len(), 1);
    }

    /// Kp is defined up to 9, Hp30 is not, so the same value is refused for
    /// one index and accepted for the other.
    #[test]
    fn a_kp_value_above_nine_is_refused() {
        let json = r#"{"Kp":[11.333],"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#;
        assert_eq!(
            parse_kp_series(json).unwrap_err().to_string(),
            "Kp value 0 is 11.333, outside the range Kp is published in"
        );
    }

    #[rstest]
    #[case::no_value_array(
        r#"{"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#,
        "response has no Kp array"
    )]
    #[case::values_not_an_array(
        r#"{"Kp":2.667,"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#,
        "Kp is a number, expected an array of values"
    )]
    #[case::more_values_than_timestamps(
        r#"{"Kp":[2.667,3.0],"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#,
        "Kp has 2 values for 1 timestamps"
    )]
    #[case::a_worded_value(
        r#"{"Kp":["quiet"],"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#,
        "Kp value 0 is a string, expected a number or null"
    )]
    #[case::a_negative_value(
        r#"{"Kp":[-1.0],"datetime":["2024-05-10T00:00:00Z"],"status":["def"]}"#,
        "Kp value 0 is -1, outside the range Kp is published in"
    )]
    #[case::a_worded_timestamp(
        r#"{"Kp":[2.667],"datetime":["yesterday"],"status":["def"]}"#,
        "timestamp 0 (\"yesterday\") is not an RFC 3339 time: premature end of input"
    )]
    #[case::no_status_array(
        r#"{"Kp":[2.667],"datetime":["2024-05-10T00:00:00Z"]}"#,
        "response has no status array, which Kp publishes one entry of per value"
    )]
    #[case::fewer_statuses_than_timestamps(
        r#"{"Kp":[2.667,3.0],
            "datetime":["2024-05-10T00:00:00Z","2024-05-10T03:00:00Z"],
            "status":["def"]}"#,
        "Kp has 1 statuses for 2 timestamps"
    )]
    #[case::an_unrecognized_status(
        r#"{"Kp":[2.667],"datetime":["2024-05-10T00:00:00Z"],"status":["maybe"]}"#,
        "Kp status 0 is \"maybe\", expected one of [\"def\", \"pre\"]"
    )]
    fn a_malformed_response_names_what_is_wrong(#[case] json: &str, #[case] expected: &str) {
        assert_eq!(parse_kp_series(json).unwrap_err().to_string(), expected);
    }

    #[rstest]
    #[case::not_json("not json at all")]
    #[case::an_array("[1,2,3]")]
    #[case::no_datetime_field(r#"{"Kp":[2.667],"status":["def"]}"#)]
    #[case::a_worded_timestamp_array(r#"{"Kp":[2.667],"datetime":"2024-05-10T00:00:00Z"}"#)]
    fn a_response_that_is_not_the_published_shape_is_refused(#[case] json: &str) {
        let error = parse_kp_series(json).unwrap_err();
        assert!(
            matches!(error, ParseError::Json(_)),
            "{json:?} produced {error:?}"
        );
        assert!(error.to_string().starts_with("response is not JSON"));
    }
}
