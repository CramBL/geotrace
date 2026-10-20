use gt_egui_mipmap::MipMap;
use gt_types::time_types::GpsTime;
use gt_types::{NavigationEvidenceSource, SatelliteReportMetadata};

#[derive(Clone, Copy, Debug)]
pub(crate) struct MetricSampleSource {
    pub point_index: usize,
    pub evidence: Option<NavigationEvidenceSource>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimedMetricSample {
    pub time: GpsTime,
    pub value: Option<f64>,
    pub source: MetricSampleSource,
    pub fix_time: GpsTime,
    pub report: Option<SatelliteReportMetadata>,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum MetricSamplePriority {
    NavigationSummary,
    ReportedAggregate,
    IdentityList,
}

impl TimedMetricSample {
    fn priority(&self) -> MetricSamplePriority {
        match self.source.evidence {
            None | Some(NavigationEvidenceSource::NavigationCount) => {
                MetricSamplePriority::NavigationSummary
            }
            Some(
                NavigationEvidenceSource::UsageAggregate(_)
                | NavigationEvidenceSource::ObservationAggregate(_),
            ) => MetricSamplePriority::ReportedAggregate,
            _ => MetricSamplePriority::IdentityList,
        }
    }

    fn compare_at_same_time(&self, other: &Self) -> std::cmp::Ordering {
        self.priority()
            .cmp(&other.priority())
            .then_with(|| match (self.report, other.report) {
                (Some(left), Some(right)) => (left.normalized_time - self.fix_time)
                    .abs()
                    .cmp(&(right.normalized_time - other.fix_time).abs())
                    .then_with(|| left.compare_for_fix(&right, self.time))
                    .then_with(|| self.source.point_index.cmp(&other.source.point_index)),
                _ => self.source.point_index.cmp(&other.source.point_index),
            })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MetricSeries {
    pub runs: Vec<MipMap>,
    /// Source references align with each run's finest mipmap level.
    sources: Vec<Vec<MetricSampleSource>>,
}

impl MetricSeries {
    pub fn from_timed_runs(runs: Vec<Vec<TimedMetricSample>>) -> Self {
        let mut result = Self::default();
        for mut run in runs {
            run.sort_by(|left, right| {
                left.time
                    .cmp(&right.time)
                    .then_with(|| right.value.is_some().cmp(&left.value.is_some()))
                    .then_with(|| left.compare_at_same_time(right))
            });
            run.dedup_by_key(|sample| sample.time);
            let mut available = Vec::new();
            for sample in run {
                if sample.value.is_some() {
                    available.push(sample);
                } else {
                    result.push_available_run(std::mem::take(&mut available));
                }
            }
            result.push_available_run(available);
        }
        result
    }

    fn push_available_run(&mut self, samples: Vec<TimedMetricSample>) {
        if samples.is_empty() {
            return;
        }
        self.runs.push(MipMap::build(
            samples
                .iter()
                .filter_map(|sample| {
                    Some([sample.time.as_secs_f64_with_subseconds(), sample.value?])
                })
                .collect(),
        ));
        self.sources
            .push(samples.into_iter().map(|sample| sample.source).collect());
    }

    pub fn sample_source(&self, run_index: usize, x: f64) -> Option<MetricSampleSource> {
        let samples = self.runs.get(run_index)?.finest_level();
        let index = samples
            .binary_search_by(|sample| sample.x.total_cmp(&x))
            .ok()?;
        self.sources.get(run_index)?.get(index).copied()
    }
}
