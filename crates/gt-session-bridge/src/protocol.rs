use std::num::NonZeroUsize;

use chrono::{DateTime, Utc};
use gt_loaded_files::{
    LoadedFileEntry, LoadedFilesView, RecordingPage, RecordingSummary, SessionTrackRef,
};
use gt_types::{TimeRange as LoadedTimeRange, TrackIdx, TrackRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::inspection::{RangeDetails, RecordingDetails, TrackDetails};
use crate::metrics::Metrics;
use crate::query_job::{QueryJobRequest, QueryJobStatus};
use crate::query_results::{GetQueryResults, QueryResults};
use crate::query_validation::QueryValidation;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    CancelQuery(QueryJobRequest),
    CheckQuery(CheckQuery),
    GetQueryResults(GetQueryResults),
    GetQueryStatus(QueryJobRequest),
    GetRangeDetails(GetRangeDetails),
    GetRecording(GetRecording),
    GetTrack(GetTrack),
    ListMetrics(ListMetrics),
    ListRecordings(ListRecordings),
    OpenPoint(OpenPoint),
    RunQuery(CheckQuery),
    ShowOnMap(ShowOnMap),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OpenPoint {
    pub track: TrackReference,
    pub point: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ShowOnMap {
    pub target: MapTarget,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapTarget {
    Track {
        track: TrackReference,
        start: Option<usize>,
        end: Option<usize>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CheckQuery {
    pub text: String,
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

impl RecordingReference {
    pub(crate) fn number_in(self, session: Uuid) -> Result<u64, ErrorCode> {
        if self.session == session {
            Ok(self.number)
        } else {
            Err(ErrorCode::StaleSession)
        }
    }

    pub(crate) fn resolve(
        self,
        files: LoadedFilesView<'_>,
        session: Uuid,
    ) -> Result<LoadedFileEntry<'_>, ErrorCode> {
        files
            .entry_for_id_number(self.number_in(session)?)
            .ok_or(ErrorCode::RecordingNotFound)
    }
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

    pub fn resolve(self, files: LoadedFilesView<'_>, session: Uuid) -> Result<TrackRef, ErrorCode> {
        let entry = self.recording.resolve(files, session)?;
        files
            .resolve_track_at_revision(entry.id(), TrackIdx::new(self.index), self.revision)
            .ok_or(ErrorCode::StaleTrack)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Error { code: ErrorCode },
    MapPresentationQueued,
    Metrics { page: Metrics },
    QueryCheck { validation: QueryValidation },
    QueryJob { job: QueryJobStatus },
    QueryResults { results: QueryResults },
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
    InvalidPoint,
    InvalidQueryCursor,
    InvalidRange,
    InvalidRequest,
    NoVisibleMapGeometry,
    QueryColumnsTooLarge,
    QueryJobNotFound,
    QueryNotComplete,
    QueryResultsUnavailable,
    RecordingNotFound,
    ReplyTooLarge,
    RequestTooLarge,
    StaleQueryResults,
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
