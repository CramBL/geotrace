use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use nmea::SentenceType;

use super::config::{NmeaConfig, SentencePeriod};
use super::diagnostics::{NmeaDiagnostic, NmeaDiagnosticKind, NmeaDiagnostics};
use super::time::CaptureContext;

#[derive(Debug)]
pub(super) struct Schedules {
    entries: Vec<Schedule>,
    first_host: Option<DateTime<Utc>>,
    last_host: Option<DateTime<Utc>>,
    first_receiver: Option<DateTime<Utc>>,
    last_receiver: Option<DateTime<Utc>>,
}

impl Schedules {
    pub fn new(config: &NmeaConfig) -> Self {
        Self {
            entries: config
                .periods
                .iter()
                .map(|configured| Schedule {
                    configured: *configured,
                    lateness: if matches!(
                        configured.sentence,
                        SentenceType::GSA | SentenceType::GSV
                    ) {
                        config.satellite_window
                    } else {
                        config.navigation_window
                    },
                    capacity: config.max_buffered_lines,
                    host: UpdateClock::new(ClockDomain::Host),
                    receiver: UpdateClock::new(ClockDomain::Receiver),
                    seen: false,
                    latest_source: None,
                })
                .collect(),
            first_host: None,
            last_host: None,
            first_receiver: None,
            last_receiver: None,
        }
    }

    #[cfg(test)]
    pub(super) fn assert_buffer_limits(&self) {
        for schedule in &self.entries {
            for clock in [&schedule.host, &schedule.receiver] {
                assert!(clock.pending.len() <= schedule.capacity.get());
                assert!(clock.finalized.len() <= clock.pending.len() + 1);
            }
        }
    }

    pub fn observe(
        &mut self,
        sentence: SentenceType,
        time: CaptureContext,
        diagnostics: &mut NmeaDiagnostics,
    ) {
        let Some(schedule) = self
            .entries
            .iter_mut()
            .find(|entry| entry.configured.sentence == sentence)
        else {
            return;
        };
        if self
            .last_host
            .zip(time.host)
            .is_some_and(|(latest, capture)| {
                latest
                    .signed_duration_since(capture)
                    .to_std()
                    .is_ok_and(|age| age > schedule.lateness)
            })
        {
            diagnostics.report(NmeaDiagnostic {
                kind: NmeaDiagnosticKind::LateObservation,
                line: time.line,
                sentence: Some(sentence),
                reason: "Sentence capture is older than the finalized host schedule window"
                    .to_owned(),
            });
            return;
        }
        schedule.seen = true;
        if schedule
            .latest_source
            .is_none_or(|previous| time.sequence >= previous.sequence)
        {
            schedule.latest_source = Some(time);
        }
        if time.host.is_none() {
            schedule.host.disconnect(time.sequence);
        }
        if time.receiver_progress.is_none() {
            schedule.receiver.disconnect(time.sequence);
        }
        let receiver_segment = time
            .receiver_progress
            .map(|timestamp| schedule.receiver.segment_for(timestamp));
        if let Some(timestamp) = time.host {
            schedule.host.insert(
                TimedUpdate {
                    timestamp,
                    line: time.line,
                    receiver_segment,
                    host: Some(timestamp),
                },
                schedule.configured,
                schedule.capacity,
                diagnostics,
            );
        }
        if let Some(timestamp) = time.receiver_progress {
            schedule.receiver.insert(
                TimedUpdate {
                    timestamp,
                    line: time.line,
                    receiver_segment,
                    host: time.host,
                },
                schedule.configured,
                schedule.capacity,
                diagnostics,
            );
        }
    }

    pub fn advance(&mut self, time: CaptureContext, diagnostics: &mut NmeaDiagnostics) {
        self.first_host = self.first_host.into_iter().chain(time.host).min();
        self.last_host = self.last_host.into_iter().chain(time.host).max();
        self.first_receiver = self
            .first_receiver
            .into_iter()
            .chain(time.receiver_progress)
            .min();
        self.last_receiver = self
            .last_receiver
            .into_iter()
            .chain(time.receiver_progress)
            .max();
        for schedule in &mut self.entries {
            let lateness = TimeDelta::from_std(schedule.lateness).unwrap_or(TimeDelta::MAX);
            if let Some(cutoff) = self
                .last_host
                .and_then(|time| time.checked_sub_signed(lateness))
            {
                schedule
                    .receiver
                    .flush_host(cutoff, schedule.configured, diagnostics);
                schedule
                    .host
                    .flush(Some(cutoff), schedule.configured, diagnostics);
            }
            if let Some(cutoff) = self
                .last_receiver
                .and_then(|time| time.checked_sub_signed(lateness))
            {
                schedule
                    .receiver
                    .flush(Some(cutoff), schedule.configured, diagnostics);
            }
        }
    }

    pub fn receiver_boundary(&mut self, sequence: u64) {
        for schedule in &mut self.entries {
            schedule.receiver.disconnect(sequence);
            if let Some(previous) = &mut schedule.latest_source {
                previous.receiver_progress = None;
            }
        }
        self.first_receiver = None;
        self.last_receiver = None;
    }

    pub fn finish(&mut self, line: u64, diagnostics: &mut NmeaDiagnostics) {
        for schedule in &mut self.entries {
            schedule.host.flush(None, schedule.configured, diagnostics);
            schedule
                .receiver
                .flush(None, schedule.configured, diagnostics);
            let span = self
                .first_receiver
                .zip(self.last_receiver)
                .or(self.first_host.zip(self.last_host));
            if !schedule.seen {
                if let Some((first, last)) = span {
                    schedule.configured.report_gap(
                        TimedGap {
                            start: first,
                            end: TimedUpdate {
                                timestamp: last,
                                line,
                                receiver_segment: None,
                                host: None,
                            },
                        },
                        diagnostics,
                    );
                }
            } else {
                let tail = if schedule
                    .latest_source
                    .is_some_and(|time| time.receiver_progress.is_some())
                {
                    schedule.receiver.last_finalized().zip(self.last_receiver)
                } else if schedule
                    .latest_source
                    .is_some_and(|time| time.host.is_some())
                {
                    schedule.host.last_finalized().zip(self.last_host)
                } else {
                    None
                };
                if let Some((previous, last)) = tail {
                    schedule.configured.report_gap(
                        TimedGap {
                            start: previous.timestamp,
                            end: TimedUpdate {
                                timestamp: last,
                                line,
                                receiver_segment: None,
                                host: None,
                            },
                        },
                        diagnostics,
                    );
                }
            }
        }
    }
}

#[derive(Debug)]
struct Schedule {
    configured: SentencePeriod,
    lateness: Duration,
    capacity: NonZeroUsize,
    host: UpdateClock,
    receiver: UpdateClock,
    seen: bool,
    latest_source: Option<CaptureContext>,
}

#[derive(Debug)]
struct UpdateClock {
    domain: ClockDomain,
    pending: BTreeMap<DateTime<Utc>, ClockUpdate>,
    finalized: BTreeMap<ClockSegment, TimedUpdate>,
    segment: ClockSegment,
}

impl UpdateClock {
    fn new(domain: ClockDomain) -> Self {
        Self {
            domain,
            pending: BTreeMap::new(),
            finalized: BTreeMap::new(),
            segment: ClockSegment::default(),
        }
    }

    fn disconnect(&mut self, sequence: u64) {
        self.segment = ClockSegment(sequence);
        self.retain_finalized();
    }

    fn insert(
        &mut self,
        update: TimedUpdate,
        configured: SentencePeriod,
        capacity: NonZeroUsize,
        diagnostics: &mut NmeaDiagnostics,
    ) {
        let segment = self.segment_for(update.timestamp);
        if let Some(previous) = self
            .finalized
            .get_mut(&segment)
            .filter(|previous| update.timestamp <= previous.timestamp)
        {
            if update.timestamp < previous.timestamp {
                diagnostics.report(NmeaDiagnostic {
                    kind: NmeaDiagnosticKind::LateObservation,
                    line: update.line,
                    sentence: Some(configured.sentence),
                    reason: "Sentence update is older than the finalized schedule window"
                        .to_owned(),
                });
            } else {
                previous.receiver_segment = previous.receiver_segment.or(update.receiver_segment);
            }
            return;
        }
        self.pending
            .entry(update.timestamp)
            .and_modify(|previous| {
                previous.source.receiver_segment =
                    previous.source.receiver_segment.or(update.receiver_segment);
                previous.source.host = previous.source.host.or(update.host);
            })
            .or_insert(ClockUpdate {
                source: update,
                segment,
            });
        if self.pending.len() > capacity.get() {
            diagnostics.report(NmeaDiagnostic {
                kind: NmeaDiagnosticKind::BufferLimit,
                line: update.line,
                sentence: Some(configured.sentence),
                reason: "Sentence schedule capacity reached".to_owned(),
            });
            if let Some((_timestamp, update)) = self.pending.pop_first() {
                self.finalize(update, configured, diagnostics);
            }
        }
    }

    fn segment_for(&self, timestamp: DateTime<Utc>) -> ClockSegment {
        let preceding = self
            .pending
            .range(..timestamp)
            .next_back()
            .map(|(_, update)| *update)
            .or_else(|| {
                self.finalized
                    .iter()
                    .filter(|(_, previous)| previous.timestamp < timestamp)
                    .max_by_key(|(_, previous)| previous.timestamp)
                    .map(|(segment, previous)| ClockUpdate {
                        source: *previous,
                        segment: *segment,
                    })
            });
        let following = self
            .pending
            .range(timestamp..)
            .next()
            .map(|(_, update)| *update);
        preceding
            .zip(following)
            .filter(|(previous, next)| previous.segment == next.segment)
            .map_or(self.segment, |(previous, _)| previous.segment)
    }

    fn flush(
        &mut self,
        cutoff: Option<DateTime<Utc>>,
        configured: SentencePeriod,
        diagnostics: &mut NmeaDiagnostics,
    ) {
        let matured: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, update)| {
                cutoff.is_none()
                    || self.domain == ClockDomain::Host
                    || update.segment == self.segment
            })
            .take_while(|(timestamp, _)| cutoff.is_none_or(|cutoff| **timestamp < cutoff))
            .map(|(timestamp, _)| *timestamp)
            .collect();
        for timestamp in matured {
            if let Some(update) = self.pending.remove(&timestamp) {
                self.finalize(update, configured, diagnostics);
            }
        }
    }

    fn flush_host(
        &mut self,
        cutoff: DateTime<Utc>,
        configured: SentencePeriod,
        diagnostics: &mut NmeaDiagnostics,
    ) {
        let mut blocked = BTreeSet::new();
        let matured: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(timestamp, update)| {
                if blocked.contains(&update.segment) {
                    return None;
                }
                if update.source.host.is_some_and(|host| host < cutoff) {
                    Some(*timestamp)
                } else {
                    blocked.insert(update.segment);
                    None
                }
            })
            .collect();
        for timestamp in matured {
            if let Some(update) = self.pending.remove(&timestamp) {
                self.finalize(update, configured, diagnostics);
            }
        }
    }

    fn finalize(
        &mut self,
        update: ClockUpdate,
        configured: SentencePeriod,
        diagnostics: &mut NmeaDiagnostics,
    ) {
        if self
            .finalized
            .get(&update.segment)
            .is_some_and(|previous| previous.timestamp >= update.source.timestamp)
        {
            return;
        }
        if let Some(previous) = self.finalized.get(&update.segment).filter(|previous| {
            self.domain == ClockDomain::Receiver
                || previous.receiver_segment.is_none()
                || previous.receiver_segment != update.source.receiver_segment
        }) {
            configured.report_gap(
                TimedGap {
                    start: previous.timestamp,
                    end: update.source,
                },
                diagnostics,
            );
        }
        self.finalized.insert(update.segment, update.source);
        self.retain_finalized();
    }

    fn retain_finalized(&mut self) {
        let active = self.segment;
        let pending = &self.pending;
        self.finalized.retain(|segment, _| {
            *segment == active || pending.values().any(|update| update.segment == *segment)
        });
    }

    fn last_finalized(&self) -> Option<TimedUpdate> {
        self.finalized.get(&self.segment).copied()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct ClockSegment(u64);

#[derive(Clone, Copy, Debug)]
struct ClockUpdate {
    source: TimedUpdate,
    segment: ClockSegment,
}

#[derive(Clone, Copy, Debug)]
struct TimedUpdate {
    timestamp: DateTime<Utc>,
    line: u64,
    receiver_segment: Option<ClockSegment>,
    host: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug)]
struct TimedGap {
    start: DateTime<Utc>,
    end: TimedUpdate,
}

impl SentencePeriod {
    fn report_gap(self, gap: TimedGap, diagnostics: &mut NmeaDiagnostics) {
        let Ok(duration) = gap.end.timestamp.signed_duration_since(gap.start).to_std() else {
            return;
        };
        // Half a period permits capture jitter and multipart transmission time.
        if duration <= self.period.saturating_add(self.period / 2) {
            return;
        }
        let elapsed_periods = duration
            .as_nanos()
            .saturating_add(self.period.as_nanos() / 2)
            / self.period.as_nanos();
        let count = u64::try_from(elapsed_periods.saturating_sub(1)).unwrap_or(u64::MAX);
        diagnostics.report_repeated(
            NmeaDiagnostic {
                kind: NmeaDiagnosticKind::MissingUpdate,
                line: gap.end.line,
                sentence: Some(self.sentence),
                reason: format!(
                    "Missing {} update within configured period {:?}",
                    self.sentence, self.period
                ),
            },
            count,
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClockDomain {
    Host,
    Receiver,
}
