use std::mem;
use std::time::{Duration, Instant};

use gt_loaded_files::RecordingData;
use gt_map::SkyTrailsWindow;
use gt_session_bridge::{
    ErrorCode, OpenPanel, PanelOpenState, Reply, SessionPanel, SessionReply, TrackReference,
};
use gt_types::{Generation, SkyGeometry, TrackRef};
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

    fn request_raise(&mut self, target: FloatingPanelTarget, waiter: PanelWaiter) {
        let notify_when_frontmost = !self.changed_panels.contains(&target.panel());
        if let Some(previous) = self
            .raise
            .as_mut()
            .filter(|previous| previous.target == target)
        {
            previous.notify_when_frontmost &= notify_when_frontmost;
            previous.waiters.push(waiter);
            return;
        }
        if let Some(previous) = self.raise.replace(PanelRaise {
            target,
            waiters: vec![waiter],
            phase: RaisePhase::Pending,
            notify_when_frontmost,
        }) {
            previous.finish(PanelOpenState::Superseded);
        }
    }

    #[cfg(test)]
    pub(super) fn expire_oldest(&mut self) {
        if let Some(waiter) = self
            .raise
            .as_mut()
            .and_then(|raise| raise.waiters.first_mut())
        {
            waiter.expires = Instant::now();
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FloatingPanelTarget {
    History,
    LogViewer,
    Query,
    Settings,
    SkyTrails(CheckedSkyTrack),
}

impl FloatingPanelTarget {
    fn for_request(app: &App, session: Uuid, args: OpenPanel) -> Result<Option<Self>, ErrorCode> {
        let target = match args {
            OpenPanel::History {} => Self::History,
            OpenPanel::LogViewer {} => Self::LogViewer,
            OpenPanel::Plot {} => return Ok(None),
            OpenPanel::Query {} => Self::Query,
            OpenPanel::Settings {} => Self::Settings,
            OpenPanel::SkyTrails { track } => {
                Self::SkyTrails(app.checked_sky_track(session, track)?)
            }
        };
        Ok(Some(target))
    }

    fn panel(self) -> SessionPanel {
        match self {
            Self::History => SessionPanel::History,
            Self::LogViewer => SessionPanel::LogViewer,
            Self::Query => SessionPanel::Query,
            Self::Settings => SessionPanel::Settings,
            Self::SkyTrails(_) => SessionPanel::SkyTrails,
        }
    }

    fn window_id(self) -> egui::Id {
        match self {
            Self::History => HistoryWindow::window_id(),
            Self::LogViewer => LogViewerWindow::window_id(),
            Self::Query => QueryWindow::window_id(),
            Self::Settings => egui::Id::unique(settings_ui::WINDOW_ID),
            Self::SkyTrails(_) => SkyTrailsWindow::window_id(),
        }
    }

    fn reveal(self, app: &mut App) -> bool {
        let was_closed = !app.session_panel_is_open(self.panel());
        let changed = match self {
            Self::History => {
                app.set_history_window_open(true);
                false
            }
            Self::LogViewer => {
                app.log_viewer.open_window();
                false
            }
            Self::Query => return app.query_window.reveal_editor(&app.ctx),
            Self::Settings => {
                app.settings_open = true;
                false
            }
            Self::SkyTrails(target) => app
                .sky_trails_window
                .open(SkyTrailsRequest::whole_track(target.track)),
        };
        let expanded = ToolWindow::expand(&app.ctx, self.window_id());
        was_closed || changed || expanded
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct CheckedSkyTrack {
    track: TrackRef,
    generation: Generation<RecordingData>,
}

struct PanelWaiter {
    reply: SessionReply,
    expires: Instant,
}

struct PanelRaise {
    target: FloatingPanelTarget,
    waiters: Vec<PanelWaiter>,
    phase: RaisePhase,
    notify_when_frontmost: bool,
}

enum RaisePhase {
    Pending,
    RetriedLayout,
    Verify,
}

impl PanelRaise {
    fn finish(self, state: PanelOpenState) {
        for waiter in self.waiters {
            waiter.reply.send(Reply::PanelOpen {
                panel: self.target.panel(),
                state,
            });
        }
    }

    fn retain_waiters(&mut self, ctx: &egui::Context) -> bool {
        let now = Instant::now();
        for waiter in self.waiters.extract_if(.., |waiter| {
            now >= waiter.expires || !waiter.reply.client_is_waiting()
        }) {
            if waiter.reply.client_is_waiting() {
                waiter.reply.send(Reply::PanelOpen {
                    panel: self.target.panel(),
                    state: PanelOpenState::TimedOut,
                });
            }
        }
        let Some(expires) = self.waiters.iter().map(|waiter| waiter.expires).min() else {
            return false;
        };
        ctx.request_repaint_after(expires.saturating_duration_since(now));
        true
    }
}

impl App {
    pub(super) fn open_session_panel(
        &mut self,
        session: Uuid,
        args: OpenPanel,
        reply: SessionReply,
    ) {
        if self.shutdown_requested() {
            reply.send(ErrorCode::ShuttingDown);
            return;
        }
        self.complete_session_panel_raise();
        let target = match FloatingPanelTarget::for_request(self, session, args) {
            Ok(target) => target,
            Err(error) => {
                reply.send(error);
                return;
            }
        };
        if !reply.client_is_waiting() {
            return;
        }

        let panel = target.map_or(SessionPanel::Plot, FloatingPanelTarget::panel);
        let changed = if let Some(target) = target {
            target.reveal(self)
        } else {
            let was_closed = !self.plot_is_visible();
            self.tiles_tree.set_visible(self.plot_tile_id, true);
            was_closed
        };
        if changed {
            self.pending_session_panel_reveals.record_view_change(panel);
        }
        if let Some(target) = target {
            self.pending_session_panel_reveals.request_raise(
                target,
                PanelWaiter {
                    reply,
                    expires: Instant::now() + PANEL_RAISE_WAIT,
                },
            );
            self.ctx.request_repaint_after(PANEL_RAISE_WAIT);
        } else {
            reply.send(Reply::PanelOpen {
                panel,
                state: PanelOpenState::Docked,
            });
        }
        if self
            .ctx
            .input(|input| input.viewport().visible() != Some(false))
        {
            self.ctx.request_repaint();
        }
    }

    fn take_current_session_panel_raise(&mut self) -> Option<PanelRaise> {
        let raise = self.pending_session_panel_reveals.raise.take()?;
        if !self.session_panel_is_open(raise.target.panel()) {
            raise.finish(PanelOpenState::Closed);
            return None;
        }
        if let FloatingPanelTarget::SkyTrails(target) = raise.target
            && (self.sky_trails_window.track() != Some(target.track)
                || self.shared.borrow().loaded_files.data_generation() != target.generation)
        {
            raise.finish(PanelOpenState::Superseded);
            return None;
        }
        Some(raise)
    }

    pub(super) fn complete_session_panel_raise(&mut self) {
        let Some(mut raise) = self.take_current_session_panel_raise() else {
            return;
        };
        if matches!(raise.phase, RaisePhase::Verify)
            && let Some(frontmost) =
                ToolWindow::is_frontmost_after_layout(&self.ctx, raise.target.window_id())
        {
            let state = if frontmost {
                if raise.notify_when_frontmost {
                    McpActionNotice::RevealPanel(raise.target.panel()).enqueue(&mut self.toasts);
                }
                PanelOpenState::Frontmost
            } else {
                PanelOpenState::Superseded
            };
            raise.finish(state);
        } else if raise.retain_waiters(&self.ctx) {
            self.pending_session_panel_reveals.raise = Some(raise);
        }
    }

    pub(super) fn cancel_pending_session_panel_raise(&mut self) {
        self.pending_session_panel_reveals.raise = None;
    }

    pub(super) fn apply_session_panel_reveals(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() || ctx.input(|input| input.viewport().visible() == Some(false)) {
            return;
        }
        for panel in mem::take(&mut self.pending_session_panel_reveals.changed_panels) {
            if self.session_panel_is_open(panel) {
                McpActionNotice::RevealPanel(panel).enqueue(&mut self.toasts);
            }
        }
        let Some(mut raise) = self.take_current_session_panel_raise() else {
            return;
        };
        if matches!(raise.phase, RaisePhase::Verify) {
            self.pending_session_panel_reveals.raise = Some(raise);
            return;
        }
        if !raise.retain_waiters(ctx) {
            return;
        }
        match ToolWindow::raise(ctx, raise.target.window_id()) {
            ToolWindowRaise::AlreadyFrontmost => {
                // egui updates the layer order at the end of the pass.
                raise.notify_when_frontmost = false;
                raise.phase = RaisePhase::Verify;
                ctx.request_repaint();
            }
            ToolWindowRaise::Queued => raise.phase = RaisePhase::Verify,
            ToolWindowRaise::WaitingForLayout if matches!(raise.phase, RaisePhase::Pending) => {
                raise.phase = RaisePhase::RetriedLayout;
                ctx.request_repaint();
            }
            ToolWindowRaise::WaitingForLayout | ToolWindowRaise::WaitingForPointer => {}
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

    fn checked_sky_track(
        &self,
        session: Uuid,
        track: TrackReference,
    ) -> Result<CheckedSkyTrack, ErrorCode> {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.view();
        let track = track.resolve(files, session)?;
        let loaded_track = track.resolve(files.files()).ok_or(ErrorCode::StaleTrack)?;
        if !loaded_track
            .points
            .iter()
            .any(|point| point.supports_sky_geometry(SkyGeometry::Position))
        {
            return Err(ErrorCode::NoSkyTrailReports);
        }
        Ok(CheckedSkyTrack {
            track,
            generation: shared.loaded_files.data_generation(),
        })
    }
}

const PANEL_RAISE_WAIT: Duration = Duration::from_millis(500);
