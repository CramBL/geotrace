use std::cmp::Ordering;

use chrono::Duration;
use gt_types::track::FixStats;
use gt_types::{NavPoint, NavigationEvidenceSource, SatelliteReportMetadata, SolutionState};

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum ObservationPriority {
    ExplicitState,
    NavigationCount,
    ReportedCount,
    UsageMembership,
}

pub(crate) struct NavigationObservation<'a> {
    pub point: &'a NavPoint,
    pub point_index: usize,
    pub state: SolutionState,
    evidence: NavigationEvidenceSource,
}

impl NavigationObservation<'_> {
    fn priority(&self) -> (ObservationPriority, Option<&SatelliteReportMetadata>) {
        let reports = &self.point.satellite_evidence;
        match self.evidence {
            NavigationEvidenceSource::ExplicitState => (ObservationPriority::ExplicitState, None),
            NavigationEvidenceSource::NavigationCount => {
                (ObservationPriority::NavigationCount, None)
            }
            NavigationEvidenceSource::UsageAggregate(index) => (
                ObservationPriority::ReportedCount,
                reports.usage.get(index).map(|report| &report.metadata),
            ),
            NavigationEvidenceSource::UsageList(index) => (
                ObservationPriority::UsageMembership,
                reports.usage.get(index).map(|report| &report.metadata),
            ),
            NavigationEvidenceSource::ObservationAggregate(index) => (
                ObservationPriority::ReportedCount,
                reports
                    .observations
                    .get(index)
                    .map(|report| &report.metadata),
            ),
            NavigationEvidenceSource::ObservationList(index) => (
                ObservationPriority::UsageMembership,
                reports
                    .observations
                    .get(index)
                    .map(|report| &report.metadata),
            ),
        }
    }

    fn compare_precedence(&self, other: &Self) -> Ordering {
        let (priority, metadata) = self.priority();
        let (other_priority, other_metadata) = other.priority();
        priority
            .cmp(&other_priority)
            .then_with(|| match (metadata, other_metadata) {
                (Some(metadata), Some(other_metadata)) => {
                    metadata.compare_for_fix(other_metadata, self.point.tpv.time())
                }
                _ => Ordering::Equal,
            })
    }
}

pub(crate) struct NavigationRun<'a> {
    points: &'a [NavPoint],
    pub observations: Vec<NavigationObservation<'a>>,
}

impl<'a> NavigationRun<'a> {
    fn from_points(points: &'a [NavPoint], offset: usize) -> Self {
        let mut observations: Vec<NavigationObservation<'a>> = Vec::new();
        for (index, point) in points.iter().enumerate() {
            let resolved = point.resolved_navigation_state();
            let Some(evidence) = resolved.evidence else {
                continue;
            };
            let observation = NavigationObservation {
                point,
                point_index: offset + index,
                state: resolved.state,
                evidence,
            };
            if let Some(previous) = observations.last_mut()
                && previous.point.tpv.time() == point.tpv.time()
            {
                if observation.compare_precedence(previous).is_lt() {
                    *previous = observation;
                }
            } else {
                observations.push(observation);
            }
        }
        Self {
            points,
            observations,
        }
    }

    pub fn statistics(&self) -> FixStats {
        let mut stats = FixStats::default();
        let Some(first) = self.points.first() else {
            return stats;
        };
        let Some(last) = self.points.last() else {
            return stats;
        };
        let mut current_absence = Duration::zero();
        for pair in self.observations.windows(2) {
            let [left, right] = pair else { continue };
            let interval = right.point.tpv.time() - left.point.tpv.time();
            match left.state {
                SolutionState::Fix => {
                    stats.time_with_fix += interval;
                    current_absence = Duration::zero();
                    if matches!(
                        right.state,
                        SolutionState::NoFix | SolutionState::EstimatedOnly
                    ) {
                        stats.fix_loss_count = stats.fix_loss_count.saturating_add(1);
                    }
                }
                SolutionState::NoFix | SolutionState::EstimatedOnly => {
                    match left.state {
                        SolutionState::NoFix => stats.time_without_fix += interval,
                        _ => stats.time_estimated_only += interval,
                    }
                    current_absence += interval;
                    stats.max_continuous_no_fix = stats.max_continuous_no_fix.max(current_absence);
                }
                SolutionState::Unknown => {
                    stats.time_unknown += interval;
                    current_absence = Duration::zero();
                }
            }
        }
        let run_duration = last.tpv.time() - first.tpv.time();
        stats.time_unknown += run_duration - stats.total_duration();
        stats
    }
}

pub(crate) fn runs(points: &[NavPoint]) -> impl Iterator<Item = NavigationRun<'_>> {
    points
        .chunk_by(|left, right| left.tpv.time() <= right.tpv.time())
        .scan(0, |offset, points| {
            let run = NavigationRun::from_points(points, *offset);
            *offset += points.len();
            Some(run)
        })
}
