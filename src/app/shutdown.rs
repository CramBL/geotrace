//! Closing the window with background writes still running.
//!
//! The close request is intercepted, the app keeps painting, and the window
//! closes once the pending-write registry is idle.

use std::mem;
use std::time::{Duration, Instant};

use egui::CentralPanel;
use gt_pending_writes::WriteKind;

use super::{App, history_db};

/// How long shutdown runs before the panel appears. Under it a close with
/// nothing pending shows no panel at all.
pub(in crate::app) const PANEL_GRACE: Duration = Duration::from_millis(200);

/// Shutdown repaints on this interval so the writes it lists advance without
/// new input.
const SHUTDOWN_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

const SETTINGS_FLUSH_LABEL: &str = "Saving settings";
const HISTORY_SHUTDOWN_LABEL: &str = "Finishing recording history work";

/// How far the app has got in closing.
#[derive(Debug, Default)]
pub(in crate::app) enum ShutdownState {
    #[default]
    NotStarted,
    Started {
        begun_at: Instant,
    },
    /// The close request the app sends itself, which it must not intercept.
    CloseAllowed,
}

impl ShutdownState {
    pub(in crate::app) fn begin(&mut self, now: Instant) {
        if matches!(self, Self::NotStarted) {
            *self = Self::Started { begun_at: now };
        }
    }

    pub(in crate::app) fn has_begun(&self) -> bool {
        !matches!(self, Self::NotStarted)
    }

    pub(in crate::app) fn allow_close(&mut self) {
        *self = Self::CloseAllowed;
    }

    pub(in crate::app) fn close_allowed(&self) -> bool {
        matches!(self, Self::CloseAllowed)
    }

    pub(in crate::app) fn shows_pending_writes_panel(
        &self,
        now: Instant,
        registry_is_idle: bool,
    ) -> bool {
        !registry_is_idle
            && self
                .elapsed_since_begin(now)
                .is_some_and(|elapsed| elapsed >= PANEL_GRACE)
    }

    pub(in crate::app) fn close_may_proceed(&self, registry_is_idle: bool) -> bool {
        matches!(self, Self::Started { .. }) && registry_is_idle
    }

    pub(in crate::app) fn elapsed_since_begin(&self, now: Instant) -> Option<Duration> {
        match self {
            Self::NotStarted | Self::CloseAllowed => None,
            Self::Started { begun_at } => Some(now.saturating_duration_since(*begun_at)),
        }
    }
}

impl App {
    /// Answers the window's close button and drives the shutdown it starts,
    /// returning whether the normal UI is skipped this frame.
    pub(in crate::app) fn intercept_close_request(&mut self, ui: &mut egui::Ui) -> bool {
        let close_requested = ui.ctx().input(|i| i.viewport().close_requested());
        if close_requested && !self.shutdown.close_allowed() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.begin_shutdown();
        }
        if !self.shutdown.has_begun() {
            return false;
        }

        // Results still arrive while the writes finish: draining them is what
        // takes the registry to idle.
        self.apply_finished_background_work(ui);

        let registry_is_idle = self.pending_writes.is_idle();
        if self
            .shutdown
            .shows_pending_writes_panel(Instant::now(), registry_is_idle)
        {
            self.show_pending_writes_panel(ui);
        }
        if self.shutdown.close_may_proceed(registry_is_idle) {
            self.shutdown.allow_close();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(SHUTDOWN_REPAINT_INTERVAL);
        true
    }

    fn show_pending_writes_panel(&self, ui: &mut egui::Ui) {
        CentralPanel::default().show(ui, |ui| {
            ui.heading("Shutting down");
            for status in self.pending_writes.snapshot().running {
                ui.label(status.label);
            }
        });
    }

    pub(in crate::app) fn begin_shutdown(&mut self) {
        if self.shutdown.has_begun() {
            return;
        }
        self.pending_writes.begin_shutdown();

        let settings_write = self
            .pending_writes
            .begin_shutdown_write(SETTINGS_FLUSH_LABEL, WriteKind::Settings);
        self.flush_settings();
        drop(settings_write);

        self.shut_down_history_worker_off_the_gui_thread();
        self.shutdown.begin(Instant::now());
    }

    /// Hands the worker to a thread of its own, which joins its `history-db`
    /// thread while the GUI thread carries on painting. The app is left with
    /// a disabled worker, which has nothing to join.
    fn shut_down_history_worker_off_the_gui_thread(&mut self) {
        let history = mem::replace(&mut self.history, history_db::HistoryWorker::disabled());
        let history_write = self
            .pending_writes
            .begin_shutdown_write(HISTORY_SHUTDOWN_LABEL, WriteKind::RecordingDatabase);
        history.shutdown_on_a_thread_of_its_own(history_write);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started_at(begun_at: Instant) -> ShutdownState {
        let mut state = ShutdownState::default();
        state.begin(begun_at);
        state
    }

    #[test]
    fn nothing_is_drawn_before_shutdown_begins() {
        let state = ShutdownState::default();

        assert!(!state.has_begun());
        assert!(!state.shows_pending_writes_panel(Instant::now(), false));
        assert!(!state.close_may_proceed(true));
    }

    #[test]
    fn a_running_write_raises_the_panel_once_the_grace_elapses() {
        let begun_at = Instant::now();
        let state = started_at(begun_at);

        assert!(!state.shows_pending_writes_panel(begun_at + PANEL_GRACE / 2, false));
        assert!(state.shows_pending_writes_panel(begun_at + PANEL_GRACE, false));
    }

    #[test]
    fn an_idle_registry_closes_without_ever_raising_the_panel() {
        let begun_at = Instant::now();
        let state = started_at(begun_at);

        assert!(!state.shows_pending_writes_panel(begun_at + PANEL_GRACE * 2, true));
        assert!(state.close_may_proceed(true));
    }

    #[test]
    fn a_running_write_holds_the_close_back() {
        let state = started_at(Instant::now());

        assert!(!state.close_may_proceed(false));
    }

    #[test]
    fn the_close_the_app_sends_itself_is_not_intercepted_again() {
        let mut state = started_at(Instant::now());

        state.allow_close();

        assert!(state.close_allowed());
        assert!(
            !state.close_may_proceed(true),
            "the close command is sent once"
        );
    }

    #[test]
    fn beginning_shutdown_twice_keeps_the_first_start() {
        let begun_at = Instant::now();
        let mut state = started_at(begun_at);

        state.begin(begun_at + PANEL_GRACE);

        assert_eq!(
            state.elapsed_since_begin(begun_at + PANEL_GRACE),
            Some(PANEL_GRACE)
        );
    }
}
