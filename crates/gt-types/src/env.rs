//! Process-environment switches shared across the workspace.

use std::env;

/// Name of the offline switch. Exposed for documentation and error text;
/// checks go through [`offline`].
pub const OFFLINE_ENV_VAR: &str = "GEOTRACE_OFFLINE";

/// Whether GeoTrace runs offline (`GEOTRACE_OFFLINE` set, any value): no
/// map tile fetching, no snap-to-road requests, no update checks. Set by
/// `just test` so tests never touch the network.
pub fn offline() -> bool {
    env::var_os(OFFLINE_ENV_VAR).is_some()
}
