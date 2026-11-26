use chrono::{DateTime, Utc};
use gt_egui_mipmap::{IndexedMipMap, IndexedSample, LevelSelection};
use gt_types::time_types::GpsTime;
use gt_types::{LoadedTrack, NavigationEvidenceSource, SatelliteReportMetadata};

#[derive(Clone, Copy, Debug)]
pub(crate) struct MetricSampleSource {
    pub point_index: usize,
    pub evidence: Option<NavigationEvidenceSource>,
}

impl MetricSampleSource {
    pub(crate) fn normalized_time(self, track: &LoadedTrack) -> Option<GpsTime> {
        let point = track.points.get(self.point_index)?;
        let report = match self.evidence {
            Some(
                NavigationEvidenceSource::UsageAggregate(index)
                | NavigationEvidenceSource::UsageList(index),
            ) => point
                .satellite_evidence
                .usage
                .get(index)
                .map(|report| report.metadata),
            Some(
                NavigationEvidenceSource::ObservationAggregate(index)
                | NavigationEvidenceSource::ObservationList(index),
            ) => point
                .satellite_evidence
                .observations
                .get(index)
                .map(|report| report.metadata),
            _ => None,
        };
        Some(report.map_or(point.tpv.time(), |report| report.normalized_time))
    }
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
    pub runs: Vec<MetricRun>,
}

#[derive(Clone, Debug)]
pub(crate) struct MetricRun {
    pub mipmap: IndexedMipMap,
    sources: Vec<MetricSampleSource>,
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
        let mut indexed = Vec::with_capacity(samples.len());
        let mut sources = Vec::with_capacity(samples.len());
        for sample in samples {
            if let Some(value) = sample.value {
                indexed.push(IndexedSample {
                    point: [sample.time.as_secs_f64_with_subseconds(), value],
                    source_index: sources.len(),
                });
                sources.push(sample.source);
            }
        }
        self.runs.push(MetricRun {
            mipmap: IndexedMipMap::build(indexed, None),
            sources,
        });
    }
}

impl MetricRun {
    pub fn sample_time(&self, track: &LoadedTrack, index: usize) -> Option<DateTime<Utc>> {
        self.sources
            .get(index)?
            .normalized_time(track)
            .map(GpsTime::utc)
    }

    pub fn sample_source(
        &self,
        level: LevelSelection,
        offset: usize,
    ) -> Option<MetricSampleSource> {
        let source = self.mipmap.source_at(level, offset)?;
        self.sources.get(source).copied()
    }
}
