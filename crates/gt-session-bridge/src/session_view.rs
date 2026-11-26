use chrono::{DateTime, Utc};
use gt_filter::TimeWindow;
use gt_ui_types::DisplayCategory;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
#[cfg(feature = "schema")]
use strum::EnumCount as _;

use crate::protocol::{self, ErrorCode};
use crate::session_state::SessionMapLayer;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetSessionView {
    pub base_layer: Option<SessionMapLayer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "schema", schemars(length(max = "DisplayCategory::COUNT")))]
    pub display_categories: Vec<DisplayCategoryChange>,
    pub time_filter: Option<TimeFilterChange>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DisplayCategoryChange {
    pub category: DisplayCategory,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimeFilterChange {
    Clear {},
    Set {
        #[serde(default, deserialize_with = "self::deserialize_utc_bound")]
        start: Option<DateTime<Utc>>,
        #[serde(default, deserialize_with = "self::deserialize_utc_bound")]
        end: Option<DateTime<Utc>>,
    },
}

impl TimeFilterChange {
    pub fn window(self) -> Result<TimeWindow, ErrorCode> {
        match self {
            Self::Clear {} => Ok(TimeWindow::All),
            Self::Set { start, end } => {
                if [start, end]
                    .into_iter()
                    .flatten()
                    .any(|bound| protocol::rfc3339_bound(bound).is_none())
                {
                    return Err(ErrorCode::InvalidTimeFilter);
                }
                match TimeWindow::from_bounds(start, end) {
                    TimeWindow::All | TimeWindow::Empty => Err(ErrorCode::InvalidTimeFilter),
                    window => Ok(window),
                }
            }
        }
    }
}

fn deserialize_utc_bound<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|time| {
            let utc = time.strip_suffix('Z').ok_or_else(|| {
                D::Error::custom("Specify time bounds as RFC 3339 UTC timestamps ending in Z")
            })?;
            if utc
                .split_once('.')
                .is_some_and(|(_, fraction)| fraction.len() > FRACTIONAL_SECOND_DIGITS)
            {
                return Err(D::Error::custom("Time bounds support nanosecond precision"));
            }
            DateTime::parse_from_rfc3339(&time)
                .map(|instant| instant.to_utc())
                .map_err(D::Error::custom)
        })
        .transpose()
}

const FRACTIONAL_SECOND_DIGITS: usize = 9;
