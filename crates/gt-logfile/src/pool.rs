//! The worker pool this crate indexes and associates logs on.

use std::{num::NonZeroUsize, sync::OnceLock, thread};

use rayon::{ThreadPool, ThreadPoolBuilder};

/// The share of the machine's cores the pool may occupy. Reading a log is a
/// background job behind a desktop the user is still working in, and a log is
/// large enough to saturate every core for long enough to be felt: measured on
/// an 80 MiB journal, half the cores index it as fast as all of them.
const CORES_PER_WORKER: usize = 2;

/// The pool the chunked passes over a log run on, or `None` when it could not
/// be built and the caller must do the work on the calling thread.
///
/// A pool dedicated to this crate. gt-plot renders frames on rayon's global
/// pool, and a log pass sharing it would stall frame rendering.
pub(crate) fn log_worker_pool() -> Option<&'static ThreadPool> {
    static LOG_WORKER_POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();

    LOG_WORKER_POOL
        .get_or_init(|| {
            let cores = thread::available_parallelism().map_or(1, NonZeroUsize::get);
            let workers = (cores / CORES_PER_WORKER).max(1);
            match ThreadPoolBuilder::new()
                .num_threads(workers)
                .thread_name(|index| format!("gt-logfile-{index}"))
                .build()
            {
                Ok(pool) => Some(pool),
                Err(err) => {
                    log::warn!("Reading logs on the calling thread only: {err:#}");
                    None
                }
            }
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_leaves_at_least_half_the_cores_to_the_rest_of_the_machine() {
        let cores = thread::available_parallelism().map_or(1, NonZeroUsize::get);
        let pool = log_worker_pool().expect("the pool builds");
        assert_eq!(
            pool.current_num_threads(),
            (cores / CORES_PER_WORKER).max(1),
            "on {cores} cores"
        );
    }
}
