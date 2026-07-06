//! Ad-hoc sensor channels carried alongside the nav track.
//!
//! A [`Channel`] is a named time series sampled at its own rate (an
//! accelerometer's axes, an inclinometer angle), correlated with the track by
//! timestamp rather than resampled onto the nav points. Channels arrive
//! file-level from the `.gtd` reader and are partitioned to tracks by timestamp
//! when a file is segmented.

use chrono::{DateTime, Utc};
use uom::si::f64::Angle;

/// A named scalar or vector sensor channel.
///
/// `components` is empty for a scalar channel, or holds one label per column for
/// a vector channel (`["x", "y", "z"]`). `values` is row-major: [`times`]`.len()`
/// rows of [`component_count`](Self::component_count) columns each. `times` is
/// sorted ascending.
///
/// [`times`]: Self::times
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub name: String,
    /// Unit of the values (`"g"`, `"deg"`), or `None`.
    pub unit: Option<String>,
    /// Wrap period of an angular channel (a heading wraps at 360°), or `None`
    /// for a linear value.
    pub period: Option<Angle>,
    pub description: Option<String>,
    /// Vector component labels, or empty for a scalar channel.
    pub components: Vec<String>,
    /// Sample timestamps, one per row of [`values`](Self::values).
    pub times: Vec<DateTime<Utc>>,
    /// Row-major sample values.
    pub values: Vec<f64>,
}

impl Channel {
    /// Whether this is a vector channel (has named components).
    pub fn is_vector(&self) -> bool {
        !self.components.is_empty()
    }

    /// Value columns per sample: the component count for a vector channel, or 1
    /// for a scalar channel.
    pub fn component_count(&self) -> usize {
        self.components.len().max(1)
    }

    /// The samples whose timestamp falls in `[start, end]`, as a new channel
    /// with the same metadata. Assumes [`times`](Self::times) is sorted; keeps
    /// each value row aligned with its timestamp.
    pub fn slice_time_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Channel {
        let columns = self.component_count();
        debug_assert_eq!(
            self.values.len(),
            self.times.len() * columns,
            "channel values must be times.len() * component_count()"
        );
        let mut times = Vec::new();
        let mut values = Vec::new();
        for (time, row) in self.times.iter().zip(self.values.chunks(columns)) {
            if *time >= start && *time <= end {
                times.push(*time);
                values.extend_from_slice(row);
            }
        }
        Channel {
            name: self.name.clone(),
            unit: self.unit.clone(),
            period: self.period,
            description: self.description.clone(),
            components: self.components.clone(),
            times,
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("valid timestamp")
    }

    fn vector_channel() -> Channel {
        Channel {
            name: "accel".to_owned(),
            unit: Some("g".to_owned()),
            period: None,
            description: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            times: vec![at(0), at(1), at(2)],
            // Row-major: three samples of (x, y, z).
            values: vec![0.0, 0.1, 1.0, 1.0, 1.1, 2.0, 2.0, 2.1, 3.0],
        }
    }

    #[test]
    fn is_vector_and_component_count() {
        let scalar = Channel {
            name: "incline".to_owned(),
            unit: None,
            period: None,
            description: None,
            components: vec![],
            times: vec![at(0)],
            values: vec![5.0],
        };
        assert!(!scalar.is_vector());
        assert_eq!(scalar.component_count(), 1);

        let accel = vector_channel();
        assert!(accel.is_vector());
        assert_eq!(accel.component_count(), 3);
    }

    #[test]
    fn slice_keeps_value_rows_aligned_with_timestamps() {
        // Keep samples in [1, 2]: drops row 0, keeps rows 1 and 2 intact.
        let sliced = vector_channel().slice_time_range(at(1), at(2));
        assert_eq!(sliced.times, vec![at(1), at(2)]);
        assert_eq!(sliced.values, vec![1.0, 1.1, 2.0, 2.0, 2.1, 3.0]);
        // Metadata is carried through unchanged.
        assert_eq!(sliced.components, ["x", "y", "z"]);
        assert_eq!(sliced.unit.as_deref(), Some("g"));
    }

    #[test]
    fn slice_with_no_samples_in_range_is_empty() {
        let sliced = vector_channel().slice_time_range(at(100), at(200));
        assert!(sliced.times.is_empty());
        assert!(sliced.values.is_empty());
    }
}
