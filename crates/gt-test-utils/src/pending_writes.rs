//! Write registries in each of the states that refuse a write, for the tests
//! that check both refusals side by side.

use gt_pending_writes::PendingWrites;

/// A registry that has begun refusing writes because the process is on its
/// way out.
pub fn shutting_down_registry() -> PendingWrites {
    let pending_writes = PendingWrites::default();
    pending_writes.begin_shutdown();
    pending_writes
}
