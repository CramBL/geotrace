use chrono::{DateTime, FixedOffset, Utc};
use geotrace_sdk::{
    Annotation, AnnotationIcon, EventMarker, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerPoint, EventMarkerStyle, Marker, MarkerIcon,
};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;

use crate::enums::PyMarkerIcon;
use crate::errors;

/// A user-defined map annotation with an optional label and an icon.
///
/// `time` must be a timezone-aware `datetime.datetime`. An empty `label` gives a
/// `label` of `None`. A `label` longer than the 255 bytes the `markers/label`
/// field holds raises `ValueError`.
#[pyclass(skip_from_py_object, name = "Annotation")]
#[derive(Clone, Debug)]
pub struct PyAnnotation {
    pub(crate) inner: Annotation,
}

#[pymethods]
impl PyAnnotation {
    #[new]
    #[pyo3(signature = (time, *, label=None, icon=PyMarkerIcon::Pin))]
    fn new(
        time: DateTime<FixedOffset>,
        label: Option<String>,
        icon: PyMarkerIcon,
    ) -> PyResult<Self> {
        let inner = Annotation::builder()
            .time(time.to_utc())
            .maybe_label(label)
            .icon(MarkerIcon::from(icon))
            .build()
            .map_err(errors::file_error)?;
        Ok(Self { inner })
    }

    /// Timestamp (timezone-aware UTC).
    #[getter]
    fn time(&self) -> DateTime<FixedOffset> {
        self.inner.time().fixed_offset()
    }

    /// Display label, or `None`.
    #[getter]
    fn label(&self) -> Option<&str> {
        self.inner.label()
    }

    /// Visual icon, or ``None`` for an icon code this build does not have.
    /// :attr:`icon_code` holds such a code.
    #[getter]
    fn icon(&self) -> Option<PyMarkerIcon> {
        match self.inner.icon() {
            AnnotationIcon::Icon(icon) => Some(PyMarkerIcon::from(icon)),
            AnnotationIcon::Unrecognized(_) => None,
        }
    }

    /// The stored code of :attr:`icon`.
    ///
    /// An icon code this build does not have is returned unchanged, and
    /// :attr:`icon` is ``None`` for it.
    #[getter]
    fn icon_code(&self) -> u8 {
        self.inner.icon().wire_code()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!("Annotation(time={:?})", self.inner.time())
    }
}

/// A map annotation with its interpolated position on the nav track.
#[pyclass(skip_from_py_object, name = "Marker")]
#[derive(Clone, Debug)]
pub struct PyMarker {
    inner: Marker,
}

#[pymethods]
impl PyMarker {
    /// Interpolated latitude in degrees.
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.lat.as_degrees()
    }

    /// Interpolated longitude in degrees.
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.lon.as_degrees()
    }

    /// The annotation at :attr:`lat` and :attr:`lon`.
    #[getter]
    fn annotation(&self) -> PyAnnotation {
        PyAnnotation {
            inner: self.inner.annotation.clone(),
        }
    }

    /// Display label from the annotation, or `None`.
    #[getter]
    fn label(&self) -> Option<&str> {
        self.inner.annotation.label()
    }

    /// Visual icon from the annotation, or ``None`` for an icon code this
    /// build does not have. :attr:`icon_code` holds such a code.
    #[getter]
    fn icon(&self) -> Option<PyMarkerIcon> {
        match self.inner.annotation.icon() {
            AnnotationIcon::Icon(icon) => Some(PyMarkerIcon::from(icon)),
            AnnotationIcon::Unrecognized(_) => None,
        }
    }

    /// The stored code of :attr:`icon`.
    ///
    /// An icon code this build does not have is returned unchanged, and
    /// :attr:`icon` is ``None`` for it.
    #[getter]
    fn icon_code(&self) -> u8 {
        self.inner.annotation.icon().wire_code()
    }

    /// Annotation timestamp (timezone-aware UTC).
    #[getter]
    fn time(&self) -> DateTime<FixedOffset> {
        self.inner.annotation.time().fixed_offset()
    }

    fn __repr__(&self) -> String {
        format!(
            "Marker(lat={:.6}, lon={:.6})",
            self.inner.lat.as_degrees(),
            self.inner.lon.as_degrees(),
        )
    }
}

impl From<&Marker> for PyMarker {
    fn from(marker: &Marker) -> Self {
        Self {
            inner: marker.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SkippableEventMarker {
    Record(EventMarker),
    Skip {
        sys_time: DateTime<Utc>,
        annotation: Option<String>,
    },
}

/// An event marker to add to the nav track.
///
/// ``variant_path`` is a slash-separated hierarchy, e.g. ``"power/boot"`` or
/// ``"connectivity/agps/request"``, or ``None`` (or the ``event_kind.skip``
/// sentinel value) to skip this marker. Anything else raises ``TypeError``.
/// Allowed characters: ASCII alphanumeric, hyphen, underscore, and slash.
/// No leading or trailing slash. No empty segments (``//``). Max 255 bytes.
/// ``annotation`` holds at most 511 bytes. An empty ``annotation`` reads back as none.
/// The constructor raises ``ValueError`` if ``variant_path`` is a malformed string, or if
/// ``annotation`` is past 511 bytes for a marker that is not skipped.
///
/// ``sys_time`` must be a timezone-aware ``datetime.datetime``.
#[pyclass(skip_from_py_object, name = "EventMarker")]
#[derive(Clone, Debug)]
pub struct PyEventMarker {
    pub(crate) marker: SkippableEventMarker,
}

#[pymethods]
impl PyEventMarker {
    /// Create an ``EventMarker``.
    ///
    /// ``variant_path`` may be a path string, ``None``, or the
    /// ``event_kind.skip`` sentinel value - the latter two are a no-op when the
    /// marker is passed to ``NavFileBuilder.add()``. Any other type raises
    /// ``TypeError``.
    #[new]
    #[pyo3(signature = (variant_path, sys_time, *, annotation=None))]
    fn new(
        variant_path: Option<Bound<'_, PyAny>>,
        sys_time: DateTime<FixedOffset>,
        annotation: Option<String>,
    ) -> PyResult<Self> {
        let path = match variant_path {
            None => None,
            Some(value) => {
                if let Ok(path) = value.extract::<String>() {
                    Some(path)
                } else if is_skip_sentinel(&value)? {
                    None
                } else {
                    return Err(PyTypeError::new_err(format!(
                        "variant_path must be a str, None, or event_kind.skip, not {}",
                        value.get_type().name()?
                    )));
                }
            }
        };
        let sys_time = sys_time.to_utc();
        let marker = match path {
            Some(path) => SkippableEventMarker::Record(
                EventMarker::builder()
                    .variant_path(path)
                    .sys_time(sys_time)
                    .maybe_annotation(annotation)
                    .build()
                    .map_err(|e| PyValueError::new_err(e.to_string()))?,
            ),
            None => SkippableEventMarker::Skip {
                sys_time,
                annotation,
            },
        };
        Ok(Self { marker })
    }

    #[getter]
    fn variant_path(&self) -> Option<&str> {
        match &self.marker {
            SkippableEventMarker::Record(marker) => Some(marker.variant_path()),
            SkippableEventMarker::Skip { .. } => None,
        }
    }

    #[getter]
    fn sys_time(&self) -> DateTime<FixedOffset> {
        (match &self.marker {
            SkippableEventMarker::Record(marker) => marker.sys_time(),
            SkippableEventMarker::Skip { sys_time, .. } => *sys_time,
        })
        .fixed_offset()
    }

    #[getter]
    fn annotation(&self) -> Option<&str> {
        match &self.marker {
            SkippableEventMarker::Record(marker) => marker.annotation(),
            SkippableEventMarker::Skip { annotation, .. } => annotation.as_deref(),
        }
    }

    fn __repr__(&self) -> String {
        format!("EventMarker(variant_path={:?})", self.variant_path())
    }
}

/// Per-variant icon and color style stored in the file.
///
/// ``variant_path`` must exactly match a path used in an event marker, and follows the rules of
/// :class:`EventMarker`.
/// ``icon`` is a :class:`MarkerIcon` value, or ``None`` for the application default (Pin).
/// ``color`` is ``#RRGGBB``, e.g. ``"#FF9900"``, or ``None`` or an empty string for the
/// deterministic hash color. The constructor raises ``ValueError`` for a malformed variant path
/// and for any other color.
#[pyclass(skip_from_py_object, name = "EventMarkerStyle")]
#[derive(Clone, Debug)]
pub struct PyEventMarkerStyle {
    pub(crate) inner: EventMarkerStyle,
}

#[pymethods]
impl PyEventMarkerStyle {
    #[new]
    #[pyo3(signature = (variant_path, *, icon=None, color=None))]
    fn new(
        variant_path: String,
        icon: Option<PyMarkerIcon>,
        color: Option<String>,
    ) -> PyResult<Self> {
        let inner = EventMarkerStyle::builder()
            .variant_path(variant_path)
            .icon(EventMarkerIconChoice::from(icon.map(MarkerIcon::from)))
            .maybe_color(color)
            .build()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self { inner })
    }

    #[getter]
    fn variant_path(&self) -> &str {
        self.inner.variant_path()
    }

    /// Icon shape, or ``None`` for the application default and for an icon name
    /// this build does not have. :attr:`icon_name` holds such a name.
    #[getter]
    fn icon(&self) -> Option<PyMarkerIcon> {
        match self.inner.icon() {
            EventMarkerIconChoice::Icon(icon) => Some(PyMarkerIcon::from(*icon)),
            EventMarkerIconChoice::Auto | EventMarkerIconChoice::Unrecognized(_) => None,
        }
    }

    /// The stored name of :attr:`icon`, or ``None`` where the style leaves the
    /// icon to the application.
    ///
    /// An icon name this build does not have is returned verbatim, and
    /// :attr:`icon` is ``None`` for it.
    #[getter]
    fn icon_name(&self) -> Option<&str> {
        Some(self.inner.icon().wire_name()).filter(|name| !name.is_empty())
    }

    /// Fill color as ``#RRGGBB``, or ``None`` for the deterministic hash color.
    ///
    /// A file written by a newer build can hold a color in another notation.
    /// Such a color reads back verbatim.
    #[getter]
    fn color(&self) -> Option<&str> {
        match self.inner.color() {
            EventMarkerColor::Auto => None,
            EventMarkerColor::Hex(hex) | EventMarkerColor::Unrecognized(hex) => Some(hex),
        }
    }

    fn __repr__(&self) -> String {
        format!("EventMarkerStyle(variant_path={:?})", self.variant_path())
    }
}

impl From<&EventMarkerStyle> for PyEventMarkerStyle {
    fn from(style: &EventMarkerStyle) -> Self {
        Self {
            inner: style.clone(),
        }
    }
}

/// A resolved event marker as read from a ``NavFile``, with an interpolated
/// map position.
///
/// Obtain via ``NavFile.event_markers``.
#[pyclass(skip_from_py_object, name = "EventMarkerPoint")]
#[derive(Clone, Debug)]
pub struct PyEventMarkerPoint {
    inner: EventMarkerPoint,
}

#[pymethods]
impl PyEventMarkerPoint {
    #[getter]
    fn variant_path(&self) -> &str {
        &self.inner.variant_path
    }

    #[getter]
    fn sys_time(&self) -> DateTime<FixedOffset> {
        self.inner.sys_time.fixed_offset()
    }

    /// Interpolated latitude in degrees.
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.lat.as_degrees()
    }

    /// Interpolated longitude in degrees.
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.lon.as_degrees()
    }

    #[getter]
    fn annotation(&self) -> Option<&str> {
        self.inner.annotation.as_deref()
    }

    fn __repr__(&self) -> String {
        format!(
            "EventMarkerPoint(variant_path={:?}, lat={:.6}, lon={:.6})",
            self.inner.variant_path,
            self.inner.lat.as_degrees(),
            self.inner.lon.as_degrees(),
        )
    }
}

impl From<&EventMarkerPoint> for PyEventMarkerPoint {
    fn from(point: &EventMarkerPoint) -> Self {
        Self {
            inner: point.clone(),
        }
    }
}

/// The ``event_kind.skip`` sentinel value is the single instance of the private
/// ``_Skip`` class of ``geotrace_sdk.event_kind``.
fn is_skip_sentinel(value: &Bound<'_, PyAny>) -> PyResult<bool> {
    let skip = value
        .py()
        .import("geotrace_sdk.event_kind")?
        .getattr("event_kind")?
        .getattr("skip")?;
    let skip_type = skip.get_type();
    value.is_instance(skip_type.as_any())
}
