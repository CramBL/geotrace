//! Unix termination signals, taken as a request to shut down.
//!
//! A dedicated thread waits for SIGINT, SIGTERM and SIGHUP, raises a flag and
//! wakes the GUI. The frame loop reads the flag in
//! `App::intercept_close_request` and starts the shutdown the window's close
//! button starts.
//!
//! Windows is not covered: a GUI-subsystem build receives
//! `WM_QUERYENDSESSION` and `WM_ENDSESSION` at logoff or shutdown rather than
//! console control events.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::Context;

pub(crate) static TERMINATION_SIGNAL_FLAG: TerminationSignalFlag = TerminationSignalFlag::new();

/// Woken once a signal arrives so the frame loop reads the flag without
/// waiting for input. Empty until the app is built: a signal raised before
/// that is left for the first frame to read.
static GUI_CONTEXT_TO_WAKE: OnceLock<Context> = OnceLock::new();

/// What the reader of a termination signal does about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminationSignalAction {
    KeepRunning,
    BeginShutdown,
    /// The process ends without waiting for the writes: the user signalled
    /// twice.
    QuitLeavingWritesUnfinished,
}

/// The two bits the signal handling keeps. Relaxed ordering is enough: they
/// publish no other data.
#[derive(Debug)]
pub(crate) struct TerminationSignalFlag {
    /// Set when a signal arrives, cleared by whoever reads it.
    raised: AtomicBool,
    /// The next signal quits at once: this is set by the first read of a
    /// raised flag. Shutdown started from the window's close button leaves it
    /// clear: a signal is then still the first one.
    already_read: AtomicBool,
}

impl TerminationSignalFlag {
    const fn new() -> Self {
        Self {
            raised: AtomicBool::new(false),
            already_read: AtomicBool::new(false),
        }
    }

    /// Raised by the Unix signal thread, and by tests on every platform.
    #[cfg(any(unix, test))]
    pub(crate) fn raise(&self) {
        self.raised.store(true, Ordering::Relaxed);
    }

    /// Clears the flag and reports what to do about it.
    pub(crate) fn take_action(&self) -> TerminationSignalAction {
        if !self.raised.swap(false, Ordering::Relaxed) {
            TerminationSignalAction::KeepRunning
        } else if self.already_read.swap(true, Ordering::Relaxed) {
            TerminationSignalAction::QuitLeavingWritesUnfinished
        } else {
            TerminationSignalAction::BeginShutdown
        }
    }
}

pub(crate) fn set_gui_context_to_wake(ctx: &Context) {
    // The test harnesses build several apps in one process: the first
    // context they set is as good as any, since no thread is watching for
    // signals.
    GUI_CONTEXT_TO_WAKE.set(ctx.clone()).ok();
}

#[cfg(unix)]
mod unix_installation {
    use std::io;
    use std::thread;

    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    use super::{GUI_CONTEXT_TO_WAKE, TERMINATION_SIGNAL_FLAG};

    const TERMINATION_SIGNALS: [i32; 3] = [SIGINT, SIGTERM, SIGHUP];

    /// Install this once the frame loop that reads the flag is about to run:
    /// from here on a termination signal only raises
    /// [`TERMINATION_SIGNAL_FLAG`].
    pub(crate) fn install_handler() {
        if let Err(error) = spawn_termination_signal_thread() {
            log::error!(
                "Could not watch for termination signals, so a signal ends GeoTrace without \
                 finishing background writes: {error:#}"
            );
        }
    }

    fn spawn_termination_signal_thread() -> io::Result<()> {
        let mut signals = Signals::new(TERMINATION_SIGNALS)?;
        thread::Builder::new()
            .name("termination-signals".to_owned())
            .spawn(move || {
                // This loop runs on an ordinary thread, not in the signal
                // handler, so calling into egui here is not restricted to
                // async-signal-safe work.
                for _signal in &mut signals {
                    TERMINATION_SIGNAL_FLAG.raise();
                    if let Some(ctx) = GUI_CONTEXT_TO_WAKE.get() {
                        ctx.request_repaint();
                    }
                }
            })?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::thread;
        use std::time::{Duration, Instant};

        use signal_hook::consts::SIGTERM;
        use signal_hook::low_level;

        use crate::termination_signal::{TERMINATION_SIGNAL_FLAG, TerminationSignalAction};

        const FLAG_POLL_INTERVAL: Duration = Duration::from_millis(10);
        const FLAG_DEADLINE: Duration = Duration::from_secs(5);

        /// Sending the process a real SIGTERM reaches the flag. A failed
        /// install leaves SIGTERM's default disposition, which kills this
        /// test process instead.
        ///
        /// No other test sees the process-global flag this raises: every test
        /// runs in its own process under `cargo nextest`.
        #[test]
        fn a_sigterm_begins_shutdown() {
            super::install_handler();

            low_level::raise(SIGTERM).expect("raise SIGTERM on this process");

            let deadline = Instant::now() + FLAG_DEADLINE;
            let mut action = TerminationSignalAction::KeepRunning;
            while action == TerminationSignalAction::KeepRunning && Instant::now() < deadline {
                thread::sleep(FLAG_POLL_INTERVAL);
                action = TERMINATION_SIGNAL_FLAG.take_action();
            }

            assert_eq!(action, TerminationSignalAction::BeginShutdown);
        }
    }
}

#[cfg(unix)]
pub(crate) use unix_installation::install_handler;

#[cfg(not(unix))]
pub(crate) fn install_handler() {}

#[cfg(test)]
mod tests {
    use super::{TerminationSignalAction, TerminationSignalFlag};

    fn raised_flag() -> TerminationSignalFlag {
        let flag = TerminationSignalFlag::new();
        flag.raise();
        flag
    }

    #[test]
    fn a_clear_flag_keeps_the_app_running() {
        assert_eq!(
            TerminationSignalFlag::new().take_action(),
            TerminationSignalAction::KeepRunning
        );
    }

    #[test]
    fn a_raised_flag_begins_shutdown() {
        assert_eq!(
            raised_flag().take_action(),
            TerminationSignalAction::BeginShutdown
        );
    }

    #[test]
    fn a_second_signal_quits_leaving_writes_unfinished() {
        let flag = raised_flag();
        flag.take_action();

        flag.raise();

        assert_eq!(
            flag.take_action(),
            TerminationSignalAction::QuitLeavingWritesUnfinished
        );
    }

    /// The shutdown one signal begins is not quit by that same signal on the
    /// next frame.
    #[test]
    fn reading_a_raised_flag_clears_it() {
        let flag = raised_flag();

        assert_eq!(flag.take_action(), TerminationSignalAction::BeginShutdown);

        assert_eq!(flag.take_action(), TerminationSignalAction::KeepRunning);
    }
}
