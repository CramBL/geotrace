use crate::navigation::SolutionState;
use crate::satellite_evidence::{SatelliteQueryScope, SatelliteScope, SatelliteUsageReport};
use crate::{FixQuality, NavPoint};

/// Report indices refer to the point's canonical usage or observation collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationEvidenceSource {
    ExplicitState,
    NavigationCount,
    ObservationAggregate(usize),
    ObservationList(usize),
    UsageAggregate(usize),
    UsageList(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedNavigationState {
    pub state: SolutionState,
    pub evidence: Option<NavigationEvidenceSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedSatelliteCount {
    pub count: u32,
    pub evidence: NavigationEvidenceSource,
}

impl NavPoint {
    pub fn resolved_navigation_state(&self) -> ResolvedNavigationState {
        if let Some(state) = self.navigation.state {
            return ResolvedNavigationState {
                state,
                evidence: Some(NavigationEvidenceSource::ExplicitState),
            };
        }
        if let Some(count) = self.navigation.used_count {
            return ResolvedNavigationState {
                state: if count > 0 {
                    SolutionState::Fix
                } else {
                    SolutionState::NoFix
                },
                evidence: Some(NavigationEvidenceSource::NavigationCount),
            };
        }
        if let Some((index, report)) = self
            .satellite_evidence
            .select_usage_report(self.tpv.time(), |report| report.aggregate_state().is_some())
        {
            return ResolvedNavigationState {
                state: report.aggregate_state().unwrap_or(SolutionState::Unknown),
                evidence: Some(NavigationEvidenceSource::UsageAggregate(index)),
            };
        }
        if let Some((index, report)) = self
            .satellite_evidence
            .select_usage_report(self.tpv.time(), |report| report.list_state().is_some())
        {
            return ResolvedNavigationState {
                state: report.list_state().unwrap_or(SolutionState::Unknown),
                evidence: Some(NavigationEvidenceSource::UsageList(index)),
            };
        }
        ResolvedNavigationState {
            state: SolutionState::Unknown,
            evidence: None,
        }
    }

    pub fn resolved_fix_quality(&self) -> FixQuality {
        match self.resolved_navigation_state().state {
            SolutionState::NoFix => FixQuality::Lost,
            SolutionState::Fix => match self.resolved_used_count(SatelliteQueryScope::Global) {
                Some(count) if count.count >= FixQuality::STRONG_USED_COUNT => FixQuality::Strong,
                Some(count) if count.count > 0 => FixQuality::Marginal,
                _ => FixQuality::Unknown,
            },
            SolutionState::EstimatedOnly | SolutionState::Unknown => FixQuality::Unknown,
        }
    }

    pub fn resolved_used_count(
        &self,
        scope: SatelliteQueryScope,
    ) -> Option<ResolvedSatelliteCount> {
        if scope == SatelliteQueryScope::Global
            && let Some(count) = self.navigation.used_count
        {
            return Some(ResolvedSatelliteCount {
                count,
                evidence: NavigationEvidenceSource::NavigationCount,
            });
        }
        for aggregate in [true, false] {
            if let Some((index, report)) =
                self.satellite_evidence
                    .select_usage_report(self.tpv.time(), |report| {
                        report
                            .resolved_used_count(scope)
                            .is_some_and(|count| count.is_aggregate == aggregate)
                    })
            {
                let count = report.resolved_used_count(scope)?;
                return Some(ResolvedSatelliteCount {
                    count: count.count,
                    evidence: if aggregate {
                        NavigationEvidenceSource::UsageAggregate(index)
                    } else {
                        NavigationEvidenceSource::UsageList(index)
                    },
                });
            }
        }
        None
    }

    pub fn resolved_seen_count(
        &self,
        scope: SatelliteQueryScope,
    ) -> Option<ResolvedSatelliteCount> {
        for aggregate in [true, false] {
            if let Some((index, report)) =
                self.satellite_evidence
                    .select_observation_report(self.tpv.time(), |report| {
                        report
                            .resolved_seen_count(scope)
                            .is_some_and(|count| count.is_aggregate == aggregate)
                    })
            {
                let count = report.resolved_seen_count(scope)?;
                return Some(ResolvedSatelliteCount {
                    count: count.count,
                    evidence: if aggregate {
                        NavigationEvidenceSource::ObservationAggregate(index)
                    } else {
                        NavigationEvidenceSource::ObservationList(index)
                    },
                });
            }
        }
        None
    }
}

impl SatelliteUsageReport {
    fn aggregate_state(&self) -> Option<SolutionState> {
        match self.metadata.reported_count {
            Some(count) if count > 0 => Some(SolutionState::Fix),
            Some(0) if self.metadata.scope == SatelliteScope::Global => Some(SolutionState::NoFix),
            _ => None,
        }
    }

    fn list_state(&self) -> Option<SolutionState> {
        if self.has_known_used() {
            Some(SolutionState::Fix)
        } else if self.list_used_count(SatelliteQueryScope::Global) == Some(0) {
            Some(SolutionState::NoFix)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;
