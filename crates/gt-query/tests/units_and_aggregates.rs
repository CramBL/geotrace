//! Unit conversion and aggregate reduction over a window.

use std::collections::BTreeMap;
use std::ops::Range;

use geotrace_sdk_units::ChannelUnit;
use gt_query::{
    ChannelInfo, ChannelSamples, ChannelSchema, Diagnostic, MetricProvider, QueryMetric, RunOutput,
    TrackInput, check, parse, run,
};
use gt_types::{FileIdx, TrackIdx, TrackRef};

/// Per-metric series in base units, plus scalar channels with their own
/// `(time_secs, value)` samples.
#[derive(Default)]
struct TrackData {
    len: usize,
    series: BTreeMap<QueryMetric, Vec<Option<f64>>>,
    channels: BTreeMap<String, Vec<(f64, f64)>>,
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

    fn indexed_time(self) -> Self {
        let len = self.len;
        self.with(
            QueryMetric::Time,
            (0..len).map(|i| Some(i as f64)).collect(),
        )
    }

    fn with_channel(mut self, name: &str, samples: Vec<(f64, f64)>) -> Self {
        self.channels.insert(name.to_owned(), samples);
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

    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        let Some(samples) = self.channels.get(name) else {
            return ChannelSamples::default();
        };
        ChannelSamples {
            values: samples
                .iter()
                .filter(|(t, _)| *t >= t_lo && *t <= t_hi)
                .map(|(_, v)| *v)
                .collect(),
            columns: 1,
        }
    }
}

fn track_ref() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

fn run_with_schema(
    src: &str,
    schema: &ChannelSchema,
    provider: &TrackData,
) -> Result<RunOutput, Diagnostic> {
    let query = check(&parse(src)?, schema)?;
    Ok(run(
        &query,
        &[TrackInput {
            track: track_ref(),
            provider,
        }],
    ))
}

fn run_on_points(src: &str, provider: &TrackData) -> Result<RunOutput, Diagnostic> {
    run_with_schema(src, &ChannelSchema::new(), provider)
}

/// A schema holding one scalar channel.
fn scalar_channel_schema(name: &str, unit: Option<&str>, period_deg: Option<f64>) -> ChannelSchema {
    let mut schema = ChannelSchema::new();
    schema.insert(
        name,
        ChannelInfo {
            unit: unit.map(ChannelUnit::from_file_label),
            period_deg,
            components: Vec::new(),
            conflicts: Vec::new(),
        },
    );
    schema
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

/// 179.95° and -179.95° are a tenth of a degree apart across the antimeridian.
#[test]
fn the_spread_of_longitude_wraps_at_the_antimeridian() {
    let provider = TrackData::new(2).with(QueryMetric::Lon, vec![Some(179.95), Some(-179.95)]);
    let output = run_on_points("points | window 2 | where spread(lon) <= 1 deg", &provider)
        .expect("a well formed query");
    assert_eq!(matched_ranges(&output), vec![0..2]);
}

/// 179 and 1 are two degrees apart on a channel with a wrap period of 180°.
#[test]
fn the_spread_of_a_channel_uses_its_declared_period() {
    let schema = scalar_channel_schema("compass", Some("deg"), Some(180.0));
    let provider = TrackData::new(2)
        .indexed_time()
        .with_channel("compass", vec![(0.0, 179.0), (1.0, 1.0)]);
    let output = run_with_schema(
        "points | window 2 | where spread(@compass) <= 5 deg",
        &schema,
        &provider,
    )
    .expect("a well formed query");
    assert_eq!(matched_ranges(&output), vec![0..2]);
}

/// A bearing of 360° is the same direction as 0°.
#[test]
fn a_heading_of_a_full_turn_has_no_spread_against_north() {
    let provider = TrackData::new(2).with(QueryMetric::Heading, vec![Some(360.0), Some(0.0)]);
    let output = run_on_points(
        "points | window 2 | where spread(heading) < 0.5 deg",
        &provider,
    )
    .expect("a well formed query");
    assert_eq!(matched_ranges(&output), vec![0..2]);
}
