//! Python bindings for `geotrace-sdk`.
//!
//! Exposes the core read/write API as a Python extension module named
//! `geotrace_sdk._geotrace_sdk`.  The public `geotrace_sdk` package re-exports
//! everything from this module via `python/geotrace_sdk/__init__.py`.

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, Utc};
use geotrace_sdk::{
    AnnotationIcon, BuildError, EventMarkerIconChoice, Meta, MetaStringWithNul, NavFile,
    NavFileBuilder, NavFixTime, NavPoint, NavRecorder, RecordedFixTimestamps, TravelMode,
};
use pyo3::IntoPyObjectExt as _;
use pyo3::exceptions::{PyIOError, PyIndexError, PyRuntimeError, PyUserWarning, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PySlice};
use pyo3_log::{Caching, Logger};

use crate::enums::TravelModeArg;
use crate::markers::SkippableEventMarker;
use crate::navigation::{
    PyPositionProvenance, PySatelliteIdentity, PySatelliteObservation,
    PySatelliteObservationReport, PySatelliteScope, PySatelliteUsage, PySatelliteUsageReport,
    PySolutionDimension, PySolutionMethod, PySolutionState,
};

pub use channel::{PyChannel, PyChannelUnit, PyUnit};
pub use enums::{PyConstellation, PyMarkerIcon, PyTravelMode};
pub use markers::{PyAnnotation, PyEventMarker, PyEventMarkerPoint, PyEventMarkerStyle, PyMarker};
pub use nav_fix::PyNavFix;
pub use satellite::{PySatellite, PySatelliteReport};

mod channel;
mod enums;
mod markers;
mod nav_fix;
mod navigation;
mod python_enum;
mod satellite;

/// Declares a sequence view over one of a [`NavFile`]'s collections together
/// with its iterator, both holding the file through an [`Arc`], and the class
/// documentation the Python side reads.
///
/// `methods` declares the columnar accessors of a collection that has them.
/// This macro emits them inside the generated `#[pymethods]` block, since PyO3
/// accepts one such block per class.
macro_rules! nav_file_sequence {
    (
        summary: $summary:literal,
        sequence: $sequence:ident as $sequence_name:literal,
        iterator: $iterator:ident as $iterator_name:literal,
        element: $element:ty as $element_name:literal,
        collection: $collection:ident,
        $(methods: { $($method:item)* },)?
    ) => {
        #[doc = $summary]
        #[doc = ""]
        #[doc = concat!(
            "Supports ``len()``, indexing, slicing and iteration, and builds one :class:`",
            $element_name,
            "` per element read. The sequence keeps working after the :class:`NavFile` object ",
            "is gone."
        )]
        #[pyclass(sequence, skip_from_py_object, name = $sequence_name)]
        pub struct $sequence {
            file: Arc<NavFile>,
        }

        #[pymethods]
        impl $sequence {
            fn __len__(&self) -> usize {
                self.file.$collection().len()
            }

            fn __getitem__<'py>(
                &self,
                py: Python<'py>,
                key: &Bound<'py, PyAny>,
            ) -> PyResult<Bound<'py, PyAny>> {
                sequence_item_or_slice::<$element, _>(py, key, self.file.$collection())
            }

            fn __iter__(&self) -> $iterator {
                $iterator {
                    file: Arc::clone(&self.file),
                    position: 0,
                }
            }

            fn __repr__(&self) -> String {
                format!(
                    concat!($sequence_name, "(len={})"),
                    self.file.$collection().len()
                )
            }

            $($($method)*)?
        }

        #[pyclass(skip_from_py_object, name = $iterator_name)]
        pub struct $iterator {
            file: Arc<NavFile>,
            position: usize,
        }

        #[pymethods]
        impl $iterator {
            fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                slf
            }

            fn __next__(&mut self) -> Option<$element> {
                let element = self
                    .file
                    .$collection()
                    .get(self.position)
                    .map(<$element>::from)?;
                self.position += 1;
                Some(element)
            }
        }
    };
}

fn to_fixed(dt: DateTime<Utc>) -> DateTime<FixedOffset> {
    dt.fixed_offset()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "called as `map_err(build_err)`, which passes the error by value"
)]
fn build_err(e: BuildError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

// Filesystem failures become `OSError` (`IOError`). Bad file content (invalid
// HDF5 container, wrong version, or a decode failure) and a value too long for
// the field that holds it become `ValueError`. Exhaustive, so a new variant must
// be classified.
#[expect(
    clippy::needless_pass_by_value,
    reason = "called as `map_err(file_err)`, which passes the error by value"
)]
fn file_err(e: geotrace_sdk::Error) -> PyErr {
    use geotrace_sdk::Error;
    let msg = e.to_string();
    match e {
        Error::Io(_) => PyIOError::new_err(msg),
        Error::Hdf5(_)
        | Error::UnsupportedVersion { .. }
        | Error::UnknownConstellation { .. }
        | Error::ShapeMismatch { .. }
        | Error::InvalidNavigationTag { .. }
        | Error::InvalidNavigationColumnType { .. }
        | Error::InvalidDatasetShape { .. }
        | Error::MissingNavigationColumn { .. }
        | Error::MissingSatelliteColumn { .. }
        | Error::MissingSatelliteGroup { .. }
        | Error::InvalidSatelliteColumnType { .. }
        | Error::InvalidSatelliteTag { .. }
        | Error::SatelliteReportWithoutTimestamp { .. }
        | Error::MixedSatelliteLayouts
        | Error::UnknownConstellationName { .. }
        | Error::UnknownMarkerIcon { .. }
        | Error::ParseError { .. }
        | Error::UnwritableField { .. }
        | Error::UnreadableField { .. }
        | Error::FixWithoutTimestamp { .. }
        | Error::ReportWithoutTimestamp { .. }
        | Error::EventMarkerWithoutTimestamp { .. }
        | Error::EmptyField { .. }
        | Error::IndexPastTable { .. }
        | Error::DebugTimeRepairTimestampOutOfRange { .. }
        | Error::TimestampOutOfRange { .. }
        | Error::TimestampCountOutOfRange { .. }
        | Error::TimestampIsTheAbsentValue { .. }
        | Error::DatasetSizePastFileLength { .. } => PyValueError::new_err(msg),
    }
}

fn consumed_err() -> PyErr {
    PyRuntimeError::new_err("builder already consumed by finish()")
}

/// The `gps_time` and `sys_time` keyword arguments of one Python constructor.
/// Each clock has its own field, so a call site cannot transpose the two.
struct TimestampArguments {
    gps_time: Option<DateTime<FixedOffset>>,
    sys_time: Option<DateTime<FixedOffset>>,
}

fn nav_fix_time_or_value_error(
    TimestampArguments { gps_time, sys_time }: TimestampArguments,
) -> PyResult<NavFixTime> {
    let recorded = RecordedFixTimestamps {
        gps: gps_time.map(|t| t.to_utc()),
        sys: sys_time.map(|t| t.to_utc()),
    };
    NavFixTime::from_recorded(recorded)
        .ok_or_else(|| PyValueError::new_err("provide gps_time or sys_time"))
}

/// Optional file-level metadata for a `.gtd` file.
///
/// Raises `ValueError` for a value with a nul byte.
#[pyclass(skip_from_py_object, name = "Meta")]
#[derive(Debug, Clone)]
pub struct PyMeta {
    inner: Meta,
}

#[pymethods]
impl PyMeta {
    #[new]
    #[pyo3(signature = (*, title=None, device=None, notes=None, identity=None, travel_mode=None))]
    fn new(
        title: Option<String>,
        device: Option<String>,
        notes: Option<String>,
        identity: Option<String>,
        travel_mode: Option<TravelModeArg>,
    ) -> PyResult<Self> {
        let inner = Meta::builder()
            .maybe_title(title)
            .maybe_device(device)
            .maybe_notes(notes)
            .maybe_identity(identity)
            .maybe_travel_mode(travel_mode.map(TravelMode::from))
            .build()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self { inner })
    }

    /// File title, or `None`.
    #[getter]
    fn title(&self) -> Option<&str> {
        self.inner.title()
    }

    /// Sensor or device that produced the data, or `None`.
    #[getter]
    fn device(&self) -> Option<&str> {
        self.inner.device()
    }

    /// Free-text notes, or `None`.
    #[getter]
    fn notes(&self) -> Option<&str> {
        self.inner.notes()
    }

    /// Opaque producer identity string, or `None`.
    #[getter]
    fn identity(&self) -> Option<&str> {
        self.inner.identity()
    }

    /// Platform the recording was made on: a `TravelMode`, or the raw wire
    /// name as `str` when it is outside the known set (preserved, never
    /// dropped), or `None` when absent.
    #[getter]
    fn travel_mode<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        match self.inner.travel_mode() {
            None => Ok(None),
            Some(mode) => match PyTravelMode::from_travel_mode(mode) {
                Some(known) => Ok(Some(known.into_pyobject(py)?.into_any())),
                None => Ok(Some(mode.name().into_pyobject(py)?.into_any())),
            },
        }
    }

    /// Version of the SDK build that wrote the file, or `None`.
    #[getter]
    fn sdk_version(&self) -> Option<&str> {
        self.inner.sdk_version()
    }

    /// Commit of the geotrace repository the writing SDK was built from, or
    /// `None`.
    #[getter]
    fn sdk_git_commit(&self) -> Option<&str> {
        self.inner.sdk_git_commit()
    }

    /// Committer timestamp of `sdk_git_commit` (timezone-aware UTC), or `None`.
    #[getter]
    fn sdk_commit_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sdk_commit_time().map(to_fixed)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!("Meta(title={:?})", self.inner.title())
    }
}

/// A navigation fix with all associated independent satellite reports.
///
/// The value ranges of `NavFix` apply here too, as expectations.
/// The SDK returns `lat` and `lon` unchanged, NaN included.
/// Optional floating-point measurements, including DOP, return `None` for an on-disk NaN.
/// Checking a value against its range is the caller's job.
#[pyclass(frozen, skip_from_py_object, name = "NavPoint")]
#[derive(Clone, Debug)]
pub struct PyNavPoint {
    inner: NavPoint,
}

impl From<&NavPoint> for PyNavPoint {
    fn from(point: &NavPoint) -> Self {
        Self {
            inner: point.clone(),
        }
    }
}

#[pymethods]
impl PyNavPoint {
    /// Latitude in degrees, expected in [-90, 90].
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.fix.lat.as_degrees()
    }

    /// Longitude in degrees, expected in [-180, 180].
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.fix.lon.as_degrees()
    }

    /// GPS-receiver timestamp (timezone-aware UTC), or `None` when unreported.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.fix.gps_time().map(to_fixed)
    }

    /// System-clock timestamp (timezone-aware UTC), or `None` when the recorder
    /// did not supply one.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.fix.sys_time().map(to_fixed)
    }

    /// Heading in degrees, expected in [0, 360), or `None`.
    #[getter]
    fn heading(&self) -> Option<f64> {
        self.inner.fix.heading.map(|h| h.as_degrees())
    }

    /// Speed in m/s, expected to be non-negative, or `None`.
    #[getter]
    fn speed_mps(&self) -> Option<f64> {
        self.inner.fix.speed.map(|s| s.as_meters_per_second())
    }

    /// Estimated horizontal accuracy radius in metres, expected to be
    /// non-negative, or `None`.
    #[getter]
    fn eph_m(&self) -> Option<f64> {
        self.inner.fix.eph_m
    }

    /// Reported state, or `None` when unreported.
    /// Explicit `SolutionState.UNKNOWN` blocks fallback.
    #[getter]
    fn state(&self) -> PyResult<Option<PySolutionState>> {
        self.inner.fix.state.map(TryInto::try_into).transpose()
    }

    /// Reported solution method, independent of state, or `None` when unreported.
    #[getter]
    fn method(&self) -> PyResult<Option<PySolutionMethod>> {
        self.inner.fix.method.map(TryInto::try_into).transpose()
    }

    /// Reported solution dimension, independent of state, or `None` when unreported.
    #[getter]
    fn dimension(&self) -> PyResult<Option<PySolutionDimension>> {
        self.inner.fix.dimension.map(TryInto::try_into).transpose()
    }

    /// Reported global used count, independent of identity rows, or `None` when unreported.
    #[getter]
    fn used_count(&self) -> Option<u32> {
        self.inner.fix.used_count
    }

    /// Dimensionless horizontal dilution of precision, or `None` when unreported.
    #[getter]
    fn hdop(&self) -> Option<f64> {
        self.inner.fix.hdop
    }

    /// Dimensionless vertical dilution of precision, or `None` when unreported.
    #[getter]
    fn vdop(&self) -> Option<f64> {
        self.inner.fix.vdop
    }

    /// Dimensionless position dilution of precision, or `None` when unreported.
    #[getter]
    fn pdop(&self) -> Option<f64> {
        self.inner.fix.pdop
    }

    /// Coordinate origin, or `None` when unreported.
    #[getter]
    fn position_provenance(&self) -> PyResult<Option<PyPositionProvenance>> {
        self.inner
            .fix
            .position_provenance
            .map(TryInto::try_into)
            .transpose()
    }

    /// Owned usage samples in input order, including every associated report.
    #[getter]
    fn usage_reports(&self) -> Vec<PySatelliteUsageReport> {
        self.inner
            .satellite_evidence()
            .usage
            .iter()
            .cloned()
            .map(|inner| PySatelliteUsageReport { inner })
            .collect()
    }

    /// Owned observation samples in input order, including every associated report.
    #[getter]
    fn observation_reports(&self) -> Vec<PySatelliteObservationReport> {
        self.inner
            .satellite_evidence()
            .observations
            .iter()
            .cloned()
            .map(|inner| PySatelliteObservationReport { inner })
            .collect()
    }

    /// Lossless combined projection, or `None` for independent evidence without that projection.
    #[getter]
    fn satellites(&self) -> Option<PySatelliteReport> {
        self.inner
            .satellites()
            .clone()
            .map(|s| PySatelliteReport { inner: s })
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!(
            "NavPoint(lat={:.6}, lon={:.6})",
            self.inner.fix.lat.as_degrees(),
            self.inner.fix.lon.as_degrees(),
        )
    }
}

fn position_or_index_error(index: isize, len: usize) -> PyResult<usize> {
    let length = isize::try_from(len).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?;
    let position = if index < 0 {
        index
            .checked_add(length)
            .ok_or_else(|| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?
    } else {
        index
    };
    if position < 0 || position >= length {
        return Err(PyIndexError::new_err(INDEX_OUT_OF_RANGE));
    }
    usize::try_from(position).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))
}

fn slice_positions(slice: &Bound<'_, PySlice>, len: usize) -> PyResult<Vec<usize>> {
    let length = isize::try_from(len).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?;
    let indices = slice.indices(length)?;
    let mut positions = Vec::with_capacity(indices.slicelength);
    let mut at = indices.start;
    for _ in 0..indices.slicelength {
        positions.push(usize::try_from(at).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?);
        at = at.saturating_add(indices.step);
    }
    Ok(positions)
}

/// One element for an integer `key`, a `list` of them for a slice.
fn sequence_item_or_slice<'py, Exposed, Stored>(
    py: Python<'py>,
    key: &Bound<'py, PyAny>,
    stored: &[Stored],
) -> PyResult<Bound<'py, PyAny>>
where
    Exposed: IntoPyObject<'py> + for<'a> From<&'a Stored>,
{
    if let Ok(slice) = key.cast::<PySlice>() {
        let selected: Vec<Exposed> = slice_positions(slice, stored.len())?
            .into_iter()
            .filter_map(|position| stored.get(position))
            .map(Exposed::from)
            .collect();
        return Ok(PyList::new(py, selected)?.into_any());
    }
    let position = position_or_index_error(key.extract()?, stored.len())?;
    let element = stored
        .get(position)
        .ok_or_else(|| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?;
    Exposed::from(element).into_bound_py_any(py)
}

nav_file_sequence! {
    summary: "The nav points of a :class:`NavFile`, in chronological order.",
    sequence: PyNavPointSequence as "NavPointSequence",
    iterator: PyNavPointIterator as "NavPointIterator",
    element: PyNavPoint as "NavPoint",
    collection: nav_points,
    methods: {
        /// Every point's latitude in degrees.
        fn latitudes(&self) -> Vec<f64> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.lat.as_degrees())
                .collect()
        }

        /// Every point's longitude in degrees.
        fn longitudes(&self) -> Vec<f64> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.lon.as_degrees())
                .collect()
        }

        /// Every point's GPS-receiver timestamp, or ``None`` when unreported.
        fn gps_times(&self) -> Vec<Option<DateTime<FixedOffset>>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.gps_time().map(to_fixed))
                .collect()
        }

        /// Every point's system-clock timestamp, ``None`` where the recorder
        /// supplied none.
        fn sys_times(&self) -> Vec<Option<DateTime<FixedOffset>>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.sys_time().map(to_fixed))
                .collect()
        }

        /// Every point's heading in degrees, or ``None`` for a fix without one.
        fn headings(&self) -> Vec<Option<f64>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.heading.map(|h| h.as_degrees()))
                .collect()
        }

        /// Every point's speed in m/s, or ``None`` for a fix without one.
        fn speeds_mps(&self) -> Vec<Option<f64>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.speed.map(|s| s.as_meters_per_second()))
                .collect()
        }

        /// Every point's horizontal accuracy radius in metres, or ``None`` for a
        /// fix without one.
        fn eph_m_values(&self) -> Vec<Option<f64>> {
            self.file.nav_points().iter().map(|p| p.fix.eph_m).collect()
        }
    },
}

nav_file_sequence! {
    summary: "The map markers of a :class:`NavFile` with their interpolated positions.",
    sequence: PyMarkerSequence as "MarkerSequence",
    iterator: PyMarkerIterator as "MarkerIterator",
    element: PyMarker as "Marker",
    collection: markers,
}

nav_file_sequence! {
    summary: "The event markers of a :class:`NavFile` with their interpolated positions.",
    sequence: PyEventMarkerPointSequence as "EventMarkerPointSequence",
    iterator: PyEventMarkerPointIterator as "EventMarkerPointIterator",
    element: PyEventMarkerPoint as "EventMarkerPoint",
    collection: event_markers,
}

nav_file_sequence! {
    summary: "The sensor channels of a :class:`NavFile`, sorted by name.",
    sequence: PyChannelSequence as "ChannelSequence",
    iterator: PyChannelIterator as "ChannelIterator",
    element: PyChannel as "Channel",
    collection: channels,
}

nav_file_sequence! {
    summary: "The per-variant event marker styles of a :class:`NavFile`.",
    sequence: PyEventMarkerStyleSequence as "EventMarkerStyleSequence",
    iterator: PyEventMarkerStyleIterator as "EventMarkerStyleIterator",
    element: PyEventMarkerStyle as "EventMarkerStyle",
    collection: event_marker_styles,
}

/// A parsed `.gtd` navigation data file.
///
/// Construct via `NavFileBuilder.finish()` to write, or `NavFile.open(path)`
/// to read.
#[pyclass(skip_from_py_object, name = "NavFile")]
pub struct PyNavFile {
    inner: Arc<NavFile>,
}

#[pymethods]
impl PyNavFile {
    /// Open and parse a `.gtd` file at `path`.
    ///
    /// Accepts any path-like value: `str`, `bytes`, or a `pathlib.Path` object.
    #[staticmethod]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "PyO3 extracts an owned PathBuf from str, bytes and os.PathLike: &Path has no FromPyObject impl"
    )]
    fn open(path: PathBuf) -> PyResult<Self> {
        NavFile::open(&path)
            .map(|f| Self { inner: Arc::new(f) })
            .map_err(file_err)
    }

    /// Parse a `.gtd` file from raw bytes.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        NavFile::read(std::io::Cursor::new(data))
            .map(|f| Self { inner: Arc::new(f) })
            .map_err(file_err)
    }

    /// Write this file to `path`.
    ///
    /// Accepts any path-like value: `str`, `bytes`, or a `pathlib.Path` object.
    /// Appends `.gtd` if `path` has no extension.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "PyO3 extracts an owned PathBuf from str, bytes and os.PathLike: &Path has no FromPyObject impl"
    )]
    fn write_to_file(&self, path: PathBuf) -> PyResult<()> {
        self.inner.write_to_file(&path).map_err(file_err)
    }

    /// Serialise the file to a `bytes` object.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = Vec::new();
        self.inner.write(&mut buf).map_err(file_err)?;
        Ok(PyBytes::new(py, &buf))
    }

    /// File-level metadata.
    #[getter]
    fn meta(&self) -> PyMeta {
        PyMeta {
            inner: self.inner.meta().clone(),
        }
    }

    /// All nav points in chronological order.
    #[getter]
    fn points(&self) -> PyNavPointSequence {
        PyNavPointSequence {
            file: Arc::clone(&self.inner),
        }
    }

    /// All map markers with their interpolated positions.
    ///
    /// Reading this attribute raises a ``UserWarning`` for a marker with an
    /// icon code unknown to this build. Its ``icon`` reads as ``None``. Its
    /// ``icon_code`` contains the code.
    #[getter]
    fn markers(&self, py: Python<'_>) -> PyResult<PyMarkerSequence> {
        for marker in self.inner.markers() {
            if let AnnotationIcon::Unrecognized(code) = marker.annotation.icon() {
                let message = CString::new(format!(
                    "map marker {:?} holds the icon code {code}, which this build does not have: \
                     icon reads as None, icon_code holds the code",
                    marker.annotation.label().unwrap_or("")
                ))
                .map_err(|err| PyValueError::new_err(err.to_string()))?;
                PyErr::warn(py, &py.get_type::<PyUserWarning>(), &message, 1)?;
            }
        }
        Ok(PyMarkerSequence {
            file: Arc::clone(&self.inner),
        })
    }

    /// All event markers with their interpolated positions.
    #[getter]
    fn event_markers(&self) -> PyEventMarkerPointSequence {
        PyEventMarkerPointSequence {
            file: Arc::clone(&self.inner),
        }
    }

    /// All ad-hoc sensor channels, sorted by name.
    #[getter]
    fn channels(&self) -> PyChannelSequence {
        PyChannelSequence {
            file: Arc::clone(&self.inner),
        }
    }

    /// Per-variant style overrides stored in the file.
    ///
    /// Reading this attribute raises a ``UserWarning`` for a style with an icon
    /// unknown to this build. Its ``icon`` reads as ``None``. Its
    /// ``icon_name`` contains the name.
    #[getter]
    fn event_marker_styles(&self, py: Python<'_>) -> PyResult<PyEventMarkerStyleSequence> {
        for style in self.inner.event_marker_styles() {
            if let EventMarkerIconChoice::Unrecognized(name) = style.icon() {
                let message = CString::new(format!(
                    "event marker style {:?} names the icon {name:?}, which this build does not \
                     have: icon reads as None, icon_name holds the name",
                    style.variant_path()
                ))
                .map_err(|err| PyValueError::new_err(err.to_string()))?;
                PyErr::warn(py, &py.get_type::<PyUserWarning>(), &message, 1)?;
            }
        }
        Ok(PyEventMarkerStyleSequence {
            file: Arc::clone(&self.inner),
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "NavFile(points={}, markers={}, event_markers={})",
            self.inner.nav_points().len(),
            self.inner.markers().len(),
            self.inner.event_markers().len(),
        )
    }
}

/// Dispatch target for [`PyNavFileBuilder::add`].
#[derive(FromPyObject)]
enum AddItem<'py> {
    Fix(Bound<'py, PyNavFix>),
    Usage(Bound<'py, PySatelliteUsageReport>),
    Observation(Bound<'py, PySatelliteObservationReport>),
    Report(Bound<'py, PySatelliteReport>),
    Annotation(Bound<'py, PyAnnotation>),
    EventMarker(Bound<'py, PyEventMarker>),
    Channel(Bound<'py, PyChannel>),
}

/// Assembles nav fixes, satellite reports, and annotations into a `NavFile`.
///
/// ```python
/// nav_file = (
///     NavFileBuilder()
///     .with_meta(Meta(title="My Track"))
///     .add(NavFix(lat=51.5, lon=-0.1, gps_time=...))
///     .finish()
/// )
/// nav_file.write_to_file("track.gtd")
/// ```
///
/// Calling `finish()` consumes the builder. Further calls raise `RuntimeError`.
#[pyclass(skip_from_py_object, name = "NavFileBuilder")]
pub struct PyNavFileBuilder {
    /// Pre-open config. None once the recorder has been opened.
    config: Option<NavFileBuilder>,
    /// Opened data recorder. None until the first add() or finish().
    recorder: Option<NavRecorder>,
}

impl PyNavFileBuilder {
    /// Apply `configure` to the pre-open config, raising `RuntimeError` under
    /// the name of the method that called it once data has been added. When `configure` returns
    /// an error, this raises `ValueError` and the config stays unchanged: `configure` receives a
    /// clone.
    fn configure_before_data(
        &mut self,
        method_name: &str,
        configure: impl FnOnce(NavFileBuilder) -> Result<NavFileBuilder, MetaStringWithNul>,
    ) -> PyResult<()> {
        if self.recorder.is_some() {
            return Err(PyRuntimeError::new_err(format!(
                "{method_name}() must be called before adding data"
            )));
        }
        let config = self.config.as_ref().ok_or_else(consumed_err)?;
        let configured =
            configure(config.clone()).map_err(|error| PyValueError::new_err(error.to_string()))?;
        self.config = Some(configured);
        Ok(())
    }

    /// Ensure the recorder is open, opening it from config if needed.
    fn ensure_recorder(&mut self) -> PyResult<&mut NavRecorder> {
        if self.recorder.is_none() {
            let config = self.config.take().ok_or_else(consumed_err)?;
            self.recorder = Some(config.open());
        }
        self.recorder.as_mut().ok_or_else(consumed_err)
    }
}

#[pymethods]
impl PyNavFileBuilder {
    #[new]
    fn new() -> Self {
        Self {
            config: Some(NavFileBuilder::new()),
            recorder: None,
        }
    }

    /// Attach file-level metadata.
    ///
    /// Must be called before any `add()` call. Returns `self` for chaining.
    fn with_meta(slf: Bound<'_, Self>, meta: &PyMeta) -> PyResult<Py<Self>> {
        slf.borrow_mut()
            .configure_before_data("with_meta", |config| {
                Ok(config.with_meta(meta.inner.clone()))
            })?;
        Ok(slf.unbind())
    }

    /// Set the file title. Must be called before any `add()` call.
    ///
    /// Raises `ValueError` for a title with a nul byte.
    fn with_title(slf: Bound<'_, Self>, title: String) -> PyResult<Py<Self>> {
        slf.borrow_mut()
            .configure_before_data("with_title", |config| config.with_title(title))?;
        Ok(slf.unbind())
    }

    /// Set the device or sensor name. Must be called before any `add()` call.
    ///
    /// Raises `ValueError` for a device name with a nul byte.
    fn with_device(slf: Bound<'_, Self>, device: String) -> PyResult<Py<Self>> {
        slf.borrow_mut()
            .configure_before_data("with_device", |config| config.with_device(device))?;
        Ok(slf.unbind())
    }

    /// Set free-text notes. Must be called before any `add()` call.
    ///
    /// Raises `ValueError` for notes with a nul byte.
    fn with_notes(slf: Bound<'_, Self>, notes: String) -> PyResult<Py<Self>> {
        slf.borrow_mut()
            .configure_before_data("with_notes", |config| config.with_notes(notes))?;
        Ok(slf.unbind())
    }

    /// Clamp an annotation outside the nav fix time range to the nearest fix.
    ///
    /// The strict build raises `ValueError` for such an annotation.
    ///
    /// Must be called before any `add()` call. Returns `self` for chaining.
    fn with_lenient_errors(slf: Bound<'_, Self>) -> PyResult<Py<Self>> {
        slf.borrow_mut()
            .configure_before_data("with_lenient_errors", |config| {
                Ok(config.with_lenient_errors())
            })?;
        Ok(slf.unbind())
    }

    /// Set how far a satellite report may be from a nav fix to be associated
    /// with it. The default is 500 ms.
    ///
    /// Must be called before any `add()` call. Returns `self` for chaining.
    ///
    /// Raises `ValueError` for a negative window.
    fn with_satellite_window(slf: Bound<'_, Self>, window: Duration) -> PyResult<Py<Self>> {
        let window = window
            .to_std()
            .map_err(|_| PyValueError::new_err("the satellite window must be zero or longer"))?;
        slf.borrow_mut()
            .configure_before_data("with_satellite_window", |config| {
                Ok(config.with_satellite_window(window))
            })?;
        Ok(slf.unbind())
    }

    /// Add a nav fix, satellite report, annotation, event marker, or channel to
    /// the file.
    ///
    /// Returns `self` to allow chaining:
    ///
    /// ```python
    /// builder.add(NavFix(...)).add(SatelliteReport(...)).add(Annotation(...))
    /// ```
    fn add(slf: Bound<'_, Self>, item: AddItem<'_>) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            let recorder = b.ensure_recorder()?;
            match item {
                AddItem::Usage(report) => {
                    recorder.add_satellite_usage_report(report.borrow().inner.clone());
                }
                AddItem::Observation(report) => {
                    recorder.add_satellite_observation_report(report.borrow().inner.clone());
                }
                AddItem::Fix(f) => {
                    recorder.add_nav_fix(f.borrow().inner);
                }
                AddItem::Report(r) => {
                    recorder.add_satellite_report(r.borrow().inner.clone());
                }
                AddItem::Annotation(a) => {
                    recorder.add_annotation(a.borrow().inner.clone());
                }
                AddItem::EventMarker(em) => {
                    if let SkippableEventMarker::Record(marker) = &em.borrow().marker {
                        recorder.add_event_marker(marker.clone());
                    }
                }
                AddItem::Channel(c) => {
                    recorder.add_channel(c.borrow().inner.clone());
                }
            }
        }
        Ok(slf.unbind())
    }

    /// Add a per-variant style override to the file.
    ///
    /// Returns ``self`` to allow chaining. The builder keeps one style per variant path, the style
    /// of the last call for that path. The file lists the styles in variant path order. A style
    /// from ``NavFile.event_marker_styles`` is written back verbatim, a color outside the
    /// ``#RRGGBB`` form included.
    fn add_event_marker_style(
        slf: Bound<'_, Self>,
        style: &PyEventMarkerStyle,
    ) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            let recorder = b.ensure_recorder()?;
            recorder.add_event_marker_style(style.inner.clone());
        }
        Ok(slf.unbind())
    }

    /// Process all data and return a `NavFile`.
    ///
    /// Consumes the builder - calling `finish()` again raises `RuntimeError`.
    fn finish(&mut self) -> PyResult<PyNavFile> {
        let recorder = if let Some(recorder) = self.recorder.take() {
            recorder
        } else {
            self.config.take().ok_or_else(consumed_err)?.open()
        };
        recorder
            .finish()
            .map(|f| PyNavFile { inner: Arc::new(f) })
            .map_err(build_err)
    }
}

/// A record's Rust module path becomes its Python logger name.
/// `geotrace_sdk::builder` writes to the `geotrace_sdk.builder` logger.
///
/// A `logging` level changed after the first record still takes effect.
/// [`Caching::Loggers`] reads the Python logger's level again for every record.
///
/// Importing the module succeeds even when the install fails.
fn install_python_logging_bridge(py: Python<'_>) {
    let installed = Logger::new(py, Caching::Loggers)
        .map_err(|e| e.to_string())
        .and_then(|logger| logger.install().map_err(|e| e.to_string()));
    if let Err(reason) = installed {
        log::warn!("the sdk's log records stay off python's logging: {reason}");
    }
}

#[pymodule]
fn _geotrace_sdk(m: &Bound<'_, PyModule>) -> PyResult<()> {
    install_python_logging_bridge(m.py());

    m.add_class::<PySatellite>()?;
    m.add_class::<PySatelliteReport>()?;
    m.add_class::<PyUnit>()?;
    m.add_class::<PyChannelUnit>()?;
    m.add_class::<PyChannel>()?;
    m.add_class::<PyNavFix>()?;
    m.add_class::<PySatelliteIdentity>()?;
    m.add_class::<PySatelliteScope>()?;
    m.add_class::<PySatelliteUsage>()?;
    m.add_class::<PySatelliteObservation>()?;
    m.add_class::<PySatelliteUsageReport>()?;
    m.add_class::<PySatelliteObservationReport>()?;

    m.add_class::<PyAnnotation>()?;
    m.add_class::<PyMeta>()?;
    m.add_class::<PyNavPoint>()?;
    m.add_class::<PyMarker>()?;
    m.add_class::<PyEventMarker>()?;
    m.add_class::<PyEventMarkerStyle>()?;
    m.add_class::<PyEventMarkerPoint>()?;
    m.add_class::<PyNavPointSequence>()?;
    m.add_class::<PyNavPointIterator>()?;
    m.add_class::<PyMarkerSequence>()?;
    m.add_class::<PyMarkerIterator>()?;
    m.add_class::<PyEventMarkerPointSequence>()?;
    m.add_class::<PyEventMarkerPointIterator>()?;
    m.add_class::<PyChannelSequence>()?;
    m.add_class::<PyChannelIterator>()?;
    m.add_class::<PyEventMarkerStyleSequence>()?;
    m.add_class::<PyEventMarkerStyleIterator>()?;
    m.add_class::<PyNavFile>()?;
    m.add_class::<PyNavFileBuilder>()?;
    m.add_function(wrap_pyfunction!(enums::constellation_from_name, m)?)?;
    m.add_function(wrap_pyfunction!(enums::marker_icon_from_name, m)?)?;
    m.add_function(wrap_pyfunction!(channel::mps_from_kmh, m)?)?;
    m.add_function(wrap_pyfunction!(channel::mps_from_knots, m)?)?;
    m.add_function(wrap_pyfunction!(channel::kmh_from_mps, m)?)?;
    m.add_function(wrap_pyfunction!(channel::knots_from_mps, m)?)?;
    m.add_function(wrap_pyfunction!(satellite::snr_is_no_data_sentinel, m)?)?;
    m.add_function(wrap_pyfunction!(enums::rust_sdk_enum_members, m)?)?;
    Ok(())
}

const INDEX_OUT_OF_RANGE: &str = "index out of range";
