//! Unix termination signals, taken as a request to shut down.
//!
//! A dedicated thread waits for SIGINT, SIGTERM and SIGHUP, raises a flag and
//! wakes the GUI. The frame loop reads the flag in
//! `App::intercept_close_request` and starts the shutdown the window's close
//! button starts.
//!
//! Windows is not covered: a GUI-subsystem build receives
//! `WM_QUERYENDSESSION` and `WM_ENDSESSION` at logoff or shutdown.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::Context;

#[cfg(unix)]
pub(crate) use unix_installation::install_handler;

#[cfg(unix)]
mod unix_installation;

/// What the reader of a termination signal does about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminationSignalAction {
    BeginShutdown,
    KeepRunning,
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

#[cfg(not(unix))]
pub(crate) fn install_handler() {}

pub(crate) static TERMINATION_SIGNAL_FLAG: TerminationSignalFlag = TerminationSignalFlag::new();

/// Woken once a signal arrives so the frame loop reads the flag without
/// waiting for input. Empty until the app is built: a signal raised before
/// that is left for the first frame to read.
static GUI_CONTEXT_TO_WAKE: OnceLock<Context> = OnceLock::new();

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
