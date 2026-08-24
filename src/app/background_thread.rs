//! Spawning the app's background threads.
//!
//! A failed spawn panics here: callers have no state for a thread that never
//! started, each recording its job as in flight before the spawn and clearing
//! it when the thread's message arrives.

use std::thread::{self, JoinHandle};

#[expect(
    clippy::panic,
    reason = "thread spawn can only fail under extreme system resource exhaustion"
)]
pub(in crate::app) fn spawn_or_panic<T: Send + 'static>(
    name: impl Into<String>,
    work: impl FnOnce() -> T + Send + 'static,
) -> JoinHandle<T> {
    let name = name.into();
    thread::Builder::new()
        .name(name.clone())
        .spawn(work)
        .unwrap_or_else(|error| panic!("failed to spawn the {name} thread: {error}"))
}

#[cfg(test)]
mod tests {
    use crate::app::background_thread;

    #[test]
    fn the_thread_runs_under_the_requested_name() {
        let name = background_thread::spawn_or_panic("archive-inspect", || {
            std::thread::current().name().map(str::to_owned)
        })
        .join()
        .expect("the thread finished");

        assert_eq!(name.as_deref(), Some("archive-inspect"));
    }
}
