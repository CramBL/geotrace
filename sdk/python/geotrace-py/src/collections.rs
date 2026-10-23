use std::sync::Arc;

use chrono::{DateTime, FixedOffset};
use geotrace_sdk::NavFile;
use pyo3::IntoPyObjectExt as _;
use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::types::{PyList, PySlice};

use crate::channel::PyChannel;
use crate::markers::{PyEventMarkerPoint, PyEventMarkerStyle, PyMarker};
use crate::nav_point::PyNavPoint;

/// Sequence views and iterators retain the file through an [`Arc`].
/// PyO3 requires column getters and sequence methods in one `#[pymethods]` block.
macro_rules! nav_file_sequence {
    (
        summary: $summary:literal,
        sequence: $sequence:ident as $sequence_name:literal,
        iterator: $iterator:ident as $iterator_name:literal,
        element: $element:ty as $element_name:literal,
        collection: $collection:ident,
        $(methods: { $($method:item)* },)?
    ) => {
        #[doc = $summary]
        #[doc = ""]
        #[doc = concat!(
            "Supports ``len()``, indexing, slicing and iteration, and builds one :class:`",
            $element_name,
            "` per element read. The sequence keeps working after the :class:`NavFile` object ",
            "is gone."
        )]
        #[pyclass(sequence, skip_from_py_object, name = $sequence_name)]
        pub struct $sequence {
            file: Arc<NavFile>,
        }

        #[pymethods]
        impl $sequence {
            fn __len__(&self) -> usize {
                self.file.$collection().len()
            }

            fn __getitem__<'py>(
                &self,
                py: Python<'py>,
                key: &Bound<'py, PyAny>,
            ) -> PyResult<Bound<'py, PyAny>> {
                sequence_item_or_slice::<$element, _>(py, key, self.file.$collection())
            }

            fn __iter__(&self) -> $iterator {
                $iterator {
                    file: Arc::clone(&self.file),
                    position: 0,
                }
            }

            fn __repr__(&self) -> String {
                format!(
                    concat!($sequence_name, "(len={})"),
                    self.file.$collection().len()
                )
            }

            $($($method)*)?
        }

        impl From<Arc<NavFile>> for $sequence {
            fn from(file: Arc<NavFile>) -> Self {
                Self { file }
            }
        }

        #[pyclass(skip_from_py_object, name = $iterator_name)]
        pub struct $iterator {
            file: Arc<NavFile>,
            position: usize,
        }

        #[pymethods]
        impl $iterator {
            fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                slf
            }

            fn __next__(&mut self) -> Option<$element> {
                let element = self
                    .file
                    .$collection()
                    .get(self.position)
                    .map(<$element>::from)?;
                self.position += 1;
                Some(element)
            }
        }
    };
}

nav_file_sequence! {
    summary: "The nav points of a :class:`NavFile`, in chronological order.",
    sequence: PyNavPointSequence as "NavPointSequence",
    iterator: PyNavPointIterator as "NavPointIterator",
    element: PyNavPoint as "NavPoint",
    collection: nav_points,
    methods: {
        /// Every point's latitude in degrees.
        fn latitudes(&self) -> Vec<f64> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.lat.as_degrees())
                .collect()
        }

        /// Every point's longitude in degrees.
        fn longitudes(&self) -> Vec<f64> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.lon.as_degrees())
                .collect()
        }

        /// Every point's GPS-receiver timestamp, or ``None`` when unreported.
        fn gps_times(&self) -> Vec<Option<DateTime<FixedOffset>>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.gps_time().map(|time| time.fixed_offset()))
                .collect()
        }

        /// Every point's system-clock timestamp, ``None`` where the recorder
        /// supplied none.
        fn sys_times(&self) -> Vec<Option<DateTime<FixedOffset>>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.sys_time().map(|time| time.fixed_offset()))
                .collect()
        }

        /// Every point's heading in degrees, or ``None`` for a fix without one.
        fn headings(&self) -> Vec<Option<f64>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.heading.map(|h| h.as_degrees()))
                .collect()
        }

        /// Every point's speed in m/s, or ``None`` for a fix without one.
        fn speeds_mps(&self) -> Vec<Option<f64>> {
            self.file
                .nav_points()
                .iter()
                .map(|p| p.fix.speed.map(|s| s.as_meters_per_second()))
                .collect()
        }

        /// Every point's horizontal accuracy radius in metres, or ``None`` for a
        /// fix without one.
        fn eph_m_values(&self) -> Vec<Option<f64>> {
            self.file.nav_points().iter().map(|p| p.fix.eph_m).collect()
        }
    },
}

nav_file_sequence! {
    summary: "The map markers of a :class:`NavFile` with their interpolated positions.",
    sequence: PyMarkerSequence as "MarkerSequence",
    iterator: PyMarkerIterator as "MarkerIterator",
    element: PyMarker as "Marker",
    collection: markers,
}

nav_file_sequence! {
    summary: "The event markers of a :class:`NavFile` with their interpolated positions.",
    sequence: PyEventMarkerPointSequence as "EventMarkerPointSequence",
    iterator: PyEventMarkerPointIterator as "EventMarkerPointIterator",
    element: PyEventMarkerPoint as "EventMarkerPoint",
    collection: event_markers,
}

nav_file_sequence! {
    summary: "The sensor channels of a :class:`NavFile`, sorted by name.",
    sequence: PyChannelSequence as "ChannelSequence",
    iterator: PyChannelIterator as "ChannelIterator",
    element: PyChannel as "Channel",
    collection: channels,
}

nav_file_sequence! {
    summary: "The per-variant event marker styles of a :class:`NavFile`.",
    sequence: PyEventMarkerStyleSequence as "EventMarkerStyleSequence",
    iterator: PyEventMarkerStyleIterator as "EventMarkerStyleIterator",
    element: PyEventMarkerStyle as "EventMarkerStyle",
    collection: event_marker_styles,
}

fn position_or_index_error(index: isize, len: usize) -> PyResult<usize> {
    let length = isize::try_from(len).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?;
    let position = if index < 0 {
        index
            .checked_add(length)
            .ok_or_else(|| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?
    } else {
        index
    };
    if position < 0 || position >= length {
        return Err(PyIndexError::new_err(INDEX_OUT_OF_RANGE));
    }
    usize::try_from(position).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))
}

fn slice_positions(slice: &Bound<'_, PySlice>, len: usize) -> PyResult<Vec<usize>> {
    let length = isize::try_from(len).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?;
    let indices = slice.indices(length)?;
    let mut positions = Vec::with_capacity(indices.slicelength);
    let mut at = indices.start;
    for _ in 0..indices.slicelength {
        positions.push(usize::try_from(at).map_err(|_| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?);
        at = at.saturating_add(indices.step);
    }
    Ok(positions)
}

fn sequence_item_or_slice<'py, Exposed, Stored>(
    py: Python<'py>,
    key: &Bound<'py, PyAny>,
    stored: &[Stored],
) -> PyResult<Bound<'py, PyAny>>
where
    Exposed: IntoPyObject<'py> + for<'a> From<&'a Stored>,
{
    if let Ok(slice) = key.cast::<PySlice>() {
        let selected: Vec<Exposed> = slice_positions(slice, stored.len())?
            .into_iter()
            .filter_map(|position| stored.get(position))
            .map(Exposed::from)
            .collect();
        return Ok(PyList::new(py, selected)?.into_any());
    }
    let position = position_or_index_error(key.extract()?, stored.len())?;
    let element = stored
        .get(position)
        .ok_or_else(|| PyIndexError::new_err(INDEX_OUT_OF_RANGE))?;
    Exposed::from(element).into_bound_py_any(py)
}

const INDEX_OUT_OF_RANGE: &str = "index out of range";
