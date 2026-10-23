use chrono::Duration;
use geotrace_sdk::{Meta, MetaStringWithNul, NavFileBuilder, NavRecorder};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use crate::channel::PyChannel;
use crate::markers::{PyAnnotation, PyEventMarker, PyEventMarkerStyle, SkippableEventMarker};
use crate::metadata::PyMeta;
use crate::nav_file::PyNavFile;
use crate::nav_fix::PyNavFix;
use crate::navigation::{PySatelliteObservationReport, PySatelliteUsageReport};
use crate::satellite::PySatelliteReport;

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
    config: Option<NavFileBuilder>,
    recorder: Option<NavRecorder>,
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
            .configure_before_data("with_meta", |config| Ok(config.with_meta(Meta::from(meta))))?;
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
            self.config
                .take()
                .ok_or_else(|| PyRuntimeError::new_err(BUILDER_CONSUMED))?
                .open()
        };
        recorder
            .finish()
            .map(PyNavFile::from)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }
}

impl PyNavFileBuilder {
    /// Configuration failures preserve the previous config through a clone.
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
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err(BUILDER_CONSUMED))?;
        let configured =
            configure(config.clone()).map_err(|error| PyValueError::new_err(error.to_string()))?;
        self.config = Some(configured);
        Ok(())
    }

    fn ensure_recorder(&mut self) -> PyResult<&mut NavRecorder> {
        if self.recorder.is_none() {
            let config = self
                .config
                .take()
                .ok_or_else(|| PyRuntimeError::new_err(BUILDER_CONSUMED))?;
            self.recorder = Some(config.open());
        }
        self.recorder
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err(BUILDER_CONSUMED))
    }
}

/// PyO3 includes this declaration order in argument error messages.
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

const BUILDER_CONSUMED: &str = "builder already consumed by finish()";
