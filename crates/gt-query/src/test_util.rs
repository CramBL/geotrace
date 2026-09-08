#![cfg(test)]
//! The metric provider the test modules of gt-query run queries over, together
//! with the helpers that parse, check and run one against it.

use std::collections::{BTreeMap, BTreeSet};

use geotrace_sdk_units::ChannelUnit;
use gt_types::{FileIdx, TrackIdx, TrackRef};

use crate::{
    ChannelInfo, ChannelSamples, ChannelSchema, ChannelTimeline, CheckedQuery, Diagnostic,
    MetricProvider, Query, QueryMetric, RunOutput, TrackInput,
};

pub fn track_ref() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

/// Per-metric series in base units. Anything absent is missing. Channels
/// carry their own `(time, row)` samples, keyed by name, where each row
/// holds one value per component (one for a scalar channel).
#[derive(Default)]
pub struct TestProvider {
    len: usize,
    series: BTreeMap<QueryMetric, Vec<Option<f64>>>,
    channels: BTreeMap<String, Vec<(f64, Vec<f64>)>>,
    filtered_out: BTreeSet<usize>,
}

impl TestProvider {
    pub fn new(len: usize) -> Self {
        Self {
            len,
            series: BTreeMap::new(),
            channels: BTreeMap::new(),
            filtered_out: BTreeSet::new(),
        }
    }

    /// Filter `index` out of the run, as the global time window does in
    /// `gt-query-run`: it has no value for any metric and falls in no match.
    pub fn filtering_out(mut self, index: usize) -> Self {
        self.filtered_out.insert(index);
        self
    }

    pub fn with(mut self, metric: QueryMetric, values: Vec<Option<f64>>) -> Self {
        assert_eq!(values.len(), self.len);
        self.series.insert(metric, values);
        self
    }

    /// A provider whose points sit one second apart and take `values` as
    /// their velocity in m/s. Every other metric is missing.
    pub fn velocities_one_second_apart(values: Vec<f64>) -> Self {
        Self::new(values.len()).indexed_time().with(
            QueryMetric::Velocity,
            values.into_iter().map(Some).collect(),
        )
    }

    pub fn indexed_time(self) -> Self {
        let len = self.len;
        self.with(
            QueryMetric::Time,
            (0..len).map(|i| Some(i as f64)).collect(),
        )
    }

    /// Attach a scalar channel's native `(time_secs, value)` samples.
    pub fn with_channel(mut self, name: &str, samples: Vec<(f64, f64)>) -> Self {
        let rows = samples.into_iter().map(|(t, v)| (t, vec![v])).collect();
        self.channels.insert(name.to_owned(), rows);
        self
    }

    /// Attach a vector channel's native `(time_secs, row)` samples, each row
    /// one value per component.
    pub fn with_vector_channel(mut self, name: &str, samples: Vec<(f64, Vec<f64>)>) -> Self {
        self.channels.insert(name.to_owned(), samples);
        self
    }
}

impl MetricProvider for TestProvider {
    fn len(&self) -> usize {
        self.len
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        if self.filtered_out.contains(&index) {
            return None;
        }
        self.series
            .get(&metric)
            .and_then(|values| values.get(index).copied().flatten())
    }

    fn point_can_match(&self, index: usize) -> bool {
        index < self.len && !self.filtered_out.contains(&index)
    }

    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        let Some(rows) = self.channels.get(name) else {
            return ChannelSamples::default();
        };
        let columns = rows.first().map_or(1, |(_, row)| row.len());
        let mut in_span: Vec<&(f64, Vec<f64>)> = rows
            .iter()
            .filter(|(t, _)| *t >= t_lo && *t <= t_hi)
            .collect();
        in_span.sort_by(|(a, _), (b, _)| a.total_cmp(b));
        let values = in_span
            .iter()
            .flat_map(|(_, row)| row.iter().copied())
            .collect();
        ChannelSamples { values, columns }
    }

    fn channel_timeline(&self, name: &str) -> ChannelTimeline {
        let Some(rows) = self.channels.get(name) else {
            return ChannelTimeline::default();
        };
        let columns = rows.first().map_or(1, |(_, row)| row.len());
        ChannelTimeline {
            times: rows.iter().map(|(t, _)| *t).collect(),
            values: rows
                .iter()
                .flat_map(|(_, row)| row.iter().copied())
                .collect(),
            columns,
        }
    }
}

pub fn checked(src: &str) -> CheckedQuery {
    crate::check(&crate::parse(src).unwrap(), &ChannelSchema::new()).unwrap()
}

/// Check with an empty channel schema, for the many tests that reference no
/// channels. Tests that need channels build their own schema.
pub fn chk(query: &Query) -> Result<CheckedQuery, Diagnostic> {
    crate::check(query, &ChannelSchema::new())
}

pub fn run_one(src: &str, provider: &TestProvider) -> RunOutput {
    let query = checked(src);
    crate::run(
        &query,
        &[TrackInput {
            track: track_ref(),
            provider,
        }],
    )
}

pub fn run_channel(src: &str, schema: &ChannelSchema, provider: &TestProvider) -> RunOutput {
    let query = crate::check(&crate::parse(src).unwrap(), schema).expect(src);
    crate::run(
        &query,
        &[TrackInput {
            track: track_ref(),
            provider,
        }],
    )
}

/// A scalar channel schema entry for `@name` with `unit` and `period_deg`.
pub fn schema_with(name: &str, unit: Option<&str>, period_deg: Option<f64>) -> ChannelSchema {
    let mut schema = ChannelSchema::new();
    schema.insert(
        name,
        ChannelInfo {
            unit: unit.map(ChannelUnit::from_file_label),
            period_deg,
            components: vec![],
            conflicts: Vec::new(),
        },
    );
    schema
}

/// A single vector channel with `unit` and `components` as its labels.
pub fn vector_schema(name: &str, unit: Option<&str>, components: &[&str]) -> ChannelSchema {
    let mut schema = ChannelSchema::new();
    schema.insert(
        name,
        ChannelInfo {
            unit: unit.map(ChannelUnit::from_file_label),
            period_deg: None,
            components: components.iter().map(|c| (*c).to_owned()).collect(),
            conflicts: Vec::new(),
        },
    );
    schema
}
