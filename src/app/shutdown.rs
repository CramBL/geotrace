//! Closing the window with background writes still running.
//!
//! The close request is intercepted, the app keeps painting, and the window
//! closes once the pending-write registry is idle.

use std::mem;
use std::time::{Duration, Instant};

use egui::{CentralPanel, Label, Panel, ProgressBar, RichText, ScrollArea, Sides};
use egui_phosphor::regular::CHECK as ICON_CHECK;
use gt_pending_writes::{PendingWriteStatus, WriteKind};

use super::{App, history_db};

/// Under this a close with nothing pending never leaves the normal UI.
pub(in crate::app) const SHUTDOWN_WINDOW_GRACE: Duration = Duration::from_millis(200);

/// Shutdown repaints on this interval so the writes it lists advance without
/// new input.
const SHUTDOWN_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

const SHUTDOWN_WINDOW_INNER_SIZE: egui::Vec2 = egui::vec2(420.0, 260.0);

const SETTINGS_FLUSH_LABEL: &str = "Saving settings";
const HISTORY_SHUTDOWN_LABEL: &str = "Finishing recording history work";

/// What the app paints this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum FrameContents {
    NormalUi,
    ShutdownWindow,
}

/// How far the app has got in closing.
#[derive(Debug, Default)]
pub(in crate::app) struct ShutdownState {
    begun_at: Option<Instant>,
    /// Set on the frame the shutdown window first appears, and never unset:
    /// the window stays up until the app's window goes away.
    shutdown_window_is_up: bool,
    /// Set for the close request the app sends itself, which it must not
    /// intercept.
    close_allowed: bool,
}

impl ShutdownState {
    pub(in crate::app) fn begin(&mut self, now: Instant) {
        self.begun_at.get_or_insert(now);
    }

    pub(in crate::app) fn has_begun(&self) -> bool {
        self.begun_at.is_some()
    }

    pub(in crate::app) fn allow_close(&mut self) {
        self.close_allowed = true;
    }

    pub(in crate::app) fn close_allowed(&self) -> bool {
        self.close_allowed
    }

    pub(in crate::app) fn shutdown_window_is_up(&self) -> bool {
        self.shutdown_window_is_up
    }

    /// Raises the shutdown window once the grace elapses over a write that is
    /// still running, and reports what to paint from then on.
    pub(in crate::app) fn contents_to_paint(
        &mut self,
        now: Instant,
        registry_is_idle: bool,
    ) -> FrameContents {
        let grace_elapsed = self
            .elapsed_since_begin(now)
            .is_some_and(|elapsed| elapsed >= SHUTDOWN_WINDOW_GRACE);
        if grace_elapsed && !registry_is_idle {
            self.shutdown_window_is_up = true;
        }
        if self.shutdown_window_is_up {
            FrameContents::ShutdownWindow
        } else {
            FrameContents::NormalUi
        }
    }

    pub(in crate::app) fn close_may_proceed(&self, registry_is_idle: bool) -> bool {
        self.has_begun() && !self.close_allowed && registry_is_idle
    }

    pub(in crate::app) fn elapsed_since_begin(&self, now: Instant) -> Option<Duration> {
        self.begun_at
            .map(|begun_at| now.saturating_duration_since(begun_at))
    }
}

impl App {
    /// Answers the window's close button and drives the shutdown it starts,
    /// returning what the app paints this frame.
    pub(in crate::app) fn intercept_close_request(&mut self, ui: &mut egui::Ui) -> FrameContents {
        let close_requested = ui.ctx().input(|i| i.viewport().close_requested());
        if close_requested && !self.shutdown.close_allowed() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.begin_shutdown();
        }
        if !self.shutdown.has_begun() {
            return FrameContents::NormalUi;
        }

        let now = Instant::now();
        let shutdown_window_was_up = self.shutdown.shutdown_window_is_up();
        let contents = self
            .shutdown
            .contents_to_paint(now, self.pending_writes.is_idle());
        if contents == FrameContents::ShutdownWindow {
            if !shutdown_window_was_up {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    SHUTDOWN_WINDOW_INNER_SIZE,
                ));
            }
            // Results still arrive while the writes finish: draining them is
            // what takes the registry to idle.
            self.apply_finished_background_work(ui);
            self.show_shutdown_window(ui, now);
        }
        if self
            .shutdown
            .close_may_proceed(self.pending_writes.is_idle())
        {
            self.shutdown.allow_close();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(SHUTDOWN_REPAINT_INTERVAL);
        contents
    }

    fn show_shutdown_window(&mut self, ui: &mut egui::Ui, now: Instant) {
        let snapshot = self.pending_writes.snapshot();
        let elapsed = self.shutdown.elapsed_since_begin(now).unwrap_or_default();

        Panel::bottom("shutdown_actions").show(ui, |ui| {
            ui.add_space(4.0);
            Sides::new().show(
                ui,
                |ui| {
                    ui.label(
                        RichText::new(format!("Elapsed {:.1}s", elapsed.as_secs_f32())).weak(),
                    );
                },
                |ui| {
                    if ui
                        .button("Run in background")
                        .on_hover_text(
                            "Closes the window: GeoTrace keeps finishing the work above and exits when it is done",
                        )
                        .clicked()
                    {
                        self.shutdown.allow_close();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                },
            );
            ui.add_space(4.0);
        });

        CentralPanel::default().show(ui, |ui| {
            ui.heading("Shutting down");
            ui.add_space(4.0);
            ScrollArea::vertical().show(ui, |ui| {
                for status in &snapshot.running {
                    running_write_ui(ui, status);
                }
                for label in &snapshot.recently_finished {
                    ui.add(
                        Label::new(RichText::new(format!("{ICON_CHECK} {label}")).weak())
                            .truncate(),
                    );
                }
            });
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

fn running_write_ui(ui: &mut egui::Ui, status: &PendingWriteStatus) {
    Sides::new().shrink_left().truncate().show(
        ui,
        |ui| {
            if status.progress.is_none() {
                ui.spinner();
            }
            ui.add(Label::new(RichText::new(&status.label).strong()).truncate());
        },
        |ui| {
            if let Some(stage) = &status.stage {
                ui.label(RichText::new(stage).small().weak());
            }
        },
    );
    if let Some(progress) = status.progress {
        ui.add(ProgressBar::new(progress).animate(true));
    }
    ui.add_space(2.0);
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
        let mut state = ShutdownState::default();

        assert!(!state.has_begun());
        assert_eq!(
            state.contents_to_paint(Instant::now(), false),
            FrameContents::NormalUi
        );
        assert!(!state.close_may_proceed(true));
    }

    #[test]
    fn the_normal_ui_paints_through_the_grace_and_a_running_write_then_raises_the_window() {
        let begun_at = Instant::now();
        let mut state = started_at(begun_at);

        assert_eq!(
            state.contents_to_paint(begun_at + SHUTDOWN_WINDOW_GRACE / 2, false),
            FrameContents::NormalUi
        );
        assert_eq!(
            state.contents_to_paint(begun_at + SHUTDOWN_WINDOW_GRACE, false),
            FrameContents::ShutdownWindow
        );
    }

    #[test]
    fn an_idle_registry_closes_without_ever_raising_the_window() {
        let begun_at = Instant::now();
        let mut state = started_at(begun_at);

        assert_eq!(
            state.contents_to_paint(begun_at + SHUTDOWN_WINDOW_GRACE * 2, true),
            FrameContents::NormalUi
        );
        assert!(state.close_may_proceed(true));
    }

    /// Nothing paints blank: the window the writes raised keeps painting
    /// while the close the app sent itself is in flight.
    #[test]
    fn the_window_stays_up_once_the_writes_finish() {
        let begun_at = Instant::now();
        let mut state = started_at(begun_at);
        state.contents_to_paint(begun_at + SHUTDOWN_WINDOW_GRACE, false);

        state.allow_close();

        assert_eq!(
            state.contents_to_paint(begun_at + SHUTDOWN_WINDOW_GRACE * 2, true),
            FrameContents::ShutdownWindow
        );
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

        state.begin(begun_at + SHUTDOWN_WINDOW_GRACE);

        assert_eq!(
            state.elapsed_since_begin(begun_at + SHUTDOWN_WINDOW_GRACE),
            Some(SHUTDOWN_WINDOW_GRACE)
        );
    }
}
