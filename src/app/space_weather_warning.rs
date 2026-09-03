//! Whether the archived environment values could have disturbed a loaded
//! recording, and the warning the load toast and the map indicator show.
//!
//! Every value is read from the archives, so a recording loaded before its
//! days were downloaded is assessed again as each day is stored, and a
//! recording no archived day overlaps warns about nothing.
//!
//! The evidence is kept per track: the indicator names each affected track and
//! the value every metric reached over it, and the load toast states the peaks
//! over the recording the track belongs to.

use std::fmt;
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use gt_flare::{FlareClassification, MarkedFlare, RadioBlackoutClass};
use gt_fmt::UTC_MINUTE_FORMAT;
use gt_ionex::quiet_time::{self, QuietTimeDeviation, QuietTimeDeviationPeak};
use gt_loaded_files::LoadedFileId;
use gt_solar::GeomagneticIndex;
use gt_solar::activity::{GeomagneticActivity, GeomagneticStormClass};
use gt_types::{SunlitSide, TimeRange, TrackRef};
use gt_ui_types::{
    ArcIdentity, GeomagneticPoint, GeomagneticSeries, JammingPoint, JammingSeries, TecPoint,
    TecSeries, TrackSpaceWeatherWarning, WarningLevelExplanation,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// Leads the toast raised the first time a loaded recording is found to
/// overlap an archived value that can disturb reception. The recording's name
/// follows on the same line, and each metric's finding on a line below it.
const SPACE_WEATHER_DURING: &str = "Space weather during";

/// Share of aircraft, in percent, at or above which a cell-day a track
/// crossed counts. It is the share gpsjam starts colouring its cells at.
const INTERFERENCE_TRIGGER_PERCENT: f32 = gt_ui_theme::INTERFERENCE_LOW_BREAKPOINT * 100.0;

/// Leads the geomagnetic line. The metric is named "geomagnetic activity"
/// where any value is shown. Only storm-level values reach a line here.
const GEOMAGNETIC_STORM: &str = "Geomagnetic storm";

/// Leads the solar flare line, which states one flare.
const SOLAR_FLARE: &str = "Solar flare";

/// Leads the TEC line, which states a range of values.
const TEC_OVER_TRACK: &str = "TEC over track";

/// Leads the TEC deviation line, which states one share of the quiet median,
/// the storm grade it reaches, how long that grade held, and the geomagnetic
/// activity before it.
const TEC_DEVIATION: &str = "ΔTEC";

/// Hours before a TEC deviation's peak epoch the archived geomagnetic indices
/// are read over, looking for a storm that would account for the deviation.
///
/// A day-long window would miss the storm behind a late depletion: the
/// negative phase of an ionospheric storm follows the geomagnetic storm
/// driving it by up to about a day. Twice that lag is GeoTrace's own choice,
/// as the reference material states no figure to take.
const GEOMAGNETIC_LOOKBACK_HOURS: i64 = 48;

/// Closes the solar flare line's limit. Only a flare that peaked while the
/// receiver was in daylight counts.
const SUNLIT_RECEIVER: &str = "sunlit";

/// The weakest flare classification the NOAA radio blackout scale covers, as
/// the catalog writes it. The scale's own floor is a peak flux, so the test
/// `the_stated_flare_class_is_the_first_blackout_level` pins this string
/// against the scale.
const FLARE_TRIGGER_CLASSIFICATION: &str = "M1";

/// One row per environment metric, stating the level at which it raises a
/// warning, listed by the popup behind the map's warning icon.
pub static WARNING_LEVELS: LazyLock<Vec<WarningLevelExplanation>> = LazyLock::new(|| {
    vec![
        WarningLevelExplanation {
            trigger: format!(
                "{}: ≥{INTERFERENCE_TRIGGER_PERCENT:.0}% of aircraft in a crossed cell reported \
                 low navigation accuracy (gpsjam.org's yellow level).",
                gt_jam::text::LAYER_LABEL
            ),
            reference: gt_jam::reference::AIRCRAFT_INTERFERENCE,
        },
        WarningLevelExplanation {
            trigger: format!(
                "Geomagnetic activity: {} or {} ≥{} (NOAA {}).",
                GeomagneticIndex::Kp,
                GeomagneticIndex::Hp30,
                GeomagneticStormClass::Minor.lowest_value(),
                GeomagneticStormClass::Minor.scale_name()
            ),
            reference: gt_solar::reference::GEOMAGNETIC_ACTIVITY,
        },
        WarningLevelExplanation {
            trigger: format!(
                "Solar flares: class {FLARE_TRIGGER_CLASSIFICATION} or stronger (NOAA {}), \
                 receiver is on the sunlit side.",
                RadioBlackoutClass::Minor.scale_name()
            ),
            reference: gt_flare::reference::SOLAR_FLARES,
        },
        WarningLevelExplanation {
            trigger: gt_ionex::text::DEVIATION_WARNING_TRIGGER.clone(),
            reference: gt_ionex::reference::IONOSPHERIC_TEC,
        },
    ]
});

/// One metric's finding: the metric that reached its disturbance level, the
/// level it warns from, and the value it reached there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WarningLine {
    metric: &'static str,
    limit: String,
    finding: String,
}

impl fmt::Display for WarningLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.metric, self.limit, self.finding)
    }
}

/// The peak archived value of each environment metric over one track, or over
/// the tracks of one recording, kept only where it reaches the level that can
/// disturb reception.
///
/// Aircraft interference, geomagnetic activity, solar flares and the TEC
/// deviation each have a published level above which reception is known to
/// suffer, and those four levels are what raises the warning. The absolute
/// TEC range has no such level, so it is kept as context beside a warning
/// another metric raised and never raises one.
#[derive(Debug, Default, Clone, PartialEq)]
struct DisturbanceEvidence {
    aircraft_interference: Option<f64>,
    geomagnetic: Option<GeomagneticStormPeak>,
    solar_flare: Option<SunlitFlarePeak>,
    tec_deviation: Option<TecDeviationEvidence>,
    total_electron_content: Option<TecSpan>,
}

impl DisturbanceEvidence {
    /// The evidence over one track: its fixes' own values, and the flares of
    /// `flares` that peaked while it was recording.
    fn of(series: &TrackSeries<'_>, flares: &[MarkedFlare], recorded: TimeRange) -> Self {
        let mut evidence = Self::default();
        if let Some(points) = series.interference {
            evidence.collect_interference_from(points);
        }
        if let Some(points) = series.geomagnetic {
            evidence.collect_geomagnetic_from(points);
        }
        if let Some(points) = series.tec {
            evidence.collect_tec_from(points);
        }
        evidence.keep_stronger_tec_deviation(series.tec_deviation);
        evidence.collect_flares_peaking_in(flares, recorded);
        evidence
    }

    /// Whether a metric reached its disturbance level. The TEC range alone
    /// never makes this true.
    fn warns(&self) -> bool {
        let Self {
            aircraft_interference,
            geomagnetic,
            solar_flare,
            tec_deviation,
            total_electron_content: _,
        } = self;
        aircraft_interference.is_some()
            || geomagnetic.is_some()
            || solar_flare.is_some()
            || tec_deviation.is_some()
    }

    /// One line per metric that reached its disturbance level, closed by the
    /// TEC range where the archive holds one.
    ///
    /// Empty while no metric reached its level, which is what keeps the track
    /// out of the indicator's list.
    fn warning_lines(&self) -> Vec<String> {
        if !self.warns() {
            return Vec::new();
        }
        let mut lines: Vec<String> = self.warnings().iter().map(WarningLine::to_string).collect();
        lines.extend(self.total_electron_content.map(TecSpan::context_line));
        lines
    }

    /// The toast raised for a recording the archives place a disturbance in:
    /// the recording, then the peak each of its metrics reached over it,
    /// closed by what a stated TEC grade is measured against.
    fn load_toast_text(&self, recording: &str) -> String {
        let mut lines = vec![format!("{SPACE_WEATHER_DURING} {recording}")];
        lines.extend(self.warnings().iter().map(WarningLine::to_string));
        if self.tec_deviation.is_some() {
            lines.push(gt_ionex::text::DEVIATION_REFERENCE_CAVEAT.clone());
        }
        lines.join("\n")
    }

    /// One entry per metric that reached its disturbance level, in the order
    /// every surface lists them.
    fn warnings(&self) -> Vec<WarningLine> {
        let Self {
            aircraft_interference,
            geomagnetic,
            solar_flare,
            tec_deviation,
            total_electron_content: _,
        } = self;
        let mut lines = Vec::new();
        if let Some(peak) = *geomagnetic {
            lines.push(peak.warning_line());
        }
        if let Some(percent) = *aircraft_interference {
            lines.push(WarningLine {
                metric: gt_jam::text::LAYER_LABEL,
                limit: format!("≥{INTERFERENCE_TRIGGER_PERCENT:.0}%"),
                finding: format!("up to {percent:.1}% of aircraft in a crossed cell"),
            });
        }
        if let Some(peak) = *solar_flare {
            lines.push(peak.warning_line());
        }
        if let Some(evidence) = *tec_deviation {
            lines.push(evidence.warning_line());
        }
        lines
    }

    /// Fold `other`'s peaks into these, so one set of lines states the
    /// strongest evidence over every track of a recording.
    fn merge(&mut self, other: &Self) {
        let Self {
            aircraft_interference,
            geomagnetic,
            solar_flare,
            tec_deviation,
            total_electron_content,
        } = *other;
        self.keep_stronger_interference(aircraft_interference);
        self.keep_stronger_storm(geomagnetic);
        self.keep_stronger_flare(solar_flare);
        self.keep_stronger_tec_deviation(tec_deviation);
        self.widen_tec_to(total_electron_content);
    }

    /// Keep the highest share these fixes crossed, where it reaches the
    /// trigger.
    fn collect_interference_from(&mut self, points: &[JammingPoint]) {
        for percent in points.iter().filter_map(|point| point.percent) {
            if percent >= f64::from(INTERFERENCE_TRIGGER_PERCENT) {
                self.keep_stronger_interference(Some(percent));
            }
        }
    }

    /// Keep the highest storm-level value the periods these fixes fall in
    /// carry, from either index.
    fn collect_geomagnetic_from(&mut self, points: &[GeomagneticPoint]) {
        for point in points {
            for (index, value) in [
                (GeomagneticIndex::Hp30, point.hp30),
                (GeomagneticIndex::Kp, point.kp),
            ] {
                let peak = value
                    .and_then(|value| GeomagneticActivity::from_published_value(index, value))
                    .and_then(|activity| GeomagneticStormPeak::of(index, activity));
                self.keep_stronger_storm(peak);
            }
        }
    }

    /// Widen the TEC range to every value these fixes carry.
    fn collect_tec_from(&mut self, points: &[TecPoint]) {
        for tecu in points.iter().filter_map(|point| point.tecu) {
            self.widen_tec_to(Some(TecSpan {
                lowest: tecu,
                highest: tecu,
            }));
        }
    }

    /// Keep the strongest flare that counts and peaked while the track was
    /// recording. A flare over another track of the same recording says
    /// nothing about this one.
    fn collect_flares_peaking_in(&mut self, flares: &[MarkedFlare], recorded: TimeRange) {
        for marked in flares
            .iter()
            .filter(|marked| recorded.contains(marked.flare.peak))
        {
            self.keep_stronger_flare(SunlitFlarePeak::of(marked));
        }
    }

    fn keep_stronger_interference(&mut self, candidate: Option<f64>) {
        if let Some(percent) = candidate
            && self.aircraft_interference.is_none_or(|kept| percent > kept)
        {
            self.aircraft_interference = Some(percent);
        }
    }

    fn keep_stronger_storm(&mut self, candidate: Option<GeomagneticStormPeak>) {
        if let Some(peak) = candidate
            && self
                .geomagnetic
                .is_none_or(|kept| peak.activity > kept.activity)
        {
            self.geomagnetic = Some(peak);
        }
    }

    fn keep_stronger_flare(&mut self, candidate: Option<SunlitFlarePeak>) {
        if let Some(peak) = candidate
            && self
                .solar_flare
                .is_none_or(|kept| peak.classification > kept.classification)
        {
            self.solar_flare = Some(peak);
        }
    }

    /// Keep the deviation standing furthest from its quiet median, of those
    /// the index grades a storm.
    fn keep_stronger_tec_deviation(&mut self, candidate: Option<TecDeviationEvidence>) {
        if let Some(evidence) =
            candidate.filter(|evidence| evidence.peak.deviation.grade().is_a_storm())
            && self.tec_deviation.is_none_or(|kept| {
                evidence.peak.deviation.log_ratio().abs() > kept.peak.deviation.log_ratio().abs()
            })
        {
            self.tec_deviation = Some(evidence);
        }
    }

    fn widen_tec_to(&mut self, candidate: Option<TecSpan>) {
        let Some(span) = candidate else {
            return;
        };
        self.total_electron_content = Some(match self.total_electron_content {
            Some(kept) => TecSpan {
                lowest: kept.lowest.min(span.lowest),
                highest: kept.highest.max(span.highest),
            },
            None => span,
        });
    }
}

/// One archived index period overlapping the window before a TEC deviation's
/// peak epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArchivedIndexPeriod {
    pub end: DateTime<Utc>,
    pub activity: GeomagneticActivity,
}

/// What the archived geomagnetic indices hold over the
/// [`GEOMAGNETIC_LOOKBACK_HOURS`] before a TEC deviation's peak epoch.
///
/// Stated beside a TEC deviation and never a warning level of its own: a storm
/// before the peak accounts for the depletion, while a window the archive
/// covers without one leaves the 27-day reference as the likelier explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeomagneticActivityBeforePeak {
    /// The archive covers the window, or part of it, and reaches no storm
    /// level anywhere in what it holds.
    NoStorm,
    /// The strongest storm class the window holds, and the whole hours from
    /// the end of the period carrying it to the peak epoch.
    Storm {
        class: GeomagneticStormClass,
        hours_before_peak: i64,
    },
}

impl GeomagneticActivityBeforePeak {
    /// The strongest storm-level period of `periods`, which are the archived
    /// index periods of both indices overlapping the window before `peak`.
    pub fn of(peak: DateTime<Utc>, periods: &[ArchivedIndexPeriod]) -> Self {
        let strongest = periods
            .iter()
            .filter_map(|period| Some((period.activity.storm_class()?, period.end)))
            .max_by(|(left, left_end), (right, right_end)| {
                left.cmp(right).then_with(|| left_end.cmp(right_end))
            });
        let Some((class, end)) = strongest else {
            return Self::NoStorm;
        };
        Self::Storm {
            class,
            hours_before_peak: peak.signed_duration_since(end).num_hours().max(0),
        }
    }

    /// The clause the TEC deviation line closes with.
    fn finding(self) -> String {
        match self {
            Self::NoStorm => {
                format!("no geomagnetic storm in the {GEOMAGNETIC_LOOKBACK_HOURS}h before")
            }
            Self::Storm {
                class,
                hours_before_peak,
            } => format!(
                "after a {} storm {hours_before_peak}h before",
                class.scale_name()
            ),
        }
    }
}

/// The window before `peak` the archived geomagnetic indices are read over.
pub fn geomagnetic_lookback_window(peak: DateTime<Utc>) -> TimeRange {
    TimeRange::new(
        peak.checked_sub_signed(TimeDelta::hours(GEOMAGNETIC_LOOKBACK_HOURS))
            .unwrap_or(peak),
        peak,
    )
}

/// One track's peak TEC deviation and what the archives say about it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TecDeviationEvidence {
    pub peak: QuietTimeDeviationPeak,
    /// [`None`] where the archive holds none of the hours before the peak,
    /// which is stated neither way.
    pub geomagnetic_before_peak: Option<GeomagneticActivityBeforePeak>,
}

impl TecDeviationEvidence {
    /// The deviation's own share of the median, the grade the index puts it
    /// at, how long that grade held at the node the peak was read from, and
    /// what the geomagnetic indices hold over the hours before it. The limit
    /// is the trigger on the same side of the median as the deviation itself.
    fn warning_line(self) -> WarningLine {
        let deviation = self.peak.deviation;
        let trigger = QuietTimeDeviation::from_log_ratio(
            quiet_time::MODERATE_STORM_LOG_RATIO.copysign(deviation.log_ratio()),
        );
        let mut clauses = vec![
            format!(
                "{:+.0}% from the {}-day median",
                deviation.percent_from_median(),
                quiet_time::BACKGROUND_WINDOW_DAYS
            ),
            format!(
                "{} (W = {})",
                deviation.grade(),
                deviation.storm_index_value()
            ),
        ];
        clauses.extend(self.peak.storm_grade_run.map(|run| run.to_string()));
        clauses.extend(
            self.geomagnetic_before_peak
                .map(GeomagneticActivityBeforePeak::finding),
        );
        WarningLine {
            metric: TEC_DEVIATION,
            limit: format!(
                "{} {:+.0}%",
                if deviation.log_ratio() < 0.0 {
                    '<'
                } else {
                    '>'
                },
                trigger.percent_from_median()
            ),
            finding: clauses.join(", "),
        }
    }
}

/// A geomagnetic index value that reaches the NOAA G scale, whose first
/// level starts at 5 on the Kp scale both indices are published on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GeomagneticStormPeak {
    index: GeomagneticIndex,
    activity: GeomagneticActivity,
    storm: GeomagneticStormClass,
}

impl GeomagneticStormPeak {
    /// The peak, or [`None`] for a value below the first storm level.
    fn of(index: GeomagneticIndex, activity: GeomagneticActivity) -> Option<Self> {
        activity.storm_class().map(|storm| Self {
            index,
            activity,
            storm,
        })
    }

    fn warning_line(self) -> WarningLine {
        WarningLine {
            metric: GEOMAGNETIC_STORM,
            limit: format!("≥{}", GeomagneticStormClass::Minor.lowest_value()),
            finding: format!(
                "{} {}, {}",
                self.index,
                self.activity,
                self.storm.scale_name()
            ),
        }
    }
}

/// A flare that peaked while the receiver was in daylight and reaches the
/// NOAA radio blackout scale, whose first level starts at class M1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SunlitFlarePeak {
    classification: FlareClassification,
    blackout: RadioBlackoutClass,
    peak: DateTime<Utc>,
}

impl SunlitFlarePeak {
    /// The peak, or [`None`] for a flare below the blackout scale or one the
    /// receiver spent on the night side, where the ionization it raises never
    /// reached the signal path.
    fn of(marked: &MarkedFlare) -> Option<Self> {
        let blackout = marked.flare.classification.radio_blackout_class()?;
        (marked.receiver_side == Some(SunlitSide::Sunlit)).then_some(Self {
            classification: marked.flare.classification,
            blackout,
            peak: marked.flare.peak,
        })
    }

    fn warning_line(self) -> WarningLine {
        WarningLine {
            metric: SOLAR_FLARE,
            limit: format!("≥{FLARE_TRIGGER_CLASSIFICATION}, {SUNLIT_RECEIVER}"),
            finding: format!(
                "{} at {} UTC, {}",
                self.classification,
                self.peak.format(UTC_MINUTE_FORMAT),
                self.blackout.scale_name()
            ),
        }
    }
}

/// The lowest and highest TEC value a track's fixes carry, in TEC units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TecSpan {
    lowest: f64,
    highest: f64,
}

impl TecSpan {
    fn context_line(self) -> String {
        format!(
            "{TEC_OVER_TRACK}: {:.0}–{:.0} {}",
            self.lowest,
            self.highest,
            gt_ionex::text::LEGEND_UNIT
        )
    }
}

/// The per-fix series of one track, borrowed from the ones the plot draws.
pub struct TrackSeries<'a> {
    interference: Option<&'a Arc<Vec<JammingPoint>>>,
    geomagnetic: Option<&'a Arc<Vec<GeomagneticPoint>>>,
    tec: Option<&'a Arc<Vec<TecPoint>>>,
    /// The track's peak deviation. It is read from days the track itself
    /// never spans, so it is kept as its own field.
    tec_deviation: Option<TecDeviationEvidence>,
}

impl<'a> TrackSeries<'a> {
    /// The series of `track`. A metric whose day the archive does not hold is
    /// absent and contributes nothing.
    pub fn of(
        track: TrackRef,
        jamming: &'a JammingSeries,
        geomagnetic: &'a GeomagneticSeries,
        tec: &'a TecSeries,
        tec_deviations: &FxHashMap<TrackRef, TecDeviationEvidence>,
    ) -> Self {
        Self {
            interference: jamming.points_by_track.get(&track),
            geomagnetic: geomagnetic.points_by_track.get(&track),
            tec: tec.points_by_track.get(&track),
            tec_deviation: tec_deviations.get(&track).copied(),
        }
    }

    /// The allocations the values were read from. A fetch worker archiving a
    /// day replaces the ones its day reaches, which is what re-assesses the
    /// track.
    fn identities(&self) -> Vec<ArcIdentity> {
        let interference = self.interference.map(ArcIdentity::of);
        let geomagnetic = self.geomagnetic.map(ArcIdentity::of);
        let tec = self.tec.map(ArcIdentity::of);
        interference
            .into_iter()
            .chain(geomagnetic)
            .chain(tec)
            .collect()
    }
}

/// One loaded track as it enters the assessment.
pub struct TrackUnderAssessment<'a> {
    /// How the warning names the track, as the rest of the app names it.
    pub label: String,
    /// The span the track's own fixes cover, which decides the flares it is
    /// assessed against.
    pub recorded: TimeRange,
    pub series: TrackSeries<'a>,
}

/// One loaded recording as it enters the assessment.
pub struct RecordingUnderAssessment<'a> {
    pub id: LoadedFileId,
    /// How the load toast names the recording.
    pub label: String,
    /// The span its tracks cover, which its flares are read over.
    pub span: TimeRange,
    pub tracks: Vec<TrackUnderAssessment<'a>>,
    /// The archived days a flare peaking inside [`Self::span`] can be filed
    /// under.
    pub archived_flare_days: Vec<NaiveDate>,
    /// Identity of the fix timeline the receiver's side of Earth is read at.
    pub positions: ArcIdentity,
}

/// What one recording's warning was read from: it is read again exactly when
/// this changes.
#[derive(Debug, Clone, PartialEq)]
struct WarningSource {
    series: Vec<ArcIdentity>,
    /// Carried by value: no allocation the track holds changes when a
    /// deviation or the activity before it moves, since both are read from
    /// days outside the track's own.
    tec_deviations: Vec<Option<TecDeviationEvidence>>,
    archived_flare_days: Vec<NaiveDate>,
    positions: ArcIdentity,
    /// The names the lines carry, which move with the user's recording-name
    /// template and with the tracks a file holds.
    labels: Vec<String>,
}

impl WarningSource {
    fn of(recording: &RecordingUnderAssessment<'_>) -> Self {
        Self {
            series: recording
                .tracks
                .iter()
                .flat_map(|track| track.series.identities())
                .collect(),
            tec_deviations: recording
                .tracks
                .iter()
                .map(|track| track.series.tec_deviation)
                .collect(),
            archived_flare_days: recording.archived_flare_days.clone(),
            positions: recording.positions,
            labels: recording
                .tracks
                .iter()
                .map(|track| track.label.clone())
                .chain([recording.label.clone()])
                .collect(),
        }
    }
}

/// What one recording warns about: one entry per affected track, and the
/// toast text for the recording as a whole.
struct AssessedRecording {
    resolved_from: WarningSource,
    /// In track order, holding only the tracks a metric reached its level
    /// over.
    tracks: Vec<TrackSpaceWeatherWarning>,
    /// [`None`] while no track of the recording warns.
    toast: Option<String>,
}

impl AssessedRecording {
    fn of(
        recording: &RecordingUnderAssessment<'_>,
        flares: &[MarkedFlare],
        resolved_from: WarningSource,
    ) -> Self {
        let mut over_the_recording = DisturbanceEvidence::default();
        let mut tracks = Vec::new();
        for track in &recording.tracks {
            let evidence = DisturbanceEvidence::of(&track.series, flares, track.recorded);
            over_the_recording.merge(&evidence);
            let lines = evidence.warning_lines();
            if !lines.is_empty() {
                tracks.push(TrackSpaceWeatherWarning {
                    track_label: track.label.clone(),
                    lines,
                    states_tec_deviation: evidence.tec_deviation.is_some(),
                });
            }
        }
        let toast = over_the_recording
            .warns()
            .then(|| over_the_recording.load_toast_text(&recording.label));
        Self {
            resolved_from,
            tracks,
            toast,
        }
    }
}

/// What the loaded recordings' archived environment values warn about: the
/// affected tracks the map indicator lists, and which recordings the load
/// toast has already been raised for.
#[derive(Default)]
pub struct SpaceWeatherWarning {
    assessed: FxHashMap<LoadedFileId, AssessedRecording>,
    /// Recordings the toast has been shown for, so a day archived later never
    /// repeats it.
    toasted: FxHashSet<LoadedFileId>,
    track_warnings: Vec<TrackSpaceWeatherWarning>,
}

impl SpaceWeatherWarning {
    /// Read the warning of every recording the archives moved under, and
    /// report the load toast of each one that warns for the first time.
    ///
    /// `flares_peaking_in` reads the archive, so it runs only for a recording
    /// being assessed again.
    pub fn reassess(
        &mut self,
        recordings: &[RecordingUnderAssessment<'_>],
        mut flares_peaking_in: impl FnMut(TimeRange) -> Vec<MarkedFlare>,
    ) -> Vec<String> {
        let mut changed = false;
        for recording in recordings {
            let resolved_from = WarningSource::of(recording);
            if self
                .assessed
                .get(&recording.id)
                .is_some_and(|assessed| assessed.resolved_from == resolved_from)
            {
                continue;
            }
            let flares = flares_peaking_in(recording.span);
            self.assessed.insert(
                recording.id,
                AssessedRecording::of(recording, &flares, resolved_from),
            );
            changed = true;
        }

        let loaded: FxHashSet<LoadedFileId> = recordings.iter().map(|r| r.id).collect();
        let assessed_before = self.assessed.len();
        self.assessed.retain(|id, _| loaded.contains(id));
        self.toasted.retain(|id| loaded.contains(id));
        if changed || self.assessed.len() != assessed_before {
            self.track_warnings = recordings
                .iter()
                .filter_map(|recording| self.assessed.get(&recording.id))
                .flat_map(|assessed| assessed.tracks.iter().cloned())
                .collect();
        }

        let mut toasts = Vec::new();
        for recording in recordings {
            if self.toasted.contains(&recording.id) {
                continue;
            }
            let Some(toast) = self
                .assessed
                .get(&recording.id)
                .and_then(|assessed| assessed.toast.clone())
            else {
                continue;
            };
            self.toasted.insert(recording.id);
            toasts.push(toast);
        }
        toasts
    }

    /// One entry per loaded track a metric reached its disturbance level over,
    /// in the order the recordings are loaded in. Empty while none did.
    pub fn track_warnings(&self) -> &[TrackSpaceWeatherWarning] {
        &self.track_warnings
    }
}

#[cfg(test)]
mod tests {
    use gt_flare::SolarFlare;
    use gt_ionex::quiet_time::StormGradeRun;
    use gt_ionex::tec::TotalElectronContent;
    use gt_loaded_files::{FileHistory, LoadedFiles};
    use gt_types::{FileSource, LoadedFile};
    use rstest::rstest;

    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(2024, 5, 11)
            .and_then(|day| day.and_hms_opt(hour, minute, 0))
            .map(|naive| naive.and_utc())
            .unwrap_or_default()
    }

    fn day(day_of_month: u32) -> NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2024, 5, day_of_month).unwrap_or_default()
    }

    /// The whole span the fixtures record over, which every track covers
    /// unless a test states its own.
    fn whole_recording() -> TimeRange {
        TimeRange::new(at(0, 0), at(6, 0))
    }

    /// The per-fix series of one track, owning what [`TrackSeries`] borrows.
    #[derive(Default)]
    struct SeriesFixture {
        interference: Option<Arc<Vec<JammingPoint>>>,
        geomagnetic: Option<Arc<Vec<GeomagneticPoint>>>,
        tec: Option<Arc<Vec<TecPoint>>>,
        tec_deviation: Option<TecDeviationEvidence>,
    }

    impl SeriesFixture {
        fn series(&self) -> TrackSeries<'_> {
            TrackSeries {
                interference: self.interference.as_ref(),
                geomagnetic: self.geomagnetic.as_ref(),
                tec: self.tec.as_ref(),
                tec_deviation: self.tec_deviation,
            }
        }
    }

    /// The deviation a track reaches at `percent` of a fully archived quiet
    /// window's median, built the way the archive read builds it, holding at
    /// the storm grade over five two-hour epochs, which span 8h.
    fn tec_deviation(percent: f64) -> Option<TecDeviationEvidence> {
        tec_deviation_corroborated_by(percent, None)
    }

    /// The same deviation, with what the archived indices hold over the hours
    /// before its peak epoch.
    fn tec_deviation_corroborated_by(
        percent: f64,
        geomagnetic_before_peak: Option<GeomagneticActivityBeforePeak>,
    ) -> Option<TecDeviationEvidence> {
        let median = 20.0;
        let window =
            vec![TotalElectronContent::from_tecu(median); quiet_time::BACKGROUND_WINDOW_DAYS];
        let deviation = quiet_time::deviation_from_quiet_time(
            TotalElectronContent::from_tecu(median * (1.0 + percent / 100.0)),
            &window,
        )?;
        Some(TecDeviationEvidence {
            peak: QuietTimeDeviationPeak {
                deviation,
                epoch: at(4, 0),
                storm_grade_run: StormGradeRun::containing_epoch(
                    &[Some(deviation); 5],
                    2,
                    TimeDelta::hours(2),
                ),
            },
            geomagnetic_before_peak,
        })
    }

    fn interference_points(percents: &[f64]) -> Option<Arc<Vec<JammingPoint>>> {
        Some(Arc::new(
            percents
                .iter()
                .enumerate()
                .map(|(index, &percent)| JammingPoint {
                    x_secs: index as f64,
                    percent: Some(percent),
                    aircraft: 100,
                    bad: 1,
                })
                .collect(),
        ))
    }

    fn geomagnetic_points(
        hp30: Option<f64>,
        kp: Option<f64>,
    ) -> Option<Arc<Vec<GeomagneticPoint>>> {
        Some(Arc::new(vec![GeomagneticPoint {
            x_secs: 0.0,
            hp30,
            kp,
        }]))
    }

    fn tec_points(values: &[f64]) -> Option<Arc<Vec<TecPoint>>> {
        Some(Arc::new(
            values
                .iter()
                .enumerate()
                .map(|(index, &tecu)| TecPoint {
                    x_secs: index as f64,
                    tecu: Some(tecu),
                })
                .collect(),
        ))
    }

    fn flare(class_type: &str, receiver_side: Option<SunlitSide>) -> MarkedFlare {
        flare_peaking_at(class_type, receiver_side, at(2, 1))
    }

    fn flare_peaking_at(
        class_type: &str,
        receiver_side: Option<SunlitSide>,
        peak: DateTime<Utc>,
    ) -> MarkedFlare {
        MarkedFlare {
            flare: SolarFlare {
                id: format!("{class_type}-FLR-001"),
                begin: peak - chrono::TimeDelta::minutes(1),
                peak,
                end: None,
                classification: class_type.parse().expect("a published class"),
                source_location: None,
                active_region: None,
            },
            receiver_side,
        }
    }

    /// One archived index period ending at `end` and carrying `value` on the
    /// scale both indices are published on.
    fn archived_period(end: DateTime<Utc>, value: f64) -> ArchivedIndexPeriod {
        ArchivedIndexPeriod {
            end,
            activity: GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, value)
                .expect("a published value"),
        }
    }

    fn evidence_of(fixture: &SeriesFixture, flares: &[MarkedFlare]) -> DisturbanceEvidence {
        DisturbanceEvidence::of(&fixture.series(), flares, whole_recording())
    }

    /// Ids as the loader hands them out, so an assessment is keyed by the
    /// same values the application passes it.
    fn loaded_recording_ids(count: usize) -> Vec<LoadedFileId> {
        let mut files = LoadedFiles::new();
        for _ in 0..count {
            files.push(
                LoadedFile {
                    metadata: gt_test_utils::empty_file_metadata(),
                    tracks: Vec::new(),
                    event_marker_styles: FxHashMap::default(),
                    orphaned_event_markers: Vec::new(),
                    load_warnings: Vec::new(),
                    source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
                },
                FileHistory::None,
            );
        }
        files.view().entries().map(|entry| entry.id()).collect()
    }

    /// One recording of one track named `label`, covering the whole span the
    /// fixtures record over.
    fn recording<'a>(
        id: LoadedFileId,
        label: &str,
        series: &'a SeriesFixture,
        archived_flare_days: &[NaiveDate],
        positions: &Arc<u8>,
    ) -> RecordingUnderAssessment<'a> {
        let track = TrackUnderAssessment {
            label: label.to_owned(),
            recorded: whole_recording(),
            series: series.series(),
        };
        RecordingUnderAssessment {
            id,
            label: label.to_owned(),
            span: whole_recording(),
            tracks: vec![track],
            archived_flare_days: archived_flare_days.to_vec(),
            positions: ArcIdentity::of(positions),
        }
    }

    fn no_flares(_span: TimeRange) -> Vec<MarkedFlare> {
        Vec::new()
    }

    /// The lines of the one track the assessment holds a warning for.
    fn only_track_warning(warning: &SpaceWeatherWarning) -> &TrackSpaceWeatherWarning {
        match warning.track_warnings() {
            [only] => only,
            listed => panic!("one track warns, not {}", listed.len()),
        }
    }

    /// A crossed cell-day below the trigger is background, one at it is not.
    /// The boundary is the 2% share gpsjam starts colouring its cells at.
    #[rstest]
    #[case::below(1.9, false)]
    #[case::at_the_trigger(2.0, true)]
    #[case::heavy(34.2, true)]
    fn a_crossed_cell_warns_from_the_trigger_share_up(
        #[case] percent: f64,
        #[case] expected: bool,
    ) {
        let fixture = SeriesFixture {
            interference: interference_points(&[0.4, percent]),
            ..SeriesFixture::default()
        };

        let evidence = evidence_of(&fixture, &[]);

        assert_eq!(evidence.warns(), expected);
        assert_eq!(evidence.aircraft_interference, expected.then_some(percent));
    }

    /// The G scale starts at 5, which is where the geomagnetic warning does.
    #[rstest]
    #[case::below_the_first_storm(4.667, false)]
    #[case::g1_floor(5.0, true)]
    #[case::g3(7.667, true)]
    fn a_period_warns_from_the_first_storm_level_up(#[case] value: f64, #[case] expected: bool) {
        let fixture = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(value), None),
            ..SeriesFixture::default()
        };

        assert_eq!(evidence_of(&fixture, &[]).warns(), expected);
    }

    /// Either index raises the warning, and the line names the one that
    /// carried the higher value.
    #[test]
    fn the_geomagnetic_line_names_the_index_that_peaked() {
        let fixture = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), Some(5.0)),
            ..SeriesFixture::default()
        };

        assert_eq!(
            evidence_of(&fixture, &[]).warning_lines(),
            ["Geomagnetic storm (≥5): Hp30 7.667, G3"]
        );
    }

    /// The radio blackout scale starts at M1, which is where the flare
    /// warning does.
    #[rstest]
    #[case::below_the_scale("C9.9", false)]
    #[case::m1_floor("M1.0", true)]
    #[case::the_may_2024_peak("X5.8", true)]
    fn a_flare_warns_from_the_first_blackout_level_up(
        #[case] class_type: &str,
        #[case] expected: bool,
    ) {
        let flares = [flare(class_type, Some(SunlitSide::Sunlit))];

        assert_eq!(
            evidence_of(&SeriesFixture::default(), &flares).warns(),
            expected
        );
    }

    /// A flare the receiver was not in daylight for raised no ionization over
    /// it, however strong the flare was.
    #[rstest]
    #[case::night(Some(SunlitSide::Night))]
    #[case::no_recording_placed_the_receiver(None)]
    fn a_flare_the_receiver_was_not_in_daylight_for_warns_about_nothing(
        #[case] receiver_side: Option<SunlitSide>,
    ) {
        let flares = [flare("X5.8", receiver_side)];

        assert!(!evidence_of(&SeriesFixture::default(), &flares).warns());
    }

    /// A flare is evidence only for the track that was recording when it
    /// peaked, so a recording's other tracks stay clear of it.
    #[rstest]
    #[case::while_the_track_recorded(at(1, 0), true)]
    #[case::at_the_end_of_the_track(at(2, 0), true)]
    #[case::after_the_track_ended(at(2, 1), false)]
    fn a_flare_is_evidence_for_the_track_it_peaked_over(
        #[case] peak: DateTime<Utc>,
        #[case] expected: bool,
    ) {
        let flares = [flare_peaking_at("X5.8", Some(SunlitSide::Sunlit), peak)];
        let recorded = TimeRange::new(at(0, 0), at(2, 0));

        let evidence =
            DisturbanceEvidence::of(&SeriesFixture::default().series(), &flares, recorded);

        assert_eq!(evidence.warns(), expected);
    }

    /// An absolute TEC value is context, so it raises no warning on its own
    /// and is listed only once another metric has.
    #[test]
    fn the_tec_range_never_warns_on_its_own() {
        let fixture = SeriesFixture {
            tec: tec_points(&[12.4, 175.2, 40.0]),
            ..SeriesFixture::default()
        };

        let evidence = evidence_of(&fixture, &[]);

        assert!(!evidence.warns());
        assert!(evidence.warning_lines().is_empty());
    }

    /// Every line of a fully disturbed track, in the order the hover lists
    /// them, each stating the peak that produced it and the level it crossed.
    #[test]
    fn every_metric_that_warns_states_its_peak() {
        let fixture = SeriesFixture {
            interference: interference_points(&[0.4, 34.2]),
            geomagnetic: geomagnetic_points(Some(7.667), Some(9.0)),
            tec: tec_points(&[12.4, 175.2]),
            tec_deviation: tec_deviation(62.0),
        };
        let flares = [
            flare("M1.2", Some(SunlitSide::Sunlit)),
            flare("X5.8", Some(SunlitSide::Sunlit)),
        ];

        assert_eq!(
            evidence_of(&fixture, &flares).warning_lines(),
            [
                "Geomagnetic storm (≥5): Kp 9, G5",
                "Aircraft interference (≥2%): up to 34.2% of aircraft in a crossed cell",
                "Solar flare (≥M1, sunlit): X5.8 at 2024-05-11 02:01 UTC, R3",
                "ΔTEC (> +43%): +62% from the 27-day median, moderate ionospheric storm (W = 3), 8h",
                "TEC over track: 12–175 TECU",
            ]
        );
    }

    /// A deviation the index grades below a storm is quiet-time variation,
    /// and is left out of the evidence entirely.
    #[test]
    fn a_deviation_short_of_the_storm_grade_warns_about_nothing() {
        let fixture = SeriesFixture {
            tec_deviation: tec_deviation(42.5),
            ..SeriesFixture::default()
        };

        let evidence = evidence_of(&fixture, &[]);

        assert!(!evidence.warns());
        assert!(evidence.warning_lines().is_empty());
    }

    /// The line states the signed share of the median, the grade the index
    /// puts it at, how long that grade held at the node the peak was read
    /// from, and the trigger on the deviation's own side of the median.
    #[rstest]
    #[case::a_moderate_storm(
        62.0,
        "ΔTEC (> +43%): +62% from the 27-day median, moderate ionospheric storm (W = 3), 8h"
    )]
    #[case::a_negative_moderate_storm(
        -35.0,
        "ΔTEC (< -30%): -35% from the 27-day median, moderate ionospheric storm (W = -3), 8h"
    )]
    #[case::an_intense_storm(
        200.0,
        "ΔTEC (> +43%): +200% from the 27-day median, intense ionospheric storm (W = 4), 8h"
    )]
    fn the_deviation_line_states_its_share_and_grade(#[case] percent: f64, #[case] expected: &str) {
        let fixture = SeriesFixture {
            tec_deviation: tec_deviation(percent),
            ..SeriesFixture::default()
        };

        assert_eq!(evidence_of(&fixture, &[]).warning_lines(), [expected]);
    }

    /// A grade read from a single map epoch states that epoch's own length,
    /// which the day's map interval sets.
    #[test]
    fn a_deviation_read_from_one_epoch_states_the_epoch() {
        let Some(mut evidence) = tec_deviation(62.0) else {
            panic!("a fully archived window grades the value");
        };
        evidence.peak.storm_grade_run = StormGradeRun::containing_epoch(
            &[Some(evidence.peak.deviation)],
            0,
            TimeDelta::hours(2),
        );
        let fixture = SeriesFixture {
            tec_deviation: Some(evidence),
            ..SeriesFixture::default()
        };

        assert_eq!(
            evidence_of(&fixture, &[]).warning_lines(),
            [
                "ΔTEC (> +43%): +62% from the 27-day median, moderate ionospheric storm (W = 3), \
                 one 2h epoch"
            ]
        );
    }

    /// The archived indices qualify the deviation: a storm before the peak
    /// accounts for it, a quiet window says the 27-day reference is doing the
    /// work, and hours the archive holds none of are stated neither way.
    #[rstest]
    #[case::a_storm_before_the_peak(
        Some(GeomagneticActivityBeforePeak::Storm {
            class: GeomagneticStormClass::Strong,
            hours_before_peak: 5,
        }),
        "ΔTEC (< -30%): -35% from the 27-day median, moderate ionospheric storm (W = -3), 8h, \
         after a G3 storm 5h before"
    )]
    #[case::a_quiet_window(
        Some(GeomagneticActivityBeforePeak::NoStorm),
        "ΔTEC (< -30%): -35% from the 27-day median, moderate ionospheric storm (W = -3), 8h, \
         no geomagnetic storm in the 48h before"
    )]
    #[case::an_unarchived_window(
        None,
        "ΔTEC (< -30%): -35% from the 27-day median, moderate ionospheric storm (W = -3), 8h"
    )]
    fn the_deviation_line_states_the_geomagnetic_activity_before_its_peak(
        #[case] geomagnetic_before_peak: Option<GeomagneticActivityBeforePeak>,
        #[case] expected: &str,
    ) {
        let fixture = SeriesFixture {
            tec_deviation: tec_deviation_corroborated_by(-35.0, geomagnetic_before_peak),
            ..SeriesFixture::default()
        };

        assert_eq!(evidence_of(&fixture, &[]).warning_lines(), [expected]);
    }

    /// The strongest storm class the window holds is the one named, and a
    /// period the peak falls inside is stated as no hours before it.
    #[rstest]
    #[case::before_the_peak(at(4, 0), Some(GeomagneticActivityBeforePeak::Storm {
        class: GeomagneticStormClass::Severe,
        hours_before_peak: 2,
    }))]
    #[case::overlapping_the_peak(at(1, 0), Some(GeomagneticActivityBeforePeak::Storm {
        class: GeomagneticStormClass::Severe,
        hours_before_peak: 0,
    }))]
    fn the_strongest_archived_storm_is_the_one_named(
        #[case] peak: DateTime<Utc>,
        #[case] expected: Option<GeomagneticActivityBeforePeak>,
    ) {
        let periods = [
            archived_period(at(1, 0), 5.0),
            archived_period(at(2, 0), 8.0),
            archived_period(at(3, 0), 6.0),
            archived_period(at(3, 30), 4.667),
        ];

        assert_eq!(
            Some(GeomagneticActivityBeforePeak::of(peak, &periods)),
            expected
        );
    }

    /// A window in which the archive holds only quiet periods names no storm.
    #[test]
    fn an_archived_window_without_a_storm_names_none() {
        let periods = [archived_period(at(2, 0), 4.667)];

        assert_eq!(
            GeomagneticActivityBeforePeak::of(at(4, 0), &periods),
            GeomagneticActivityBeforePeak::NoStorm
        );
    }

    /// Every level the map indicator's popup states, with the link each row
    /// carries.
    #[test]
    fn the_warning_levels_state_what_raises_each_warning() {
        let listed: Vec<String> = WARNING_LEVELS
            .iter()
            .map(|level| format!("{}\n{}", level.trigger, level.reference.link_question))
            .collect();

        insta::assert_snapshot!("warning_levels", listed.join("\n\n"));
    }

    /// The flare row names a classification, while the blackout scale is
    /// defined in peak flux: M1 is the weakest class that reaches the scale.
    #[test]
    fn the_stated_flare_class_is_the_first_blackout_level() {
        let stated: FlareClassification = format!("{FLARE_TRIGGER_CLASSIFICATION}.0")
            .parse()
            .expect("a published class");
        let below: FlareClassification = "C9.9".parse().expect("a published class");

        assert_eq!(
            stated.radio_blackout_class(),
            Some(RadioBlackoutClass::Minor)
        );
        assert_eq!(below.radio_blackout_class(), None);
    }

    /// A recording no archived day overlaps carries no value at all, so
    /// nothing is guessed for it.
    #[test]
    fn a_recording_without_archived_values_warns_about_nothing() {
        let evidence = evidence_of(&SeriesFixture::default(), &[]);

        assert!(!evidence.warns());
        assert_eq!(evidence, DisturbanceEvidence::default());
    }

    /// A recording loaded before its days arrive warns as soon as one of them
    /// reaches its series, and is toasted once however many days follow.
    #[test]
    fn a_day_archived_after_the_load_warns_once() {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(1);
        let Some(&id) = ids.first() else {
            panic!("one recording was loaded");
        };
        let quiet = SeriesFixture::default();

        assert!(
            warning
                .reassess(
                    &[recording(id, "morning.gtd", &quiet, &[], &positions)],
                    no_flares
                )
                .is_empty()
        );
        assert!(warning.track_warnings().is_empty());

        let stormy = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), None),
            ..SeriesFixture::default()
        };
        assert_eq!(
            warning.reassess(
                &[recording(id, "morning.gtd", &stormy, &[], &positions)],
                no_flares
            ),
            ["Space weather during morning.gtd\nGeomagnetic storm (≥5): Hp30 7.667, G3"],
            "the archived day reached the loaded recording"
        );
        assert_eq!(
            only_track_warning(&warning).lines,
            ["Geomagnetic storm (≥5): Hp30 7.667, G3"]
        );

        let stormier = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), Some(9.0)),
            ..SeriesFixture::default()
        };
        assert!(
            warning
                .reassess(
                    &[recording(id, "morning.gtd", &stormier, &[], &positions)],
                    no_flares
                )
                .is_empty(),
            "a second archived day does not toast the same recording again"
        );
    }

    /// A toast stating a TEC deviation closes with what the grade is measured
    /// against, and one without a deviation ends at its own findings.
    #[rstest]
    #[case::a_deviation(
        SeriesFixture {
            tec_deviation: tec_deviation(62.0),
            ..SeriesFixture::default()
        },
        format!(
            "Space weather during morning.gtd\nΔTEC (> +43%): +62% from the 27-day median, \
             moderate ionospheric storm (W = 3), 8h\n{}",
            *gt_ionex::text::DEVIATION_REFERENCE_CAVEAT
        )
    )]
    #[case::a_storm_without_one(
        SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), None),
            ..SeriesFixture::default()
        },
        "Space weather during morning.gtd\nGeomagnetic storm (≥5): Hp30 7.667, G3".to_owned()
    )]
    fn a_toast_stating_a_deviation_closes_with_its_reference(
        #[case] series: SeriesFixture,
        #[case] expected: String,
    ) {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(1);
        let Some(&id) = ids.first() else {
            panic!("one recording was loaded");
        };

        let toasts = warning.reassess(
            &[recording(id, "morning.gtd", &series, &[], &positions)],
            no_flares,
        );

        assert_eq!(toasts, [expected]);
    }

    /// Every disturbed recording is toasted, and only the disturbed ones, each
    /// toast naming its own recording.
    #[test]
    fn each_disturbed_recording_is_toasted_once() {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(3);
        let quiet = SeriesFixture::default();
        let stormy = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(5.0), None),
            ..SeriesFixture::default()
        };
        let series = |index: usize| {
            if index < 2 { &stormy } else { &quiet }
        };
        let recordings: Vec<RecordingUnderAssessment<'_>> = ids
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                recording(
                    id,
                    &format!("ride-{index}.gtd"),
                    series(index),
                    &[],
                    &positions,
                )
            })
            .collect();

        assert_eq!(
            warning.reassess(&recordings, no_flares),
            [
                "Space weather during ride-0.gtd\nGeomagnetic storm (≥5): Hp30 5, G1",
                "Space weather during ride-1.gtd\nGeomagnetic storm (≥5): Hp30 5, G1",
            ]
        );
    }

    /// Each affected track is listed on its own, named the way the rest of
    /// the app names it, and a quiet track is left out.
    #[test]
    fn every_affected_track_is_listed_with_its_own_values() {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(1);
        let Some(&id) = ids.first() else {
            panic!("one recording was loaded");
        };
        let quiet = SeriesFixture::default();
        let stormy = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(5.0), None),
            ..SeriesFixture::default()
        };
        let stormier = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(9.0), None),
            ..SeriesFixture::default()
        };
        let tracks: Vec<TrackUnderAssessment<'_>> = [&stormy, &quiet, &stormier]
            .into_iter()
            .enumerate()
            .map(|(index, series)| TrackUnderAssessment {
                label: format!("morning.gtd (track {})", index + 1),
                recorded: whole_recording(),
                series: series.series(),
            })
            .collect();
        let recording = RecordingUnderAssessment {
            id,
            label: "morning.gtd".to_owned(),
            span: whole_recording(),
            tracks,
            archived_flare_days: Vec::new(),
            positions: ArcIdentity::of(&positions),
        };

        warning.reassess(&[recording], no_flares);

        let listed: Vec<(&str, &[String])> = warning
            .track_warnings()
            .iter()
            .map(|warning| (warning.track_label.as_str(), warning.lines.as_slice()))
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "morning.gtd (track 1)",
                    ["Geomagnetic storm (≥5): Hp30 5, G1".to_owned()].as_slice()
                ),
                (
                    "morning.gtd (track 3)",
                    ["Geomagnetic storm (≥5): Hp30 9, G5".to_owned()].as_slice()
                ),
            ]
        );
    }

    /// The flare archive is read only for a recording being assessed again,
    /// and a day stored later is what makes it read again.
    #[test]
    fn the_flare_archive_is_read_once_per_archived_day_set() {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(1);
        let Some(&id) = ids.first() else {
            panic!("one recording was loaded");
        };
        let quiet = SeriesFixture::default();
        let mut reads = 0;
        let mut count_read = |_span: TimeRange| {
            reads += 1;
            vec![flare("X5.8", Some(SunlitSide::Sunlit))]
        };

        warning.reassess(
            &[recording(id, "morning.gtd", &quiet, &[day(11)], &positions)],
            &mut count_read,
        );
        warning.reassess(
            &[recording(id, "morning.gtd", &quiet, &[day(11)], &positions)],
            &mut count_read,
        );
        warning.reassess(
            &[recording(
                id,
                "morning.gtd",
                &quiet,
                &[day(11), day(12)],
                &positions,
            )],
            &mut count_read,
        );

        assert_eq!(reads, 2);
    }

    /// Renaming a recording renames it in the warning, without waiting for an
    /// archived day to move.
    #[test]
    fn a_renamed_recording_is_named_again_in_its_warning() {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(1);
        let Some(&id) = ids.first() else {
            panic!("one recording was loaded");
        };
        let stormy = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), None),
            ..SeriesFixture::default()
        };

        warning.reassess(
            &[recording(id, "morning.gtd", &stormy, &[], &positions)],
            no_flares,
        );
        warning.reassess(
            &[recording(id, "Morning ride", &stormy, &[], &positions)],
            no_flares,
        );

        assert_eq!(only_track_warning(&warning).track_label, "Morning ride");
    }

    /// A recording that is closed takes its warning out of the map's list.
    #[test]
    fn unloading_a_recording_drops_its_evidence() {
        let mut warning = SpaceWeatherWarning::default();
        let positions = Arc::new(0_u8);
        let ids = loaded_recording_ids(1);
        let Some(&id) = ids.first() else {
            panic!("one recording was loaded");
        };
        let stormy = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), None),
            ..SeriesFixture::default()
        };

        warning.reassess(
            &[recording(id, "morning.gtd", &stormy, &[], &positions)],
            no_flares,
        );
        assert!(!warning.track_warnings().is_empty());

        warning.reassess(&[], no_flares);
        assert!(warning.track_warnings().is_empty());
    }
}
