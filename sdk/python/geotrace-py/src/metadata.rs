use chrono::{DateTime, FixedOffset};
use geotrace_sdk::{Meta, TravelMode};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::enums::{PyTravelMode, TravelModeArg};

/// Optional file-level metadata for a `.gtd` file.
///
/// Raises `ValueError` for a value with a nul byte.
#[pyclass(skip_from_py_object, name = "Meta")]
#[derive(Clone, Debug)]
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
        self.inner.sdk_commit_time().map(|time| time.fixed_offset())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!("Meta(title={:?})", self.inner.title())
    }
}

impl From<&Meta> for PyMeta {
    fn from(meta: &Meta) -> Self {
        Self {
            inner: meta.clone(),
        }
    }
}

impl From<&PyMeta> for Meta {
    fn from(meta: &PyMeta) -> Self {
        meta.inner.clone()
    }
}
