use std::num::NonZeroUsize;
use std::ops::Range;

use chrono::{DateTime, Utc};
use gt_loaded_files::{
    LoadedFileEntry, LoadedFilesView, MAX_RECORDINGS_PER_PAGE, RecordingPage, RecordingSummary,
    SessionTrackRef,
};
use gt_types::{TimeRange as LoadedTimeRange, TrackIdx, TrackRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::framing::{Frame, MAX_REPLY_BYTES};
use crate::history::{
    GetHistoryOpenStatus, HistoryOpenStatus, HistoryRecordings, ListHistoryRecordings,
    OpenHistoryRecording,
};
use crate::inspection::{RangeDetails, RecordingDetails, TrackDetails};
use crate::metrics::Metrics;
use crate::query_job::{QueryJobId, QueryJobRequest, QueryJobStatus};
use crate::query_results::{GetQueryResults, QueryMatchPosition, QueryResults};
use crate::query_validation::QueryValidation;
use crate::settings::{SetSetting, SettingInfo, SettingSelection};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    ApplyProcessing(ApplyProcessing),
    CancelQuery(QueryJobRequest),
    CheckQuery(QueryText),
    DescribeSetting(SettingSelection),
    GetHistoryOpenStatus(GetHistoryOpenStatus),
    GetQueryResults(GetQueryResults),
    GetQueryStatus(QueryJobRequest),
    GetRangeDetails(GetRangeDetails),
    GetRecording(GetRecording),
    GetSettings,
    GetTrack(GetTrack),
    ListHistoryRecordings(ListHistoryRecordings),
    ListMetrics(ListMetrics),
    ListRecordings(ListRecordings),
    OpenHistoryRecording(OpenHistoryRecording),
    OpenPoint(OpenPoint),
    OpenQuery(QueryText),
    OpenSetting(SettingSelection),
    RunQuery(QueryText),
    SetSetting(SetSetting),
    ShowOnMap(ShowOnMap),
    ShowOnPlot(ShowOnPlot),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ApplyProcessing {}

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
    pub target: RangeTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ShowOnPlot {
    pub target: RangeTarget,
    /// Query metric identifiers to enable on the plot.
    #[serde(default)]
    pub metrics: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RangeTarget {
    QueryMatch {
        job: QueryJobId,
        query: usize,
        matched: QueryMatchPosition,
    },
    Track {
        track: TrackReference,
        start: Option<usize>,
        end: Option<usize>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct QueryText {
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

    pub fn resolve_range(
        self,
        files: LoadedFilesView<'_>,
        session: Uuid,
        bounds: Range<Option<usize>>,
    ) -> Result<(TrackRef, Range<usize>), ErrorCode> {
        let track = self.resolve(files, session)?;
        let loaded = track.resolve(files.files()).ok_or(ErrorCode::StaleTrack)?;
        let points = bounds.start.unwrap_or_default()..bounds.end.unwrap_or(loaded.points.len());
        if points.is_empty() || points.end > loaded.points.len() {
            return Err(ErrorCode::InvalidRange);
        }
        Ok((track, points))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Error { code: ErrorCode },
    HistoryOpen { operation: HistoryOpenStatus },
    HistoryRecordings { page: HistoryRecordings },
    MapPresentationQueued,
    Metrics { page: Metrics },
    PlotPresentationQueued,
    ProcessingApplied,
    QueryCheck { validation: QueryValidation },
    QueryDraftQueued,
    QueryJob { job: QueryJobStatus },
    QueryResults { results: QueryResults },
    Range { details: RangeDetails },
    Recording { details: RecordingDetails },
    Recordings { page: Recordings },
    Setting { setting: SettingInfo },
    SettingOpenQueued,
    Settings { settings: Vec<SettingInfo> },
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
    HistoryListingFailed,
    HistoryNotReady,
    HistoryOpenFailed,
    HistoryOpenNotFound,
    HistorySelectionExpired,
    HistorySelectionRequiresDesktop,
    HistoryTrackLayoutUnavailable,
    HistoryUnavailable,
    InactiveSetting,
    InvalidHistoryCursor,
    InvalidHistoryDateRange,
    InvalidPlotMetric,
    InvalidPoint,
    InvalidQueryCursor,
    InvalidRange,
    InvalidRequest,
    InvalidSettingType,
    InvalidSettingValue,
    NoLoadedNavPoints,
    NoVisibleMapGeometry,
    NoVisiblePlotTime,
    QueryColumnsTooLarge,
    QueryHighlightsDisabled,
    QueryJobNotFound,
    QueryMatchIsChannel,
    QueryNotComplete,
    QueryResultsUnavailable,
    RecordingNotFound,
    ReplyTooLarge,
    RequestTooLarge,
    ShuttingDown,
    StaleQueryResults,
    StaleSession,
    StaleTrack,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NumericObservation {
    Finite { value: f64 },
    Missing,
    NonFinite,
}

impl From<Option<f64>> for NumericObservation {
    fn from(value: Option<f64>) -> Self {
        match value {
            Some(value) if value.is_finite() => Self::Finite { value },
            Some(_) => Self::NonFinite,
            None => Self::Missing,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recordings {
    pub recordings: Vec<Recording>,
    pub next_after: Option<RecordingReference>,
}

impl Recordings {
    pub(crate) fn new(page: &RecordingPage<'_>, session: Uuid, name_template: &str) -> Self {
        Self {
            recordings: page
                .recordings
                .iter()
                .map(|summary| Recording::new(summary, session, name_template))
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub metadata_truncated: bool,
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
    pub(crate) fn new(
        recording: &RecordingSummary<'_>,
        session: Uuid,
        name_template: &str,
    ) -> Self {
        let fields = recording.name_fields;
        let (name, truncated) = gt_fmt::render_name_template_prefix(
            name_template,
            &fields,
            MAX_METADATA_JSON_BYTES - JSON_QUOTE_BYTES,
        );
        let mut metadata = MetadataPreview { truncated };
        let display_name = metadata.prefix(&name).to_owned();
        let filename = metadata.prefix(fields.filename).to_owned();
        let title = fields.title.map(|value| metadata.prefix(value).to_owned());
        let device = fields.device.map(|value| metadata.prefix(value).to_owned());
        Self {
            id: RecordingReference {
                session,
                number: recording.id.into(),
            },
            display_name,
            filename,
            title,
            device,
            metadata_truncated: metadata.truncated,
            time_range: recording.time_range.map(Into::into),
            track_count: recording.track_count,
        }
    }
}

struct MetadataPreview {
    truncated: bool,
}

impl MetadataPreview {
    fn prefix<'a>(&mut self, text: &'a str) -> &'a str {
        let mut upper = text.len().min(MAX_METADATA_JSON_BYTES - JSON_QUOTE_BYTES);
        let candidate = text
            .get(..text.floor_char_boundary(upper))
            .unwrap_or_default();
        let prefix = if Frame::encode(&candidate, MAX_METADATA_JSON_BYTES).is_ok() {
            candidate
        } else {
            let mut lower = 0;
            while lower < upper {
                let middle = lower + (upper - lower).div_ceil(2);
                let candidate = text
                    .get(..text.floor_char_boundary(middle))
                    .unwrap_or_default();
                if Frame::encode(&candidate, MAX_METADATA_JSON_BYTES).is_ok() {
                    lower = middle;
                } else {
                    upper = middle - 1;
                }
            }
            text.get(..text.floor_char_boundary(lower))
                .unwrap_or_default()
        };
        self.truncated |= prefix.len() < text.len();
        prefix
    }
}

const JSON_QUOTE_BYTES: usize = 2;
// Four strings use one fifth each. The remaining fifth covers the row and page structure.
const MAX_METADATA_JSON_BYTES: usize = MAX_REPLY_BYTES / MAX_RECORDINGS_PER_PAGE / 5;
