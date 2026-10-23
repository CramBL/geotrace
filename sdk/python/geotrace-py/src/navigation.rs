use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use geotrace_sdk::{
    ConstellationSet, PositionProvenance, SatelliteCoverage, SatelliteIdentity,
    SatelliteMembership, SatelliteObservation, SatelliteObservationReport, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport, SolutionDimension, SolutionMethod, SolutionState,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;
use strum::IntoEnumIterator as _;

use crate::enums::{PyConstellation, PythonEnumValue};
use crate::python_enum::PythonEnumMirror;
use crate::timestamps::{self, TimestampArguments};

macro_rules! navigation_enum {
    ($python:ident, $rust:ident, $name:literal, $($variant:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, strum::EnumString, strum::IntoStaticStr)]
        #[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
        pub(crate) enum $python {
            $($variant),+
        }

        impl $python {
            fn members() -> PyResult<Vec<(&'static str, PythonEnumValue)>> {
                $rust::iter()
                    .map(|value| {
                        Ok((
                            Self::try_from(value)?.into(),
                            PythonEnumValue::WireName(format!("{value:?}")),
                        ))
                    })
                    .collect()
            }
        }

        impl PythonEnumMirror for $python {
            const CLASS_NAME: &'static str = $name;

            fn class_lock() -> &'static PyOnceLock<Py<PyType>> {
                static CLASS: PyOnceLock<Py<PyType>> = PyOnceLock::new();
                &CLASS
            }
        }

        impl<'py> FromPyObject<'_, 'py> for $python {
            type Error = PyErr;

            fn extract(object: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
                Self::from_python_member(&object)
            }
        }

        impl<'py> IntoPyObject<'py> for $python {
            type Target = PyAny;
            type Output = Bound<'py, PyAny>;
            type Error = PyErr;

            fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
                self.to_python_member(py)
            }
        }

        impl From<$python> for $rust {
            fn from(value: $python) -> Self {
                match value {
                    $($python::$variant => Self::$variant),+
                }
            }
        }

        impl TryFrom<$rust> for $python {
            type Error = PyErr;

            fn try_from(value: $rust) -> PyResult<Self> {
                match value {
                    $($rust::$variant => Ok(Self::$variant),)+
                    _ => Err(PyValueError::new_err(concat!("unsupported ", $name))),
                }
            }
        }
    };
}

navigation_enum!(
    PySolutionState,
    SolutionState,
    "SolutionState",
    EstimatedOnly,
    Fix,
    NoFix,
    Unknown
);
navigation_enum!(
    PySolutionMethod,
    SolutionMethod,
    "SolutionMethod",
    Autonomous,
    DeadReckoning,
    Differential,
    Hybrid,
    Other,
    RtkFixed,
    RtkFloat
);
navigation_enum!(
    PySolutionDimension,
    SolutionDimension,
    "SolutionDimension",
    ThreeD,
    TwoD
);
navigation_enum!(
    PyPositionProvenance,
    PositionProvenance,
    "PositionProvenance",
    SdkSynthesized,
    SourceProvided
);
navigation_enum!(
    PySatelliteMembership,
    SatelliteMembership,
    "SatelliteMembership",
    Unknown,
    Unused,
    Used
);
navigation_enum!(
    PySatelliteCoverage,
    SatelliteCoverage,
    "SatelliteCoverage",
    Complete,
    Partial
);

/// Constellation and PRN identify one satellite within a report.
/// PRN values are unsigned 32-bit integers.
#[pyclass(frozen, skip_from_py_object, name = "SatelliteIdentity")]
#[derive(Clone, Debug)]
pub(crate) struct PySatelliteIdentity {
    inner: SatelliteIdentity,
}

#[pymethods]
impl PySatelliteIdentity {
    /// PRN values are unsigned 32-bit integers.
    #[new]
    fn new(constellation: PyConstellation, prn: u32) -> Self {
        Self {
            inner: SatelliteIdentity::new(constellation.into(), prn),
        }
    }

    /// GNSS constellation of the satellite.
    #[getter]
    fn constellation(&self) -> PyConstellation {
        self.inner.constellation.into()
    }

    /// Unsigned 32-bit identifier within the constellation.
    #[getter]
    fn prn(&self) -> u32 {
        self.inner.prn
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Global scope remains distinct from an explicit set of all constellations.
#[pyclass(frozen, skip_from_py_object, name = "SatelliteScope")]
#[derive(Clone, Debug)]
pub(crate) struct PySatelliteScope {
    inner: SatelliteScope,
}

#[pymethods]
impl PySatelliteScope {
    /// Declare global scope independently of the identity list.
    #[staticmethod]
    fn global_() -> Self {
        Self {
            inner: SatelliteScope::Global,
        }
    }

    /// An empty list raises `ValueError`.
    /// Duplicate entries are removed in Rust declaration order.
    #[staticmethod]
    fn constellations(values: Vec<PyConstellation>) -> PyResult<Self> {
        let mut values = values.into_iter().map(Into::into);
        let first = values
            .next()
            .ok_or_else(|| PyValueError::new_err("scope requires at least one constellation"))?;
        Ok(Self {
            inner: SatelliteScope::Constellations(ConstellationSet::new(first, values)),
        })
    }

    /// True only for explicitly declared global scope.
    #[getter]
    fn is_global(&self) -> bool {
        self.inner == SatelliteScope::Global
    }

    /// Declared constellations in Rust declaration order, or an empty list for global scope.
    #[getter]
    fn constellation_set(&self) -> Vec<PyConstellation> {
        match &self.inner {
            SatelliteScope::Constellations(values) => {
                values.as_slice().iter().copied().map(Into::into).collect()
            }
            _ => Vec::new(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// One identity's source-reported used, unused or unknown membership.
#[pyclass(frozen, skip_from_py_object, name = "SatelliteUsage")]
#[derive(Clone, Debug)]
pub(crate) struct PySatelliteUsage {
    inner: SatelliteUsage,
}

#[pymethods]
impl PySatelliteUsage {
    /// Membership is required independently of observation measurements.
    #[new]
    fn new(identity: &PySatelliteIdentity, membership: PySatelliteMembership) -> Self {
        Self {
            inner: SatelliteUsage::new(identity.inner, membership.into()),
        }
    }

    /// Constellation and PRN for this membership row.
    #[getter]
    fn identity(&self) -> PySatelliteIdentity {
        PySatelliteIdentity {
            inner: self.inner.identity,
        }
    }

    /// `USED` and `UNUSED` are known memberships.
    /// `UNKNOWN` remains uncertain under complete coverage.
    #[getter]
    fn membership(&self) -> PyResult<PySatelliteMembership> {
        self.inner.membership.try_into()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Each geometry and signal measurement is optional. A measured zero remains present.
/// Measurements use 32-bit floats.
#[pyclass(frozen, skip_from_py_object, name = "SatelliteObservation")]
#[derive(Clone, Debug)]
pub(crate) struct PySatelliteObservation {
    inner: SatelliteObservation,
}

#[pymethods]
impl PySatelliteObservation {
    /// Geometry uses degrees and signal uses dB-Hz.
    /// Measurements are stored as 32-bit floats.
    #[new]
    #[pyo3(signature = (identity, *, elevation=None, azimuth=None, snr=None))]
    fn new(
        identity: &PySatelliteIdentity,
        elevation: Option<f32>,
        azimuth: Option<f32>,
        snr: Option<f32>,
    ) -> Self {
        Self {
            inner: SatelliteObservation::builder()
                .identity(identity.inner)
                .maybe_elevation(elevation)
                .maybe_azimuth(azimuth)
                .maybe_snr(snr)
                .build(),
        }
    }

    /// Constellation and PRN for this observation row.
    #[getter]
    fn identity(&self) -> PySatelliteIdentity {
        PySatelliteIdentity {
            inner: self.inner.identity,
        }
    }

    /// Degrees above the horizon, or `None` when unreported.
    #[getter]
    fn elevation(&self) -> Option<f32> {
        self.inner.elevation
    }

    /// Degrees clockwise from true north, or `None` when unreported.
    #[getter]
    fn azimuth(&self) -> Option<f32> {
        self.inner.azimuth
    }

    /// Signal in dB-Hz, or `None` when unreported.
    /// A measured zero remains present.
    #[getter]
    fn snr(&self) -> Option<f32> {
        self.inner.snr
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

macro_rules! evidence_report {
    ($python:ident, $rust:ident, $name:literal, $row:ident, $coverage_doc:literal) => {
        /// Independent satellite report with original source clocks and ordered rows.
        /// Equality includes source rows, metadata and optional recorder placement.
        /// At least one timezone-aware source timestamp is required.
        #[pyclass(frozen, skip_from_py_object, name = $name)]
        #[derive(Clone, Debug)]
        pub(crate) struct $python {
            pub(crate) inner: $rust,
        }

        #[pymethods]
        impl $python {
            /// Copy ordered rows with independent source clocks.
            /// At least one timezone-aware timestamp is required.
            #[new]
            #[pyo3(signature = (satellites, *, scope, coverage, reported_count=None, gps_time=None, sys_time=None))]
            fn new(
                satellites: Vec<PyRef<'_, $row>>,
                scope: &PySatelliteScope,
                coverage: PySatelliteCoverage,
                reported_count: Option<u32>,
                gps_time: Option<DateTime<FixedOffset>>,
                sys_time: Option<DateTime<FixedOffset>>,
            ) -> PyResult<Self> {
                let time = timestamps::nav_fix_time_or_value_error(TimestampArguments { gps_time, sys_time })?;
                Ok(Self {
                    inner: $rust::builder()
                        .time(time)
                        .scope(scope.inner.clone())
                        .coverage(coverage.into())
                        .maybe_reported_count(reported_count)
                        .satellites(satellites.iter().map(|row| row.inner).collect())
                        .build(),
                })
            }

            /// Copies of the source rows in input order, including repeated identities.
            #[getter]
            fn satellites(&self) -> Vec<$row> {
                self.inner.satellites.iter().map(|row| $row { inner: *row }).collect()
            }

            /// Scope of both identity coverage and the reported aggregate.
            #[getter]
            fn scope(&self) -> PySatelliteScope {
                PySatelliteScope { inner: self.inner.scope.clone() }
            }

            #[doc = $coverage_doc]
            #[getter]
            fn coverage(&self) -> PyResult<PySatelliteCoverage> {
                self.inner.coverage.try_into()
            }

            /// Exact source count for scope, independent of list coverage or conflicting rows.
            /// `None` means unreported.
            #[getter]
            fn reported_count(&self) -> Option<u32> {
                self.inner.reported_count
            }

            /// Original receiver timestamp in UTC, or `None` when unreported.
            #[getter]
            fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
                self.inner.time.gps_time().map(|time| time.fixed_offset())
            }

            /// Original host timestamp in UTC, or `None` when unreported.
            #[getter]
            fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
                self.inner.time.sys_time().map(|time| time.fixed_offset())
            }

            /// Placement timestamp on the common navigation clock,
            /// or `None` when placement is unreported.
            #[getter]
            fn normalized_time(&self) -> Option<DateTime<FixedOffset>> {
                self.inner.placement.map(|placement| placement.normalized_time.fixed_offset())
            }

            /// Zero-based insertion order across usage and observation reports,
            /// or `None` when placement is unreported.
            #[getter]
            fn input_order(&self) -> Option<u64> {
                self.inner.placement.map(|placement| placement.input_order)
            }

            fn __eq__(&self, other: &Self) -> bool {
                self.inner == other.inner
            }
        }
    };
}

evidence_report!(
    PySatelliteUsageReport,
    SatelliteUsageReport,
    "SatelliteUsageReport",
    PySatelliteUsage,
    "COMPLETE makes omitted identities unused within scope. Explicit UNKNOWN rows remain unknown."
);
evidence_report!(
    PySatelliteObservationReport,
    SatelliteObservationReport,
    "SatelliteObservationReport",
    PySatelliteObservation,
    "COMPLETE declares every observed identity in scope. PARTIAL leaves omitted identities unreported."
);

pub(crate) fn enum_members()
-> PyResult<BTreeMap<&'static str, Vec<(&'static str, PythonEnumValue)>>> {
    Ok(BTreeMap::from([
        (PySolutionState::CLASS_NAME, PySolutionState::members()?),
        (PySolutionMethod::CLASS_NAME, PySolutionMethod::members()?),
        (
            PySolutionDimension::CLASS_NAME,
            PySolutionDimension::members()?,
        ),
        (
            PyPositionProvenance::CLASS_NAME,
            PyPositionProvenance::members()?,
        ),
        (
            PySatelliteMembership::CLASS_NAME,
            PySatelliteMembership::members()?,
        ),
        (
            PySatelliteCoverage::CLASS_NAME,
            PySatelliteCoverage::members()?,
        ),
    ]))
}
