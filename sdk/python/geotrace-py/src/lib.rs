//! Python bindings for `geotrace-sdk`.
//!
//! Exposes the core read/write API as a Python extension module named
//! `geotrace_sdk._geotrace_sdk`.  The public `geotrace_sdk` package re-exports
//! everything from this module via `python/geotrace_sdk/__init__.py`.

use std::collections::hash_map::DefaultHasher;
use std::ffi::CString;
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, Utc};
use geotrace_sdk::{
    Angle, Annotation, AnnotationIcon, BuildError, Channel, ChannelUnit, Constellation,
    EventMarker, EventMarkerColor, EventMarkerIconChoice, EventMarkerPoint, EventMarkerStyle,
    Marker, MarkerIcon, Meta, NavFile, NavFileBuilder, NavFix, NavFixTime, NavPoint, NavRecorder,
    RecordedFixTimestamps, Satellite, SatelliteReport, TravelMode, Unit, Velocity,
};
use pyo3::exceptions::{PyIOError, PyRuntimeError, PyUserWarning, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_log::{Caching, Logger};

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

/// GNSS constellation identifier.
#[pyclass(eq, from_py_object, name = "Constellation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyConstellation {
    #[pyo3(name = "GPS")]
    Gps,
    #[pyo3(name = "GLONASS")]
    Glonass,
    #[pyo3(name = "GALILEO")]
    Galileo,
    #[pyo3(name = "BEIDOU")]
    Beidou,
    #[pyo3(name = "NAVIC")]
    Navic,
    #[pyo3(name = "QZSS")]
    Qzss,
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

#[cfg(test)]
mod py_constellation_tests {
    use super::*;

    /// `Satellite::__repr__` derives the Python member name from
    /// `PyConstellation`'s `Debug` output (upper-cased).
    /// This pins down that the result still matches the literal
    /// `#[pyo3(name = "...")]` strings declared on `PyConstellation` above. A
    /// rename of one without the other - the exact desync this issue is
    /// about - fails here.
    #[test]
    fn repr_name_matches_pyo3_name() {
        for (variant, pyo3_name) in [
            (PyConstellation::Gps, "GPS"),
            (PyConstellation::Glonass, "GLONASS"),
            (PyConstellation::Galileo, "GALILEO"),
            (PyConstellation::Beidou, "BEIDOU"),
            (PyConstellation::Navic, "NAVIC"),
            (PyConstellation::Qzss, "QZSS"),
        ] {
            assert_eq!(format!("{variant:?}").to_uppercase(), pyo3_name);
        }
    }
}

/// Visual icon for a map annotation marker.
#[pyclass(eq, from_py_object, name = "MarkerIcon")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyMarkerIcon {
    #[pyo3(name = "PIN")]
    Pin,
    #[pyo3(name = "CROSS")]
    Cross,
    #[pyo3(name = "CIRCLE")]
    Circle,
    #[pyo3(name = "LIGHTNING")]
    Lightning,
    #[pyo3(name = "WARNING")]
    Warning,
    #[pyo3(name = "ERROR")]
    Error,
    #[pyo3(name = "CHECK")]
    Check,
    #[pyo3(name = "SATELLITE")]
    Satellite,
    #[pyo3(name = "SATELLITE_LOST")]
    SatelliteLost,
    #[pyo3(name = "GEAR")]
    Gear,
    #[pyo3(name = "REFRESH")]
    Refresh,
    #[pyo3(name = "DOWNLOAD")]
    Download,
    #[pyo3(name = "UPLOAD")]
    Upload,
    #[pyo3(name = "WRENCH")]
    Wrench,
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

/// Platform a recording was made on, declared by the recorder.
#[pyclass(eq, from_py_object, name = "TravelMode")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyTravelMode {
    #[pyo3(name = "CAR")]
    Car,
    #[pyo3(name = "MOTORCYCLE")]
    Motorcycle,
    #[pyo3(name = "BICYCLE")]
    Bicycle,
    #[pyo3(name = "PEDESTRIAN")]
    Pedestrian,
    #[pyo3(name = "BOAT")]
    Boat,
    #[pyo3(name = "RAIL")]
    Rail,
    #[pyo3(name = "AIRCRAFT")]
    Aircraft,
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

impl PyTravelMode {
    /// [`TravelMode::Unknown`] has no Python `enum` member - its preserved wire
    /// value crosses into Python as a plain `str` instead (see
    /// [`PyMeta::travel_mode`]).
    fn from_travel_mode(mode: &TravelMode) -> Option<Self> {
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

/// `Meta(travel_mode=...)` accepts the `enum` or a wire-name string, so values
/// read back as a preserved unknown `str` can be written again unchanged.
#[derive(FromPyObject)]
enum TravelModeArg {
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

/// One tracked satellite with optional signal metrics.
///
/// `elevation`, `azimuth` and `snr` are stored as 32-bit floats: a value read
/// back can differ from the value written in its eighth significant digit.
#[pyclass(from_py_object, name = "Satellite")]
#[derive(Debug, Clone, Copy)]
pub struct PySatellite {
    inner: Satellite,
}

#[pymethods]
impl PySatellite {
    /// Create a satellite entry.
    ///
    /// `elevation` and `azimuth` are in degrees. `snr` is in dB-Hz.
    #[new]
    #[pyo3(signature = (constellation, prn, *, in_fix=false, elevation=None, azimuth=None, snr=None))]
    fn new(
        constellation: PyConstellation,
        prn: u32,
        in_fix: bool,
        elevation: Option<f32>,
        azimuth: Option<f32>,
        snr: Option<f32>,
    ) -> Self {
        let inner = Satellite::builder()
            .constellation(Constellation::from(constellation))
            .prn(prn)
            .in_fix(in_fix)
            .maybe_elevation(elevation)
            .maybe_azimuth(azimuth)
            .maybe_snr(snr)
            .build();
        Self { inner }
    }

    #[getter]
    fn constellation(&self) -> PyConstellation {
        self.inner.constellation.into()
    }

    #[getter]
    fn prn(&self) -> u32 {
        self.inner.prn
    }

    #[getter]
    fn in_fix(&self) -> bool {
        self.inner.in_fix
    }

    /// Elevation above horizon in degrees, or `None`.
    #[getter]
    fn elevation(&self) -> Option<f32> {
        self.inner.elevation
    }

    /// Azimuth from true north in degrees, or `None`.
    #[getter]
    fn azimuth(&self) -> Option<f32> {
        self.inner.azimuth
    }

    /// Signal-to-noise ratio in dB-Hz, or `None`.
    #[getter]
    fn snr(&self) -> Option<f32> {
        self.inner.snr
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.constellation == other.inner.constellation
            && self.inner.prn == other.inner.prn
            && self.inner.in_fix == other.inner.in_fix
            && self.inner.elevation == other.inner.elevation
            && self.inner.azimuth == other.inner.azimuth
            && self.inner.snr == other.inner.snr
    }

    fn __repr__(&self) -> String {
        // Upper-cased `Debug` of the *Python-facing* enum's variant identifier -
        // derived from the same identifiers `#[pyo3(name = "...")]` spells out
        // below, so a rename can't desync `__repr__` from the actual Python
        // member name (see `py_constellation_repr_name_matches_pyo3_name`).
        let name = format!("{:?}", PyConstellation::from(self.inner.constellation)).to_uppercase();
        format!(
            "Satellite(constellation=Constellation.{name}, prn={})",
            self.inner.prn
        )
    }
}

/// A set of satellites tracked at a point in time.
///
/// Raises `ValueError` when `gps_time` and `sys_time` are both `None`.
/// Both must be timezone-aware `datetime.datetime` objects.
#[pyclass(skip_from_py_object, name = "SatelliteReport")]
#[derive(Debug, Clone)]
pub struct PySatelliteReport {
    inner: SatelliteReport,
}

#[pymethods]
impl PySatelliteReport {
    #[new]
    #[pyo3(signature = (tracked, *, gps_time=None, sys_time=None))]
    fn new(
        tracked: Vec<PySatellite>,
        gps_time: Option<DateTime<FixedOffset>>,
        sys_time: Option<DateTime<FixedOffset>>,
    ) -> PyResult<Self> {
        let time = nav_fix_time_or_value_error(TimestampArguments { gps_time, sys_time })?;
        let inner = SatelliteReport::builder()
            .time(time)
            .tracked(tracked.into_iter().map(|s| s.inner).collect())
            .build();
        Ok(Self { inner })
    }

    /// All satellites currently tracked (may include satellites not in the fix).
    #[getter]
    fn tracked(&self) -> Vec<PySatellite> {
        self.inner
            .tracked
            .iter()
            .map(|s| PySatellite { inner: *s })
            .collect()
    }

    /// GPS-domain timestamp (timezone-aware UTC), or `None` for a report
    /// constructed without one, or read from a file that stores none.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.gps_time().map(to_fixed)
    }

    /// System-clock timestamp at capture time (timezone-aware UTC), or `None`
    /// for a report constructed without one, or read from a file that stores
    /// none.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sys_time().map(to_fixed)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.gps_time() == other.inner.gps_time()
            && self.inner.sys_time() == other.inner.sys_time()
            && self.inner.tracked.len() == other.inner.tracked.len()
            && self
                .inner
                .tracked
                .iter()
                .zip(other.inner.tracked.iter())
                .all(|(a, b)| {
                    a.constellation == b.constellation
                        && a.prn == b.prn
                        && a.in_fix == b.in_fix
                        && a.elevation == b.elevation
                        && a.azimuth == b.azimuth
                        && a.snr == b.snr
                })
    }

    fn __repr__(&self) -> String {
        format!("SatelliteReport(tracked={})", self.inner.tracked.len())
    }
}

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
pub struct PyChannel {
    inner: Channel,
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
        self.inner.times().iter().map(|t| to_fixed(*t)).collect()
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

/// A single GPS/GNSS fix: position, optional heading, and optional speed.
///
/// Raises `ValueError` when `gps_time` and `sys_time` are both `None`.
/// All `datetime` arguments must be timezone-aware.
/// `lat` is expected in [-90, 90] degrees, `lon` in [-180, 180], `heading` in
/// [0, 360), `speed_mps` in m/s and `eph_m` in metres, both non-negative.
/// These are data quality expectations, not parse rules.
/// The SDK writes every value it is given, NaN included: a recorder that
/// captured bad data must be able to write it.
/// An absent `heading`, `speed_mps` or `eph_m` is written as NaN: a NaN given
/// for one of the three reads back as `None`.
#[pyclass(skip_from_py_object, name = "NavFix")]
#[derive(Debug, Clone, Copy)]
pub struct PyNavFix {
    inner: NavFix,
}

#[pymethods]
impl PyNavFix {
    #[new]
    #[pyo3(signature = (lat, lon, *, gps_time=None, sys_time=None, heading=None, speed_mps=None, eph_m=None))]
    fn new(
        lat: f64,
        lon: f64,
        gps_time: Option<DateTime<FixedOffset>>,
        sys_time: Option<DateTime<FixedOffset>>,
        heading: Option<f64>,
        speed_mps: Option<f64>,
        eph_m: Option<f64>,
    ) -> PyResult<Self> {
        let time = nav_fix_time_or_value_error(TimestampArguments { gps_time, sys_time })?;
        let inner = NavFix::builder()
            .time(time)
            .lat(Angle::degrees(lat))
            .lon(Angle::degrees(lon))
            .maybe_heading(heading.map(Angle::degrees))
            .maybe_speed(speed_mps.map(Velocity::meter_per_second))
            .maybe_eph_m(eph_m)
            .build();
        Ok(Self { inner })
    }

    /// Latitude in degrees, expected in [-90, 90].
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.lat.as_degrees()
    }

    /// Longitude in degrees, expected in [-180, 180].
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.lon.as_degrees()
    }

    /// GPS-receiver timestamp (timezone-aware UTC), or `None` when the receiver
    /// had no lock.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.gps_time().map(to_fixed)
    }

    /// System-clock timestamp (timezone-aware UTC), or `None` when the recorder
    /// did not supply one.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sys_time().map(to_fixed)
    }

    /// Heading in degrees, expected in [0, 360), or `None` for
    /// ghost/unknown-direction fixes.
    #[getter]
    fn heading(&self) -> Option<f64> {
        self.inner.heading.map(|h| h.as_degrees())
    }

    /// Speed in m/s, expected to be non-negative, or `None`.
    #[getter]
    fn speed_mps(&self) -> Option<f64> {
        self.inner.speed.map(|s| s.as_meters_per_second())
    }

    /// Estimated horizontal accuracy radius in metres, expected to be
    /// non-negative, or `None`.
    #[getter]
    fn eph_m(&self) -> Option<f64> {
        self.inner.eph_m
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.gps_time() == other.inner.gps_time()
            && self.inner.sys_time() == other.inner.sys_time()
            && self.inner.lat.as_degrees() == other.inner.lat.as_degrees()
            && self.inner.lon.as_degrees() == other.inner.lon.as_degrees()
            && self.inner.heading.map(|h| h.as_degrees())
                == other.inner.heading.map(|h| h.as_degrees())
            && self.inner.speed.map(|s| s.as_meters_per_second())
                == other.inner.speed.map(|s| s.as_meters_per_second())
            && self.inner.eph_m == other.inner.eph_m
    }

    fn __repr__(&self) -> String {
        format!(
            "NavFix(lat={:.6}, lon={:.6})",
            self.inner.lat.as_degrees(),
            self.inner.lon.as_degrees(),
        )
    }
}

/// A user-defined map annotation with an optional label and an icon.
///
/// `time` must be a timezone-aware `datetime.datetime`. A `label` longer than
/// the 255 bytes the `markers/label` field holds raises `ValueError`.
#[pyclass(skip_from_py_object, name = "Annotation")]
#[derive(Debug, Clone)]
pub struct PyAnnotation {
    inner: Annotation,
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
            .map_err(file_err)?;
        Ok(Self { inner })
    }

    /// Timestamp (timezone-aware UTC).
    #[getter]
    fn time(&self) -> DateTime<FixedOffset> {
        to_fixed(self.inner.time())
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

/// Optional file-level metadata for a `.gtd` file.
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
    ) -> Self {
        let inner = Meta::builder()
            .maybe_title(title)
            .maybe_device(device)
            .maybe_notes(notes)
            .maybe_identity(identity)
            .maybe_travel_mode(travel_mode.map(TravelMode::from))
            .build();
        Self { inner }
    }

    /// File title, or `None`.
    #[getter]
    fn title(&self) -> Option<&str> {
        self.inner.title.as_deref()
    }

    /// Sensor or device that produced the data, or `None`.
    #[getter]
    fn device(&self) -> Option<&str> {
        self.inner.device.as_deref()
    }

    /// Free-text notes, or `None`.
    #[getter]
    fn notes(&self) -> Option<&str> {
        self.inner.notes.as_deref()
    }

    /// Opaque producer identity string, or `None`.
    #[getter]
    fn identity(&self) -> Option<&str> {
        self.inner.identity.as_deref()
    }

    /// Platform the recording was made on: a `TravelMode`, or the raw wire
    /// name as `str` when it is outside the known set (preserved, never
    /// dropped), or `None` when absent.
    #[getter]
    fn travel_mode<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        match &self.inner.travel_mode {
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
        format!("Meta(title={:?})", self.inner.title)
    }
}

/// A nav fix combined with its associated satellite report, as read from a file.
///
/// The value ranges of `NavFix` apply here too, as expectations.
/// The SDK returns `lat` and `lon` unchanged, NaN included.
/// A NaN `heading`, `speed_mps` or `eph_m` is returned as `None`: NaN is how the
/// write path stores an absent one.
/// Checking a value against its range is the caller's job.
#[pyclass(skip_from_py_object, name = "NavPoint")]
#[derive(Debug, Clone)]
pub struct PyNavPoint {
    inner: NavPoint,
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

    /// GPS-receiver timestamp (timezone-aware UTC), or `None` when the receiver
    /// had no lock.
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

    /// Associated satellite report, or `None` if none was recorded at this fix.
    #[getter]
    fn satellites(&self) -> Option<PySatelliteReport> {
        self.inner
            .satellites
            .clone()
            .map(|s| PySatelliteReport { inner: s })
    }

    fn __repr__(&self) -> String {
        format!(
            "NavPoint(lat={:.6}, lon={:.6})",
            self.inner.fix.lat.as_degrees(),
            self.inner.fix.lon.as_degrees(),
        )
    }
}

/// A map annotation with its interpolated position on the nav track.
#[pyclass(skip_from_py_object, name = "Marker")]
#[derive(Debug, Clone)]
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
        to_fixed(self.inner.annotation.time())
    }

    fn __repr__(&self) -> String {
        format!(
            "Marker(lat={:.6}, lon={:.6})",
            self.inner.lat.as_degrees(),
            self.inner.lon.as_degrees(),
        )
    }
}

/// An event marker to add to the nav track.
///
/// ``variant_path`` is a slash-separated hierarchy, e.g. ``"power/boot"`` or
/// ``"connectivity/agps/request"``, or ``None`` (or the ``event_kind.skip``
/// sentinel value) to silently skip this marker.
/// Allowed characters: ASCII alphanumeric, hyphen, underscore, and slash.
/// No leading or trailing slash. No empty segments (``//``). Max 255 bytes.
/// ``annotation`` holds at most 511 bytes, checked when ``NavFileBuilder.add()``
/// takes the marker.
///
/// ``sys_time`` must be a timezone-aware ``datetime.datetime``.
#[pyclass(skip_from_py_object, name = "EventMarker")]
#[derive(Debug, Clone)]
pub struct PyEventMarker {
    variant_path: Option<String>,
    sys_time: DateTime<Utc>,
    annotation: Option<String>,
}

#[pymethods]
impl PyEventMarker {
    /// Create an ``EventMarker``.
    ///
    /// ``variant_path`` may be a path string, ``None``, or the
    /// ``event_kind.skip`` sentinel value - the latter two are treated as a
    /// silent no-op when passed to ``NavFileBuilder.add()``.
    #[new]
    #[pyo3(signature = (variant_path, sys_time, *, annotation=None))]
    fn new(
        variant_path: Option<Bound<'_, PyAny>>,
        sys_time: DateTime<FixedOffset>,
        annotation: Option<String>,
    ) -> PyResult<Self> {
        // Accept None or any non-string value (e.g. the skip sentinel value) as None.
        let path = variant_path.and_then(|v| v.extract::<String>().ok());
        // Validate the path if one is supplied.
        if let Some(ref p) = path {
            geotrace_sdk::EventMarker::builder()
                .variant_path(p.as_str())
                .sys_time(sys_time.to_utc())
                .build()
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
        }
        Ok(Self {
            variant_path: path,
            sys_time: sys_time.to_utc(),
            annotation,
        })
    }

    #[getter]
    fn variant_path(&self) -> Option<&str> {
        self.variant_path.as_deref()
    }

    #[getter]
    fn sys_time(&self) -> DateTime<FixedOffset> {
        to_fixed(self.sys_time)
    }

    #[getter]
    fn annotation(&self) -> Option<&str> {
        self.annotation.as_deref()
    }

    fn __repr__(&self) -> String {
        format!("EventMarker(variant_path={:?})", self.variant_path)
    }
}

/// Per-variant icon and color style stored in the file.
///
/// ``variant_path`` must exactly match a path used in an event marker.
/// ``icon`` is a :class:`MarkerIcon` value, or ``None`` for the application default (Pin).
/// ``color`` is ``#RRGGBB``, e.g. ``"#FF9900"``, or ``None`` for the deterministic hash color.
#[pyclass(skip_from_py_object, name = "EventMarkerStyle")]
#[derive(Debug, Clone)]
pub struct PyEventMarkerStyle {
    variant_path: String,
    icon: EventMarkerIconChoice,
    color: Option<String>,
}

#[pymethods]
impl PyEventMarkerStyle {
    #[new]
    #[pyo3(signature = (variant_path, *, icon=None, color=None))]
    fn new(variant_path: String, icon: Option<PyMarkerIcon>, color: Option<String>) -> Self {
        Self {
            variant_path,
            icon: EventMarkerIconChoice::from(icon.map(MarkerIcon::from)),
            color,
        }
    }

    #[getter]
    fn variant_path(&self) -> &str {
        &self.variant_path
    }

    /// Icon shape, or ``None`` for the application default and for an icon name
    /// this build does not have. :attr:`icon_name` holds such a name.
    #[getter]
    fn icon(&self) -> Option<PyMarkerIcon> {
        match &self.icon {
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
        Some(self.icon.wire_name()).filter(|name| !name.is_empty())
    }

    /// Fill color as ``#RRGGBB``, or ``None`` for the deterministic hash color.
    ///
    /// A file written by a newer build can hold a color in another notation.
    /// Such a color reads back verbatim.
    #[getter]
    fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    fn __repr__(&self) -> String {
        format!("EventMarkerStyle(variant_path={:?})", self.variant_path)
    }
}

/// A resolved event marker as read from a ``NavFile``, with an interpolated
/// map position.
///
/// Obtain via ``NavFile.event_markers``.
#[pyclass(skip_from_py_object, name = "EventMarkerPoint")]
#[derive(Debug, Clone)]
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
        to_fixed(self.inner.sys_time)
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

/// A parsed `.gtd` navigation data file.
///
/// Construct via `NavFileBuilder.finish()` to write, or `NavFile.open(path)`
/// to read.
#[pyclass(skip_from_py_object, name = "NavFile")]
pub struct PyNavFile {
    inner: NavFile,
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
            .map(|f| Self { inner: f })
            .map_err(file_err)
    }

    /// Parse a `.gtd` file from raw bytes.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        NavFile::read(std::io::Cursor::new(data))
            .map(|f| Self { inner: f })
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
    fn points(&self) -> Vec<PyNavPoint> {
        self.inner
            .nav_points()
            .iter()
            .map(|p| PyNavPoint { inner: p.clone() })
            .collect()
    }

    /// All map markers with their interpolated positions.
    ///
    /// A marker with an icon code this build does not have raises a
    /// ``UserWarning``. Its ``icon`` reads as ``None``. Its ``icon_code``
    /// holds the code.
    #[getter]
    fn markers(&self, py: Python<'_>) -> PyResult<Vec<PyMarker>> {
        let markers = self.inner.markers();
        let mut converted = Vec::with_capacity(markers.len());
        for m in markers {
            if let AnnotationIcon::Unrecognized(code) = m.annotation.icon() {
                let message = CString::new(format!(
                    "map marker {:?} holds the icon code {code}, which this build does not have: \
                     icon reads as None, icon_code holds the code",
                    m.annotation.label().unwrap_or("")
                ))
                .map_err(|err| PyValueError::new_err(err.to_string()))?;
                PyErr::warn(py, &py.get_type::<PyUserWarning>(), &message, 1)?;
            }
            converted.push(PyMarker { inner: m.clone() });
        }
        Ok(converted)
    }

    /// All event markers with their interpolated positions.
    #[getter]
    fn event_markers(&self) -> Vec<PyEventMarkerPoint> {
        self.inner
            .event_markers()
            .iter()
            .map(|em| PyEventMarkerPoint { inner: em.clone() })
            .collect()
    }

    /// All ad-hoc sensor channels, sorted by name.
    #[getter]
    fn channels(&self) -> Vec<PyChannel> {
        self.inner
            .channels()
            .iter()
            .map(|c| PyChannel { inner: c.clone() })
            .collect()
    }

    /// Per-variant style overrides stored in the file.
    ///
    /// A style with an icon unknown to this build raises a ``UserWarning``.
    /// Its ``icon`` reads as ``None``. Its ``icon_name`` holds the name.
    #[getter]
    fn event_marker_styles(&self, py: Python<'_>) -> PyResult<Vec<PyEventMarkerStyle>> {
        let styles = self.inner.event_marker_styles();
        let mut converted = Vec::with_capacity(styles.len());
        for s in styles {
            if let EventMarkerIconChoice::Unrecognized(name) = &s.icon {
                let message = CString::new(format!(
                    "event marker style {:?} names the icon {name:?}, which this build does not \
                     have: icon reads as None, icon_name holds the name",
                    s.variant_path
                ))
                .map_err(|err| PyValueError::new_err(err.to_string()))?;
                PyErr::warn(py, &py.get_type::<PyUserWarning>(), &message, 1)?;
            }
            converted.push(PyEventMarkerStyle {
                variant_path: s.variant_path.clone(),
                icon: s.icon.clone(),
                color: match &s.color {
                    EventMarkerColor::Auto => None,
                    EventMarkerColor::Hex(h) | EventMarkerColor::Unrecognized(h) => Some(h.clone()),
                },
            });
        }
        Ok(converted)
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
        {
            let mut b = slf.borrow_mut();
            if b.recorder.is_some() {
                return Err(PyRuntimeError::new_err(
                    "with_meta() must be called before adding data",
                ));
            }
            let config = b.config.take().ok_or_else(consumed_err)?;
            b.config = Some(config.with_meta(meta.inner.clone()));
        }
        Ok(slf.unbind())
    }

    /// Set the file title. Must be called before any `add()` call.
    fn with_title(slf: Bound<'_, Self>, title: String) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            if b.recorder.is_some() {
                return Err(PyRuntimeError::new_err(
                    "with_title() must be called before adding data",
                ));
            }
            let config = b.config.take().ok_or_else(consumed_err)?;
            b.config = Some(config.with_title(title));
        }
        Ok(slf.unbind())
    }

    /// Set the device or sensor name. Must be called before any `add()` call.
    fn with_device(slf: Bound<'_, Self>, device: String) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            if b.recorder.is_some() {
                return Err(PyRuntimeError::new_err(
                    "with_device() must be called before adding data",
                ));
            }
            let config = b.config.take().ok_or_else(consumed_err)?;
            b.config = Some(config.with_device(device));
        }
        Ok(slf.unbind())
    }

    /// Set free-text notes. Must be called before any `add()` call.
    fn with_notes(slf: Bound<'_, Self>, notes: String) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            if b.recorder.is_some() {
                return Err(PyRuntimeError::new_err(
                    "with_notes() must be called before adding data",
                ));
            }
            let config = b.config.take().ok_or_else(consumed_err)?;
            b.config = Some(config.with_notes(notes));
        }
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
                    let m = em.borrow();
                    let Some(path) = m.variant_path.clone() else {
                        return Ok(slf.unbind());
                    };
                    let marker = EventMarker::builder()
                        .variant_path(path)
                        .sys_time(m.sys_time)
                        .maybe_annotation(m.annotation.clone())
                        .build()
                        .map_err(|e| PyValueError::new_err(e.to_string()))?;
                    recorder.add_event_marker(marker);
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
    /// Returns ``self`` to allow chaining.
    fn add_event_marker_style(
        slf: Bound<'_, Self>,
        style: &PyEventMarkerStyle,
    ) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            let recorder = b.ensure_recorder()?;
            recorder.add_event_marker_style(
                EventMarkerStyle::builder()
                    .variant_path(style.variant_path.clone())
                    .icon(style.icon.clone())
                    .maybe_color(style.color.clone())
                    .build()
                    .map_err(file_err)?,
            );
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
            .map(|f| PyNavFile { inner: f })
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

    m.add_class::<PyConstellation>()?;
    m.add_class::<PyMarkerIcon>()?;
    m.add_class::<PyTravelMode>()?;
    m.add_class::<PySatellite>()?;
    m.add_class::<PySatelliteReport>()?;
    m.add_class::<PyUnit>()?;
    m.add_class::<PyChannelUnit>()?;
    m.add_class::<PyChannel>()?;
    m.add_class::<PyNavFix>()?;
    m.add_class::<PyAnnotation>()?;
    m.add_class::<PyMeta>()?;
    m.add_class::<PyNavPoint>()?;
    m.add_class::<PyMarker>()?;
    m.add_class::<PyEventMarker>()?;
    m.add_class::<PyEventMarkerStyle>()?;
    m.add_class::<PyEventMarkerPoint>()?;
    m.add_class::<PyNavFile>()?;
    m.add_class::<PyNavFileBuilder>()?;
    Ok(())
}
