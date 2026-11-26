use std::mem;

use gt_map::SkyTrailsWindow;
use gt_session_bridge::{ErrorCode, OpenPanel, Reply, SessionPanel, TrackReference};
use gt_types::SkyGeometry;
use gt_ui_components::{ToolWindow, ToolWindowRaise};
use gt_ui_types::SkyTrailsRequest;
use uuid::Uuid;

use super::App;
use super::history::HistoryWindow;
use super::log_viewer::LogViewerWindow;
use super::mcp_action_notice::McpActionNotice;
use super::query::QueryWindow;
use super::settings_ui;

#[derive(Default)]
pub(super) struct PendingPanelReveals {
    changed_panels: Vec<SessionPanel>,
    raise: Option<PanelRaise>,
}

impl PendingPanelReveals {
    fn record_view_change(&mut self, panel: SessionPanel) {
        if !self.changed_panels.contains(&panel) {
            self.changed_panels.push(panel);
        }
    }

    fn request_raise(&mut self, panel: SessionPanel, window_id: egui::Id) {
        let notify_when_frontmost = !self.changed_panels.contains(&panel);
        if let Some(previous) = self
            .raise
            .as_mut()
            .filter(|previous| previous.panel == panel)
        {
            previous.notify_when_frontmost &= notify_when_frontmost;
            return;
        }
        self.raise = Some(PanelRaise {
            panel,
            window_id,
            queued: false,
            notify_when_frontmost,
        });
    }
}

struct PanelRaise {
    panel: SessionPanel,
    window_id: egui::Id,
    queued: bool,
    notify_when_frontmost: bool,
}

impl App {
    fn session_panel_window_id(panel: SessionPanel) -> Option<egui::Id> {
        match panel {
            SessionPanel::History => Some(HistoryWindow::window_id()),
            SessionPanel::LogViewer => Some(LogViewerWindow::window_id()),
            SessionPanel::Plot => None,
            SessionPanel::Query => Some(QueryWindow::window_id()),
            SessionPanel::Settings => Some(egui::Id::unique(settings_ui::WINDOW_ID)),
            SessionPanel::SkyTrails => Some(SkyTrailsWindow::window_id()),
        }
    }

    pub(super) fn open_session_panel(
        &mut self,
        session: Uuid,
        args: OpenPanel,
    ) -> Result<Reply, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let panel = args.panel();
        let was_closed = !self.session_panel_is_open(panel);
        let changed = match args {
            OpenPanel::Query {} => self.query_window.reveal_editor(&self.ctx),
            OpenPanel::SkyTrails { track } => self.open_session_sky_trails(session, track)?,
            OpenPanel::History {} => {
                self.set_history_window_open(true);
                false
            }
            OpenPanel::LogViewer {} => {
                self.log_viewer.open_window();
                false
            }
            OpenPanel::Plot {} => {
                self.tiles_tree.set_visible(self.plot_tile_id, true);
                false
            }
            OpenPanel::Settings {} => {
                self.settings_open = true;
                false
            }
        };
        let window_id = Self::session_panel_window_id(panel);
        let expanded = panel != SessionPanel::Query
            && window_id.is_some_and(|id| ToolWindow::expand(&self.ctx, id));
        if was_closed || changed || expanded {
            self.pending_session_panel_reveals.record_view_change(panel);
        }
        if let Some(window_id) = window_id {
            self.complete_session_panel_raise();
            self.pending_session_panel_reveals
                .request_raise(panel, window_id);
        }
        self.ctx.request_repaint();
        Ok(Reply::PanelOpenQueued { panel })
    }

    pub(super) fn complete_session_panel_raise(&mut self) {
        let completed = self.pending_session_panel_reveals.raise.take_if(|raise| {
            raise.queued
                && ToolWindow::is_frontmost_after_layout(&self.ctx, raise.window_id) == Some(true)
        });
        if let Some(raise) = completed
            && raise.notify_when_frontmost
        {
            McpActionNotice::RevealPanel(raise.panel).enqueue(&mut self.toasts);
        }
    }

    pub(super) fn apply_session_panel_reveals(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        for panel in mem::take(&mut self.pending_session_panel_reveals.changed_panels) {
            if self.session_panel_is_open(panel) {
                McpActionNotice::RevealPanel(panel).enqueue(&mut self.toasts);
            }
        }
        let Some(mut raise) = self.pending_session_panel_reveals.raise.take() else {
            return;
        };
        if !self.session_panel_is_open(raise.panel) {
            return;
        }
        match ToolWindow::raise(ctx, raise.window_id) {
            ToolWindowRaise::AlreadyFrontmost if !raise.queued => return,
            ToolWindowRaise::Queued => raise.queued = true,
            ToolWindowRaise::AlreadyFrontmost | ToolWindowRaise::WaitingForLayout => {
                ctx.request_repaint();
            }
            ToolWindowRaise::WaitingForPointer => {}
        }
        self.pending_session_panel_reveals.raise = Some(raise);
    }

    pub(super) fn session_panel_is_open(&self, panel: SessionPanel) -> bool {
        match panel {
            SessionPanel::History => self.history_window.open,
            SessionPanel::LogViewer => self.log_viewer.open,
            SessionPanel::Plot => self.plot_is_visible(),
            SessionPanel::Query => self.query_window.open,
            SessionPanel::Settings => self.settings_open,
            SessionPanel::SkyTrails => self.sky_trails_window.is_open(),
        }
    }

    fn open_session_sky_trails(
        &mut self,
        session: Uuid,
        track: TrackReference,
    ) -> Result<bool, ErrorCode> {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.view();
        let reference = track.resolve(files, session)?;
        let loaded_track = reference
            .resolve(files.files())
            .ok_or(ErrorCode::StaleTrack)?;
        if !loaded_track
            .points
            .iter()
            .any(|point| point.supports_sky_geometry(SkyGeometry::Position))
        {
            return Err(ErrorCode::NoSkyTrailReports);
        }
        Ok(self
            .sky_trails_window
            .open(SkyTrailsRequest::whole_track(reference)))
    }
}
