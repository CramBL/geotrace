//! Unit conversion between a query's thresholds and the evaluator's base units.

use std::collections::BTreeMap;
use std::ops::Range;

use gt_query::{
    ChannelSchema, Diagnostic, MetricProvider, QueryMetric, RunOutput, TrackInput, check, parse,
    run,
};
use gt_types::{FileIdx, TrackIdx, TrackRef};

/// Per-metric series in base units.
#[derive(Default)]
struct TrackData {
    len: usize,
    series: BTreeMap<QueryMetric, Vec<Option<f64>>>,
}

impl TrackData {
    fn new(len: usize) -> Self {
        Self {
            len,
            ..Self::default()
        }
    }

    fn with(mut self, metric: QueryMetric, values: Vec<Option<f64>>) -> Self {
        assert_eq!(values.len(), self.len);
        self.series.insert(metric, values);
        self
    }
}

impl MetricProvider for TrackData {
    fn len(&self) -> usize {
        self.len
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        self.series
            .get(&metric)
            .and_then(|values| values.get(index).copied().flatten())
    }
}

fn track_ref() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

fn run_on_points(src: &str, provider: &TrackData) -> Result<RunOutput, Diagnostic> {
    let query = check(&parse(src)?, &ChannelSchema::new())?;
    Ok(run(
        &query,
        &[TrackInput {
            track: track_ref(),
            provider,
        }],
    ))
}

fn matched_ranges(output: &RunOutput) -> Vec<Range<usize>> {
    output
        .matches
        .first()
        .map(|m| m.ranges.clone())
        .unwrap_or_default()
}

/// 10 m/s over 10 m is one length per second, which is 60 per minute.
#[test]
fn a_rate_from_a_speed_and_a_length_compares_in_per_minute() {
    let provider = TrackData::new(1)
        .with(QueryMetric::Velocity, vec![Some(10.0)])
        .with(QueryMetric::Eph, vec![Some(10.0)]);
    let output = run_on_points("points | where velocity / eph > 2 per min", &provider)
        .expect("a well formed query");
    assert_eq!(matched_ranges(&output), vec![0..1]);
}

/// A threshold of 10 kn is 5.14444 m/s.
#[test]
fn a_threshold_in_knots_converts_to_metres_per_second() {
    let provider = TrackData::new(2).with(QueryMetric::Velocity, vec![Some(5.0), Some(5.2)]);
    let output =
        run_on_points("points | where velocity > 10 kn", &provider).expect("a well formed query");
    assert_eq!(matched_ranges(&output), vec![1..2]);
}
