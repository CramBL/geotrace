//! Process-environment switches shared across the workspace.

use std::env;

/// Name of the offline switch. Exposed for documentation and error text. Checks
/// go through [`offline`].
pub const OFFLINE_ENV_VAR: &str = "GEOTRACE_OFFLINE";

/// Why a request failed while offline, in logs and in the UI.
pub const OFFLINE_DETAIL: &str = "GeoTrace is running offline";

/// Whether GeoTrace runs offline (`GEOTRACE_OFFLINE` set, any value): no
/// map tile fetching, no snap-to-road requests, no update checks. Read by
/// `main` at startup and passed down.
pub fn offline() -> bool {
    env::var_os(OFFLINE_ENV_VAR).is_some()
}
