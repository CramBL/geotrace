//! Write registries in each of the states that reject a write, for the tests
//! that check both refusals side by side.

use gt_pending_writes::PendingWrites;

/// A registry that rejects every write that has not started, because the
/// process is on its way out.
pub fn shutting_down_registry() -> PendingWrites {
    let pending_writes = PendingWrites::default();
    pending_writes.begin_shutdown();
    pending_writes
}
