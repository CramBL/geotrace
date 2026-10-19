/// Version 3 adds navigation state, coordinate origins, counts and DOP columns.
pub(crate) const WRITTEN_FORMAT_VERSION: u32 = 3;

/// Version 1 has one time axis. Version 2 adds receiver and host timestamps.
pub(crate) const SUPPORTED_FORMAT_VERSIONS: [u32; 3] = [1, 2, WRITTEN_FORMAT_VERSION];
