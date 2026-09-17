#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("GeoTrace SDK build error: {0}")]
    Build(#[from] geotrace_sdk::BuildError),

    #[error("invalid debug time repair configuration: {0}")]
    DebugTimeRepair(#[from] geotrace_sdk::DebugTimeRepairError),

    #[error("invalid event marker: {0}")]
    EventMarker(#[from] geotrace_sdk::EventMarkerError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("GeoTrace SDK error: {0}")]
    Sdk(#[from] geotrace_sdk::Error),
}
