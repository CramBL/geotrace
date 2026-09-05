use crate::fixed_width_string::{FixedWidthStringError, VariantPathField};

/// Validate that `path` is a well-formed event marker variant path.
///
/// Rules: non-empty, ASCII alphanumeric + `-` + `_` + `/`, no leading/trailing slash,
/// no empty segments (`//`), at most [`VariantPathField::CONTENT_CAPACITY`] bytes.
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
    if path.len() > VariantPathField::CONTENT_CAPACITY {
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

/// Error returned by `EventMarker::builder().build()` when the variant path is
/// malformed, or when the annotation does not fit the field that holds it.
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

    #[error(
        "invalid event marker variant path {path:?}: {len} bytes, past the {} bytes the field holds",
        VariantPathField::CONTENT_CAPACITY
    )]
    TooLong { path: String, len: usize },

    #[error(
        "invalid event marker variant path {path:?}: contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"
    )]
    InvalidChars { path: String },

    #[error("invalid event marker annotation: {source}")]
    UnwritableAnnotation { source: FixedWidthStringError },
}

/// Errors that can occur when building a [`Channel`](crate::Channel).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChannelError {
    #[error(
        "invalid channel name {name:?}: must be a lowercase identifier (a lowercase letter or underscore, then lowercase letters, digits, or underscores)"
    )]
    InvalidName { name: String },

    #[error("channel {name:?}: a vector channel needs at least one component")]
    EmptyComponents { name: String },

    #[error(
        "channel {name:?}: invalid component label {component:?}: must be a lowercase identifier"
    )]
    InvalidComponent { name: String, component: String },

    #[error("channel {name:?}: duplicate component label {component:?}")]
    DuplicateComponent { name: String, component: String },

    #[error("channel {name:?}: expected {expected} values but got {actual}")]
    LengthMismatch {
        name: String,
        expected: usize,
        actual: usize,
    },

    #[error("channel {name:?}: wrap period must be finite and positive")]
    InvalidPeriod { name: String },

    #[error("channel {name:?}: wrap period requires a recognized angular unit")]
    PeriodNeedsAngularUnit { name: String },

    #[error("channel {name:?}: legacy unit metadata {unit:?} is not valid writer input")]
    UnwritableUnit { name: String, unit: String },
}

/// A lowercase identifier: a lowercase letter or underscore, then lowercase
/// letters, digits, or underscores. Channel names and vector component labels
/// must both be identifiers, since queries reference them as `@name.component`.
fn is_identifier(s: &str) -> bool {
    let mut bytes = s.bytes();
    let valid_start = matches!(bytes.next(), Some(b) if b.is_ascii_lowercase() || b == b'_');
    valid_start && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Validate that `name` is a well-formed channel name: a lowercase identifier,
/// so it can be referenced as `@name` in the query language.
pub(crate) fn validate_channel_name(name: &str) -> Result<(), ChannelError> {
    if is_identifier(name) {
        Ok(())
    } else {
        Err(ChannelError::InvalidName {
            name: name.to_owned(),
        })
    }
}

/// Validate a vector channel's component labels: a non-empty list of unique
/// identifiers, since each is referenced as `@name.label`.
pub(crate) fn validate_components(name: &str, components: &[String]) -> Result<(), ChannelError> {
    if components.is_empty() {
        return Err(ChannelError::EmptyComponents {
            name: name.to_owned(),
        });
    }
    for (i, component) in components.iter().enumerate() {
        if !is_identifier(component) {
            return Err(ChannelError::InvalidComponent {
                name: name.to_owned(),
                component: component.clone(),
            });
        }
        if components
            .iter()
            .take(i)
            .any(|earlier| earlier == component)
        {
            return Err(ChannelError::DuplicateComponent {
                name: name.to_owned(),
                component: component.clone(),
            });
        }
    }
    Ok(())
}

/// Errors that can occur when building a [`NavFile`](crate::NavFile).
#[derive(Debug, Clone, thiserror::Error)]
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

    /// Two channels share a name. Names are the primary key (queries reference
    /// them as `@name`) and become HDF5 group names, so they must be unique.
    #[error("two channels share the name {name:?}; channel names must be unique")]
    DuplicateChannelName { name: String },
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

    #[error("{group}/{dataset}: {source}")]
    UnwritableField {
        group: &'static str,
        dataset: &'static str,
        source: FixedWidthStringError,
    },

    #[error("{group}/{dataset}: {source}")]
    UnreadableField {
        group: &'static str,
        dataset: &'static str,
        source: FixedWidthStringError,
    },

    #[error("nav point {record} has neither a receiver nor a host timestamp")]
    FixWithoutTimestamp { record: usize },

    #[error(
        "dataset {path:?} declares {declared_bytes} bytes of data, past what a {file_bytes}-byte file can hold"
    )]
    DatasetSizePastFileLength {
        path: String,
        declared_bytes: u128,
        file_bytes: u64,
    },
}

/// The group and dataset of one fixed-width string field, named by
/// [`Error::UnwritableField`] and [`Error::UnreadableField`].
#[derive(Clone, Copy)]
pub(crate) struct FieldLocation {
    pub(crate) group: &'static str,
    pub(crate) dataset: &'static str,
}

/// The `markers/label` field, named both by `Annotation::builder().build()` and
/// by the writer.
pub(crate) const MARKER_LABEL_LOCATION: FieldLocation = FieldLocation {
    group: "markers",
    dataset: "label",
};

impl Error {
    pub(crate) fn unwritable_field(location: FieldLocation, source: FixedWidthStringError) -> Self {
        Self::UnwritableField {
            group: location.group,
            dataset: location.dataset,
            source,
        }
    }
}

impl From<hdf5_pure::Error> for Error {
    fn from(e: hdf5_pure::Error) -> Self {
        Self::Hdf5(e.to_string())
    }
}
