use std::path::PathBuf;

use geotrace_sdk::BundledLog;
use pyo3::prelude::*;

use crate::errors;

/// A validated raw-text log bundled inside a `.gtd` file under `logs/<name>/text`.
///
/// Constructors reject invalid names, invalid UTF-8, empty content, and logs
/// whose head does not match a recognised timestamp format.
#[derive(Clone, Debug, Eq, PartialEq)]
#[pyclass(eq, frozen, name = "BundledLog")]
pub struct PyBundledLog {
    pub(crate) inner: BundledLog,
}

#[pymethods]
impl PyBundledLog {
    /// Create a validated bundled log from a log name and UTF-8 text.
    #[new]
    fn new(name: String, text: String) -> PyResult<Self> {
        BundledLog::new(name, text)
            .map(Self::from)
            .map_err(errors::bundled_log_error)
    }

    /// Create a validated bundled log from a log name and raw UTF-8 bytes.
    #[staticmethod]
    fn from_utf8_bytes(name: String, data: &[u8]) -> PyResult<Self> {
        BundledLog::from_utf8_bytes(name, data)
            .map(Self::from)
            .map_err(errors::bundled_log_error)
    }

    /// Read a raw-text log file from `path`, deriving the log name from its file name.
    ///
    /// Accepts any path-like value: `str`, `bytes`, or a `pathlib.Path` object.
    #[staticmethod]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "PyO3 extracts an owned PathBuf from str, bytes and os.PathLike: &Path has no FromPyObject impl"
    )]
    fn from_file(path: PathBuf) -> PyResult<Self> {
        BundledLog::from_file(&path)
            .map(Self::from)
            .map_err(errors::bundled_log_error)
    }

    /// The HDF5-safe log name stored under `logs/<name>`.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// The raw UTF-8 log text.
    #[getter]
    fn text(&self) -> &str {
        self.inner.text()
    }

    fn __repr__(&self) -> String {
        format!(
            "BundledLog(name={:?}, len={})",
            self.inner.name(),
            self.inner.text().len(),
        )
    }
}

impl From<BundledLog> for PyBundledLog {
    fn from(inner: BundledLog) -> Self {
        Self { inner }
    }
}

impl From<&BundledLog> for PyBundledLog {
    fn from(log: &BundledLog) -> Self {
        Self { inner: log.clone() }
    }
}
