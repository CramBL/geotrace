use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use chrono::{DateTime, FixedOffset};
use geotrace_sdk::{Angle, Channel, ChannelUnit, Unit, Velocity};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// A recognized channel unit or an explicit display-only custom unit.
///
/// A recognized unit is one of the :class:`Unit` catalog entries: it has a
/// physical quantity and a conversion factor, so a GeoTrace query compares it
/// against unit literals. A custom unit is any other label, stored and shown
/// verbatim with its values dimensionless in queries.
///
/// A unit read back from a file is also a ``ChannelUnit``. ``is_custom`` is
/// true for a label that is not a catalog unit, including a legacy label an
/// older writer stored, which raises ``ValueError`` when written again.
#[pyclass(eq, skip_from_py_object, name = "ChannelUnit")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PyChannelUnit {
    inner: ChannelUnit,
}

#[pymethods]
impl PyChannelUnit {
    /// Parse a catalog unit label, which queries convert and compare.
    ///
    /// Aliases resolve to the canonical spelling: ``"kph"`` is ``"km/h"``,
    /// ``"degrees"`` is ``"deg"``, ``"m/s²"`` is ``"m/s2"``. A label outside
    /// the catalog raises ``ValueError``: store it with :meth:`custom`.
    #[staticmethod]
    fn recognized(label: &str) -> PyResult<Self> {
        label
            .parse::<ChannelUnit>()
            .map(|inner| Self { inner })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// Construct a display-only unit whose values are dimensionless in queries.
    ///
    /// The label is trimmed. ``ValueError`` is raised for an empty label, a
    /// label with a control character, or a label that spells a catalog unit:
    /// declare that one with :meth:`recognized`.
    #[staticmethod]
    fn custom(label: &str) -> PyResult<Self> {
        ChannelUnit::custom(label)
            .map(|inner| Self { inner })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// The canonical label as stored in the file.
    #[getter]
    fn label(&self) -> String {
        self.inner.to_string()
    }

    /// True for a display-only custom label, false for a catalog unit.
    #[getter]
    fn is_custom(&self) -> bool {
        self.inner.as_recognized().is_none()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("ChannelUnit({:?})", self.inner.to_string())
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }
}

/// A recognized, convertible channel unit.
///
/// The catalog is exposed as class attributes named after the canonical
/// spelling with ``/`` written as ``_PER_``: ``Unit.KM_PER_H`` is ``km/h``,
/// ``Unit.M_PER_S2`` is ``m/s2``, ``Unit.PER_MIN`` is ``per min``. ``Unit.G``,
/// ``Unit.MG`` and ``Unit.UG`` are standard gravity and its sub-scales.
///
/// Pass one straight to :class:`Channel`, or ``Unit.MG.label`` for its
/// spelling.
#[pyclass(eq, skip_from_py_object, name = "Unit")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PyUnit {
    inner: Unit,
}

include!("unit_catalog.rs");

/// A named scalar or vector sensor channel sampled at its own rate.
///
/// Pass `components` (e.g. `["x", "y", "z"]`) for a vector channel whose values
/// have one column per component, or omit it for a scalar channel. `values` is
/// row-major: `len(times)` rows of one column (scalar) or `len(components)`
/// columns (vector). `times` must be timezone-aware `datetime.datetime` objects.
///
/// `unit` takes a :class:`Unit` catalog entry, a recognized label such as
/// `"km/h"` or `"degrees"`, or a :class:`ChannelUnit` from
/// :meth:`ChannelUnit.custom` for a label outside the catalog. A label that is
/// neither a catalog unit nor wrapped in `ChannelUnit.custom` raises
/// `ValueError`.
#[pyclass(skip_from_py_object, name = "Channel")]
#[derive(Clone, Debug)]
pub struct PyChannel {
    pub(crate) inner: Channel,
}

#[pymethods]
impl PyChannel {
    #[new]
    #[pyo3(signature = (name, times, values, *, unit=None, period_deg=None, description=None, components=None))]
    fn new(
        name: String,
        times: Vec<DateTime<FixedOffset>>,
        values: Vec<f64>,
        unit: Option<&Bound<'_, PyAny>>,
        period_deg: Option<f64>,
        description: Option<String>,
        components: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let unit = unit.map(parse_python_channel_unit).transpose()?;
        let inner = Channel::builder()
            .name(name)
            .maybe_unit(unit)
            .maybe_period(period_deg.map(Angle::degrees))
            .maybe_description(description)
            .maybe_components(components)
            .times(times.into_iter().map(|t| t.to_utc()).collect())
            .values(values)
            .build()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self { inner })
    }

    /// The channel's identifier, referenced as `@name` in queries.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// The declared unit as a :class:`ChannelUnit`, or `None`.
    #[getter]
    fn unit(&self) -> Option<PyChannelUnit> {
        self.inner
            .unit()
            .cloned()
            .map(|inner| PyChannelUnit { inner })
    }

    /// The wrap period in degrees for an angular channel, or `None` if linear.
    ///
    /// A `deg` channel without a period holds an unbounded angle.
    #[getter]
    fn period_deg(&self) -> Option<f64> {
        self.inner.period().map(Angle::as_degrees)
    }

    /// A human description of what the channel records, or `None`.
    #[getter]
    fn description(&self) -> Option<&str> {
        self.inner.description()
    }

    /// The vector component labels, or an empty list for a scalar channel.
    #[getter]
    fn components(&self) -> Vec<String> {
        self.inner.components().to_vec()
    }

    /// Whether this is a vector channel (has named components).
    #[getter]
    fn is_vector(&self) -> bool {
        self.inner.is_vector()
    }

    /// The sample timestamps, one per row of `values`.
    #[getter]
    fn times(&self) -> Vec<DateTime<FixedOffset>> {
        self.inner
            .times()
            .iter()
            .map(|t| crate::to_fixed(*t))
            .collect()
    }

    /// The sample values, row-major: `len(times)` rows by component count.
    #[getter]
    fn values(&self) -> Vec<f64> {
        self.inner.values().to_vec()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!(
            "Channel(name={:?}, samples={}, components={})",
            self.inner.name(),
            self.inner.times().len(),
            self.inner.components().len()
        )
    }
}

impl From<&Channel> for PyChannel {
    fn from(channel: &Channel) -> Self {
        Self {
            inner: channel.clone(),
        }
    }
}

/// Convert a speed in km/h to m/s, the unit ``NavFix.speed_mps`` takes.
///
/// Every SDK converts ``kmh`` to the same float. ``kmh / 3.6`` differs from it
/// in the last place for some values, 23.2 among them.
#[pyfunction]
pub(crate) fn mps_from_kmh(kmh: f64) -> f64 {
    Velocity::kilometer_per_hour(kmh).as_meters_per_second()
}

/// Convert a speed in knots to m/s, the unit ``NavFix.speed_mps`` takes.
///
/// Every SDK converts ``knots`` to the same float.
#[pyfunction]
pub(crate) fn mps_from_knots(knots: f64) -> f64 {
    Velocity::knot(knots).as_meters_per_second()
}

/// Convert a speed in m/s to km/h.
#[pyfunction]
pub(crate) fn kmh_from_mps(mps: f64) -> f64 {
    Velocity::meter_per_second(mps).as_kilometers_per_hour()
}

/// Convert a speed in m/s to knots.
#[pyfunction]
pub(crate) fn knots_from_mps(mps: f64) -> f64 {
    Velocity::meter_per_second(mps).as_knots()
}

fn parse_python_channel_unit(value: &Bound<'_, PyAny>) -> PyResult<ChannelUnit> {
    if let Ok(unit) = value.extract::<PyRef<'_, PyUnit>>() {
        return Ok(unit.inner.into());
    }
    if let Ok(unit) = value.extract::<PyRef<'_, PyChannelUnit>>() {
        return Ok(unit.inner.clone());
    }
    let label = value.extract::<String>().map_err(|_| {
        PyValueError::new_err("unit must be a recognized string or ChannelUnit.custom(...)")
    })?;
    label
        .parse::<ChannelUnit>()
        .map_err(|error| PyValueError::new_err(error.to_string()))
}
