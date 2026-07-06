/// Validate that `path` is a well-formed event marker variant path.
///
/// Rules: non-empty, ASCII alphanumeric + `-` + `_` + `/`, no leading/trailing slash,
/// no empty segments (`//`), at most 256 bytes.
pub(crate) fn validate_variant_path(path: &str) -> Result<(), EventMarkerError> {
    if path.is_empty() {
        return Err(EventMarkerError::Empty { path: path.into() });
    }
    if path.starts_with('/') {
        return Err(EventMarkerError::LeadingSlash { path: path.into() });
    }
    if path.ends_with('/') {
        return Err(EventMarkerError::TrailingSlash { path: path.into() });
    }
    if path.contains("//") {
        return Err(EventMarkerError::EmptySegment { path: path.into() });
    }
    if path.len() > 256 {
        return Err(EventMarkerError::TooLong {
            path: path.into(),
            len: path.len(),
        });
    }
    if !path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'/')
    {
        return Err(EventMarkerError::InvalidChars { path: path.into() });
    }
    Ok(())
}

/// Error returned by `EventMarker::builder().build()` when the supplied
/// variant path is malformed.
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

/// Errors that can occur when building a [`Channel`](crate::Channel).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChannelError {
    #[error(
        "invalid channel name {name:?}: must be a lowercase identifier (a letter or underscore, then letters, digits, or underscores)"
    )]
    InvalidName { name: String },

    #[error("channel {name:?} has {times} timestamps but {values} values")]
    LengthMismatch {
        name: String,
        times: usize,
        values: usize,
    },
}

/// Validate that `name` is a well-formed channel name: a lowercase identifier,
/// so it can be referenced as `@name` in the query language.
pub(crate) fn validate_channel_name(name: &str) -> Result<(), ChannelError> {
    let mut bytes = name.bytes();
    let valid_start = matches!(bytes.next(), Some(b) if b.is_ascii_lowercase() || b == b'_');
    let valid_rest = bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !valid_start || !valid_rest {
        return Err(ChannelError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Errors that can occur when building a [`NavFile`](crate::NavFile).
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum BuildError {
    /// The builder has no nav fixes. At least one is required to interpolate
    /// annotation positions. This is returned even in lenient mode.
    #[error("no nav fixes were added; at least one NavFix is required")]
    NoNavFixes,

    /// One or more annotations fall outside the time range of the nav track.
    ///
    /// Only emitted in strict mode (the default). Use
    /// [`NavFileBuilder::with_lenient_errors`](crate::NavFileBuilder::with_lenient_errors)
    /// to downgrade to a warning and continue.
    #[error("{count} annotation(s) fall outside the nav fix time range")]
    AnnotationsOutsideRange { count: usize },
}

/// Errors that can occur when reading or writing a `.gtd` file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HDF5 error: {0}")]
    Hdf5(String),

    #[error("unsupported GeoTrace file version: {version:?}")]
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

    #[error("unknown constellation name {name:?}")]
    UnknownConstellationName { name: String },

    #[error("unknown marker icon name {name:?}")]
    UnknownMarkerIcon { name: String },

    #[error("failed to parse {unit} from {input:?}: {reason}")]
    ParseError {
        unit: &'static str,
        input: String,
        reason: String,
    },
}

impl From<hdf5_pure::Error> for Error {
    fn from(e: hdf5_pure::Error) -> Self {
        Self::Hdf5(e.to_string())
    }
}
