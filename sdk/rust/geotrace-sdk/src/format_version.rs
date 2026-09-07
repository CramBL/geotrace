/// The `geotrace_version` attribute value the writer stamps, written as a bare
/// integer. Version 2 stores `gps_time_us` and `sys_time_us` per nav point and
/// per satellite report.
pub(crate) const WRITTEN_FORMAT_VERSION: u32 = 2;

/// The `geotrace_version` attribute values the reader accepts. Version 1 stores
/// a single `time` axis, which the reader takes as the receiver's timestamp.
pub(crate) const SUPPORTED_FORMAT_VERSIONS: [u32; 2] = [1, WRITTEN_FORMAT_VERSION];
