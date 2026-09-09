//! The conversions between this crate's mirror enums and the `enum.Enum`
//! classes of the `geotrace_sdk.enums` Python module.

use std::str::FromStr;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

/// A Rust enum whose variants mirror the members of a Python `enum.Enum` class.
///
/// `strum` derives the member name from the variant identifier in both
/// directions, through `IntoStaticStr` and `EnumString` with
/// `serialize_all = "SCREAMING_SNAKE_CASE"`.
pub(crate) trait PythonEnumMirror: Copy + FromStr + Into<&'static str> {
    /// The class name in [`ENUM_MODULE`].
    const CLASS_NAME: &'static str;

    /// The imported class, cached for the lifetime of the interpreter.
    fn class_lock() -> &'static PyOnceLock<Py<PyType>>;

    /// Returns the variant equivalent to the Python member `object`.
    ///
    /// An object outside the class raises `TypeError`, which stops a bare `int`
    /// or a member of one of the other classes at the boundary.
    fn from_python_member(object: &Bound<'_, PyAny>) -> PyResult<Self> {
        let class = Self::class_lock().import(object.py(), ENUM_MODULE, Self::CLASS_NAME)?;
        if !object.is_instance(class.as_any())? {
            return Err(PyTypeError::new_err(format!(
                "expected a {} member, got {}",
                Self::CLASS_NAME,
                object.get_type().name()?
            )));
        }
        let member_name: String = object.getattr(intern!(object.py(), "name"))?.extract()?;
        Self::from_str(&member_name).map_err(|_parse_error| {
            PyValueError::new_err(format!(
                "this build has no {}.{member_name}",
                Self::CLASS_NAME
            ))
        })
    }

    /// Returns the Python member equivalent to `self`.
    fn to_python_member<'py>(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let member_name: &'static str = self.into();
        Self::class_lock()
            .import(py, ENUM_MODULE, Self::CLASS_NAME)?
            .getattr(member_name)
    }
}

const ENUM_MODULE: &str = "geotrace_sdk.enums";
