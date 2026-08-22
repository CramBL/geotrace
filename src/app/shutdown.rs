//! Closing the window, or taking a termination signal, with background
//! writes still running.
//!
//! The close request is intercepted, the app keeps painting, and the window
//! closes once the pending-write registry is idle.

use std::time::{Duration, Instant};
use std::{mem, process};

use egui::{CentralPanel, Label, Panel, ProgressBar, RichText, ScrollArea, Sides};
use egui_phosphor::regular::CHECK as ICON_CHECK;
use gt_pending_writes::{PendingWriteStatus, PendingWrites, PendingWritesSnapshot, WriteKind};

use super::modals::{self, ForceQuitChoice};
use super::{App, history_db};
use crate::termination_signal::{TERMINATION_SIGNAL_FLAG, TerminationSignalAction};

/// Under this a close with nothing pending never leaves the normal UI.
pub(in crate::app) const SHUTDOWN_WINDOW_GRACE: Duration = Duration::from_millis(200);

/// Shutdown repaints on this interval so the writes it lists advance without
/// new input.
const SHUTDOWN_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

const SHUTDOWN_WINDOW_INNER_SIZE: egui::Vec2 = egui::vec2(420.0, 260.0);

/// Non-zero, so a shell or a supervisor sees that the writes still running
/// were abandoned.
pub(crate) const FORCE_QUIT_EXIT_CODE: u8 = 3;

/// Logged by both places a second signal can reach: the frame loop and the
/// wait that outlives the window.
pub(crate) const SECOND_SIGNAL_QUIT_CAUSE: &str = "Quitting on a second termination signal";

const SETTINGS_FLUSH_LABEL: &str = "Saving settings";
const HISTORY_SHUTDOWN_LABEL: &str = "Finishing recording history work";

/// Reports the writes the process is about to abandon, wherever it ends
/// before they finish.
pub(crate) fn log_writes_left_unfinished(cause: &str, pending_writes: &PendingWrites) {
    log::error!(
        "{cause}: {} background writes are left unfinished",
        pending_writes.snapshot().running.len()
    );
}

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
    force_quit_prompt: ForceQuitPrompt,
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

/// Ending the process with the registered writes unfinished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ForceQuit;

/// The confirmation the shutdown window's "Force quit…" button raises.
#[derive(Debug, Default)]
struct ForceQuitPrompt {
    open: bool,
}

impl ForceQuitPrompt {
    fn open(&mut self) {
        self.open = true;
    }

    /// What the open confirmation lists, or [`None`] while it is closed.
    ///
    /// The list is read from the registry every frame: a write that finished
    /// since the confirmation opened is no longer a consequence of quitting,
    /// and once none are left the confirmation closes - the shutdown window
    /// closes the app on its own from there.
    fn interruption_costs_to_list(
        &mut self,
        snapshot: &PendingWritesSnapshot,
    ) -> Option<Vec<String>> {
        if !self.open {
            return None;
        }
        let costs = snapshot.interruption_costs();
        if costs.is_empty() {
            self.open = false;
            return None;
        }
        Some(costs)
    }

    /// Closes the confirmation on the user's answer, reporting a confirmed
    /// quit.
    fn answer(&mut self, choice: ForceQuitChoice) -> Option<ForceQuit> {
        self.open = false;
        match choice {
            ForceQuitChoice::Quit => Some(ForceQuit),
            ForceQuitChoice::Cancel => None,
        }
    }
}

impl App {
    /// Answers the window's close button and any termination signal, drives
    /// the shutdown they start, and returns what the app paints this frame.
    pub(in crate::app) fn intercept_close_request(&mut self, ui: &mut egui::Ui) -> FrameContents {
        match TERMINATION_SIGNAL_FLAG.take_action() {
            TerminationSignalAction::KeepRunning => {}
            TerminationSignalAction::BeginShutdown => self.begin_shutdown(),
            TerminationSignalAction::QuitLeavingWritesUnfinished => {
                self.exit_leaving_writes_unfinished(SECOND_SIGNAL_QUIT_CAUSE)
            }
        }
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
            let snapshot = self.pending_writes.snapshot();
            self.show_shutdown_window(ui, now, &snapshot);
            if self.show_force_quit_prompt(ui, &snapshot).is_some() {
                self.exit_leaving_writes_unfinished("Force quit");
            }
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

    fn show_shutdown_window(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        snapshot: &PendingWritesSnapshot,
    ) {
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
                    if ui
                        .button("Force quit…")
                        .on_hover_text(
                            "Asks to end GeoTrace now, leaving the work above unfinished",
                        )
                        .clicked()
                    {
                        self.shutdown.force_quit_prompt.open();
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

    fn show_force_quit_prompt(
        &mut self,
        ui: &egui::Ui,
        snapshot: &PendingWritesSnapshot,
    ) -> Option<ForceQuit> {
        let costs = self
            .shutdown
            .force_quit_prompt
            .interruption_costs_to_list(snapshot)?;
        let choice = modals::show_force_quit_confirmation(ui, &costs)?;
        self.shutdown.force_quit_prompt.answer(choice)
    }

    /// Ends the process with the registered writes unfinished, as the
    /// force-quit button and a second termination signal both do.
    fn exit_leaving_writes_unfinished(&self, cause: &str) -> ! {
        log_writes_left_unfinished(cause, &self.pending_writes);
        process::exit(i32::from(FORCE_QUIT_EXIT_CODE))
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

    /// Hands the app's worker to a thread of its own, which joins its
    /// `history-db` thread while the GUI thread carries on painting. The app
    /// is left with a disabled worker, which has nothing to join.
    fn shut_down_history_worker_off_the_gui_thread(&mut self) {
        let history = mem::replace(&mut self.history, history_db::HistoryWorker::disabled());
        self.end_history_worker_off_the_gui_thread(history);
    }

    /// Closes `history`'s database on a thread of its own, which the close
    /// waits for. Also takes the worker a storage open lands after the app
    /// began closing, which nothing adopts.
    pub(in crate::app) fn end_history_worker_off_the_gui_thread(
        &self,
        history: history_db::HistoryWorker,
    ) {
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

    /// A write of each kind given, as the registry reports the running ones.
    fn snapshot_of(kinds: &[WriteKind]) -> PendingWritesSnapshot {
        PendingWritesSnapshot {
            running: kinds
                .iter()
                .map(|kind| PendingWriteStatus {
                    label: "A running write".to_owned(),
                    kind: *kind,
                    progress: None,
                    stage: None,
                })
                .collect(),
            recently_finished: Vec::new(),
        }
    }

    const TEC_COMPACTION: WriteKind = WriteKind::ArchiveCompaction {
        archive: "ionospheric TEC",
    };

    fn opened_force_quit_prompt() -> ForceQuitPrompt {
        let mut prompt = ForceQuitPrompt::default();
        prompt.open();
        prompt
    }

    #[test]
    fn a_closed_force_quit_prompt_lists_nothing() {
        let mut prompt = ForceQuitPrompt::default();

        assert_eq!(
            prompt.interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION])),
            None
        );
    }

    #[test]
    fn an_open_force_quit_prompt_lists_what_each_running_write_costs() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(
            prompt.interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION, WriteKind::Settings])),
            Some(vec![
                TEC_COMPACTION.interruption_cost(),
                WriteKind::Settings.interruption_cost(),
            ])
        );
    }

    /// A write that finishes while the prompt is open drops off its list: the
    /// list is read from the registry as it is now.
    #[test]
    fn a_write_that_finishes_while_the_prompt_is_open_drops_off_its_list() {
        let mut prompt = opened_force_quit_prompt();
        prompt.interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION, WriteKind::Settings]));

        assert_eq!(
            prompt.interruption_costs_to_list(&snapshot_of(&[WriteKind::Settings])),
            Some(vec![WriteKind::Settings.interruption_cost()])
        );
    }

    /// Nothing is left to interrupt once the last write finishes: the prompt
    /// closes, and the shutdown window closes the app on its own.
    #[test]
    fn the_last_write_finishing_closes_the_open_force_quit_prompt() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(prompt.interruption_costs_to_list(&snapshot_of(&[])), None);

        assert_eq!(
            prompt.interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION])),
            None,
            "the closed prompt came back up"
        );
    }

    #[test]
    fn cancelling_the_force_quit_prompt_closes_it_without_quitting() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(prompt.answer(ForceQuitChoice::Cancel), None);

        assert_eq!(
            prompt.interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION])),
            None,
            "the cancelled prompt stayed open"
        );
        prompt.open();
        assert!(
            prompt
                .interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION]))
                .is_some(),
            "the button no longer raises the prompt after a cancel"
        );
    }

    #[test]
    fn confirming_the_force_quit_prompt_yields_the_quit_action() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(prompt.answer(ForceQuitChoice::Quit), Some(ForceQuit));

        assert_eq!(
            prompt.interruption_costs_to_list(&snapshot_of(&[TEC_COMPACTION])),
            None,
            "the answered prompt stayed open"
        );
    }

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
