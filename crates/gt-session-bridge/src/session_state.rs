use std::num::NonZeroUsize;

use chrono::{DateTime, Utc};
use gt_filter::TimeWindow;
use gt_loaded_files::{ListRecordings, LoadedFilesView};
use gt_ui_types::DisplayCategory;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::protocol::{self, OpenPoint, Recordings, TrackReference};
use crate::query_job::QueryJobId;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionState {
    pub recording_count: usize,
    #[serde(flatten)]
    pub recording_preview: Recordings,
    pub open_panels: Vec<SessionPanel>,
    pub time_filter: SessionTimeFilter,
    pub map: SessionMapState,
    pub query: Option<SessionQueryState>,
}

impl SessionState {
    pub fn recording_preview(
        files: LoadedFilesView<'_>,
        session: Uuid,
        name_template: &str,
    ) -> Recordings {
        let page = ListRecordings::new(None, RECORDING_PREVIEW_LIMIT).execute(files);
        Recordings::new(&page, session, name_template)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, strum::EnumIter)]
#[serde(rename_all = "snake_case")]
pub enum SessionPanel {
    History,
    LogViewer,
    Plot,
    Query,
    Settings,
    SkyTrails,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "panel", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenPanel {
    History {},
    LogViewer {},
    Plot {},
    Query {},
    Settings {},
    SkyTrails { track: TrackReference },
}

impl OpenPanel {
    pub fn panel(self) -> SessionPanel {
        match self {
            Self::History {} => SessionPanel::History,
            Self::LogViewer {} => SessionPanel::LogViewer,
            Self::Plot {} => SessionPanel::Plot,
            Self::Query {} => SessionPanel::Query,
            Self::Settings {} => SessionPanel::Settings,
            Self::SkyTrails { .. } => SessionPanel::SkyTrails,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionTimeFilter {
    All,
    Bounds {
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        start_unrepresentable: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        end_unrepresentable: bool,
    },
    Empty,
}

impl From<TimeWindow> for SessionTimeFilter {
    fn from(window: TimeWindow) -> Self {
        match window.bounds() {
            None => Self::Empty,
            Some((None, None)) => Self::All,
            Some((start, end)) => {
                let checked_start = start.and_then(protocol::rfc3339_bound);
                let checked_end = end.and_then(protocol::rfc3339_bound);
                Self::Bounds {
                    start: checked_start,
                    end: checked_end,
                    start_unrepresentable: start.is_some() && checked_start.is_none(),
                    end_unrepresentable: end.is_some() && checked_end.is_none(),
                }
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionMapState {
    pub base_layer: SessionMapLayer,
    pub visible_categories: Vec<DisplayCategory>,
    pub bounds: Option<SessionMapBounds>,
    pub pinned_point: Option<OpenPoint>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMapLayer {
    OpenStreetMap,
    Satellite,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionMapBounds {
    pub south_deg: f64,
    pub north_deg: f64,
    pub west_deg: f64,
    pub east_deg: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionQueryState {
    pub phase: SessionQueryPhase,
    pub job: Option<QueryJobId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionQueryPhase {
    Cancelled,
    Cancelling,
    Completed,
    Failed,
    Preparing,
    Running,
    Stale,
}

const RECORDING_PREVIEW_LIMIT: NonZeroUsize = match NonZeroUsize::new(5) {
    Some(limit) => limit,
    None => NonZeroUsize::MIN,
};
