/// Error returned by [`NavFileBuilder::add_event_marker`](crate::NavFileBuilder::add_event_marker)
/// when the supplied variant path is malformed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EventMarkerError {
    #[error("invalid event marker variant path {path:?}: path is empty")]
    Empty { path: String },

    #[error("invalid event marker variant path {path:?}: starts with '/'")]
    LeadingSlash { path: String },

    #[error("invalid event marker variant path {path:?}: ends with '/'")]
    TrailingSlash { path: String },

    #[error("invalid event marker variant path {path:?}: contains '//'")]
    EmptySegment { path: String },

    #[error("invalid event marker variant path {path:?}: exceeds 256 bytes ({len} bytes)")]
    TooLong { path: String, len: usize },

    #[error(
        "invalid event marker variant path {path:?}: contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"
    )]
    InvalidChars { path: String },
}

/// Errors that can occur when building a [`NavFile`](crate::NavFile).
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum BuildError {
    /// The builder has no nav fixes; at least one is required to interpolate
    /// annotation positions. This is returned even in lenient mode.
    #[error("no nav fixes were added; at least one NavFix is required")]
    NoNavFixes,

    /// One or more annotations fall outside the time range of the nav track.
    ///
    /// Only emitted in strict mode (the default). Use
    /// [`NavFileBuilder::with_continue_on_error`](crate::NavFileBuilder::with_continue_on_error)
    /// to downgrade to a warning and continue.
    #[error("{count} annotation(s) fall outside the nav fix time range")]
    AnnotationsOutsideRange { count: usize },
}

/// Errors that can occur when reading or writing a `.nvd` file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HDF5 error: {0}")]
    Hdf5(String),

    #[error("unsupported naview file version: {version:?}")]
    UnsupportedVersion { version: String },

    #[error("unknown constellation code {code} in dataset {dataset:?}")]
    UnknownConstellation { code: i16, dataset: &'static str },

    #[error("dataset {dataset:?} in group {group:?}: expected {expected} rows but found {actual}")]
    ShapeMismatch {
        group: &'static str,
        dataset: &'static str,
        expected: usize,
        actual: usize,
    },
}

impl From<hdf5_pure::Error> for Error {
    fn from(e: hdf5_pure::Error) -> Self {
        Self::Hdf5(e.to_string())
    }
}
