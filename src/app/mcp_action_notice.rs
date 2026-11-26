use std::borrow::Cow;

use egui_notify::Toasts;

pub(super) enum McpActionNotice {
    CancelQueuedQuery,
    CenterMapOnPoint,
    ChangeSetting(&'static str),
    ChangeTrackVisibility,
    FrameMapRange,
    FramePlot,
    OpenHistoryRecording,
    OpenQueryEditor,
    QueueQuery,
    ReprocessRecordings,
    RequestQueryCancellation,
    RevealSetting(&'static str),
    UpdateQueryDraft,
}

impl McpActionNotice {
    pub(super) fn enqueue(self, toasts: &mut Toasts) {
        let action: Cow<'static, str> = match self {
            Self::OpenHistoryRecording => "opened a History recording".into(),
            Self::CenterMapOnPoint => "centered the map on a point".into(),
            Self::FrameMapRange => "framed a range on the map".into(),
            Self::FramePlot => "framed the plot".into(),
            Self::ReprocessRecordings => "reprocessed the loaded recordings".into(),
            Self::RequestQueryCancellation => "requested query cancellation".into(),
            Self::CancelQueuedQuery => "cancelled a queued query".into(),
            Self::ChangeTrackVisibility => "changed checkboxes in the track tree".into(),
            Self::OpenQueryEditor => "opened the query editor".into(),
            Self::UpdateQueryDraft => "updated the query editor".into(),
            Self::QueueQuery => "queued a query".into(),
            Self::ChangeSetting(label) => format!("changed {label}").into(),
            Self::RevealSetting(label) => format!("revealed {label}").into(),
        };
        toasts
            .info(format!("MCP {action}"))
            .show_progress_bar(false);
    }
}
