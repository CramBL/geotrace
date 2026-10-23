use std::collections::BTreeMap;

use geotrace_sdk::{Constellation, MarkerIcon, TravelMode};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;
use strum::IntoEnumIterator as _;

use crate::errors;
use crate::navigation;
use crate::python_enum::PythonEnumMirror;

#[derive(IntoPyObject)]
pub(crate) enum PythonEnumValue {
    WireCode(u8),
    WireName(String),
}

/// GNSS constellation identifier, mirroring `geotrace_sdk.enums.Constellation`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum PyConstellation {
    Beidou,
    Galileo,
    Glonass,
    Gps,
    Navic,
    Qzss,
}

impl PythonEnumMirror for PyConstellation {
    const CLASS_NAME: &'static str = "Constellation";

    fn class_lock() -> &'static PyOnceLock<Py<PyType>> {
        static CLASS: PyOnceLock<Py<PyType>> = PyOnceLock::new();
        &CLASS
    }
}

impl<'py> FromPyObject<'_, 'py> for PyConstellation {
    type Error = PyErr;

    fn extract(object: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        Self::from_python_member(&object)
    }
}

impl<'py> IntoPyObject<'py> for PyConstellation {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_python_member(py)
    }
}

impl From<Constellation> for PyConstellation {
    fn from(c: Constellation) -> Self {
        match c {
            Constellation::Gps => Self::Gps,
            Constellation::Glonass => Self::Glonass,
            Constellation::Galileo => Self::Galileo,
            Constellation::Beidou => Self::Beidou,
            Constellation::Navic => Self::Navic,
            Constellation::Qzss => Self::Qzss,
        }
    }
}

impl From<PyConstellation> for Constellation {
    fn from(c: PyConstellation) -> Self {
        match c {
            PyConstellation::Gps => Self::Gps,
            PyConstellation::Glonass => Self::Glonass,
            PyConstellation::Galileo => Self::Galileo,
            PyConstellation::Beidou => Self::Beidou,
            PyConstellation::Navic => Self::Navic,
            PyConstellation::Qzss => Self::Qzss,
        }
    }
}

/// The `Constellation` member named by ``name``, e.g. ``"beidou"``.
///
/// Raises ``ValueError`` for a name outside the set.
#[pyfunction]
pub(crate) fn constellation_from_name(name: &str) -> PyResult<PyConstellation> {
    Constellation::try_from_lower_case(name)
        .map(PyConstellation::from)
        .map_err(errors::file_error)
}

/// Visual icon for a map annotation marker, mirroring
/// `geotrace_sdk.enums.MarkerIcon`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum PyMarkerIcon {
    Check,
    Circle,
    Cross,
    Download,
    Error,
    Gear,
    Lightning,
    Pin,
    Refresh,
    Satellite,
    SatelliteLost,
    Upload,
    Warning,
    Wrench,
}

impl PythonEnumMirror for PyMarkerIcon {
    const CLASS_NAME: &'static str = "MarkerIcon";

    fn class_lock() -> &'static PyOnceLock<Py<PyType>> {
        static CLASS: PyOnceLock<Py<PyType>> = PyOnceLock::new();
        &CLASS
    }
}

impl<'py> FromPyObject<'_, 'py> for PyMarkerIcon {
    type Error = PyErr;

    fn extract(object: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        Self::from_python_member(&object)
    }
}

impl<'py> IntoPyObject<'py> for PyMarkerIcon {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_python_member(py)
    }
}

impl From<MarkerIcon> for PyMarkerIcon {
    fn from(i: MarkerIcon) -> Self {
        match i {
            MarkerIcon::Pin => Self::Pin,
            MarkerIcon::Cross => Self::Cross,
            MarkerIcon::Circle => Self::Circle,
            MarkerIcon::Lightning => Self::Lightning,
            MarkerIcon::Warning => Self::Warning,
            MarkerIcon::Error => Self::Error,
            MarkerIcon::Check => Self::Check,
            MarkerIcon::Satellite => Self::Satellite,
            MarkerIcon::SatelliteLost => Self::SatelliteLost,
            MarkerIcon::Gear => Self::Gear,
            MarkerIcon::Refresh => Self::Refresh,
            MarkerIcon::Download => Self::Download,
            MarkerIcon::Upload => Self::Upload,
            MarkerIcon::Wrench => Self::Wrench,
        }
    }
}

impl From<PyMarkerIcon> for MarkerIcon {
    fn from(i: PyMarkerIcon) -> Self {
        match i {
            PyMarkerIcon::Pin => Self::Pin,
            PyMarkerIcon::Cross => Self::Cross,
            PyMarkerIcon::Circle => Self::Circle,
            PyMarkerIcon::Lightning => Self::Lightning,
            PyMarkerIcon::Warning => Self::Warning,
            PyMarkerIcon::Error => Self::Error,
            PyMarkerIcon::Check => Self::Check,
            PyMarkerIcon::Satellite => Self::Satellite,
            PyMarkerIcon::SatelliteLost => Self::SatelliteLost,
            PyMarkerIcon::Gear => Self::Gear,
            PyMarkerIcon::Refresh => Self::Refresh,
            PyMarkerIcon::Download => Self::Download,
            PyMarkerIcon::Upload => Self::Upload,
            PyMarkerIcon::Wrench => Self::Wrench,
        }
    }
}

/// The `MarkerIcon` member named by ``name``, e.g. ``"satellite_lost"``.
///
/// Raises ``ValueError`` for a name outside the set.
#[pyfunction]
pub(crate) fn marker_icon_from_name(name: &str) -> PyResult<PyMarkerIcon> {
    MarkerIcon::try_from_lower_case(name)
        .map(PyMarkerIcon::from)
        .map_err(errors::file_error)
}

/// Platform a recording was made on, declared by the recorder, mirroring
/// `geotrace_sdk.enums.TravelMode`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum PyTravelMode {
    Aircraft,
    Bicycle,
    Boat,
    Car,
    Motorcycle,
    Pedestrian,
    Rail,
}

impl PyTravelMode {
    /// [`crate::PyMeta::travel_mode`] exposes unknown modes as their recorded strings.
    pub(crate) fn from_travel_mode(mode: &TravelMode) -> Option<Self> {
        match mode {
            TravelMode::Car => Some(Self::Car),
            TravelMode::Motorcycle => Some(Self::Motorcycle),
            TravelMode::Bicycle => Some(Self::Bicycle),
            TravelMode::Pedestrian => Some(Self::Pedestrian),
            TravelMode::Boat => Some(Self::Boat),
            TravelMode::Rail => Some(Self::Rail),
            TravelMode::Aircraft => Some(Self::Aircraft),
            TravelMode::Unknown(_) => None,
        }
    }
}

impl PythonEnumMirror for PyTravelMode {
    const CLASS_NAME: &'static str = "TravelMode";

    fn class_lock() -> &'static PyOnceLock<Py<PyType>> {
        static CLASS: PyOnceLock<Py<PyType>> = PyOnceLock::new();
        &CLASS
    }
}

impl<'py> FromPyObject<'_, 'py> for PyTravelMode {
    type Error = PyErr;

    fn extract(object: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        Self::from_python_member(&object)
    }
}

impl<'py> IntoPyObject<'py> for PyTravelMode {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_python_member(py)
    }
}

impl From<PyTravelMode> for TravelMode {
    fn from(mode: PyTravelMode) -> Self {
        match mode {
            PyTravelMode::Car => Self::Car,
            PyTravelMode::Motorcycle => Self::Motorcycle,
            PyTravelMode::Bicycle => Self::Bicycle,
            PyTravelMode::Pedestrian => Self::Pedestrian,
            PyTravelMode::Boat => Self::Boat,
            PyTravelMode::Rail => Self::Rail,
            PyTravelMode::Aircraft => Self::Aircraft,
        }
    }
}

/// String arguments preserve unrecognized travel modes when rewriting a file.
#[derive(FromPyObject)]
pub(crate) enum TravelModeArg {
    Mode(PyTravelMode),
    Name(String),
}

impl From<TravelModeArg> for TravelMode {
    fn from(arg: TravelModeArg) -> Self {
        match arg {
            TravelModeArg::Mode(mode) => mode.into(),
            TravelModeArg::Name(name) => TravelMode::from_lower_case(name),
        }
    }
}

/// The name and the ``.value`` of every ``geotrace_sdk.enums`` member the Rust SDK
/// defines, keyed by class name, in the Rust SDK's variant order.
#[pyfunction(name = "_rust_sdk_enum_members")]
pub(crate) fn rust_sdk_enum_members()
-> PyResult<BTreeMap<&'static str, Vec<(&'static str, PythonEnumValue)>>> {
    let mut members = BTreeMap::from([
        (
            PyConstellation::CLASS_NAME,
            Constellation::iter()
                .map(|constellation| {
                    (
                        PyConstellation::from(constellation).into(),
                        PythonEnumValue::WireCode(constellation.wire_code()),
                    )
                })
                .collect(),
        ),
        (
            PyMarkerIcon::CLASS_NAME,
            MarkerIcon::iter()
                .map(|icon| {
                    (
                        PyMarkerIcon::from(icon).into(),
                        PythonEnumValue::WireCode(icon.wire_code()),
                    )
                })
                .collect(),
        ),
        (
            PyTravelMode::CLASS_NAME,
            TravelMode::iter()
                .filter_map(|mode| {
                    PyTravelMode::from_travel_mode(&mode).map(|python_mode| {
                        (
                            python_mode.into(),
                            PythonEnumValue::WireName(mode.name().to_owned()),
                        )
                    })
                })
                .collect(),
        ),
    ]);
    members.extend(navigation::enum_members()?);
    Ok(members)
}
