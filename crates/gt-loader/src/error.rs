#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("GeoTrace SDK error: {0}")]
    Sdk(#[from] geotrace_sdk::Error),

    #[error("GeoTrace SDK build error: {0}")]
    Build(#[from] geotrace_sdk::BuildError),

    #[error("invalid event marker: {0}")]
    EventMarker(#[from] geotrace_sdk::EventMarkerError),

    #[error("latitude {lat} at index {idx} is out of range [-90, 90]")]
    LatitudeOutOfRange { lat: f64, idx: usize },

    #[error("longitude {lon} at index {idx} is out of range [-180, 180]")]
    LongitudeOutOfRange { lon: f64, idx: usize },
}
