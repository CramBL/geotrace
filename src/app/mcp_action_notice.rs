use std::borrow::Cow;

use egui_notify::Toasts;
use gt_session_bridge::SessionPanel;

pub(super) enum McpActionNotice {
    CancelQueuedQuery,
    CenterMapOnPoint,
    ChangeSetting(&'static str),
    ChangeTrackVisibility,
    FrameMapRange,
    FramePlot,
    OpenHistoryRecording,
    OpenQueryEditor,
    QueueFiles,
    QueueQuery,
    ReprocessRecordings,
    RequestQueryCancellation,
    RevealPanel(SessionPanel),
    RevealSetting(&'static str),
    UnloadRecordings,
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
            Self::QueueFiles => "queued files to open".into(),
            Self::UpdateQueryDraft => "updated the query editor".into(),
            Self::QueueQuery => "queued a query".into(),
            Self::RevealPanel(panel) => match panel {
                SessionPanel::History => "showed History",
                SessionPanel::LogViewer => "showed the Log viewer",
                SessionPanel::Plot => "showed the plot",
                SessionPanel::Query => "showed the Query window",
                SessionPanel::Settings => "showed Settings",
                SessionPanel::SkyTrails => "showed Sky trails",
            }
            .into(),
            Self::ChangeSetting(label) => format!("changed {label}").into(),
            Self::RevealSetting(label) => format!("revealed {label}").into(),
            Self::UnloadRecordings => "unloaded recordings".into(),
        };
        toasts
            .info(format!("MCP {action}"))
            .show_progress_bar(false);
    }
}
