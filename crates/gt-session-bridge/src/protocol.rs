use std::num::NonZeroUsize;

use chrono::{DateTime, Utc};
use gt_loaded_files::{ListRecordings, RecordingPage, RecordingSummary};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    ListRecordings {
        after: Option<u64>,
        limit: NonZeroUsize,
    },
}

impl From<Request> for ListRecordings {
    fn from(Request::ListRecordings { after, limit }: Request) -> Self {
        Self::new(after, limit)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Error { code: ErrorCode },
    Recordings { page: Recordings },
}

impl From<RecordingPage> for Reply {
    fn from(page: RecordingPage) -> Self {
        Self::Recordings {
            page: Recordings {
                recordings: page.recordings.into_iter().map(Recording::from).collect(),
                next_after: page.next_after.map(u64::from),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Busy,
    InvalidRequest,
    ReplyTooLarge,
    RequestTooLarge,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recordings {
    pub recordings: Vec<Recording>,
    pub next_after: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recording {
    pub id: u64,
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

impl From<RecordingSummary> for Recording {
    fn from(recording: RecordingSummary) -> Self {
        Self {
            id: recording.id.into(),
            display_name: recording.display_name,
            filename: recording.filename,
            title: recording.title,
            device: recording.device,
            time_range: recording.time_range.map(|range| TimeRange {
                start: range.start,
                end: range.end,
            }),
            track_count: recording.track_count,
        }
    }
}
