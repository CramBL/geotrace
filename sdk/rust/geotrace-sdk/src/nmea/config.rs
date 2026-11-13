use std::fmt::Write as _;
use std::num::NonZeroUsize;
use std::time::Duration;

use chrono::format::{Fixed, Item, StrftimeItems};
use chrono::{DateTime, NaiveDateTime, Utc};
use nmea::SentenceType;
use thiserror::Error;

use super::NmeaIngestor;

/// Host clock source selected before ingestion.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum HostTimestampMode {
    /// Require [`NmeaIngestor::push_line_at`] for nonempty input.
    CallerProvided,
    /// Accept receiver timestamps and optional direct host timestamps.
    #[default]
    None,
    /// Parse a UTC timestamp prefix with this format.
    PrefixUtc(String),
}

/// Evidence required to associate observations without their own receiver timestamp.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum NmeaAssociationPolicy {
    /// The caller guarantees capture delays preserve the uniquely nearest navigation epoch.
    /// Association uses the configured host tolerance.
    HostCaptureProximity,
    /// Preserve independent host reports without assigning them to navigation epochs.
    #[default]
    IndependentReports,
    /// The caller guarantees every navigation bracket contains only that epoch's observations.
    /// Delayed navigation duplicates and complements must not extend a bracket across updates.
    ReceiverEpochBracketing,
}

/// Evidence available to distinguish multipart GSV receiver updates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum NmeaGsvCyclePolicy {
    /// Preserve individual fragments with partial coverage.
    #[default]
    IndependentFragments,
    /// The caller guarantees one shared, unique host timestamp for every GSV update cycle.
    UniqueHostTimestamp,
}

/// Expected interval between whole updates of one supported sentence type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SentencePeriod {
    /// The supported sentence whose complete updates follow this period.
    pub sentence: SentenceType,
    /// Expected elapsed time between updates.
    pub period: Duration,
}

/// Configure clock parsing, expected periods and bounded temporal aggregation.
#[derive(Clone, Debug)]
pub struct NmeaIngestorBuilder {
    association_policy: NmeaAssociationPolicy,
    gsv_cycle_policy: NmeaGsvCyclePolicy,
    host_timestamp: HostTimestampMode,
    periods: Vec<SentencePeriod>,
    navigation_window: Option<Duration>,
    satellite_window: Option<Duration>,
    gsv_assembly_timeout: Duration,
    association_tolerance: Option<Duration>,
    date_anchor_tolerance: Option<Duration>,
    max_pending_epochs: usize,
    max_pending_reports: usize,
    max_gsv_groups: usize,
    max_gsv_fragments: usize,
    max_buffered_lines: usize,
    max_association_lines: usize,
    max_line_bytes: usize,
    diagnostic_samples: usize,
}

impl NmeaIngestorBuilder {
    fn bounded_duration(
        duration: Duration,
        field: &'static str,
    ) -> Result<Duration, NmeaConfigError> {
        if duration.is_zero() || duration >= MAX_BUFFER_DURATION {
            return Err(NmeaConfigError::InvalidDuration { field });
        }
        Ok(duration)
    }

    fn nonzero(value: usize, field: &'static str) -> Result<NonZeroUsize, NmeaConfigError> {
        NonZeroUsize::new(value).ok_or(NmeaConfigError::ZeroLimit { field })
    }

    /// Select evidence for observations without timestamps.
    /// Independent reports are the default.
    pub fn association_policy(mut self, policy: NmeaAssociationPolicy) -> Self {
        self.association_policy = policy;
        self
    }

    /// Select multipart cycle evidence. Individual partial fragments are the default.
    pub fn gsv_cycle_policy(mut self, policy: NmeaGsvCyclePolicy) -> Self {
        self.gsv_cycle_policy = policy;
        self
    }

    /// Select host clock extraction. The default accepts optional caller timestamps.
    pub fn host_timestamp_mode(mut self, mode: HostTimestampMode) -> Self {
        self.host_timestamp = mode;
        self
    }

    /// Parse a complete UTC timestamp before the first `$` using a `chrono` format.
    /// Named timezone directives are rejected. Numeric offsets must be zero.
    pub fn timestamp_prefix_utc(self, format: impl Into<String>) -> Self {
        self.host_timestamp_mode(HostTimestampMode::PrefixUtc(format.into()))
    }

    /// Require an explicit host timestamp on every nonempty input line.
    pub fn caller_timestamps(self) -> Self {
        self.host_timestamp_mode(HostTimestampMode::CallerProvided)
    }

    /// Set the period between whole updates, including every fragment of a GSV group.
    pub fn sentence_period(mut self, sentence: SentenceType, period: Duration) -> Self {
        if let Some(configured) = self
            .periods
            .iter_mut()
            .find(|entry| entry.sentence == sentence)
        {
            configured.period = period;
        } else {
            self.periods.push(SentencePeriod { sentence, period });
        }
        self
    }

    /// Set navigation buffering and update lateness.
    /// The default is twice the longest navigation period.
    pub fn navigation_window(mut self, duration: Duration) -> Self {
        self.navigation_window = Some(duration);
        self
    }

    /// Set satellite buffering and update lateness.
    /// The default is twice the longest satellite period.
    pub fn satellite_window(mut self, duration: Duration) -> Self {
        self.satellite_window = Some(duration);
        self
    }

    /// Bound the transmission duration of one multipart GSV report. The default is 2s.
    /// This timeout is independent of expected whole-update periods.
    pub fn gsv_assembly_timeout(mut self, duration: Duration) -> Self {
        self.gsv_assembly_timeout = duration;
        self
    }

    /// Set the maximum host clock distance for [`NmeaAssociationPolicy::HostCaptureProximity`].
    /// The default is half the longest navigation period.
    pub fn association_tolerance(mut self, duration: Duration) -> Self {
        self.association_tolerance = Some(duration);
        self
    }

    /// Set the maximum receiver time distance from an RMC or ZDA date anchor.
    /// The default is the navigation window, bounded to 60s through 6h.
    pub fn date_anchor_tolerance(mut self, duration: Duration) -> Self {
        self.date_anchor_tolerance = Some(duration);
        self
    }

    /// Bound simultaneous incomplete navigation epochs. The default is 64.
    pub fn max_pending_epochs(mut self, count: usize) -> Self {
        self.max_pending_epochs = count;
        self
    }

    /// Bound satellite and VTG observations pending association. The default is 128.
    pub fn max_pending_reports(mut self, count: usize) -> Self {
        self.max_pending_reports = count;
        self
    }

    /// Bound incomplete GSV groups and the completed-capture cache. The default is 32.
    pub fn max_gsv_groups(mut self, count: usize) -> Self {
        self.max_gsv_groups = count;
        self
    }

    /// Bound fragment count within one GSV group. The default is 32.
    pub fn max_gsv_fragments(mut self, count: usize) -> Self {
        self.max_gsv_fragments = count;
        self
    }

    /// Bound aggregation age, schedule capacity and recent capture history in supported sentences.
    /// Captures in an expired history interval produce
    /// [`crate::NmeaDiagnosticKind::UnplacedObservation`].
    /// The default is 4096. Unsupported and malformed lines do not advance this age.
    pub fn max_buffered_lines(mut self, count: usize) -> Self {
        self.max_buffered_lines = count;
        self
    }

    /// Set the maximum supported-sentence distance for receiver epoch association.
    /// The default is 64.
    pub fn max_association_lines(mut self, count: usize) -> Self {
        self.max_association_lines = count;
        self
    }

    /// Bound input line size in bytes, including a prefix and line terminator. The default is 4096.
    pub fn max_line_bytes(mut self, count: usize) -> Self {
        self.max_line_bytes = count;
        self
    }

    /// Bound retained diagnostic examples. The default is 32. Zero retains only counts.
    pub fn diagnostic_samples(mut self, count: usize) -> Self {
        self.diagnostic_samples = count;
        self
    }

    /// Construct validated ingestion state.
    /// Reject zero capacities, invalid formats and durations outside (0, 12h).
    pub fn build(self) -> Result<NmeaIngestor, NmeaConfigError> {
        self.into_config().map(NmeaIngestor::from_config)
    }

    fn into_config(self) -> Result<NmeaConfig, NmeaConfigError> {
        for entry in &self.periods {
            if !matches!(
                entry.sentence,
                SentenceType::GGA
                    | SentenceType::RMC
                    | SentenceType::GNS
                    | SentenceType::GLL
                    | SentenceType::VTG
                    | SentenceType::GSA
                    | SentenceType::GSV
                    | SentenceType::ZDA
            ) {
                return Err(NmeaConfigError::UnsupportedSentencePeriod {
                    sentence: entry.sentence,
                });
            }
            Self::bounded_duration(entry.period, "Sentence period")?;
        }
        let host_timestamp = match self.host_timestamp {
            HostTimestampMode::CallerProvided => ConfiguredHostTimestamp::CallerProvided,
            HostTimestampMode::None => ConfiguredHostTimestamp::None,
            HostTimestampMode::PrefixUtc(format) => {
                ConfiguredHostTimestamp::PrefixUtc(UtcTimestampFormat::new(format)?)
            }
        };
        let navigation_period = self
            .periods
            .iter()
            .filter(|entry| !matches!(entry.sentence, SentenceType::GSA | SentenceType::GSV))
            .map(|entry| entry.period)
            .max()
            .unwrap_or(DEFAULT_PERIOD);
        let satellite_period = self
            .periods
            .iter()
            .filter(|entry| matches!(entry.sentence, SentenceType::GSA | SentenceType::GSV))
            .map(|entry| entry.period)
            .max()
            .unwrap_or(DEFAULT_PERIOD);
        let navigation_window = Self::bounded_duration(
            self.navigation_window
                .unwrap_or(navigation_period.saturating_mul(2)),
            "Navigation window",
        )?;
        let satellite_window = Self::bounded_duration(
            self.satellite_window
                .unwrap_or(satellite_period.saturating_mul(2)),
            "Satellite window",
        )?;
        let association_tolerance = Self::bounded_duration(
            self.association_tolerance.unwrap_or(navigation_period / 2),
            "Association tolerance",
        )?;
        let date_anchor_tolerance = Self::bounded_duration(
            self.date_anchor_tolerance.unwrap_or(
                navigation_window
                    .max(MIN_DATE_DISTANCE)
                    .min(MAX_DATE_DISTANCE),
            ),
            "Date anchor tolerance",
        )?;
        Ok(NmeaConfig {
            association_policy: self.association_policy,
            gsv_cycle_policy: self.gsv_cycle_policy,
            host_timestamp,
            periods: self.periods,
            navigation_window,
            satellite_window,
            gsv_assembly_timeout: Self::bounded_duration(
                self.gsv_assembly_timeout,
                "GSV assembly timeout",
            )?,
            association_tolerance,
            date_anchor_tolerance,
            max_pending_epochs: Self::nonzero(self.max_pending_epochs, "Pending epochs")?,
            max_pending_reports: Self::nonzero(self.max_pending_reports, "Pending reports")?,
            max_gsv_groups: Self::nonzero(self.max_gsv_groups, "GSV groups")?,
            max_gsv_fragments: Self::nonzero(self.max_gsv_fragments, "GSV fragments")?,
            max_buffered_lines: Self::nonzero(self.max_buffered_lines, "Buffered lines")?,
            max_association_lines: Self::nonzero(self.max_association_lines, "Association lines")?,
            max_line_bytes: Self::nonzero(self.max_line_bytes, "Line bytes")?,
            diagnostic_samples: self.diagnostic_samples,
        })
    }
}

impl Default for NmeaIngestorBuilder {
    fn default() -> Self {
        Self {
            association_policy: NmeaAssociationPolicy::IndependentReports,
            gsv_cycle_policy: NmeaGsvCyclePolicy::IndependentFragments,
            host_timestamp: HostTimestampMode::None,
            periods: Vec::new(),
            navigation_window: None,
            satellite_window: None,
            gsv_assembly_timeout: DEFAULT_GSV_ASSEMBLY_TIMEOUT,
            association_tolerance: None,
            date_anchor_tolerance: None,
            max_pending_epochs: DEFAULT_PENDING_EPOCHS,
            max_pending_reports: DEFAULT_PENDING_REPORTS,
            max_gsv_groups: DEFAULT_GSV_GROUPS,
            max_gsv_fragments: DEFAULT_GSV_FRAGMENTS,
            max_buffered_lines: DEFAULT_BUFFERED_LINES,
            max_association_lines: DEFAULT_ASSOCIATION_LINES,
            max_line_bytes: DEFAULT_LINE_BYTES,
            diagnostic_samples: DEFAULT_DIAGNOSTIC_SAMPLES,
        }
    }
}

/// Validated ingestion settings returned by [`NmeaIngestor::config`].
#[derive(Clone, Debug)]
pub struct NmeaConfig {
    pub(super) association_policy: NmeaAssociationPolicy,
    pub(super) gsv_cycle_policy: NmeaGsvCyclePolicy,
    pub(super) host_timestamp: ConfiguredHostTimestamp,
    pub(super) periods: Vec<SentencePeriod>,
    pub(super) navigation_window: Duration,
    pub(super) satellite_window: Duration,
    pub(super) gsv_assembly_timeout: Duration,
    pub(super) association_tolerance: Duration,
    pub(super) date_anchor_tolerance: Duration,
    pub(super) max_pending_epochs: NonZeroUsize,
    pub(super) max_pending_reports: NonZeroUsize,
    pub(super) max_gsv_groups: NonZeroUsize,
    pub(super) max_gsv_fragments: NonZeroUsize,
    pub(super) max_buffered_lines: NonZeroUsize,
    pub(super) max_association_lines: NonZeroUsize,
    pub(super) max_line_bytes: NonZeroUsize,
    pub(super) diagnostic_samples: usize,
}

impl NmeaConfig {
    /// Return the evidence required for observations without timestamps.
    pub fn association_policy(&self) -> NmeaAssociationPolicy {
        self.association_policy
    }

    /// Return the evidence available for multipart cycle identification.
    pub fn gsv_cycle_policy(&self) -> NmeaGsvCyclePolicy {
        self.gsv_cycle_policy
    }
    /// Return the validated clock mode and its original format, if configured.
    pub fn host_timestamp_mode(&self) -> HostTimestampMode {
        match &self.host_timestamp {
            ConfiguredHostTimestamp::CallerProvided => HostTimestampMode::CallerProvided,
            ConfiguredHostTimestamp::None => HostTimestampMode::None,
            ConfiguredHostTimestamp::PrefixUtc(format) => {
                HostTimestampMode::PrefixUtc(format.format.clone())
            }
        }
    }

    /// Return configured whole-update periods in declaration order.
    pub fn sentence_periods(&self) -> &[SentencePeriod] {
        &self.periods
    }

    /// Return the navigation buffer and update lateness duration.
    pub fn navigation_window(&self) -> Duration {
        self.navigation_window
    }

    /// Return the satellite buffer and update lateness duration.
    pub fn satellite_window(&self) -> Duration {
        self.satellite_window
    }

    /// Return the maximum transmission duration of one multipart GSV report.
    pub fn gsv_assembly_timeout(&self) -> Duration {
        self.gsv_assembly_timeout
    }

    /// Return the maximum host clock distance for navigation association.
    pub fn association_tolerance(&self) -> Duration {
        self.association_tolerance
    }

    /// Return the maximum receiver clock distance for date attribution.
    pub fn date_anchor_tolerance(&self) -> Duration {
        self.date_anchor_tolerance
    }
}

/// An invalid duration, buffer capacity, sentence period or timestamp format.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum NmeaConfigError {
    /// A period or tolerance is zero or at least 12h.
    #[error("{field} must be greater than zero and shorter than 12h")]
    InvalidDuration {
        /// Configuration duration outside its supported range.
        field: &'static str,
    },
    /// The format cannot parse a complete UTC date and time with a supported zone directive.
    #[error("Timestamp format must parse a complete UTC date and time")]
    InvalidTimestampFormat,
    /// The sentence type has no supported ingestion update.
    #[error("{sentence} does not produce an ingested update")]
    UnsupportedSentencePeriod {
        /// Sentence type without a supported update.
        sentence: SentenceType,
    },
    /// A buffer capacity is zero.
    #[error("{field} must be greater than zero")]
    ZeroLimit {
        /// Configuration field with a zero capacity.
        field: &'static str,
    },
}

#[derive(Clone, Debug)]
pub(super) enum ConfiguredHostTimestamp {
    CallerProvided,
    None,
    PrefixUtc(UtcTimestampFormat),
}

#[derive(Clone, Debug)]
pub(super) struct UtcTimestampFormat {
    format: String,
    zone: PrefixZone,
}

impl UtcTimestampFormat {
    fn new(format: String) -> Result<Self, NmeaConfigError> {
        let mut zone = PrefixZone::NaiveUtc;
        for item in StrftimeItems::new(&format) {
            match item {
                Item::Error | Item::Fixed(Fixed::TimezoneName) => {
                    return Err(NmeaConfigError::InvalidTimestampFormat);
                }
                Item::Fixed(
                    Fixed::TimezoneOffset
                    | Fixed::TimezoneOffsetColon
                    | Fixed::TimezoneOffsetDoubleColon
                    | Fixed::TimezoneOffsetTripleColon
                    | Fixed::TimezoneOffsetColonZ
                    | Fixed::TimezoneOffsetZ
                    | Fixed::RFC2822
                    | Fixed::RFC3339,
                ) => zone = PrefixZone::ExplicitUtcOffset,
                _ => {}
            }
        }
        let parsed = Self { format, zone };
        let sample = "2026-10-23T12:34:56.123456789Z"
            .parse::<DateTime<Utc>>()
            .map_err(|_error| NmeaConfigError::InvalidTimestampFormat)?;
        let mut rendered = String::new();
        write!(&mut rendered, "{}", sample.format(&parsed.format))
            .map_err(|_error| NmeaConfigError::InvalidTimestampFormat)?;
        parsed
            .parse(&rendered)
            .map_err(|_error| NmeaConfigError::InvalidTimestampFormat)?;
        Ok(parsed)
    }

    pub(super) fn parse(&self, prefix: &str) -> Result<DateTime<Utc>, String> {
        match self.zone {
            PrefixZone::ExplicitUtcOffset => {
                let time = DateTime::parse_from_str(prefix, &self.format)
                    .map_err(|error| format!("Host timestamp: {error}"))?;
                if time.offset().local_minus_utc() != 0 {
                    return Err("Host timestamp must use UTC".to_owned());
                }
                Ok(time.with_timezone(&Utc))
            }
            PrefixZone::NaiveUtc => NaiveDateTime::parse_from_str(prefix, &self.format)
                .map(|time| time.and_utc())
                .map_err(|error| format!("Host timestamp: {error}")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum PrefixZone {
    ExplicitUtcOffset,
    NaiveUtc,
}

const DEFAULT_PERIOD: Duration = Duration::from_secs(1);
const DEFAULT_GSV_ASSEMBLY_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_PENDING_EPOCHS: usize = 64;
const DEFAULT_PENDING_REPORTS: usize = 128;
const DEFAULT_GSV_GROUPS: usize = 32;
const DEFAULT_GSV_FRAGMENTS: usize = 32;
const DEFAULT_BUFFERED_LINES: usize = 4_096;
const DEFAULT_ASSOCIATION_LINES: usize = 64;
const DEFAULT_LINE_BYTES: usize = 4_096;
const DEFAULT_DIAGNOSTIC_SAMPLES: usize = 32;
const MAX_DATE_DISTANCE: Duration = Duration::from_secs(6 * 60 * 60);
const MIN_DATE_DISTANCE: Duration = Duration::from_secs(60);
const MAX_BUFFER_DURATION: Duration = Duration::from_secs(12 * 60 * 60);
