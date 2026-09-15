use std::fmt;
use std::fmt::{Display, Formatter};

use crate::fixed_width_string::{self, FixedWidthStringError, VariantPathField};

/// Validate that `path` is a well-formed event marker variant path.
///
/// Rules: non-empty, ASCII alphanumeric + `-` + `_` + `/`, no leading/trailing slash,
/// no empty segments (`//`), at most [`VariantPathField::CONTENT_CAPACITY`] bytes.
pub(crate) fn validate_variant_path(path: &str) -> Result<(), VariantPathError> {
    if path.is_empty() {
        return Err(VariantPathError::Empty { path: path.into() });
    }
    if path.starts_with('/') {
        return Err(VariantPathError::LeadingSlash { path: path.into() });
    }
    if path.ends_with('/') {
        return Err(VariantPathError::TrailingSlash { path: path.into() });
    }
    if path.contains("//") {
        return Err(VariantPathError::EmptySegment { path: path.into() });
    }
    if path.len() > VariantPathField::CONTENT_CAPACITY {
        return Err(VariantPathError::TooLong {
            path: path.into(),
            len: path.len(),
        });
    }
    if !path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'/')
    {
        return Err(VariantPathError::InvalidChars { path: path.into() });
    }
    Ok(())
}

/// A malformed event marker variant path, rejected by
/// [`EventMarker::builder`](crate::EventMarker::builder) and
/// [`EventMarkerStyle::builder`](crate::EventMarkerStyle::builder).
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum VariantPathError {
    #[error("invalid event marker variant path {path:?}: path is empty")]
    Empty { path: String },

    #[error("invalid event marker variant path {path:?}: contains '//'")]
    EmptySegment { path: String },

    #[error(
        "invalid event marker variant path {path:?}: contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"
    )]
    InvalidChars { path: String },

    #[error("invalid event marker variant path {path:?}: starts with '/'")]
    LeadingSlash { path: String },

    #[error(
        "invalid event marker variant path {path:?}: {len} bytes, past the {} bytes the field holds",
        VariantPathField::CONTENT_CAPACITY
    )]
    TooLong { path: String, len: usize },

    #[error("invalid event marker variant path {path:?}: ends with '/'")]
    TrailingSlash { path: String },
}

/// Error returned by `EventMarker::builder().build()` when the variant path is
/// malformed, or when the annotation does not fit the field that holds it.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum EventMarkerError {
    #[error(transparent)]
    InvalidVariantPath {
        #[from]
        source: VariantPathError,
    },

    #[error("invalid event marker annotation: {source}")]
    UnwritableAnnotation { source: FixedWidthStringError },
}

/// Error returned by `EventMarkerStyle::builder().build()`.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum EventMarkerStyleError {
    #[error("invalid event marker color {color:?}: expected the #RRGGBB form")]
    InvalidColor { color: String },

    #[error(transparent)]
    InvalidVariantPath {
        #[from]
        source: VariantPathError,
    },

    #[error("invalid event marker icon name: {source}")]
    UnwritableIconName { source: FixedWidthStringError },
}

/// Errors that can occur when building a [`Channel`](crate::Channel).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChannelError {
    #[error("channel {name:?}: the description has a nul byte at offset {offset}")]
    DescriptionWithNul { name: String, offset: usize },

    #[error("channel {name:?}: duplicate component label {component:?}")]
    DuplicateComponent { name: String, component: String },

    #[error("channel {name:?}: a vector channel needs at least one component")]
    EmptyComponents { name: String },

    #[error(
        "channel {name:?}: invalid component label {component:?}: must be a lowercase identifier"
    )]
    InvalidComponent { name: String, component: String },

    #[error(
        "invalid channel name {name:?}: must be a lowercase identifier (a lowercase letter or underscore, then lowercase letters, digits, or underscores)"
    )]
    InvalidName { name: String },

    #[error("channel {name:?}: wrap period must be finite and positive")]
    InvalidPeriod { name: String },

    #[error("channel {name:?}: expected {expected} values but got {actual}")]
    LengthMismatch {
        name: String,
        expected: usize,
        actual: usize,
    },

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

/// A string field of [`Meta`](crate::Meta).
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "lowercase")]
#[non_exhaustive]
pub enum MetaField {
    Device,
    Identity,
    Notes,
    Title,
    #[strum(to_string = "travel mode")]
    TravelMode,
}

impl MetaField {
    pub(crate) fn reject_nul_byte(self, value: &str) -> Result<(), MetaStringWithNul> {
        match fixed_width_string::first_nul_byte_offset(value) {
            Some(offset) => Err(MetaStringWithNul {
                field: self,
                offset,
            }),
            None => Ok(()),
        }
    }
}

/// Error returned by [`Meta::builder`](crate::Meta::builder) and the metadata setters of
/// [`NavFileBuilder`](crate::NavFileBuilder) for a value with a nul byte. A C reader ends the
/// value at its first nul byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the {field} value has a nul byte at offset {offset}")]
#[non_exhaustive]
pub struct MetaStringWithNul {
    /// The field of the rejected value.
    pub field: MetaField,
    /// The byte offset of the first nul byte in the value.
    pub offset: usize,
}

/// Errors that can occur when building a [`NavFile`](crate::NavFile).
#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildError {
    /// One or more annotations fall outside the time range of the nav track.
    ///
    /// Only emitted in strict mode (the default). Use
    /// [`NavFileBuilder::with_lenient_errors`](crate::NavFileBuilder::with_lenient_errors)
    /// to clamp each to the nearest endpoint and continue.
    #[error("{count} annotation(s) fall outside the nav fix time range")]
    AnnotationsOutsideRange { count: usize },

    /// Two channels share a name. Names are the primary key (queries reference
    /// them as `@name`) and become HDF5 group names, so they must be unique.
    #[error("two channels share the name {name:?}; channel names must be unique")]
    DuplicateChannelName { name: String },

    /// One or more event markers fall outside the time range of the nav track.
    ///
    /// Only emitted in strict mode (the default). Use
    /// [`NavFileBuilder::with_lenient_errors`](crate::NavFileBuilder::with_lenient_errors)
    /// to clamp each to the nearest endpoint and continue.
    #[error("{count} event marker(s) fall outside the nav fix time range")]
    EventMarkersOutsideRange { count: usize },

    /// The builder computes a ghost nav fix's timestamp from a satellite
    /// report's own timestamp and the clock offset of the nav fixes around it.
    #[error("a ghost nav fix at {micros} microseconds is past the range a UTC timestamp covers")]
    GhostFixTimeOutOfRange { micros: i64 },

    /// An event from [`NavRecorder::add_event`](crate::NavRecorder::add_event) or
    /// [`NavRecorder::add_event_with_note`](crate::NavRecorder::add_event_with_note) has a
    /// variant path that [`EventMarker::builder`](crate::EventMarker::builder) rejects. `source`
    /// states the first such path and the rule it breaks.
    ///
    /// Only emitted in strict mode (the default). Use
    /// [`NavFileBuilder::with_lenient_errors`](crate::NavFileBuilder::with_lenient_errors)
    /// to drop each such event, log an error and continue.
    #[error(transparent)]
    InvalidEventMarkerVariantPath { source: VariantPathError },

    /// The builder has a satellite report, an annotation or an event marker and no nav fix
    /// to take its position from. This is returned even in lenient mode.
    #[error("{0} have no nav fix to take a position from: at least one nav fix is required")]
    NoNavFixes(UnplacedRecordCounts),
}

/// The satellite reports, annotations and event markers of a build without a nav fix, counted by
/// kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct UnplacedRecordCounts {
    /// The number of satellite reports.
    pub satellite_reports: usize,
    /// The number of annotations.
    pub annotations: usize,
    /// The number of event markers.
    pub event_markers: usize,
}

/// Lists each kind with a non-zero count, as in `2 satellite report(s) and 1 annotation(s)`.
impl Display for UnplacedRecordCounts {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self {
            satellite_reports,
            annotations,
            event_markers,
        } = *self;
        let counted_kinds: Vec<String> = [
            (satellite_reports, "satellite report(s)"),
            (annotations, "annotation(s)"),
            (event_markers, "event marker(s)"),
        ]
        .into_iter()
        .filter(|&(count, _)| count > 0)
        .map(|(count, kind)| format!("{count} {kind}"))
        .collect();
        match counted_kinds.split_last() {
            Some((last, [])) => f.write_str(last),
            Some((last, leading)) => write!(f, "{} and {last}", leading.join(", ")),
            None => Ok(()),
        }
    }
}

/// Errors that can occur when reading or writing a `.gtd` file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "dataset {path:?} declares {declared_bytes} bytes of data, past what a {file_bytes}-byte file can hold"
    )]
    DatasetSizePastFileLength {
        path: String,
        declared_bytes: u128,
        file_bytes: u64,
    },

    #[error("{group}/{dataset}: record {record} is empty")]
    EmptyField {
        group: &'static str,
        dataset: &'static str,
        record: usize,
    },

    #[error("event marker {record} has no timestamp")]
    EventMarkerWithoutTimestamp { record: usize },

    #[error("nav point {record} has neither a receiver nor a host timestamp")]
    FixWithoutTimestamp { record: usize },

    #[error("HDF5 error: {0}")]
    Hdf5(String),

    #[error(
        "{group}/{dataset}: record {record} holds index {index}, past the row count of {table} ({table_len})"
    )]
    IndexPastTable {
        group: &'static str,
        dataset: &'static str,
        record: usize,
        index: u64,
        table: &'static str,
        table_len: usize,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse {unit} from {input:?}: {reason}")]
    ParseError {
        unit: &'static str,
        input: String,
        reason: String,
    },

    #[error("satellite report {report} has neither a receiver nor a host timestamp")]
    ReportWithoutTimestamp { report: usize },

    #[error("dataset {dataset:?} in group {group:?}: expected {expected} rows but found {actual}")]
    ShapeMismatch {
        group: &'static str,
        dataset: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("{count} {unit} since the Unix epoch is past the range a UTC timestamp covers")]
    TimestampCountOutOfRange { count: i64, unit: &'static str },

    #[error(
        "{group}/{dataset}: record {record} has the microsecond count reserved for an absent timestamp"
    )]
    TimestampIsTheAbsentValue {
        group: &'static str,
        dataset: &'static str,
        record: usize,
    },

    #[error(
        "{group}/{dataset}: record {record} holds {micros} microseconds, past the range a UTC timestamp covers"
    )]
    TimestampOutOfRange {
        group: &'static str,
        dataset: &'static str,
        record: usize,
        micros: i64,
    },

    #[error("unknown constellation code {code} in dataset {dataset:?}")]
    UnknownConstellation { code: i16, dataset: &'static str },

    #[error("unknown constellation name {name:?}")]
    UnknownConstellationName { name: String },

    #[error("unknown marker icon name {name:?}")]
    UnknownMarkerIcon { name: String },

    #[error("{group}/{dataset}: {source}")]
    UnreadableField {
        group: &'static str,
        dataset: &'static str,
        source: FixedWidthStringError,
    },

    #[error("unsupported GeoTrace file version: {version:?}")]
    UnsupportedVersion { version: String },

    #[error("{group}/{dataset}: {source}")]
    UnwritableField {
        group: &'static str,
        dataset: &'static str,
        source: FixedWidthStringError,
    },
}

/// The group and dataset of one field, stated by the [`Error`] variants that
/// report where a value could not be written or read.
#[derive(Clone, Copy)]
pub(crate) struct FieldLocation {
    pub(crate) group: &'static str,
    pub(crate) dataset: &'static str,
}

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

/// The `markers/label` field, named both by `Annotation::builder().build()` and
/// by the writer.
pub(crate) const MARKER_LABEL_LOCATION: FieldLocation = FieldLocation {
    group: "markers",
    dataset: "label",
};
