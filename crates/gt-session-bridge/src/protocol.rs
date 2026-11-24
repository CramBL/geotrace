use std::num::NonZeroUsize;

use chrono::{DateTime, Utc};
use gt_loaded_files::{RecordingPage, RecordingSummary, SessionTrackRef};
use gt_types::TimeRange as LoadedTimeRange;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::inspection::{RangeDetails, RecordingDetails, TrackDetails};
use crate::metrics::Metrics;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    GetRangeDetails(GetRangeDetails),
    GetRecording(GetRecording),
    GetTrack(GetTrack),
    ListMetrics(ListMetrics),
    ListRecordings(ListRecordings),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GetRangeDetails {
    pub track: TrackReference,
    /// First zero-based point index. Defaults to zero.
    pub start: Option<usize>,
    /// Exclusive point index. Defaults to the track's point count.
    pub end: Option<usize>,
    pub limit: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GetRecording {
    pub recording: RecordingReference,
    /// Continue after the track reference from the previous page.
    pub after: Option<TrackReference>,
    pub limit: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GetTrack {
    pub track: TrackReference,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ListMetrics {
    /// Channel offset in name order. Defaults to zero.
    pub channel_start: Option<usize>,
    pub channel_limit: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ListRecordings {
    /// Cursor from the previous page in this desktop session.
    pub after: Option<RecordingReference>,
    pub limit: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RecordingReference {
    pub session: Uuid,
    pub number: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TrackReference {
    pub recording: RecordingReference,
    pub index: usize,
    pub revision: u64,
}

impl TrackReference {
    pub(crate) fn new(session: Uuid, track: SessionTrackRef) -> Self {
        Self {
            recording: RecordingReference {
                session,
                number: track.recording().into(),
            },
            index: track.index().as_usize(),
            revision: track.revision(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Error { code: ErrorCode },
    Metrics { page: Metrics },
    Range { details: RangeDetails },
    Recording { details: RecordingDetails },
    Recordings { page: Recordings },
    Track { details: TrackDetails },
}

impl From<ErrorCode> for Reply {
    fn from(code: ErrorCode) -> Self {
        Self::Error { code }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Busy,
    InvalidRange,
    InvalidRequest,
    RecordingNotFound,
    ReplyTooLarge,
    RequestTooLarge,
    StaleSession,
    StaleTrack,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recordings {
    pub recordings: Vec<Recording>,
    pub next_after: Option<RecordingReference>,
}

impl Recordings {
    pub(crate) fn new(page: RecordingPage, session: Uuid) -> Self {
        Self {
            recordings: page
                .recordings
                .into_iter()
                .map(|summary| Recording::new(summary, session))
                .collect(),
            next_after: page.next_after.map(|number| RecordingReference {
                session,
                number: number.into(),
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recording {
    pub id: RecordingReference,
    pub display_name: String,
    pub filename: String,
    pub title: Option<String>,
    pub device: Option<String>,
    pub time_range: Option<TimeRange>,
    pub track_count: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl From<LoadedTimeRange> for TimeRange {
    fn from(LoadedTimeRange { start, end }: LoadedTimeRange) -> Self {
        Self { start, end }
    }
}

impl Recording {
    pub(crate) fn new(recording: RecordingSummary, session: Uuid) -> Self {
        Self {
            id: RecordingReference {
                session,
                number: recording.id.into(),
            },
            display_name: recording.display_name,
            filename: recording.filename,
            title: recording.title,
            device: recording.device,
            time_range: recording.time_range.map(Into::into),
            track_count: recording.track_count,
        }
    }
}
