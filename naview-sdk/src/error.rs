/// Errors that can occur when building a [`NavFile`](crate::NavFile).
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum BuildError {
    /// The builder has no nav fixes; at least one is required to interpolate
    /// annotation positions. This is returned even in lenient mode.
    #[error("no nav fixes were added; at least one NavFix is required")]
    NoNavFixes,

    /// One or more satellite reports could not be associated with any nav fix
    /// within the configured time window.
    #[error("{count} satellite report(s) could not be associated within the {window_ms}ms window")]
    UnassociatedSatelliteReports { count: usize, window_ms: i64 },

    /// One or more annotations fall outside the time range of the nav track.
    #[error("{count} annotation(s) fall outside the nav fix time range")]
    AnnotationsOutsideRange { count: usize },

    /// Both satellite report association failures and out-of-range annotations
    /// occurred in strict mode.
    #[error(
        "{unassociated_satellite_reports} satellite report(s) unassociated (window: {window_ms}ms); \
        {annotations_outside_range} annotation(s) outside nav fix range"
    )]
    Multiple {
        unassociated_satellite_reports: usize,
        annotations_outside_range: usize,
        window_ms: i64,
    },
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
