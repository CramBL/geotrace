use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::time::Duration;

use chrono::{DateTime, Days, NaiveTime, TimeDelta, Utc};

use super::config::ConfiguredHostTimestamp;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CaptureContext {
    pub host: Option<DateTime<Utc>>,
    pub receiver_progress: Option<DateTime<Utc>>,
    pub line: u64,
    pub sequence: u64,
    pub segment: ReceiverSegment,
    pub receiver_ambiguous: bool,
    pub date_anchor_allowed: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ReceiverSegment(u64);

#[derive(Debug)]
pub(super) struct ReceiverClock {
    state: ReceiverClockState,
    segment: ReceiverSegment,
    host_anchor: Option<ReceiverHostAnchor>,
    active_span: Option<ReceiverSpan>,
    retired_span: Option<ReceiverSpan>,
    retired_anchors: VecDeque<RetiredReceiverAnchor>,
    history_limit: NonZeroUsize,
    date_anchor_tolerance: Duration,
    segment_start_host: Option<DateTime<Utc>>,
    conflicting_dates: bool,
    latest_dated_host: Option<DateTime<Utc>>,
    date_authority: DateAuthority,
}

impl ReceiverClock {
    pub fn new(history_limit: NonZeroUsize, date_anchor_tolerance: Duration) -> Self {
        Self {
            state: ReceiverClockState::default(),
            segment: ReceiverSegment::default(),
            host_anchor: None,
            active_span: None,
            retired_span: None,
            retired_anchors: VecDeque::new(),
            history_limit,
            date_anchor_tolerance,
            segment_start_host: None,
            conflicting_dates: false,
            latest_dated_host: None,
            date_authority: DateAuthority::Trusted,
        }
    }

    #[cfg(test)]
    pub(super) fn assert_buffer_limits(&self) {
        assert!(self.retired_anchors.len() <= self.history_limit.get());
    }

    pub fn observe(&mut self, sample: ReceiverSample, tolerance: Duration) -> ReceiverDecision {
        if let Some(decision) = self.classify_ambiguous_capture(sample) {
            return decision;
        }
        let (previous, pending, coherent_segment) = match self.state {
            ReceiverClockState::Coherent(previous) => (previous, None, self.segment),
            ReceiverClockState::Discontinuous {
                coherent,
                pending,
                coherent_segment,
            } => (coherent, Some(pending), coherent_segment),
            ReceiverClockState::Unanchored => {
                self.state = ReceiverClockState::Coherent(sample);
                self.host_anchor = sample.host_anchor();
                self.segment_start_host = sample.host;
                self.latest_dated_host = sample.host;
                self.include_receiver(sample.receiver);
                return ReceiverDecision {
                    segment: self.segment,
                    reliable: true,
                    boundary: false,
                    discontinuity: false,
                    confirmed_predecessor: None,
                    ambiguous: false,
                    date_anchor_allowed: true,
                };
            }
        };
        let delta = sample.receiver.signed_duration_since(previous.receiver);
        let nearby = delta.abs().to_std().is_ok_and(|distance| {
            distance
                <= if delta < TimeDelta::zero() {
                    tolerance
                } else {
                    tolerance.saturating_mul(COHERENT_FORWARD_WINDOWS)
                }
        });
        let host_progression = self
            .host_anchor
            .and_then(|anchor| anchor.permits_progression(sample, tolerance));
        if sample.date_observed
            && sample.receiver.date_naive() != previous.receiver.date_naive()
            && !nearby
            && host_progression != Some(true)
        {
            self.conflicting_dates = true;
        }
        if host_progression != Some(true)
            && let Some(decision) = self.classify_retired_capture(
                sample,
                tolerance,
                sample.date_observed
                    && sample.receiver.date_naive() != previous.receiver.date_naive(),
            )
        {
            return decision;
        }
        if host_progression == Some(true) && delta < TimeDelta::zero() {
            self.observe_date(sample, tolerance, false);
            if sample.date_observed {
                self.latest_dated_host =
                    self.latest_dated_host.into_iter().chain(sample.host).max();
                self.segment_start_host =
                    self.segment_start_host.into_iter().chain(sample.host).min();
            }
            return ReceiverDecision {
                segment: coherent_segment,
                reliable: true,
                boundary: false,
                discontinuity: false,
                confirmed_predecessor: None,
                ambiguous: false,
                date_anchor_allowed: pending.is_none() && self.date_authority.is_trusted(),
            };
        }
        if (nearby && host_progression != Some(false)) || host_progression == Some(true) {
            let boundary = pending.is_some() || !nearby;
            if boundary {
                self.retire_segment();
            }
            if boundary {
                self.segment_start_host = if host_progression == Some(true) {
                    previous.host.into_iter().chain(sample.host).min()
                } else {
                    sample.host
                };
            }

            self.state =
                ReceiverClockState::Coherent(if sample.receiver >= previous.receiver || boundary {
                    sample
                } else {
                    previous
                });
            self.include_receiver(sample.receiver);
            self.observe_date(
                sample,
                tolerance,
                !nearby
                    && host_progression != Some(true)
                    && sample.receiver.date_naive() != previous.receiver.date_naive(),
            );
            if sample.date_observed {
                self.latest_dated_host =
                    self.latest_dated_host.into_iter().chain(sample.host).max();
            }
            self.host_anchor =
                sample
                    .host_anchor()
                    .or(if boundary { None } else { self.host_anchor });
            return ReceiverDecision {
                segment: self.segment,
                reliable: true,
                boundary,
                discontinuity: boundary,
                confirmed_predecessor: None,
                ambiguous: false,
                date_anchor_allowed: self.date_authority.is_trusted(),
            };
        }
        if pending.is_some_and(|pending| {
            let step = sample
                .receiver
                .signed_duration_since(pending.sample.receiver);
            let coherent_step = step.to_std().is_ok_and(|distance| {
                !distance.is_zero()
                    && (distance <= tolerance.saturating_mul(COHERENT_FORWARD_WINDOWS)
                        || pending.previous_step.is_some_and(|previous| {
                            (step - previous)
                                .abs()
                                .to_std()
                                .is_ok_and(|difference| difference <= tolerance)
                        }))
            });
            coherent_step
                && (sample.receiver.date_naive() == previous.receiver.date_naive()
                    || sample
                        .host
                        .zip(self.latest_dated_host)
                        .is_none_or(|(host, latest)| host >= latest))
                && (sample.date_observed || pending.sample.date_observed)
                && sample
                    .host
                    .zip(pending.sample.host)
                    .is_none_or(|(current, previous)| {
                        (step - current.signed_duration_since(previous))
                            .abs()
                            .to_std()
                            .is_ok_and(|distance| distance <= tolerance)
                    })
        }) {
            self.observe_date(
                sample,
                tolerance,
                sample.receiver.date_naive() != previous.receiver.date_naive(),
            );
            self.segment_start_host = pending
                .and_then(|candidate| candidate.sample.host)
                .into_iter()
                .chain(sample.host)
                .min();
            if sample.date_observed {
                self.latest_dated_host =
                    self.latest_dated_host.into_iter().chain(sample.host).max();
            }
            self.state = ReceiverClockState::Coherent(sample);
            self.host_anchor = sample.host_anchor();
            if let Some(pending) = pending {
                self.include_receiver(pending.sample.receiver);
            }
            self.include_receiver(sample.receiver);
            return ReceiverDecision {
                segment: self.segment,
                reliable: true,
                boundary: false,
                discontinuity: true,
                confirmed_predecessor: pending.map(|pending| pending.sample.receiver),
                ambiguous: false,
                date_anchor_allowed: self.date_authority.is_trusted(),
            };
        }
        self.observe_date(
            sample,
            tolerance,
            !nearby && sample.receiver.date_naive() != previous.receiver.date_naive(),
        );
        let boundary = pending.is_none();
        if boundary {
            self.retire_segment();
        }
        self.state = ReceiverClockState::Discontinuous {
            coherent: previous,
            coherent_segment,
            pending: ReceiverCandidate {
                sample,
                previous_step: pending.and_then(|pending| {
                    let step = sample
                        .receiver
                        .signed_duration_since(pending.sample.receiver);
                    (step > TimeDelta::zero()).then_some(step)
                }),
            },
        };
        ReceiverDecision {
            segment: self.segment,
            reliable: false,
            boundary,
            discontinuity: true,
            confirmed_predecessor: None,
            ambiguous: false,
            date_anchor_allowed: true,
        }
    }

    fn classify_ambiguous_capture(&self, sample: ReceiverSample) -> Option<ReceiverDecision> {
        if matches!(self.state, ReceiverClockState::Discontinuous { coherent, .. } if sample.receiver == coherent.receiver)
            && sample.date_observed
        {
            return Some(ReceiverDecision {
                segment: self.segment,
                reliable: false,
                boundary: false,
                discontinuity: true,
                confirmed_predecessor: None,
                ambiguous: true,
                date_anchor_allowed: false,
            });
        }
        if sample.host.is_none()
            && matches!(self.state, ReceiverClockState::Coherent(_))
            && self
                .active_span
                .is_some_and(|span| span.contains(sample.receiver))
            && self
                .retired_span
                .is_some_and(|span| span.contains(sample.receiver))
        {
            return Some(ReceiverDecision {
                segment: self.segment,
                reliable: false,
                boundary: false,
                discontinuity: true,
                confirmed_predecessor: None,
                ambiguous: true,
                date_anchor_allowed: false,
            });
        }
        if matches!(self.state, ReceiverClockState::Discontinuous { .. })
            && sample.host.is_none()
            && !sample.date_observed
            && self
                .retired_span
                .is_some_and(|span| span.contains(sample.receiver))
        {
            return Some(ReceiverDecision {
                segment: self.segment,
                reliable: false,
                boundary: false,
                discontinuity: true,
                confirmed_predecessor: None,
                ambiguous: true,
                date_anchor_allowed: false,
            });
        }
        if !sample.date_observed && !self.permits_date_attribution(sample.host, sample.receiver) {
            return Some(ReceiverDecision {
                segment: self.segment,
                reliable: false,
                boundary: false,
                discontinuity: false,
                confirmed_predecessor: None,
                ambiguous: false,
                date_anchor_allowed: false,
            });
        }
        None
    }

    fn classify_retired_capture(
        &self,
        sample: ReceiverSample,
        tolerance: Duration,
        observed_date_change: bool,
    ) -> Option<ReceiverDecision> {
        if sample
            .host
            .zip(self.latest_dated_host)
            .is_some_and(|(capture, latest)| capture < latest)
            && let Some(anchor) =
                self.retired_anchors.iter().rev().find(|retired| {
                    retired.anchor.permits_progression(sample, tolerance) == Some(true)
                })
        {
            return Some(ReceiverDecision {
                segment: anchor.segment,
                reliable: true,
                boundary: false,
                discontinuity: false,
                confirmed_predecessor: None,
                ambiguous: false,
                date_anchor_allowed: false,
            });
        }
        if sample
            .host
            .zip(self.latest_dated_host)
            .is_some_and(|(host, latest)| host < latest)
            && !observed_date_change
        {
            return Some(ReceiverDecision {
                segment: self.segment,
                reliable: false,
                boundary: false,
                discontinuity: true,
                confirmed_predecessor: None,
                ambiguous: true,
                date_anchor_allowed: false,
            });
        }
        None
    }

    fn observe_date(&mut self, sample: ReceiverSample, tolerance: Duration, changed: bool) {
        if sample.date_observed {
            self.date_authority = self.date_authority.observe(sample, tolerance, changed);
        }
    }

    pub fn permits_date_attribution(
        &self,
        host: Option<DateTime<Utc>>,
        receiver: DateTime<Utc>,
    ) -> bool {
        if !self.date_authority.is_trusted() {
            return false;
        }
        if self.conflicting_dates
            && self
                .retired_span
                .is_some_and(|span| span.has_ambiguous_date(receiver, self.date_anchor_tolerance))
        {
            return false;
        }
        if self.conflicting_dates && self.active_span.is_some_and(|span| receiver < span.first) {
            return false;
        }
        if self.retired_span.is_none() {
            return true;
        }
        let same_epoch = matches!(self.state, ReceiverClockState::Coherent(sample) if sample.receiver == receiver);
        same_epoch
            || match (host, self.segment_start_host) {
                (Some(capture), Some(start)) => capture >= start,
                (Some(_), None) => false,
                (None, _) => !self.conflicting_dates,
            }
    }

    fn include_receiver(&mut self, timestamp: DateTime<Utc>) {
        self.active_span = Some(
            self.active_span
                .map_or(ReceiverSpan::new(timestamp), |span| span.include(timestamp)),
        );
    }

    fn retire_segment(&mut self) {
        self.segment_start_host = None;
        if let Some(anchor) = self.host_anchor {
            if self.retired_anchors.len() >= self.history_limit.get() {
                self.retired_anchors.pop_front();
            }
            self.retired_anchors.push_back(RetiredReceiverAnchor {
                anchor,
                segment: self.segment,
            });
        }
        if let Some(span) = self.active_span.take() {
            self.retired_span = Some(self.retired_span.map_or(span, |retired| {
                retired.include(span.first).include(span.last)
            }));
        }
        self.segment.0 = self.segment.0.saturating_add(1);
    }

    pub fn segment(&self) -> ReceiverSegment {
        self.segment
    }

    pub fn unresolved(&self) -> bool {
        matches!(self.state, ReceiverClockState::Discontinuous { .. })
            || !self.date_authority.is_trusted()
    }

    pub fn compatible_date(&self, candidate: DateTime<Utc>, tolerance: Duration) -> bool {
        if !self.date_authority.is_trusted() {
            return false;
        }
        match self.state {
            ReceiverClockState::Discontinuous {
                coherent, pending, ..
            } => {
                pending.sample.receiver.date_naive() == candidate.date_naive()
                    && pending
                        .sample
                        .receiver
                        .signed_duration_since(coherent.receiver)
                        .abs()
                        .to_std()
                        .is_ok_and(|distance| distance <= tolerance)
            }
            ReceiverClockState::Coherent(_) | ReceiverClockState::Unanchored => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum ReceiverClockState {
    Coherent(ReceiverSample),
    Discontinuous {
        coherent: ReceiverSample,
        coherent_segment: ReceiverSegment,
        pending: ReceiverCandidate,
    },
    #[default]
    Unanchored,
}

#[derive(Clone, Copy, Debug)]
struct ReceiverCandidate {
    sample: ReceiverSample,
    previous_step: Option<TimeDelta>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReceiverSample {
    pub receiver: DateTime<Utc>,
    pub host: Option<DateTime<Utc>>,
    pub date_observed: bool,
}

impl ReceiverSample {
    fn host_anchor(self) -> Option<ReceiverHostAnchor> {
        self.host.map(|host| ReceiverHostAnchor {
            receiver: self.receiver,
            host,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct ReceiverHostAnchor {
    receiver: DateTime<Utc>,
    host: DateTime<Utc>,
}

impl ReceiverHostAnchor {
    fn permits_progression(self, sample: ReceiverSample, tolerance: Duration) -> Option<bool> {
        sample.host.map(|host| {
            (sample.receiver.signed_duration_since(self.receiver)
                - host.signed_duration_since(self.host))
            .abs()
            .to_std()
            .is_ok_and(|difference| difference <= tolerance)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReceiverDecision {
    pub segment: ReceiverSegment,
    pub reliable: bool,
    pub boundary: bool,
    pub discontinuity: bool,
    pub confirmed_predecessor: Option<DateTime<Utc>>,
    pub ambiguous: bool,
    pub date_anchor_allowed: bool,
}

#[derive(Clone, Copy, Debug)]
struct RetiredReceiverAnchor {
    anchor: ReceiverHostAnchor,
    segment: ReceiverSegment,
}

#[derive(Clone, Copy, Debug)]
struct ReceiverSpan {
    first: DateTime<Utc>,
    last: DateTime<Utc>,
}

impl ReceiverSpan {
    fn new(timestamp: DateTime<Utc>) -> Self {
        Self {
            first: timestamp,
            last: timestamp,
        }
    }

    fn include(self, timestamp: DateTime<Utc>) -> Self {
        Self {
            first: self.first.min(timestamp),
            last: self.last.max(timestamp),
        }
    }

    fn has_ambiguous_date(self, receiver: DateTime<Utc>, tolerance: Duration) -> bool {
        if self.first.date_naive() == self.last.date_naive()
            && receiver.date_naive() == self.first.date_naive()
        {
            return false;
        }
        [self.first.date_naive(), self.last.date_naive()]
            .into_iter()
            .any(|date| {
                [
                    date.checked_sub_days(Days::new(1)),
                    Some(date),
                    date.checked_add_days(Days::new(1)),
                ]
                .into_iter()
                .flatten()
                .any(|date| {
                    let candidate = date.and_time(receiver.time()).and_utc();
                    let distance = if candidate < self.first {
                        self.first - candidate
                    } else if candidate > self.last {
                        candidate - self.last
                    } else {
                        TimeDelta::zero()
                    };
                    distance
                        .to_std()
                        .is_ok_and(|distance| distance <= tolerance)
                })
            })
    }

    fn contains(self, timestamp: DateTime<Utc>) -> bool {
        self.first <= timestamp && timestamp <= self.last
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Watermarks {
    pub host: Option<DateTime<Utc>>,
    pub receiver: Option<DateTime<Utc>>,
    pub receiver_segment: ReceiverSegment,
}

impl Watermarks {
    pub fn advance(&mut self, time: CaptureContext) {
        self.host = self.host.into_iter().chain(time.host).max();
        if let Some(receiver) = time.receiver_progress {
            if time.segment > self.receiver_segment {
                self.receiver = Some(receiver);
                self.receiver_segment = time.segment;
            } else if time.segment == self.receiver_segment {
                self.receiver = self.receiver.into_iter().chain([receiver]).max();
            }
        }
    }

    pub fn expired(&self, time: CaptureContext, window: Duration) -> bool {
        let host_expired = self.host.zip(time.host).is_some_and(|(now, then)| {
            now.signed_duration_since(then)
                .to_std()
                .is_ok_and(|age| age > window)
        });
        let receiver_expired = time.segment == self.receiver_segment
            && self
                .receiver
                .zip(time.receiver_progress)
                .is_some_and(|(now, then)| {
                    now.signed_duration_since(then)
                        .to_std()
                        .is_ok_and(|age| age > window)
                });
        host_expired || receiver_expired
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ReceiverDateAnchor {
    latest: Option<DateTime<Utc>>,
}

impl ReceiverDateAnchor {
    pub fn observe(&mut self, timestamp: DateTime<Utc>) {
        self.latest = self.latest.into_iter().chain([timestamp]).max();
    }

    pub fn resolve(&self, time: NaiveTime, tolerance: Duration) -> Option<DateTime<Utc>> {
        let anchor = self.latest?;
        let date = anchor.date_naive();
        [
            date.checked_sub_days(Days::new(1)),
            Some(date),
            date.checked_add_days(Days::new(1)),
        ]
        .into_iter()
        .flatten()
        .map(|date| date.and_time(time).and_utc())
        .filter_map(|candidate| {
            let distance = candidate
                .signed_duration_since(anchor)
                .abs()
                .to_std()
                .ok()?;
            (distance <= tolerance).then_some((distance, candidate))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LineInput<'a> {
    pub sentence: &'a str,
    pub host: Option<DateTime<Utc>>,
}

impl<'a> LineInput<'a> {
    pub fn parse(
        mode: &ConfiguredHostTimestamp,
        line: &'a str,
        caller_time: Option<DateTime<Utc>>,
    ) -> Result<Self, String> {
        let line = line.trim();
        match mode {
            ConfiguredHostTimestamp::None => Ok(Self {
                sentence: line,
                host: caller_time,
            }),
            ConfiguredHostTimestamp::CallerProvided => caller_time
                .map(|host| Self {
                    sentence: line,
                    host: Some(host),
                })
                .ok_or_else(|| "Caller timestamp is required".to_owned()),
            ConfiguredHostTimestamp::PrefixUtc(format) => {
                if caller_time.is_some() {
                    return Err(
                        "Caller timestamp conflicts with the configured timestamp prefix"
                            .to_owned(),
                    );
                }
                let start = line
                    .find('$')
                    .ok_or_else(|| "NMEA sentence delimiter is missing".to_owned())?;
                let prefix = line.get(..start).unwrap_or_default().trim();
                let sentence = line.get(start..).unwrap_or_default();
                let host = format.parse(prefix)?;
                Ok(Self {
                    sentence,
                    host: Some(host),
                })
            }
        }
    }
}

const COHERENT_FORWARD_WINDOWS: u32 = 4;

#[derive(Clone, Copy, Debug)]
enum DateAuthority {
    Candidate(ReceiverSample),
    Trusted,
}

impl DateAuthority {
    fn observe(self, sample: ReceiverSample, tolerance: Duration, changed: bool) -> Self {
        match self {
            Self::Trusted if changed => Self::Candidate(sample),
            Self::Trusted => Self::Trusted,
            Self::Candidate(previous) => {
                let progression = sample.receiver.signed_duration_since(previous.receiver);
                let increasing = progression > TimeDelta::zero();
                let clock_distance = progression.to_std().is_ok_and(|distance| {
                    distance <= tolerance.saturating_mul(COHERENT_FORWARD_WINDOWS)
                });
                let paired_progression =
                    sample.host.zip(previous.host).map(|(current, previous)| {
                        (progression - current.signed_duration_since(previous))
                            .abs()
                            .to_std()
                            .is_ok_and(|distance| distance <= tolerance)
                    });
                if increasing && clock_distance && paired_progression != Some(false) {
                    Self::Trusted
                } else {
                    Self::Candidate(sample)
                }
            }
        }
    }

    fn is_trusted(self) -> bool {
        matches!(self, Self::Trusted)
    }
}
