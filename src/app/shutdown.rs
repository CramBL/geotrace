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

use super::modals::{
    self, CountdownToTheClose, ForceQuitChoice, ForceQuitPromptContents, PointerOverTheDialog,
};
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

    /// The open force-quit prompt holds the close back: it reports the
    /// finished writes before the window closes.
    pub(in crate::app) fn close_may_proceed(&self, registry_is_idle: bool) -> bool {
        self.has_begun()
            && !self.close_allowed
            && registry_is_idle
            && !self.force_quit_prompt.is_up()
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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ForceQuitPrompt {
    #[default]
    Closed,
    ListingInterruptionCosts,
    /// Reporting that every write finished, counting down to its own close.
    /// The prompt never goes back to listing costs from here.
    ReportingTheFinishedWrites(CountdownToTheClose),
}

impl ForceQuitPrompt {
    fn open(&mut self) {
        *self = Self::ListingInterruptionCosts;
    }

    fn is_up(self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// What the open confirmation shows, or [`None`] while it is closed.
    ///
    /// The costs are read from the registry every frame: a write that finished
    /// since the confirmation opened is no longer a consequence of quitting.
    fn contents_to_show(
        &mut self,
        now: Instant,
        snapshot: &PendingWritesSnapshot,
    ) -> Option<ForceQuitPromptContents> {
        match *self {
            Self::Closed => None,
            Self::ListingInterruptionCosts => {
                let costs = snapshot.interruption_costs();
                if costs.is_empty() {
                    let countdown = CountdownToTheClose::started_at(now);
                    *self = Self::ReportingTheFinishedWrites(countdown);
                    return Some(ForceQuitPromptContents::WritesFinished(
                        countdown.time_until_the_close(),
                    ));
                }
                Some(ForceQuitPromptContents::InterruptionCosts(costs))
            }
            Self::ReportingTheFinishedWrites(countdown) => Some(
                ForceQuitPromptContents::WritesFinished(countdown.time_until_the_close()),
            ),
        }
    }

    /// Closes the confirmation on the user's choice, reporting a confirmed
    /// quit.
    fn record_choice(&mut self, choice: ForceQuitChoice) -> Option<ForceQuit> {
        *self = Self::Closed;
        match choice {
            ForceQuitChoice::Quit => Some(ForceQuit),
            ForceQuitChoice::Dismiss => None,
        }
    }

    /// The pointer resting over the confirmation holds the count: closing the
    /// confirmation under the pointer would send the press that follows to the
    /// shutdown window behind it.
    fn advance_the_countdown_and_close_when_it_runs_out(
        &mut self,
        now: Instant,
        pointer: PointerOverTheDialog,
    ) {
        let Self::ReportingTheFinishedWrites(mut countdown) = *self else {
            return;
        };
        countdown.advance_to(now, pointer);
        *self = if countdown.has_run_out() {
            Self::Closed
        } else {
            Self::ReportingTheFinishedWrites(countdown)
        };
    }
}

impl App {
    /// Intercepts the window's close button and any termination signal, drives
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
            self.instance_lock
                .report_shutdown_progress(&self.pending_writes);
            let snapshot = self.pending_writes.snapshot();
            self.show_shutdown_window(ui, now, &snapshot);
            if self.show_force_quit_prompt(ui, now, &snapshot).is_some() {
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
        now: Instant,
        snapshot: &PendingWritesSnapshot,
    ) -> Option<ForceQuit> {
        let contents = self
            .shutdown
            .force_quit_prompt
            .contents_to_show(now, snapshot)?;
        let response = modals::show_force_quit_confirmation(ui, &contents);
        let prompt = &mut self.shutdown.force_quit_prompt;
        match response.choice {
            Some(choice) => prompt.record_choice(choice),
            None => {
                prompt.advance_the_countdown_and_close_when_it_runs_out(now, response.pointer);
                None
            }
        }
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
        self.flush_settings_during_shutdown();
        self.shut_down_history_worker_off_the_gui_thread();
        self.shutdown.begin(Instant::now());
        self.instance_lock.mark_shutting_down(&self.pending_writes);
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
            .try_begin_shutdown_write(HISTORY_SHUTDOWN_LABEL, WriteKind::RecordingDatabase);
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
    use rstest::rstest;

    use super::*;
    use crate::app::modals::{COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES, TimeUntilTheClose};

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

    fn prompt_reporting_writes_finished_at(finished_at: Instant) -> ForceQuitPrompt {
        let mut prompt = opened_force_quit_prompt();
        prompt.contents_to_show(finished_at, &snapshot_of(&[]));
        prompt
    }

    fn writes_finished(time_until_the_close: Duration) -> Option<ForceQuitPromptContents> {
        Some(ForceQuitPromptContents::WritesFinished(TimeUntilTheClose(
            time_until_the_close,
        )))
    }

    fn costs(kinds: &[WriteKind]) -> Option<ForceQuitPromptContents> {
        Some(ForceQuitPromptContents::InterruptionCosts(
            kinds.iter().map(|kind| kind.interruption_cost()).collect(),
        ))
    }

    #[test]
    fn a_closed_force_quit_prompt_shows_nothing() {
        let mut prompt = ForceQuitPrompt::default();

        assert_eq!(
            prompt.contents_to_show(Instant::now(), &snapshot_of(&[TEC_COMPACTION])),
            None
        );
    }

    #[test]
    fn an_open_force_quit_prompt_lists_what_each_running_write_costs() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(
            prompt.contents_to_show(
                Instant::now(),
                &snapshot_of(&[TEC_COMPACTION, WriteKind::Settings])
            ),
            costs(&[TEC_COMPACTION, WriteKind::Settings])
        );
    }

    /// A write that finishes while the prompt is open drops off its list: the
    /// list is read from the registry as it is now.
    #[test]
    fn a_write_that_finishes_while_the_prompt_is_open_drops_off_its_list() {
        let now = Instant::now();
        let mut prompt = opened_force_quit_prompt();
        prompt.contents_to_show(now, &snapshot_of(&[TEC_COMPACTION, WriteKind::Settings]));

        assert_eq!(
            prompt.contents_to_show(now, &snapshot_of(&[WriteKind::Settings])),
            costs(&[WriteKind::Settings])
        );
    }

    #[test]
    fn the_last_write_finishing_leaves_the_prompt_up_reporting_the_finished_writes() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(
            prompt.contents_to_show(Instant::now(), &snapshot_of(&[])),
            writes_finished(COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES)
        );
    }

    #[test]
    fn a_prompt_reporting_the_finished_writes_never_lists_costs_again() {
        let now = Instant::now();
        let mut prompt = prompt_reporting_writes_finished_at(now);

        assert_eq!(
            prompt.contents_to_show(now, &snapshot_of(&[TEC_COMPACTION])),
            writes_finished(COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES)
        );
    }

    #[test]
    fn the_prompt_shows_the_time_it_has_left_before_it_closes() {
        let finished_at = Instant::now();
        let mut prompt = prompt_reporting_writes_finished_at(finished_at);
        let a_third_of_the_count = COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES / 3;

        prompt.advance_the_countdown_and_close_when_it_runs_out(
            finished_at + a_third_of_the_count,
            PointerOverTheDialog::Away,
        );

        assert_eq!(
            prompt.contents_to_show(finished_at + a_third_of_the_count, &snapshot_of(&[])),
            writes_finished(
                COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES.saturating_sub(a_third_of_the_count)
            )
        );
    }

    #[test]
    fn the_count_holds_while_the_pointer_rests_over_the_prompt_and_resumes_where_it_stopped() {
        let finished_at = Instant::now();
        let mut prompt = prompt_reporting_writes_finished_at(finished_at);
        let half_the_count = COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES / 2;
        let mut frame_at = finished_at + half_the_count;
        prompt
            .advance_the_countdown_and_close_when_it_runs_out(frame_at, PointerOverTheDialog::Away);

        frame_at += COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES * 10;
        prompt.advance_the_countdown_and_close_when_it_runs_out(
            frame_at,
            PointerOverTheDialog::Resting,
        );

        assert_eq!(
            prompt.contents_to_show(frame_at, &snapshot_of(&[])),
            writes_finished(half_the_count),
            "the count moved while the pointer rested over the prompt"
        );

        frame_at += half_the_count;
        prompt
            .advance_the_countdown_and_close_when_it_runs_out(frame_at, PointerOverTheDialog::Away);

        assert!(
            !prompt.is_up(),
            "the prompt stayed up past the count it resumed"
        );
    }

    #[rstest]
    #[case::the_count_ran_out_and_the_pointer_is_away(
        COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES,
        PointerOverTheDialog::Away,
        false
    )]
    #[case::the_pointer_rests_over_the_prompt(
        COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES * 10,
        PointerOverTheDialog::Resting,
        true
    )]
    #[case::the_count_has_not_run_out_yet(
        COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES / 2,
        PointerOverTheDialog::Away,
        true
    )]
    fn the_prompt_reporting_the_finished_writes_closes_itself(
        #[case] since_the_writes_finished: Duration,
        #[case] pointer: PointerOverTheDialog,
        #[case] expected_up: bool,
    ) {
        let finished_at = Instant::now();
        let mut prompt = prompt_reporting_writes_finished_at(finished_at);

        prompt.advance_the_countdown_and_close_when_it_runs_out(
            finished_at + since_the_writes_finished,
            pointer,
        );

        assert_eq!(prompt.is_up(), expected_up);
    }

    /// The window stays up while the prompt does: the close the app sends
    /// itself would take the prompt with it.
    #[test]
    fn an_open_force_quit_prompt_holds_the_close_back() {
        let mut state = started_at(Instant::now());
        state.force_quit_prompt.open();

        assert!(!state.close_may_proceed(true));

        state
            .force_quit_prompt
            .record_choice(ForceQuitChoice::Dismiss);

        assert!(state.close_may_proceed(true));
    }

    #[test]
    fn dismissing_the_force_quit_prompt_closes_it_without_quitting() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(prompt.record_choice(ForceQuitChoice::Dismiss), None);

        assert!(!prompt.is_up());
        prompt.open();
        assert!(prompt.is_up(), "the button no longer raises the prompt");
    }

    #[test]
    fn confirming_the_force_quit_prompt_yields_the_quit_action() {
        let mut prompt = opened_force_quit_prompt();

        assert_eq!(prompt.record_choice(ForceQuitChoice::Quit), Some(ForceQuit));

        assert!(!prompt.is_up());
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
