use std::io;
use std::thread;

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use super::{GUI_CONTEXT_TO_WAKE, TERMINATION_SIGNAL_FLAG};

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

const TERMINATION_SIGNALS: [i32; 3] = [SIGINT, SIGTERM, SIGHUP];

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use signal_hook::consts::SIGTERM;
    use signal_hook::low_level;

    use crate::termination_signal::{TERMINATION_SIGNAL_FLAG, TerminationSignalAction};

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

    const FLAG_POLL_INTERVAL: Duration = Duration::from_millis(10);
    const FLAG_DEADLINE: Duration = Duration::from_secs(5);
}
