use std::collections::BTreeMap;

use nmea::SentenceType;

/// Recoverable input failures and conservative attribution decisions.
#[derive(Clone, Copy, Debug, strum::EnumIter, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum NmeaDiagnosticKind {
    /// Numbered GSV fragments have no established common receiver capture.
    AmbiguousGsvCycle,
    /// A configured temporal, schedule or multipart capacity was reached.
    BufferLimit,
    /// Fragments or repeated identities conflict within a GSV update.
    ConflictingGsvFragment,
    /// Navigation or associated dilution measurements conflict within an epoch.
    ConflictingNavigation,
    /// A GSV group was finalized with missing fragments.
    IncompleteGsv,
    /// A satellite receiver timestamp was inferred from an unambiguous navigation association.
    InferredSatelliteTimestamp,
    /// A host timestamp is missing, malformed or outside UTC.
    InvalidHostTimestamp,
    /// A buffered input line is not UTF-8.
    InvalidText,
    /// An update is older than the finalized navigation or schedule window.
    LateObservation,
    /// An input line exceeds the configured byte limit.
    LineTooLong,
    /// A supported sentence has invalid syntax, encoding, checksum or fragment metadata.
    MalformedSentence,
    /// One or more expected updates are absent after the allowed lateness window.
    MissingUpdate,
    /// A signal-specific GSV sample is represented without signal identity.
    SignalDimensionOmitted,
    /// Receiver clock progression changed or an isolated timestamp was quarantined.
    TimestampDiscontinuity,
    /// An observation or its dilution values have no reliable absolute timeline association.
    UnplacedObservation,
    /// The input cannot establish a representable satellite identity or constellation scope.
    UnresolvedSatelliteIdentity,
    /// An unrecognized or unsupported sentence was ignored.
    UnsupportedSentence,
    /// The parser does not support this sentence variation.
    UnsupportedVariation,
}

/// One bounded diagnostic example with its original input location.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NmeaDiagnostic {
    /// Diagnostic category used for aggregation.
    pub kind: NmeaDiagnosticKind,
    /// One-based input line number, including malformed and unsupported lines.
    pub line: u64,
    /// Recognized sentence type, when available.
    pub sentence: Option<SentenceType>,
    /// Parser or attribution reason, truncated to 256 characters.
    pub reason: String,
}

/// Saturating count and inclusive source line range for one diagnostic kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NmeaDiagnosticCount {
    /// Number of affected occurrences or missing updates, saturated at [`u64::MAX`].
    pub count: u64,
    /// Minimum affected input line number.
    pub first_line: u64,
    /// Maximum affected input line number.
    pub last_line: u64,
}

/// Aggregated diagnostic counts and a bounded list of examples.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NmeaDiagnostics {
    counts: BTreeMap<NmeaDiagnosticKind, NmeaDiagnosticCount>,
    samples: Vec<NmeaDiagnostic>,
    pub(super) sample_limit: usize,
}

impl NmeaDiagnostics {
    pub(super) fn with_sample_limit(sample_limit: usize) -> Self {
        Self {
            sample_limit,
            ..Self::default()
        }
    }

    pub(super) fn report(&mut self, diagnostic: NmeaDiagnostic) {
        self.report_repeated(diagnostic, 1);
    }

    pub(super) fn report_repeated(&mut self, mut diagnostic: NmeaDiagnostic, count: u64) {
        let entry = self
            .counts
            .entry(diagnostic.kind)
            .or_insert(NmeaDiagnosticCount {
                count: 0,
                first_line: diagnostic.line,
                last_line: diagnostic.line,
            });
        entry.count = entry.count.saturating_add(count);
        entry.first_line = entry.first_line.min(diagnostic.line);
        entry.last_line = entry.last_line.max(diagnostic.line);
        if self.samples.len() < self.sample_limit {
            diagnostic.reason = diagnostic.reason.chars().take(MAX_REASON_CHARS).collect();
            self.samples.push(diagnostic);
        }
    }

    /// Return counts and source line ranges ordered by diagnostic kind.
    pub fn counts(&self) -> &BTreeMap<NmeaDiagnosticKind, NmeaDiagnosticCount> {
        &self.counts
    }

    /// Return at most the configured number of examples, in diagnostic emission order.
    pub fn samples(&self) -> &[NmeaDiagnostic] {
        &self.samples
    }

    /// Return the saturating occurrence count for a diagnostic kind.
    pub fn count(&self, kind: NmeaDiagnosticKind) -> u64 {
        self.counts.get(&kind).map_or(0, |entry| entry.count)
    }
}

const MAX_REASON_CHARS: usize = 256;
