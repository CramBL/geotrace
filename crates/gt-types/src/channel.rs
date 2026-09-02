//! Ad-hoc sensor channels carried alongside the nav track.
//!
//! A [`Channel`] is a named time series sampled at its own rate (an
//! accelerometer's axes, an inclinometer angle), correlated with the track by
//! timestamp rather than resampled onto the nav points. Channels arrive
//! file-level from the `.gtd` reader and are partitioned to tracks by timestamp
//! when a file is segmented.

use std::ops::Range;

use chrono::{DateTime, Duration, Utc};
use geotrace_sdk_units::ChannelUnit;
use uom::si::f64::Angle;

/// A named scalar or vector sensor channel.
///
/// `components` is empty for a scalar channel, or holds one label per column for
/// a vector channel (`["x", "y", "z"]`). `values` is row-major: [`times`]`.len()`
/// rows of [`component_count`](Self::component_count) columns each. [`times`]
/// holds the timestamps in the order the file stored them. A recording whose
/// clock stepped back leaves them out of order.
///
/// [`times`]: Self::times
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub name: String,
    /// Unit of the values (`"g"`, `"deg"`), or `None`.
    pub unit: Option<ChannelUnit>,
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

/// A channel sample whose timestamp lies before the previous sample's, from a
/// recorder whose clock stepped back while the channel was sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackwardTimeStep {
    /// Position of the earlier-stamped sample among the channel's samples.
    pub position: usize,
    /// How far this sample's timestamp lies before the previous sample's.
    pub step_back: Duration,
}

impl Channel {
    /// Every sample whose timestamp lies before the previous sample's, in
    /// stored order.
    pub fn backward_time_steps(&self) -> Vec<BackwardTimeStep> {
        self.times
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| match pair {
                [previous, time] if time < previous => Some(BackwardTimeStep {
                    position: index + 1,
                    step_back: *previous - *time,
                }),
                _ => None,
            })
            .collect()
    }

    /// The maximal stretches of samples whose timestamps never step backwards,
    /// in stored order. A channel whose timestamps never step backwards yields
    /// one range over all of them, an empty channel none. Two runs of a
    /// recorder that restarted its clock cover the same wall clock times.
    pub fn chronological_runs(&self) -> Vec<Range<usize>> {
        if self.times.is_empty() {
            return Vec::new();
        }
        let mut runs = Vec::new();
        let mut start = 0;
        for step in self.backward_time_steps() {
            runs.push(start..step.position);
            start = step.position;
        }
        runs.push(start..self.times.len());
        runs
    }

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
    /// with the same metadata, whatever order [`times`](Self::times) holds. Each
    /// value row stays aligned with its timestamp.
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
    use geotrace_sdk_units::Unit;

    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("valid timestamp")
    }

    fn vector_channel() -> Channel {
        Channel {
            name: "accel".to_owned(),
            unit: Some(Unit::G.into()),
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
        assert_eq!(
            sliced.unit.as_ref().map(ToString::to_string).as_deref(),
            Some("g")
        );
    }

    #[test]
    fn slice_keeps_the_stored_order_of_a_channel_whose_times_step_backwards() {
        let channel = Channel {
            times: vec![at(2), at(0), at(1)],
            values: vec![2.0, 2.1, 3.0, 0.0, 0.1, 1.0, 1.0, 1.1, 2.0],
            ..vector_channel()
        };

        let sliced = channel.slice_time_range(at(1), at(2));

        assert_eq!(sliced.times, vec![at(2), at(1)]);
        assert_eq!(sliced.values, vec![2.0, 2.1, 3.0, 1.0, 1.1, 2.0]);
    }

    /// The shape a recorder whose clock restarts at every boot writes: each
    /// run covers the same wall clock times as the one before it.
    fn channel_restarting_its_clock() -> Channel {
        Channel {
            times: vec![at(0), at(1), at(2), at(0), at(1), at(0), at(3)],
            values: (0..21).map(f64::from).collect(),
            ..vector_channel()
        }
    }

    #[test]
    fn a_channel_whose_timestamps_never_step_backwards_is_one_run() {
        let channel = vector_channel();
        assert!(channel.backward_time_steps().is_empty());
        assert_eq!(channel.chronological_runs(), vec![0..3]);
    }

    #[test]
    fn an_empty_channel_has_no_run() {
        let channel = Channel {
            times: Vec::new(),
            values: Vec::new(),
            ..vector_channel()
        };
        assert!(channel.chronological_runs().is_empty());
    }

    #[test]
    fn a_backward_step_names_the_earlier_stamped_sample_and_how_far_it_stepped() {
        assert_eq!(
            channel_restarting_its_clock().backward_time_steps(),
            [
                BackwardTimeStep {
                    position: 3,
                    step_back: Duration::seconds(2),
                },
                BackwardTimeStep {
                    position: 5,
                    step_back: Duration::seconds(1),
                },
            ]
        );
    }

    #[test]
    fn a_run_ends_at_every_backward_step() {
        assert_eq!(
            channel_restarting_its_clock().chronological_runs(),
            [0..3, 3..5, 5..7]
        );
    }

    #[test]
    fn a_repeated_timestamp_does_not_end_a_run() {
        let channel = Channel {
            times: vec![at(0), at(1), at(1)],
            ..vector_channel()
        };
        assert!(channel.backward_time_steps().is_empty());
        assert_eq!(channel.chronological_runs(), vec![0..3]);
    }

    #[test]
    fn slice_with_no_samples_in_range_is_empty() {
        let sliced = vector_channel().slice_time_range(at(100), at(200));
        assert!(sliced.times.is_empty());
        assert!(sliced.values.is_empty());
    }
}
