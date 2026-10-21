use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::error::{BuildError, UnplacedRecordCounts};
use crate::time_types::GpsTime;
use crate::{
    NavFixTime, SatelliteEvidence, SatelliteObservation, SatelliteObservationReport,
    SatelliteReport, SatelliteReportPlacement, SatelliteUsageReport,
};

use super::{InternalPoint, NavFix, TimelinePosition};

pub(super) struct InternalSatReport {
    pub(super) time: NavFixTime,
    evidence: SatelliteEvidence,
    input_order: u64,
}

impl InternalSatReport {
    pub(super) fn from_sat_report(report: &SatelliteReport) -> Self {
        Self {
            time: report.time,
            evidence: SatelliteEvidence::from(report),
            input_order: 0,
        }
    }

    pub(super) fn from_usage(report: SatelliteUsageReport) -> Self {
        Self {
            time: report.time,
            evidence: SatelliteEvidence::new(vec![report], Vec::new()),
            input_order: 0,
        }
    }

    pub(super) fn from_observation(report: SatelliteObservationReport) -> Self {
        Self {
            time: report.time,
            evidence: SatelliteEvidence::new(Vec::new(), vec![report]),
            input_order: 0,
        }
    }

    pub(super) fn diagnostic_rows(&self) -> Vec<SatelliteObservation> {
        if let Some(report) = self.evidence.observations.first() {
            report.satellites.clone()
        } else {
            self.evidence
                .usage
                .iter()
                .flat_map(|report| &report.satellites)
                .map(|row| {
                    SatelliteObservation::builder()
                        .identity(row.identity)
                        .build()
                })
                .collect()
        }
    }

    fn append_at(mut self, target: &mut SatelliteEvidence, normalized_time: DateTime<Utc>) {
        let placement = Some(SatelliteReportPlacement::new(
            normalized_time,
            self.input_order,
        ));
        for report in &mut self.evidence.usage {
            report.placement = placement;
        }
        for report in &mut self.evidence.observations {
            report.placement = placement;
        }
        target.usage.extend(self.evidence.usage);
        target.observations.extend(self.evidence.observations);
    }

    fn estimated_time_us(&self, anchors: &ClockDeltaAnchors) -> i64 {
        match self.time {
            NavFixTime::Receiver(gps) | NavFixTime::Both { gps, .. } => gps.timestamp_micros(),
            NavFixTime::Host(sys) => {
                let host_us = sys.timestamp_micros();
                let delta = anchors.nearest_delta(host_us);
                host_us.saturating_add(delta)
            }
        }
    }
}

struct ClockDeltaAnchors {
    timeline: FixTimeline,
    deltas: Vec<i64>,
}

impl ClockDeltaAnchors {
    fn from_fixes(fixes: &[NavFix]) -> Self {
        let anchors: Vec<_> = fixes
            .iter()
            .filter_map(|fix| {
                Some((
                    fix.host_clock_time()?.timestamp_micros(),
                    fix.gps_sys_clock_delta_us()?,
                ))
            })
            .collect();
        Self {
            timeline: FixTimeline::new(anchors.iter().enumerate().map(|(index, (time_us, _))| {
                TimedFix {
                    time_us: *time_us,
                    index,
                }
            })),
            deltas: anchors.into_iter().map(|(_, delta)| delta).collect(),
        }
    }

    fn nearest_delta(&self, host_us: i64) -> i64 {
        self.timeline
            .nearest(host_us)
            .and_then(|(_, index)| self.deltas.get(index))
            .copied()
            .unwrap_or(0)
    }
}

struct TimedFix {
    time_us: i64,
    index: usize,
}

struct FixTimeline {
    fixes: Vec<TimedFix>,
}

impl FixTimeline {
    fn new(fixes: impl Iterator<Item = TimedFix>) -> Self {
        let mut fixes: Vec<_> = fixes.collect();
        fixes.sort_by_key(|fix| (fix.time_us, fix.index));
        Self { fixes }
    }

    fn nearest(&self, time_us: i64) -> Option<(u64, usize)> {
        let next = self.fixes.partition_point(|fix| fix.time_us < time_us);
        let preceding = next
            .checked_sub(1)
            .and_then(|index| self.fixes.get(index))
            .map(|fix| {
                self.fixes
                    .partition_point(|earlier| earlier.time_us < fix.time_us)
            });
        [
            preceding.and_then(|index| self.fixes.get(index)),
            self.fixes.get(next),
        ]
        .into_iter()
        .flatten()
        .map(|fix| (fix.time_us.abs_diff(time_us), fix.index))
        .min()
    }
}

pub(super) fn associate_satellites(
    fixes: &[NavFix],
    reports: Vec<InternalSatReport>,
    window: Duration,
) -> Result<(Vec<SatelliteEvidence>, Vec<InternalSatReport>), BuildError> {
    let window_us = u64::try_from(window.as_micros())
        .unwrap_or(u64::MAX)
        .min(i64::MAX as u64);
    let effective = FixTimeline::new(fixes.iter().enumerate().map(|(index, fix)| TimedFix {
        time_us: fix.effective_time().timestamp_micros(),
        index,
    }));
    let host = FixTimeline::new(fixes.iter().enumerate().filter_map(|(index, fix)| {
        Some(TimedFix {
            time_us: fix.host_clock_time()?.timestamp_micros(),
            index,
        })
    }));
    let receiver_only = FixTimeline::new(
        fixes
            .iter()
            .enumerate()
            .filter(|(_, fix)| fix.sys_time().is_none())
            .map(|(index, fix)| TimedFix {
                time_us: fix.effective_time().timestamp_micros(),
                index,
            }),
    );
    let anchors = ClockDeltaAnchors::from_fixes(fixes);
    let mut assignments = vec![SatelliteEvidence::default(); fixes.len()];
    let mut orphans = Vec::new();
    for (ordinal, mut report) in reports.into_iter().enumerate() {
        report.input_order = ordinal as u64;
        let estimated_us = report.estimated_time_us(&anchors);
        let nearest = match report.time {
            NavFixTime::Host(time) => host
                .nearest(time.timestamp_micros())
                .into_iter()
                .chain(receiver_only.nearest(estimated_us))
                .min(),
            NavFixTime::Receiver(_) | NavFixTime::Both { .. } => effective.nearest(estimated_us),
        };
        if let Some((_, index)) = nearest.filter(|(distance, _)| *distance <= window_us)
            && let (Some(fix), Some(target)) = (fixes.get(index), assignments.get_mut(index))
        {
            let normalized_us = match (report.time, fix.sys_time()) {
                (NavFixTime::Host(sys), Some(fix_sys)) => {
                    fix.effective_time().timestamp_micros().saturating_add(
                        sys.timestamp_micros()
                            .saturating_sub(fix_sys.timestamp_micros()),
                    )
                }
                _ => estimated_us,
            };
            let normalized = DateTime::from_timestamp_micros(normalized_us).ok_or(
                BuildError::GhostFixTimeOutOfRange {
                    micros: normalized_us,
                },
            )?;
            report.append_at(target, normalized);
        } else {
            orphans.push(report);
        }
    }
    Ok((assignments, orphans))
}

pub(super) fn ghost_nav_points_for(
    fixes: &[NavFix],
    orphans: Vec<InternalSatReport>,
) -> Result<Vec<InternalPoint>, BuildError> {
    let (Some(first), Some(last)) = (fixes.first(), fixes.last()) else {
        if orphans.is_empty() {
            return Ok(Vec::new());
        }
        return Err(BuildError::NoNavFixes(UnplacedRecordCounts {
            satellite_reports: orphans.len(),
            annotations: 0,
            event_markers: 0,
        }));
    };
    let anchors = ClockDeltaAnchors::from_fixes(fixes);
    let mut batches: BTreeMap<i64, Vec<InternalSatReport>> = BTreeMap::new();
    for report in orphans {
        let estimated_us = report.estimated_time_us(&anchors);
        let segment =
            fixes.partition_point(|fix| fix.effective_time().timestamp_micros() < estimated_us);
        let corrected_us = match (
            segment.checked_sub(1).and_then(|index| fixes.get(index)),
            fixes.get(segment),
        ) {
            (Some(before), Some(after)) => {
                SegmentAnchors { before, after }.corrected_report_time(&report)
            }
            _ => estimated_us,
        };
        batches.entry(corrected_us).or_default().push(report);
    }
    let mut segments: BTreeMap<usize, BTreeMap<i64, Vec<InternalSatReport>>> = BTreeMap::new();
    for (time, mut reports) in batches {
        reports.sort_by_key(|report| report.input_order);
        let segment = fixes.partition_point(|fix| fix.effective_time().timestamp_micros() < time);
        segments.entry(segment).or_default().insert(time, reports);
    }
    let mut points = Vec::new();
    for (segment, batches) in segments {
        let batch_count = batches.len();
        let before = segment.checked_sub(1).and_then(|index| fixes.get(index));
        let after = fixes.get(segment);
        let can_resolve_report_times = before.and_then(NavFix::gps_sys_clock_delta_us).is_some()
            || after.and_then(NavFix::gps_sys_clock_delta_us).is_some()
            || (before.is_some_and(|fix| matches!(fix.time, NavFixTime::Host(_)))
                && after.is_some_and(|fix| matches!(fix.time, NavFixTime::Host(_))))
            || batches
                .values()
                .flatten()
                .any(|report| report.time.gps_time().is_some());
        let mut last_position = TimelinePosition::from_internal_fix(last);
        for (batch_index, (mut normalized_us, reports)) in batches.into_iter().enumerate() {
            let (position, heading) = match (before, after) {
                (Some(before), Some(after)) => {
                    let start_us = before.effective_time().timestamp_micros();
                    let span_us = i128::from(after.effective_time().timestamp_micros())
                        - i128::from(start_us);
                    let fraction = if can_resolve_report_times {
                        if span_us < 1 {
                            0.5
                        } else {
                            ((i128::from(normalized_us) - i128::from(start_us)) as f64
                                / span_us as f64)
                                .clamp(0.0, 1.0)
                        }
                    } else {
                        let fraction = (batch_index + 1) as f64 / (batch_count + 1) as f64;
                        normalized_us = start_us + (span_us as f64 * fraction) as i64;
                        fraction
                    };
                    let start = TimelinePosition::from_internal_fix(before);
                    let end = TimelinePosition::from_internal_fix(after);
                    (
                        start.interpolated_to(end, fraction),
                        Some(start.bearing_to(end)),
                    )
                }
                (None, _) => (TimelinePosition::from_internal_fix(first), None),
                (_, None) => {
                    if let Some(heading) = last.heading {
                        let distance_m = if batch_index == 0 {
                            FIRST_ORPHAN_DISTANCE_M
                        } else {
                            ADDITIONAL_ORPHAN_DISTANCE_M
                        };
                        last_position =
                            last_position.advanced_along_great_circle(heading, distance_m);
                    }
                    (last_position, None)
                }
            };
            let normalized = DateTime::from_timestamp_micros(normalized_us).ok_or(
                BuildError::GhostFixTimeOutOfRange {
                    micros: normalized_us,
                },
            )?;
            let mut evidence = SatelliteEvidence::default();
            for report in reports {
                report.append_at(&mut evidence, normalized);
            }
            points.push(InternalPoint {
                fix: NavFix::ghost(
                    GpsTime::from_utc(normalized),
                    position.lat,
                    position.lon,
                    heading,
                ),
                evidence,
            });
        }
    }
    Ok(points)
}

struct SegmentAnchors<'a> {
    before: &'a NavFix,
    after: &'a NavFix,
}

impl SegmentAnchors<'_> {
    fn corrected_report_time(&self, report: &InternalSatReport) -> i64 {
        let NavFixTime::Host(sys) = report.time else {
            return report.time.effective().timestamp_micros();
        };
        let host_us = sys.timestamp_micros();
        match (
            self.before.gps_sys_clock_delta_us(),
            self.after.gps_sys_clock_delta_us(),
        ) {
            (Some(start), Some(end)) => {
                let host_start = self.before.sys_time().map_or(
                    self.before.effective_time().timestamp_micros() - start,
                    |time| time.timestamp_micros(),
                );
                let host_end = self.after.sys_time().map_or(
                    self.after.effective_time().timestamp_micros() - end,
                    |time| time.timestamp_micros(),
                );
                let span = i128::from(host_end) - i128::from(host_start);
                let delta = if span < 1 {
                    (i128::from(start).saturating_add(i128::from(end)) / 2) as i64
                } else {
                    let fraction = ((i128::from(host_us) - i128::from(host_start)) as f64
                        / span as f64)
                        .clamp(0.0, 1.0);
                    start.saturating_add(((end as f64 - start as f64) * fraction) as i64)
                };
                host_us.saturating_add(delta)
            }
            (Some(delta), None) | (None, Some(delta)) => host_us.saturating_add(delta),
            (None, None) => host_us,
        }
    }
}

const FIRST_ORPHAN_DISTANCE_M: f64 = 1.0;
const ADDITIONAL_ORPHAN_DISTANCE_M: f64 = 2.0;
