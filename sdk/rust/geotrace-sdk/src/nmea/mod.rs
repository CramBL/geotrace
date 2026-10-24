//! Incremental NMEA ingestion with bounded temporal aggregation.
//!
//! Enable the `nmea` feature. Receiver dates require an RMC or ZDA anchor.
//! GSA/GSV receiver times are inferred associations and produce diagnostics.
//! GGA/GNS altitude and geoid separation have no fields in the recording model.

#![warn(missing_docs)]

use std::collections::VecDeque;
use std::io::Error as IoError;
use std::ops::RangeInclusive;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use nmea::sentences::GsvData;
use nmea::{Error as NmeaParseError, ParseResult};
use thiserror::Error;

use crate::{
    BuildError, NavFile, NavFix, NavFixTime, NavRecord, NavRecorder, SatelliteObservationReport,
    SatelliteUsageReport,
};

pub use self::config::{
    HostTimestampMode, NmeaAssociationPolicy, NmeaConfig, NmeaConfigError, NmeaGsvCyclePolicy,
    NmeaIngestorBuilder, SentencePeriod,
};
pub use self::diagnostics::{
    NmeaDiagnostic, NmeaDiagnosticCount, NmeaDiagnosticKind, NmeaDiagnostics,
};
pub use nmea::SentenceType;

use self::navigation::{NavigationEpoch, NavigationFields, NavigationInput};
use self::satellites::{GsvGroup, GsvKey, PendingKind, PendingReport};
use self::schedule::Schedules;
use self::time::{
    CaptureContext, LineInput, ReceiverClock, ReceiverDateAnchor, ReceiverDecision, ReceiverSample,
    Watermarks,
};

mod config;
mod diagnostics;
mod navigation;
mod reader;
#[cfg(test)]
mod resource_properties;
mod satellites;
mod schedule;
mod time;

/// A generated navigation or satellite record accepted by [`NavRecorder::add`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NmeaRecord {
    /// A navigation epoch with independently observed receiver and host clocks.
    Navigation(NavFix),
    /// An independent constellation-scoped observation update.
    SatelliteObservation(SatelliteObservationReport),
    /// An independent constellation-scoped usage update.
    SatelliteUsage(SatelliteUsageReport),
}

impl NavRecord for NmeaRecord {
    fn add_to(self, recorder: &mut NavRecorder) {
        match self {
            Self::Navigation(fix) => {
                recorder.add_nav_fix(fix);
            }
            Self::SatelliteUsage(report) => {
                recorder.add_satellite_usage_report(report);
            }
            Self::SatelliteObservation(report) => {
                recorder.add_satellite_observation_report(report);
            }
        }
    }
}

/// Saturating input and generated-record counts, independent of recoverable diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct NmeaStatistics {
    /// Number of input lines, including blank, malformed and unsupported lines.
    pub lines: u64,
    /// Number of successfully parsed supported sentences.
    pub supported_sentences: u64,
    /// Number of generated navigation epochs, including status-only records.
    pub navigation_fixes: u64,
    /// Number of generated independent usage reports.
    pub satellite_usage_reports: u64,
    /// Number of generated independent observation reports.
    pub satellite_observation_reports: u64,
}

/// Final buffered records and the complete diagnostic and record totals.
#[derive(Debug)]
#[non_exhaustive]
pub struct NmeaCompletion {
    /// Records remaining after finalization. Submit these after all incremental records.
    pub records: Vec<NmeaRecord>,
    /// Complete aggregated counts and bounded diagnostic examples.
    pub diagnostics: NmeaDiagnostics,
    /// Complete input and generated-record totals.
    pub statistics: NmeaStatistics,
}

/// A generated recording with diagnostic summaries and ingestion totals.
#[derive(Debug)]
#[non_exhaustive]
pub struct NmeaRecording {
    /// The recording constructed through the existing satellite association API.
    pub recording: NavFile,
    /// Complete aggregated counts and bounded diagnostic examples.
    pub diagnostics: NmeaDiagnostics,
    /// Complete input and generated-record totals.
    pub statistics: NmeaStatistics,
}

/// Fatal source I/O, timeline or recording-construction failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NmeaError {
    /// The source could not supply the next input line.
    #[error("Reading NMEA input at line {line}: {source}")]
    Input {
        /// One-based line being read when I/O failed.
        line: u64,
        /// Original source I/O error.
        #[source]
        source: IoError,
    },
    /// No generated navigation observation has a usable absolute timestamp.
    #[error("NMEA input has no navigation observation with a usable absolute timestamp")]
    MissingNavigationTimeline {
        /// Recoverable diagnostics collected before the timeline failure.
        diagnostics: NmeaDiagnostics,
        /// Generated record counts, including satellite-only evidence.
        statistics: NmeaStatistics,
    },
    /// The existing recorder could not construct a recording.
    #[error("Constructing the NMEA recording: {0}")]
    Recording(#[source] BuildError),
}

/// Incrementally parse NMEA sentences and finalize bounded navigation and satellite aggregation.
#[derive(Debug)]
pub struct NmeaIngestor {
    config: NmeaConfig,
    diagnostics: NmeaDiagnostics,
    statistics: NmeaStatistics,
    sequence: u64,
    anchor: ReceiverDateAnchor,
    receiver_clock: ReceiverClock,
    watermarks: Watermarks,
    schedules: Schedules,
    epochs: Vec<NavigationEpoch>,
    pending: Vec<PendingReport>,
    gsv_groups: Vec<GsvGroup>,
    completed_gsv: Vec<GsvGroup>,
    capture_history: CaptureHistory,
    finalized_receiver_epochs: VecDeque<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SentenceCapture {
    Host(DateTime<Utc>),
    Receiver(DateTime<Utc>),
}

impl SentenceCapture {
    fn from_sentence(input: &LineInput<'_>, parsed: &ParseResult) -> Option<Self> {
        input.host.map(Self::Host).or_else(|| {
            if let ParseResult::RMC(data) = parsed {
                data.fix_date
                    .zip(data.fix_time)
                    .map(|(date, time)| Self::Receiver(date.and_time(time).and_utc()))
            } else {
                None
            }
        })
    }
}

#[derive(Debug)]
struct CapturedSentence {
    capture: SentenceCapture,
    sentence: String,
}

#[derive(Clone, Copy, Debug)]
enum CaptureStatus {
    Forgotten,
    New,
    Repeated,
}

#[derive(Debug, Default)]
struct CaptureHistory {
    recent: VecDeque<CapturedSentence>,
    forgotten_host: Option<RangeInclusive<DateTime<Utc>>>,
    forgotten_receiver: Option<RangeInclusive<DateTime<Utc>>>,
}

impl CaptureHistory {
    fn classify(&self, input: &LineInput<'_>, capture: Option<SentenceCapture>) -> CaptureStatus {
        let Some(capture) = capture else {
            return CaptureStatus::New;
        };
        if self
            .recent
            .iter()
            .any(|previous| previous.capture == capture && previous.sentence == input.sentence)
        {
            return CaptureStatus::Repeated;
        }
        let forgotten = match capture {
            SentenceCapture::Host(host) => self
                .forgotten_host
                .as_ref()
                .is_some_and(|range| range.contains(&host)),
            SentenceCapture::Receiver(receiver) => self
                .forgotten_receiver
                .as_ref()
                .is_some_and(|range| range.contains(&receiver)),
        };
        if forgotten {
            CaptureStatus::Forgotten
        } else {
            CaptureStatus::New
        }
    }

    fn remember(&mut self, captured: CapturedSentence, capacity: usize) {
        if self.recent.len() >= capacity
            && let Some(expired) = self.recent.pop_front()
        {
            let (range, timestamp) = match expired.capture {
                SentenceCapture::Host(host) => (&mut self.forgotten_host, host),
                SentenceCapture::Receiver(receiver) => (&mut self.forgotten_receiver, receiver),
            };
            *range = Some(range.as_ref().map_or(timestamp..=timestamp, |range| {
                (*range.start()).min(timestamp)..=(*range.end()).max(timestamp)
            }));
        }
        self.recent.push_back(captured);
    }
}

impl NmeaIngestor {
    /// Start with optional caller timestamps and default buffer limits.
    /// Expected sentence schedules are initially empty.
    pub fn builder() -> NmeaIngestorBuilder {
        NmeaIngestorBuilder::default()
    }

    fn supported(sentence: SentenceType) -> bool {
        matches!(
            sentence,
            SentenceType::GGA
                | SentenceType::RMC
                | SentenceType::GNS
                | SentenceType::GLL
                | SentenceType::ZDA
                | SentenceType::GSA
                | SentenceType::GSV
                | SentenceType::VTG
        )
    }

    pub(super) fn from_config(config: NmeaConfig) -> Self {
        let diagnostics = NmeaDiagnostics::with_sample_limit(config.diagnostic_samples);
        let schedules = Schedules::new(&config);
        let receiver_clock =
            ReceiverClock::new(config.max_pending_epochs, config.date_anchor_tolerance);
        Self {
            config,
            diagnostics,
            statistics: NmeaStatistics::default(),
            sequence: 0,
            anchor: ReceiverDateAnchor::default(),
            receiver_clock,
            watermarks: Watermarks::default(),
            schedules,
            epochs: Vec::new(),
            pending: Vec::new(),
            gsv_groups: Vec::new(),
            completed_gsv: Vec::new(),
            capture_history: CaptureHistory::default(),
            finalized_receiver_epochs: VecDeque::new(),
        }
    }

    /// Process one complete input line and return every record finalized by this call.
    /// Recoverable failures are added to [`Self::diagnostics`].
    /// Call [`Self::finish`] after input ends.
    ///
    /// ```
    /// use geotrace_sdk::{NavFileBuilder, NmeaIngestor};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut ingestor = NmeaIngestor::builder().build()?;
    /// let mut recorder = NavFileBuilder::new().open();
    /// let line = "$GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A*52";
    /// for record in ingestor.push_line(line) {
    ///     recorder.add(record);
    /// }
    /// let completion = ingestor.finish();
    /// for record in completion.records {
    ///     recorder.add(record);
    /// }
    /// let recording = recorder.finish()?;
    /// assert_eq!(recording.nav_points().len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn push_line(&mut self, line: &str) -> Vec<NmeaRecord> {
        self.process_line(line, None)
    }

    /// Process a sentence with its caller-observed UTC host timestamp.
    /// The default and caller timestamp modes accept this clock. Prefix mode reports a conflict.
    pub fn push_line_at(&mut self, line: &str, host: DateTime<Utc>) -> Vec<NmeaRecord> {
        self.process_line(line, Some(host))
    }

    /// Flush incomplete navigation epochs and partial satellite reports with usable timestamps.
    /// The returned records must be submitted after records from incremental calls.
    pub fn finish(mut self) -> NmeaCompletion {
        let mut records = Vec::new();
        let groups = std::mem::take(&mut self.gsv_groups);
        for group in groups {
            self.push_gsv_group(group, &mut records);
        }
        let pending = std::mem::take(&mut self.pending);
        for report in pending {
            self.emit_pending(report, &mut records);
        }
        while !self.epochs.is_empty() {
            self.flush_epoch(0, &mut records);
        }
        self.schedules
            .finish(self.statistics.lines, &mut self.diagnostics);
        self.count_records(&records);
        NmeaCompletion {
            records,
            diagnostics: self.diagnostics,
            statistics: self.statistics,
        }
    }

    /// Borrow the validated clock, schedule and buffer configuration.
    pub fn config(&self) -> &NmeaConfig {
        &self.config
    }

    /// Borrow cumulative diagnostic counts and bounded examples during ingestion.
    pub fn diagnostics(&self) -> &NmeaDiagnostics {
        &self.diagnostics
    }

    /// Return cumulative, saturating input and output counts.
    pub fn statistics(&self) -> NmeaStatistics {
        self.statistics
    }

    fn process_line(&mut self, line: &str, host: Option<DateTime<Utc>>) -> Vec<NmeaRecord> {
        self.statistics.lines = self.statistics.lines.saturating_add(1);
        if line.len() > self.config.max_line_bytes.get() {
            self.report(
                NmeaDiagnosticKind::LineTooLong,
                None,
                "Input line exceeds the configured byte limit",
            );
            return Vec::new();
        }
        if line.trim().is_empty() {
            return Vec::new();
        }
        let input = match LineInput::parse(&self.config.host_timestamp, line, host) {
            Ok(input) => input,
            Err(reason) => {
                self.report(NmeaDiagnosticKind::InvalidHostTimestamp, None, reason);
                return Vec::new();
            }
        };
        let Some(parsed) = self.parse_supported_sentence(input.sentence) else {
            return Vec::new();
        };
        self.statistics.supported_sentences = self.statistics.supported_sentences.saturating_add(1);
        let capture = SentenceCapture::from_sentence(&input, &parsed);
        match self.capture_history.classify(&input, capture) {
            CaptureStatus::Repeated => return Vec::new(),
            CaptureStatus::Forgotten => {
                self.report(
                    NmeaDiagnosticKind::UnplacedObservation,
                    Some(SentenceType::from(&parsed)),
                    "Capture identity is ambiguous after bounded history expiration",
                );
                return Vec::new();
            }
            CaptureStatus::New => {}
        }
        let sentence = SentenceType::from(&parsed);
        self.sequence = self.sequence.saturating_add(1);
        let mut time = CaptureContext {
            host: input.host,
            receiver_progress: None,
            line: self.statistics.lines,
            sequence: self.sequence,
            segment: self.receiver_clock.segment(),
            receiver_ambiguous: false,
            date_anchor_allowed: true,
        };
        let mut records = Vec::new();
        match parsed {
            ParseResult::GGA(_)
            | ParseResult::RMC(_)
            | ParseResult::GNS(_)
            | ParseResult::GLL(_) => {
                if let Some(mut fields) = NavigationFields::from_parsed(parsed) {
                    let date_observed = fields.receiver_time.is_some();
                    if fields.receiver_time.is_none() {
                        fields.receiver_time = fields
                            .time_of_day
                            .and_then(|time| {
                                self.anchor.resolve(time, self.config.date_anchor_tolerance)
                            })
                            .filter(|candidate| {
                                self.receiver_clock
                                    .compatible_date(*candidate, self.config.date_anchor_tolerance)
                            });
                    }
                    let reliable = fields.receiver_time.is_some_and(|receiver| {
                        self.observe_receiver(receiver, date_observed, &mut time, &mut records)
                    });
                    if time.receiver_ambiguous || (!reliable && !date_observed) {
                        fields.receiver_time = None;
                    }
                    if reliable && time.date_anchor_allowed {
                        self.establish_date_anchor(time, fields.receiver_time);
                    }
                    self.insert_navigation(
                        NavigationInput {
                            segment: time.segment,
                            receiver_reliable: reliable,
                            date_attribution_allowed: !self.receiver_clock.unresolved()
                                && !time.receiver_ambiguous,
                            fields,
                            host_time: input.host,
                            line: self.statistics.lines,
                            sequence: self.sequence,
                            sentence,
                        },
                        &mut records,
                    );
                }
            }
            ParseResult::ZDA(data) => {
                let date =
                    data.year
                        .zip(data.month)
                        .zip(data.day)
                        .and_then(|((year, month), day)| {
                            NaiveDate::from_ymd_opt(
                                i32::from(year),
                                u32::from(month),
                                u32::from(day),
                            )
                        });
                time.receiver_progress = date
                    .zip(data.utc_time)
                    .map(|(date, time)| date.and_time(time).and_utc());
                if let Some(receiver) = time.receiver_progress {
                    time.receiver_progress = None;
                    if self.observe_receiver(receiver, true, &mut time, &mut records)
                        && time.date_anchor_allowed
                    {
                        self.establish_date_anchor(time, Some(receiver));
                    }
                } else {
                    self.report(
                        NmeaDiagnosticKind::UnplacedObservation,
                        Some(sentence),
                        "ZDA has no complete receiver date and time",
                    );
                }
            }
            ParseResult::GSA(data) => {
                time.receiver_progress = self.watermarks.receiver;
                let pending = PendingReport::gsa(data, time, &mut self.diagnostics);
                self.push_pending(pending, &mut records);
            }
            ParseResult::GSV(data) => {
                time.receiver_progress = self.watermarks.receiver;
                let talker = input.sentence.get(1..3).unwrap_or_default();
                self.insert_gsv(talker, data, time, &mut records);
            }
            ParseResult::VTG(data) => {
                time.receiver_progress = self.watermarks.receiver;
                self.push_pending(
                    PendingReport {
                        navigation_association_lost: false,
                        time,
                        last_sequence: time.sequence,
                        kind: PendingKind::Velocity(data),
                    },
                    &mut records,
                );
            }
            _ => {}
        }
        self.remember_capture(&input, capture, time);
        self.watermarks.advance(time);
        self.schedules.advance(time, &mut self.diagnostics);
        if sentence != SentenceType::GSV
            && (time.host.is_some() || !matches!(sentence, SentenceType::GSA | SentenceType::VTG))
        {
            let schedule_time = if matches!(sentence, SentenceType::GSA | SentenceType::VTG) {
                CaptureContext {
                    receiver_progress: None,
                    ..time
                }
            } else {
                time
            };
            self.schedules
                .observe(sentence, schedule_time, &mut self.diagnostics);
        }
        self.expire(&mut records);
        self.count_records(&records);
        records
    }

    fn parse_supported_sentence(&mut self, sentence: &str) -> Option<ParseResult> {
        if !sentence.is_ascii() {
            self.report(
                NmeaDiagnosticKind::MalformedSentence,
                None,
                "NMEA sentences must contain ASCII characters",
            );
            return None;
        }
        let sentence_type = sentence
            .strip_prefix('$')
            .and_then(|text| text.split(',').next())
            .and_then(|header| header.get(2..))
            .and_then(|kind| SentenceType::try_from(kind).ok());
        let parsed = match nmea::parse_str(sentence) {
            Ok(parsed) => parsed,
            Err(error) => {
                let kind = if matches!(error, NmeaParseError::SystemIdMismatch { .. }) {
                    NmeaDiagnosticKind::UnresolvedSatelliteIdentity
                } else if matches!(error, NmeaParseError::UnknownGnssType(_)) {
                    NmeaDiagnosticKind::UnsupportedVariation
                } else if sentence_type.is_none()
                    || sentence_type.is_some_and(|sentence| !Self::supported(sentence))
                {
                    NmeaDiagnosticKind::UnsupportedSentence
                } else {
                    NmeaDiagnosticKind::MalformedSentence
                };
                self.report(kind, sentence_type, error.to_string());
                return None;
            }
        };
        let sentence = SentenceType::from(&parsed);
        if !Self::supported(sentence) {
            self.report(
                NmeaDiagnosticKind::UnsupportedSentence,
                Some(sentence),
                "Sentence has no supported recording observation",
            );
            return None;
        }
        Some(parsed)
    }

    fn remember_capture(
        &mut self,
        input: &LineInput<'_>,
        capture: Option<SentenceCapture>,
        time: CaptureContext,
    ) {
        let Some(capture) = capture else {
            return;
        };
        if matches!(capture, SentenceCapture::Receiver(_)) && time.receiver_progress.is_none() {
            return;
        }
        self.capture_history.remember(
            CapturedSentence {
                capture,
                sentence: input.sentence.to_owned(),
            },
            self.config.max_buffered_lines.get(),
        );
    }

    fn observe_receiver(
        &mut self,
        receiver: DateTime<Utc>,
        date_observed: bool,
        time: &mut CaptureContext,
        records: &mut Vec<NmeaRecord>,
    ) -> bool {
        let decision = self.receiver_clock.observe(
            ReceiverSample {
                receiver,
                host: time.host,
                date_observed,
            },
            self.config.navigation_window,
        );
        if decision.discontinuity && !decision.reliable {
            self.invalidate_inferred_dates(receiver);
        }
        if decision.discontinuity {
            self.report(
                NmeaDiagnosticKind::TimestampDiscontinuity,
                None,
                if decision.reliable {
                    "Receiver clock progression established a coherent clock segment"
                } else {
                    "Receiver timestamp is isolated from the coherent clock segment"
                },
            );
        }
        if decision.boundary {
            for pending in &mut self.pending {
                pending.navigation_association_lost = true;
            }
            for group in &mut self.gsv_groups {
                group.navigation_association_lost = true;
                group.ambiguous_cycle = true;
            }
            let groups = std::mem::take(&mut self.gsv_groups);
            for group in groups {
                self.push_gsv_group(group, records);
            }
            let pending = std::mem::take(&mut self.pending);
            for report in pending {
                self.emit_pending(report, records);
            }
            while !self.epochs.is_empty() {
                self.flush_epoch(0, records);
            }
            self.completed_gsv
                .retain(|group| group.first.host.is_some());
            self.finalized_receiver_epochs.clear();
            self.watermarks.receiver = None;
            self.schedules.receiver_boundary(time.sequence);
        }
        time.segment = decision.segment;
        time.receiver_ambiguous = decision.ambiguous;
        time.date_anchor_allowed = decision.date_anchor_allowed;
        time.receiver_progress = decision.reliable.then_some(receiver);
        if decision.reliable {
            if decision.boundary || decision.confirmed_predecessor.is_some() {
                self.anchor = ReceiverDateAnchor::default();
            }
            self.confirm_receiver_predecessor(decision);
        }
        decision.reliable
    }

    fn invalidate_inferred_dates(&mut self, receiver: DateTime<Utc>) {
        for epoch in &mut self.epochs {
            if !epoch.sentences.contains(&SentenceType::RMC)
                && epoch
                    .receiver_time
                    .is_some_and(|previous| previous.date_naive() != receiver.date_naive())
            {
                epoch.receiver_time = None;
                epoch.receiver_reliable = false;
                epoch.date_attribution_allowed = false;
            }
        }
    }

    fn confirm_receiver_predecessor(&mut self, decision: ReceiverDecision) {
        for epoch in &mut self.epochs {
            if epoch.segment == decision.segment
                && epoch.receiver_time.is_some()
                && epoch.receiver_time == decision.confirmed_predecessor
            {
                epoch.receiver_reliable = true;
                for sentence in &epoch.sentences {
                    self.schedules.observe(
                        *sentence,
                        CaptureContext {
                            host: epoch.host_time,
                            receiver_progress: epoch.receiver_time,
                            line: epoch.first_line,
                            sequence: epoch.first_sequence,
                            segment: epoch.segment,
                            receiver_ambiguous: false,
                            date_anchor_allowed: true,
                        },
                        &mut self.diagnostics,
                    );
                }
            }
        }
    }

    fn establish_date_anchor(&mut self, capture: CaptureContext, receiver: Option<DateTime<Utc>>) {
        let Some(receiver) = receiver else {
            return;
        };
        self.anchor.observe(receiver);
        for epoch in &mut self.epochs {
            if epoch.segment != capture.segment
                || epoch.receiver_time.is_some()
                || !epoch.date_attribution_allowed
            {
                continue;
            }
            let resolved = epoch
                .time_of_day
                .and_then(|time| self.anchor.resolve(time, self.config.date_anchor_tolerance));
            let compatible_host = epoch.host_time.is_none()
                || epoch.host_time.zip(capture.host).zip(resolved).is_some_and(
                    |((host, anchor_host), candidate)| {
                        (receiver.signed_duration_since(candidate)
                            - anchor_host.signed_duration_since(host))
                        .abs()
                        .to_std()
                        .is_ok_and(|difference| difference <= self.config.navigation_window)
                    },
                );
            if compatible_host
                && resolved.is_some_and(|receiver| {
                    self.receiver_clock
                        .permits_date_attribution(epoch.host_time, receiver)
                })
            {
                epoch.receiver_time = resolved;
                epoch.receiver_reliable = resolved.is_some();
            }
        }
    }

    fn insert_navigation(&mut self, input: NavigationInput, records: &mut Vec<NmeaRecord>) {
        let matching = self.epochs.iter().position(|epoch| {
            if epoch.segment != input.segment {
                return false;
            }
            if let Some((left, right)) = epoch.receiver_time.zip(input.fields.receiver_time) {
                left == right
            } else if epoch.receiver_time.is_some() || input.fields.receiver_time.is_some() {
                false
            } else if let Some((left, right)) = epoch.time_of_day.zip(input.fields.time_of_day) {
                left == right && epoch.host_time == input.host_time
            } else {
                epoch.time_of_day.is_none()
                    && input.fields.time_of_day.is_none()
                    && epoch.host_time.is_some()
                    && epoch.host_time == input.host_time
            }
        });
        if let Some(index) = matching {
            if self
                .epochs
                .get_mut(index)
                .is_some_and(|epoch| epoch.merge(input))
            {
                self.report(NmeaDiagnosticKind::ConflictingNavigation,Some(input.sentence),"Navigation observations contain conflicting state, position, or measurement evidence");
            }
        } else {
            let finalized = input.receiver_reliable
                && input
                    .fields
                    .receiver_time
                    .is_some_and(|receiver| self.finalized_receiver_epochs.contains(&receiver));
            let expired = input.receiver_reliable
                && self.watermarks.expired(
                    CaptureContext {
                        receiver_progress: input.fields.receiver_time,
                        segment: input.segment,
                        ..CaptureContext::default()
                    },
                    self.config.navigation_window,
                );
            if finalized || expired {
                self.report(
                    NmeaDiagnosticKind::LateObservation,
                    Some(input.sentence),
                    "Navigation epoch was finalized or is outside the buffering window",
                );
                return;
            }
            self.epochs.push(NavigationEpoch::new(input));
            if self.epochs.len() > self.config.max_pending_epochs.get() {
                self.report(
                    NmeaDiagnosticKind::BufferLimit,
                    Some(input.sentence),
                    "Navigation epoch capacity reached",
                );
                self.flush_epoch(0, records);
            }
        }
    }

    fn push_pending(&mut self, pending: PendingReport, records: &mut Vec<NmeaRecord>) {
        if self.pending.len() >= self.config.max_pending_reports.get() {
            self.report(
                NmeaDiagnosticKind::BufferLimit,
                Some(pending.sentence()),
                "Pending observation capacity reached",
            );
            let mut oldest = self.pending.remove(0);
            oldest.navigation_association_lost = true;
            self.emit_pending(oldest, records);
        }
        self.pending.push(pending);
    }

    fn insert_gsv(
        &mut self,
        talker: &str,
        data: GsvData,
        time: CaptureContext,
        records: &mut Vec<NmeaRecord>,
    ) {
        if data.number_of_sentences == 0
            || data.sentence_num == 0
            || data.sentence_num > data.number_of_sentences
            || usize::from(data.number_of_sentences) > self.config.max_gsv_fragments.get()
        {
            self.report(
                NmeaDiagnosticKind::MalformedSentence,
                Some(SentenceType::GSV),
                "GSV fragment numbers exceed the configured bounds",
            );
            return;
        }
        let key = GsvKey {
            talker: talker.to_owned(),
            constellation: GsvGroup::constellation(data.gnss_type),
            signal: data.signal_id,
        };
        let repeated_capture = self.completed_gsv.iter().any(|previous| {
            let original = previous.fragment_times.get(&data.sentence_num);
            let same_capture = time
                .host
                .zip(original.and_then(|context| context.host))
                .is_some_and(|(current, previous)| current == previous)
                || (time.host.is_none()
                    && original.is_some_and(|previous| {
                        previous.host.is_none()
                            && time.receiver_progress == previous.receiver_progress
                    }));
            previous.key == key
                && same_capture
                && previous.fragments.get(&data.sentence_num) == Some(&data)
        });
        if repeated_capture {
            if time.host.is_none() {
                self.report(NmeaDiagnosticKind::ConflictingGsvFragment,Some(SentenceType::GSV),"Repeated GSV fragment without a host timestamp has an uncertain update identity");
            }
            return;
        }
        let mut index = self.gsv_groups.iter().position(|group| group.key == key);
        let repeated_capture = index
            .and_then(|index| self.gsv_groups.get(index))
            .and_then(|group| group.fragment_times.get(&data.sentence_num))
            .is_some_and(|previous| time.host.is_some() && previous.host != time.host);

        if let Some(group_index) = index {
            let restart = self.gsv_groups.get(group_index).is_some_and(|group| {
                repeated_capture
                    || group.number_of_sentences != data.number_of_sentences
                    || group.sats_in_view != data.sats_in_view
                    || group
                        .fragments
                        .get(&data.sentence_num)
                        .is_some_and(|previous| previous != &data)
                    || Watermarks {
                        host: time.host,
                        receiver: time.receiver_progress,
                        receiver_segment: time.segment,
                    }
                    .expired(group.first, self.gsv_window())
            });
            if restart {
                let previous = self.gsv_groups.remove(group_index);
                self.report(NmeaDiagnosticKind::ConflictingGsvFragment,Some(SentenceType::GSV),"GSV group restarted after conflicting metadata, a repeated fragment, or expiration");
                self.push_gsv_group(previous, records);
                index = None;
            }
        }
        let index = if let Some(index) = index {
            index
        } else {
            if self.gsv_groups.len() >= self.config.max_gsv_groups.get() {
                self.report(
                    NmeaDiagnosticKind::BufferLimit,
                    Some(SentenceType::GSV),
                    "GSV group capacity reached",
                );
                let previous = self.gsv_groups.remove(0);
                self.push_gsv_group(previous, records);
            }
            self.gsv_groups.push(GsvGroup::new(key, time, &data));
            self.gsv_groups.len().saturating_sub(1)
        };
        if let Some(group) = self.gsv_groups.get_mut(index) {
            group.last = time;
            group.first.host = group.first.host.into_iter().chain(time.host).min();
            group.ambiguous_cycle |= repeated_capture;
            group
                .fragment_times
                .entry(data.sentence_num)
                .or_insert(time);
            group.fragments.entry(data.sentence_num).or_insert(data);
        }
        if self.gsv_groups.get(index).is_some_and(GsvGroup::complete) {
            let group = self.gsv_groups.remove(index);
            let schedule_time = CaptureContext {
                receiver_progress: None,
                ..group.first
            };
            if schedule_time.host.is_some() {
                self.schedules
                    .observe(SentenceType::GSV, schedule_time, &mut self.diagnostics);
            }
            self.push_gsv_group(group, records);
        }
    }

    fn push_gsv_group(&mut self, group: GsvGroup, records: &mut Vec<NmeaRecord>) {
        if self.completed_gsv.len() >= self.config.max_gsv_groups.get() {
            self.completed_gsv.remove(0);
        }
        self.completed_gsv.push(group.clone());
        for pending in
            group.into_pending_reports(self.config.gsv_cycle_policy, &mut self.diagnostics)
        {
            self.push_pending(pending, records);
        }
    }

    fn emit_pending(&mut self, pending: PendingReport, records: &mut Vec<NmeaRecord>) {
        let candidates = self.candidate_epochs(&pending);
        let associated = match candidates.as_slice() {
            [index] => Some(*index),
            _ => None,
        };
        let dop_conflict =
            if let Some(epoch) = associated.and_then(|index| self.epochs.get_mut(index)) {
                match &pending.kind {
                    PendingKind::Usage { data, .. } => epoch.merge_gsa(data),
                    PendingKind::Velocity(data) => {
                        epoch.merge_velocity(data);
                        false
                    }
                    PendingKind::Observation { .. } => false,
                }
            } else {
                false
            };
        if dop_conflict {
            self.report_at(
                NmeaDiagnosticKind::ConflictingNavigation,
                pending.time.line,
                Some(SentenceType::GSA),
                "Conflicting GSA dilution values are omitted from the navigation epoch",
            );
        }
        if associated.is_none()
            && pending.time.host.is_some()
            && matches!(&pending.kind, PendingKind::Usage { data, .. } if data.pdop.is_some() || data.hdop.is_some() || data.vdop.is_some())
        {
            self.report_at(
                NmeaDiagnosticKind::UnplacedObservation,
                pending.time.line,
                Some(SentenceType::GSA),
                "GSA dilution and dimension have no unambiguous navigation epoch",
            );
        }
        let time = if let Some(host) = pending.time.host {
            Some(NavFixTime::Host(host))
        } else {
            associated
                .and_then(|index| self.epochs.get(index))
                .and_then(|epoch| epoch.receiver_time)
                .map(NavFixTime::Receiver)
        };
        self.schedules.observe(
            pending.sentence(),
            CaptureContext {
                receiver_progress: time.and_then(NavFixTime::gps_time),
                ..pending.time
            },
            &mut self.diagnostics,
        );
        if matches!(pending.kind, PendingKind::Velocity(_)) {
            if associated.is_none() {
                self.report_at(
                    NmeaDiagnosticKind::UnplacedObservation,
                    pending.time.line,
                    Some(SentenceType::VTG),
                    "VTG has no unambiguous navigation epoch",
                );
            }
            return;
        }
        let Some(time) = time else {
            self.report_at(
                NmeaDiagnosticKind::UnplacedObservation,
                pending.time.line,
                Some(pending.sentence()),
                "Satellite observation has no host timestamp or unambiguous receiver epoch",
            );
            return;
        };
        if pending.time.host.is_none() {
            self.report_at(
                NmeaDiagnosticKind::InferredSatelliteTimestamp,
                pending.time.line,
                Some(pending.sentence()),
                "Receiver timestamp was inferred from an associated navigation epoch",
            );
        }
        if let Some(record) = pending.into_record(time) {
            records.push(record);
        }
    }

    fn expire(&mut self, records: &mut Vec<NmeaRecord>) {
        let mut index = 0;
        while index < self.gsv_groups.len() {
            let expired = self.gsv_groups.get(index).is_some_and(|group| {
                self.watermarks.expired(group.first, self.gsv_window())
                    || self.sequence.saturating_sub(group.first.sequence)
                        > u64::try_from(self.config.max_buffered_lines.get()).unwrap_or(u64::MAX)
            });
            if expired {
                let group = self.gsv_groups.remove(index);
                self.push_gsv_group(group, records);
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.pending.len() {
            let expired = self.pending.get(index).is_some_and(|pending| {
                self.watermarks
                    .expired(pending.time, self.pending_window(pending))
                    || self.sequence.saturating_sub(pending.time.sequence)
                        > u64::try_from(self.config.max_buffered_lines.get()).unwrap_or(u64::MAX)
            });
            if expired {
                let pending = self.pending.remove(index);
                self.emit_pending(pending, records);
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.epochs.len() {
            let expired = self.epochs.get(index).is_some_and(|epoch| {
                self.watermarks.expired(
                    CaptureContext {
                        host: epoch.host_time,
                        receiver_progress: epoch.receiver_time,
                        line: epoch.first_line,
                        sequence: epoch.first_sequence,
                        segment: epoch.segment,
                        receiver_ambiguous: false,
                        date_anchor_allowed: true,
                    },
                    self.config.navigation_window,
                ) || self.sequence.saturating_sub(epoch.first_sequence)
                    > u64::try_from(self.config.max_buffered_lines.get()).unwrap_or(u64::MAX)
            });
            if expired {
                self.flush_epoch(index, records);
            } else {
                index += 1;
            }
        }
    }

    fn flush_epoch(&mut self, index: usize, records: &mut Vec<NmeaRecord>) {
        let mut pending_index = 0;
        while pending_index < self.pending.len() {
            let flush = self
                .pending
                .get(pending_index)
                .is_some_and(|pending| self.candidate_epochs(pending).contains(&index));
            if flush {
                let pending = self.pending.remove(pending_index);
                self.emit_pending(pending, records);
            } else {
                pending_index += 1;
            }
        }
        if index >= self.epochs.len() {
            return;
        }
        for group in &mut self.gsv_groups {
            if group.first.host.is_none() {
                group.navigation_association_lost = true;
            }
        }
        let epoch = self.epochs.remove(index);
        let line = epoch.first_line;
        if let Some(receiver) = epoch.receiver_time {
            if self.finalized_receiver_epochs.len() >= self.config.max_buffered_lines.get() {
                self.finalized_receiver_epochs.pop_front();
            }
            self.finalized_receiver_epochs.push_back(receiver);
        }
        if let Some(fix) = epoch.into_fix() {
            records.push(NmeaRecord::Navigation(fix));
        } else {
            self.report_at(
                NmeaDiagnosticKind::UnplacedObservation,
                line,
                None,
                "Navigation observation has no absolute receiver or host timestamp",
            );
        }
    }

    fn count_records(&mut self, records: &[NmeaRecord]) {
        for record in records {
            let counter = match record {
                NmeaRecord::Navigation(_) => &mut self.statistics.navigation_fixes,
                NmeaRecord::SatelliteUsage(_) => &mut self.statistics.satellite_usage_reports,
                NmeaRecord::SatelliteObservation(_) => {
                    &mut self.statistics.satellite_observation_reports
                }
            };
            *counter = counter.saturating_add(1);
        }
    }

    fn report(
        &mut self,
        kind: NmeaDiagnosticKind,
        sentence: Option<SentenceType>,
        reason: impl Into<String>,
    ) {
        self.report_at(kind, self.statistics.lines, sentence, reason);
    }

    fn report_at(
        &mut self,
        kind: NmeaDiagnosticKind,
        line: u64,
        sentence: Option<SentenceType>,
        reason: impl Into<String>,
    ) {
        self.diagnostics.report(NmeaDiagnostic {
            kind,
            line,
            sentence,
            reason: reason.into(),
        });
    }

    fn gsv_window(&self) -> Duration {
        self.config.gsv_assembly_timeout
    }

    fn candidate_epochs(&self, pending: &PendingReport) -> Vec<usize> {
        if pending.navigation_association_lost {
            return Vec::new();
        }

        if let Some(host) = pending.time.host.filter(|_| {
            self.config.association_policy == NmeaAssociationPolicy::HostCaptureProximity
        }) {
            let mut candidates: Vec<_> = self
                .epochs
                .iter()
                .enumerate()
                .filter_map(|(index, epoch)| {
                    let distance = epoch
                        .host_time?
                        .signed_duration_since(host)
                        .abs()
                        .to_std()
                        .ok()?;
                    (distance <= self.config.association_tolerance).then_some((distance, index))
                })
                .collect();
            candidates.sort_unstable();
            let Some((minimum, _)) = candidates.first().copied() else {
                return Vec::new();
            };
            return candidates
                .into_iter()
                .filter(|(distance, _)| *distance == minimum)
                .map(|(_, index)| index)
                .collect();
        }
        if self.receiver_clock.unresolved()
            || self.config.association_policy != NmeaAssociationPolicy::ReceiverEpochBracketing
        {
            return Vec::new();
        }
        let bracketed: Vec<_> = self
            .epochs
            .iter()
            .enumerate()
            .filter(|(_, epoch)| {
                epoch.bracket_first_sequence <= pending.time.sequence
                    && epoch.bracket_last_sequence >= pending.last_sequence
                    && epoch.receiver_time.is_some()
                    && epoch.receiver_reliable
                    && epoch.segment == pending.time.segment
                    && epoch.last_sequence.saturating_sub(epoch.first_sequence)
                        <= u64::try_from(self.config.max_association_lines.get())
                            .unwrap_or(u64::MAX)
            })
            .map(|(index, _)| index)
            .collect();
        if !bracketed.is_empty() {
            let mut candidates = bracketed.clone();
            for bracket in bracketed {
                let Some(bracket) = self.epochs.get(bracket) else {
                    continue;
                };
                for (index, other) in self.epochs.iter().enumerate() {
                    if other.receiver_time.is_some()
                        && other.first_sequence <= bracket.last_sequence
                        && other.last_sequence >= bracket.first_sequence
                    {
                        candidates.push(index);
                    }
                }
            }
            candidates.sort_unstable();
            candidates.dedup();
            return candidates;
        }
        Vec::new()
    }

    fn pending_window(&self, pending: &PendingReport) -> Duration {
        self.config
            .periods
            .iter()
            .find(|entry| entry.sentence == pending.sentence())
            .map_or(self.config.satellite_window, |entry| {
                self.config
                    .satellite_window
                    .min(entry.period.saturating_mul(2))
            })
    }
}
