use std::ffi::CString;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use geotrace_sdk::{AnnotationIcon, EventMarkerIconChoice, NavFile};
use pyo3::exceptions::{PyUserWarning, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::collections::{
    PyChannelSequence, PyEventMarkerPointSequence, PyEventMarkerStyleSequence, PyMarkerSequence,
    PyNavPointSequence,
};
use crate::errors;
use crate::metadata::PyMeta;

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
            .map(Self::from)
            .map_err(errors::file_error)
    }

    /// Parse a `.gtd` file from raw bytes.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        NavFile::read(Cursor::new(data))
            .map(Self::from)
            .map_err(errors::file_error)
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
        self.inner.write_to_file(&path).map_err(errors::file_error)
    }

    /// Serialise the file to a `bytes` object.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = Vec::new();
        self.inner.write(&mut buf).map_err(errors::file_error)?;
        Ok(PyBytes::new(py, &buf))
    }

    /// File-level metadata.
    #[getter]
    fn meta(&self) -> PyMeta {
        PyMeta::from(self.inner.meta())
    }

    /// All nav points in chronological order.
    #[getter]
    fn points(&self) -> PyNavPointSequence {
        Arc::clone(&self.inner).into()
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
        Ok(Arc::clone(&self.inner).into())
    }

    /// All event markers with their interpolated positions.
    #[getter]
    fn event_markers(&self) -> PyEventMarkerPointSequence {
        Arc::clone(&self.inner).into()
    }

    /// All ad-hoc sensor channels, sorted by name.
    #[getter]
    fn channels(&self) -> PyChannelSequence {
        Arc::clone(&self.inner).into()
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
        Ok(Arc::clone(&self.inner).into())
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

impl From<NavFile> for PyNavFile {
    fn from(file: NavFile) -> Self {
        Self {
            inner: Arc::new(file),
        }
    }
}
