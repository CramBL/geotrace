//! Whether the archived environment values could have disturbed a loaded
//! recording, and the warning the load toast and the map indicator show.
//!
//! Every value is read from the archives, so a recording loaded before its
//! days were downloaded is assessed again as each day is stored, and a
//! recording no archived day overlaps warns about nothing.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, NaiveDate, Utc};
use gt_flare::{FlareClassification, MarkedFlare, RadioBlackoutClass};
use gt_ionex::quiet_time::{self, QuietTimeDeviation};
use gt_loaded_files::LoadedFileId;
use gt_solar::GeomagneticIndex;
use gt_solar::activity::{GeomagneticActivity, GeomagneticStormClass};
use gt_types::{SunlitSide, TimeRange, TrackRef};
use gt_ui_types::{
    ArcIdentity, GeomagneticPoint, GeomagneticSeries, JammingPoint, JammingSeries, TecPoint,
    TecSeries, WarningLevelExplanation,
};

/// Shown as a toast the first time a loaded recording is found to overlap an
/// archived value that can disturb reception.
pub const LOAD_WARNING: &str = "Space weather or interference may have affected this recording";

/// Share of aircraft, in percent, at or above which a cell-day a track
/// crossed counts. It is the share gpsjam starts colouring its cells at.
const INTERFERENCE_TRIGGER_PERCENT: f32 = gt_ui_theme::INTERFERENCE_LOW_BREAKPOINT * 100.0;

/// Leads the geomagnetic line. The metric is named "geomagnetic activity"
/// where any value is shown. Only storm-level values reach a line here.
const GEOMAGNETIC_STORM: &str = "Geomagnetic storm";

/// Leads the solar flare line, which states one flare.
const SOLAR_FLARE: &str = "Solar flare";

/// Leads the TEC line, which states a range of values.
const TEC_OVER_THE_RECORDING: &str = "TEC over the recording";

/// Leads the TEC deviation line, which states one share of the quiet median
/// and the storm grade it reaches.
const TEC_DEVIATION: &str = "TEC deviation";

/// Closes the solar flare line. Only a flare that peaked while the receiver
/// was in daylight counts, so every flare line ends this way.
const RECEIVER_ON_THE_SUNLIT_SIDE: &str = "receiver on the sunlit side";

/// How the flare line writes a peak instant, which the catalog publishes to
/// the minute.
const FLARE_PEAK_FORMAT: &str = "%Y-%m-%d %H:%M";

/// The weakest flare classification the NOAA radio blackout scale covers, as
/// the catalog writes it. The scale's own floor is a peak flux rather than a
/// classification, so the test `the_stated_flare_class_is_the_first_blackout_level`
/// pins this string against the scale.
const FLARE_TRIGGER_CLASSIFICATION: &str = "M1";

/// One row per environment metric, stating the level at which it raises a
/// warning, listed by the popup behind the map's warning icon.
pub static WARNING_LEVELS: LazyLock<Vec<WarningLevelExplanation>> = LazyLock::new(|| {
    vec![
        WarningLevelExplanation {
            trigger: format!(
                "{}: {INTERFERENCE_TRIGGER_PERCENT:.0} % or more of aircraft in a crossed cell \
                 reported low navigation accuracy (gpsjam.org's own yellow level).",
                gt_jam::text::LAYER_LABEL
            ),
            reference: gt_jam::reference::AIRCRAFT_INTERFERENCE,
        },
        WarningLevelExplanation {
            trigger: format!(
                "Geomagnetic activity: {} or {} at {} or higher, NOAA's {} storm level.",
                GeomagneticIndex::Kp,
                GeomagneticIndex::Hp30,
                GeomagneticStormClass::Minor.lowest_value(),
                GeomagneticStormClass::Minor.scale_name()
            ),
            reference: gt_solar::reference::GEOMAGNETIC_ACTIVITY,
        },
        WarningLevelExplanation {
            trigger: format!(
                "Solar flares: class {FLARE_TRIGGER_CLASSIFICATION} or stronger, NOAA's {} level, \
                 while the receiver was on the sunlit side.",
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

/// The peak archived value of each environment metric over one or more
/// recordings, kept only where it reaches the level that can disturb
/// reception.
///
/// Aircraft interference, geomagnetic activity, solar flares and the TEC
/// deviation each have a published level above which reception is known to
/// suffer, and those four levels are what raises the warning. The absolute
/// TEC range has no such level, so it is kept as context beside a warning
/// another metric raised and never raises one.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DisturbanceEvidence {
    aircraft_interference: Option<f64>,
    geomagnetic: Option<GeomagneticStormPeak>,
    solar_flare: Option<SunlitFlarePeak>,
    tec_deviation: Option<QuietTimeDeviation>,
    total_electron_content: Option<TecSpan>,
}

impl DisturbanceEvidence {
    /// The evidence over one recording: its fixes' own values, and the
    /// flares that peaked while it was recording.
    fn of(series: &RecordingSeries<'_>, flares: &[MarkedFlare]) -> Self {
        let mut evidence = Self::default();
        for points in &series.interference {
            evidence.collect_interference_from(points);
        }
        for points in &series.geomagnetic {
            evidence.collect_geomagnetic_from(points);
        }
        for points in &series.tec {
            evidence.collect_tec_from(points);
        }
        for deviation in &series.tec_deviations {
            evidence.keep_stronger_tec_deviation(Some(*deviation));
        }
        evidence.collect_flares_from(flares);
        evidence
    }

    /// Whether a metric reached its disturbance level. The TEC range alone
    /// never makes this true.
    pub fn warns(&self) -> bool {
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
    /// Empty while no metric reached its level, which is what keeps the
    /// indicator off the map.
    pub fn warning_lines(&self) -> Vec<String> {
        if !self.warns() {
            return Vec::new();
        }
        let Self {
            aircraft_interference,
            geomagnetic,
            solar_flare,
            tec_deviation,
            total_electron_content,
        } = self;
        let mut lines = Vec::new();
        if let Some(peak) = *geomagnetic {
            lines.push(peak.warning_line());
        }
        if let Some(percent) = *aircraft_interference {
            lines.push(format!(
                "{}: up to {percent:.1} % of aircraft in a crossed cell",
                gt_jam::text::LAYER_LABEL
            ));
        }
        if let Some(peak) = *solar_flare {
            lines.push(peak.warning_line());
        }
        if let Some(deviation) = *tec_deviation {
            lines.push(format!(
                "{TEC_DEVIATION}: {:+.0} % from the {}-day median, {} (W = {})",
                deviation.percent_from_median(),
                quiet_time::BACKGROUND_WINDOW_DAYS,
                deviation.grade(),
                deviation.storm_index_value()
            ));
        }
        if let Some(span) = *total_electron_content {
            lines.push(span.context_line());
        }
        lines
    }

    /// Fold `other`'s peaks into these, so one set of lines states the
    /// strongest evidence over every loaded recording.
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

    /// Keep the strongest of `flares` that counts. They are the ones peaking
    /// over the recording, as the archive read produced them.
    fn collect_flares_from(&mut self, flares: &[MarkedFlare]) {
        for marked in flares {
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
    fn keep_stronger_tec_deviation(&mut self, candidate: Option<QuietTimeDeviation>) {
        if let Some(deviation) = candidate.filter(|deviation| deviation.grade().is_a_storm())
            && self
                .tec_deviation
                .is_none_or(|kept| deviation.log_ratio().abs() > kept.log_ratio().abs())
        {
            self.tec_deviation = Some(deviation);
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

    fn warning_line(self) -> String {
        format!(
            "{GEOMAGNETIC_STORM}: {} reached {} ({})",
            self.index,
            self.activity,
            self.storm.scale_name()
        )
    }
}

/// A flare that peaked while the receiver was in daylight and reaches the
/// NOAA radio blackout scale, whose first level starts at class M1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SunlitFlarePeak {
    classification: FlareClassification,
    peak: DateTime<Utc>,
}

impl SunlitFlarePeak {
    /// The peak, or [`None`] for a flare below the blackout scale or one the
    /// receiver spent on the night side, where the ionization it raises never
    /// reached the signal path.
    fn of(marked: &MarkedFlare) -> Option<Self> {
        let sunlit = marked.receiver_side == Some(SunlitSide::Sunlit);
        let reaches_the_blackout_scale =
            marked.flare.classification.radio_blackout_class().is_some();
        (sunlit && reaches_the_blackout_scale).then_some(Self {
            classification: marked.flare.classification,
            peak: marked.flare.peak,
        })
    }

    fn warning_line(self) -> String {
        format!(
            "{SOLAR_FLARE}: {} at {} UTC, {RECEIVER_ON_THE_SUNLIT_SIDE}",
            self.classification,
            self.peak.format(FLARE_PEAK_FORMAT)
        )
    }
}

/// The lowest and highest TEC value a recording's fixes carry, in TEC units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TecSpan {
    lowest: f64,
    highest: f64,
}

impl TecSpan {
    fn context_line(self) -> String {
        format!(
            "{TEC_OVER_THE_RECORDING}: {:.0} to {:.0} {}",
            self.lowest,
            self.highest,
            gt_ionex::text::LEGEND_UNIT
        )
    }
}

/// The per-fix series of one recording's tracks, borrowed from the ones the
/// plot draws.
pub struct RecordingSeries<'a> {
    interference: Vec<&'a Arc<Vec<JammingPoint>>>,
    geomagnetic: Vec<&'a Arc<Vec<GeomagneticPoint>>>,
    tec: Vec<&'a Arc<Vec<TecPoint>>>,
    /// The peak deviation of each track that has one. It is read from days
    /// the track itself never spans, so it is kept as its own field.
    tec_deviations: Vec<QuietTimeDeviation>,
}

impl<'a> RecordingSeries<'a> {
    /// The series of `tracks`. A track whose day the archive does not hold is
    /// absent from a series and contributes nothing.
    pub fn of(
        tracks: impl Iterator<Item = TrackRef> + Clone,
        jamming: &'a JammingSeries,
        geomagnetic: &'a GeomagneticSeries,
        tec: &'a TecSeries,
        tec_deviations: &HashMap<TrackRef, QuietTimeDeviation>,
    ) -> Self {
        Self {
            interference: tracks
                .clone()
                .filter_map(|track| jamming.points_by_track.get(&track))
                .collect(),
            geomagnetic: tracks
                .clone()
                .filter_map(|track| geomagnetic.points_by_track.get(&track))
                .collect(),
            tec: tracks
                .clone()
                .filter_map(|track| tec.points_by_track.get(&track))
                .collect(),
            tec_deviations: tracks
                .filter_map(|track| tec_deviations.get(&track).copied())
                .collect(),
        }
    }

    /// The allocations the values were read from. A fetch worker archiving a
    /// day replaces the ones its day reaches, which is what re-assesses the
    /// recording.
    fn identities(&self) -> Vec<ArcIdentity> {
        let interference = self.interference.iter().copied().map(ArcIdentity::of);
        let geomagnetic = self.geomagnetic.iter().copied().map(ArcIdentity::of);
        let tec = self.tec.iter().copied().map(ArcIdentity::of);
        interference.chain(geomagnetic).chain(tec).collect()
    }
}

/// One loaded recording as it enters the assessment.
pub struct RecordingUnderAssessment<'a> {
    pub id: LoadedFileId,
    /// The span its tracks cover, which its flares are read over.
    pub span: TimeRange,
    pub series: RecordingSeries<'a>,
    /// The archived days a flare peaking inside [`Self::span`] can be filed
    /// under.
    pub archived_flare_days: Vec<NaiveDate>,
    /// Identity of the fix timeline the receiver's side of Earth is read at.
    pub positions: ArcIdentity,
}

/// What one recording's evidence was read from: it is read again exactly when
/// this changes.
#[derive(Debug, Clone, PartialEq)]
struct EvidenceSource {
    series: Vec<ArcIdentity>,
    /// Carried by value: a deviation is read from days outside the track's
    /// own, so no allocation the track holds changes when one moves.
    tec_deviations: Vec<QuietTimeDeviation>,
    archived_flare_days: Vec<NaiveDate>,
    positions: ArcIdentity,
}

impl EvidenceSource {
    fn of(recording: &RecordingUnderAssessment<'_>) -> Self {
        Self {
            series: recording.series.identities(),
            tec_deviations: recording.series.tec_deviations.clone(),
            archived_flare_days: recording.archived_flare_days.clone(),
            positions: recording.positions,
        }
    }
}

struct AssessedRecording {
    resolved_from: EvidenceSource,
    evidence: DisturbanceEvidence,
}

/// What the loaded recordings' archived environment values warn about: the
/// lines the map indicator lists, and which recordings the load toast has
/// already been raised for.
#[derive(Default)]
pub struct SpaceWeatherWarning {
    assessed: HashMap<LoadedFileId, AssessedRecording>,
    /// Recordings the toast has been shown for, so a day archived later never
    /// repeats it.
    toasted: HashSet<LoadedFileId>,
    lines: Vec<String>,
}

impl SpaceWeatherWarning {
    /// Read the evidence of every recording the archives moved under, and
    /// report how many of them warn for the first time - one load toast each.
    ///
    /// `flares_peaking_in` reads the archive, so it runs only for a recording
    /// being assessed again.
    pub fn reassess(
        &mut self,
        recordings: &[RecordingUnderAssessment<'_>],
        mut flares_peaking_in: impl FnMut(TimeRange) -> Vec<MarkedFlare>,
    ) -> usize {
        let mut changed = false;
        for recording in recordings {
            let resolved_from = EvidenceSource::of(recording);
            if self
                .assessed
                .get(&recording.id)
                .is_some_and(|assessed| assessed.resolved_from == resolved_from)
            {
                continue;
            }
            let evidence =
                DisturbanceEvidence::of(&recording.series, &flares_peaking_in(recording.span));
            self.assessed.insert(
                recording.id,
                AssessedRecording {
                    resolved_from,
                    evidence,
                },
            );
            changed = true;
        }

        let loaded: HashSet<LoadedFileId> = recordings.iter().map(|r| r.id).collect();
        let assessed_before = self.assessed.len();
        self.assessed.retain(|id, _| loaded.contains(id));
        self.toasted.retain(|id| loaded.contains(id));
        if changed || self.assessed.len() != assessed_before {
            self.rebuild_lines();
        }

        let newly_warned: Vec<LoadedFileId> = self
            .assessed
            .iter()
            .filter(|(id, assessed)| assessed.evidence.warns() && !self.toasted.contains(id))
            .map(|(id, _)| *id)
            .collect();
        self.toasted.extend(&newly_warned);
        newly_warned.len()
    }

    /// One line per metric that reached its disturbance level over any loaded
    /// recording, empty while none did.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    fn rebuild_lines(&mut self) {
        let mut evidence = DisturbanceEvidence::default();
        for assessed in self.assessed.values() {
            evidence.merge(&assessed.evidence);
        }
        self.lines = evidence.warning_lines();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gt_flare::SolarFlare;
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

    /// The per-fix series of one recording's single track, owning what
    /// [`RecordingSeries`] borrows.
    #[derive(Default)]
    struct SeriesFixture {
        interference: Vec<Arc<Vec<JammingPoint>>>,
        geomagnetic: Vec<Arc<Vec<GeomagneticPoint>>>,
        tec: Vec<Arc<Vec<TecPoint>>>,
        tec_deviations: Vec<QuietTimeDeviation>,
    }

    impl SeriesFixture {
        fn series(&self) -> RecordingSeries<'_> {
            RecordingSeries {
                interference: self.interference.iter().collect(),
                geomagnetic: self.geomagnetic.iter().collect(),
                tec: self.tec.iter().collect(),
                tec_deviations: self.tec_deviations.clone(),
            }
        }
    }

    /// The deviation a track reaches at `percent` of a fully archived quiet
    /// window's median, built the way the archive read builds it.
    fn tec_deviation(percent: f64) -> QuietTimeDeviation {
        let median = 20.0;
        let window =
            vec![TotalElectronContent::from_tecu(median); quiet_time::BACKGROUND_WINDOW_DAYS];
        quiet_time::deviation_from_quiet_time(
            TotalElectronContent::from_tecu(median * (1.0 + percent / 100.0)),
            &window,
        )
        .expect("a fully archived window")
    }

    fn interference_points(percents: &[f64]) -> Vec<Arc<Vec<JammingPoint>>> {
        vec![Arc::new(
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
        )]
    }

    fn geomagnetic_points(hp30: Option<f64>, kp: Option<f64>) -> Vec<Arc<Vec<GeomagneticPoint>>> {
        vec![Arc::new(vec![GeomagneticPoint {
            x_secs: 0.0,
            hp30,
            kp,
        }])]
    }

    fn tec_points(values: &[f64]) -> Vec<Arc<Vec<TecPoint>>> {
        vec![Arc::new(
            values
                .iter()
                .enumerate()
                .map(|(index, &tecu)| TecPoint {
                    x_secs: index as f64,
                    tecu: Some(tecu),
                })
                .collect(),
        )]
    }

    fn flare(class_type: &str, receiver_side: Option<SunlitSide>) -> MarkedFlare {
        MarkedFlare {
            flare: SolarFlare {
                id: format!("{class_type}-FLR-001"),
                begin: at(2, 0),
                peak: at(2, 1),
                end: None,
                classification: class_type.parse().expect("a published class"),
                source_location: None,
                active_region: None,
            },
            receiver_side,
        }
    }

    fn evidence_of(fixture: &SeriesFixture, flares: &[MarkedFlare]) -> DisturbanceEvidence {
        DisturbanceEvidence::of(&fixture.series(), flares)
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
                    event_marker_styles: HashMap::new(),
                    orphaned_event_markers: Vec::new(),
                    load_warnings: Vec::new(),
                    source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
                },
                FileHistory::None,
            );
        }
        files.view().entries().map(|entry| entry.id()).collect()
    }

    fn recording<'a>(
        id: LoadedFileId,
        series: &'a SeriesFixture,
        archived_flare_days: &[NaiveDate],
        positions: &Arc<u8>,
    ) -> RecordingUnderAssessment<'a> {
        RecordingUnderAssessment {
            id,
            span: TimeRange::new(at(0, 0), at(6, 0)),
            series: series.series(),
            archived_flare_days: archived_flare_days.to_vec(),
            positions: ArcIdentity::of(positions),
        }
    }

    fn no_flares(_span: TimeRange) -> Vec<MarkedFlare> {
        Vec::new()
    }

    /// A crossed cell-day below the trigger is background, one at it is not.
    /// The boundary is the 2 % share gpsjam starts colouring its cells at.
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
            ["Geomagnetic storm: Hp30 reached 7.667 (G3)"]
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

    /// Every line of a fully disturbed recording, in the order the hover
    /// lists them, each stating the peak that produced it.
    #[test]
    fn every_metric_that_warns_states_its_peak() {
        let fixture = SeriesFixture {
            interference: interference_points(&[0.4, 34.2]),
            geomagnetic: geomagnetic_points(Some(7.667), Some(9.0)),
            tec: tec_points(&[12.4, 175.2]),
            tec_deviations: vec![tec_deviation(62.0)],
        };
        let flares = [
            flare("M1.2", Some(SunlitSide::Sunlit)),
            flare("X5.8", Some(SunlitSide::Sunlit)),
        ];

        assert_eq!(
            evidence_of(&fixture, &flares).warning_lines(),
            [
                "Geomagnetic storm: Kp reached 9 (G5)",
                "Aircraft interference: up to 34.2 % of aircraft in a crossed cell",
                "Solar flare: X5.8 at 2024-05-11 02:01 UTC, receiver on the sunlit side",
                "TEC deviation: +62 % from the 27-day median, moderate ionospheric storm (W = 3)",
                "TEC over the recording: 12 to 175 TECU",
            ]
        );
    }

    /// A deviation the index grades below a storm is quiet-time variation,
    /// and is left out of the evidence entirely.
    #[test]
    fn a_deviation_short_of_the_storm_grade_warns_about_nothing() {
        let fixture = SeriesFixture {
            tec_deviations: vec![tec_deviation(42.5)],
            ..SeriesFixture::default()
        };

        let evidence = evidence_of(&fixture, &[]);

        assert!(!evidence.warns());
        assert!(evidence.warning_lines().is_empty());
    }

    /// The line states the signed share of the median and the grade the index
    /// puts it at, and a deviation either side of the median raises the
    /// warning.
    #[rstest]
    #[case::a_moderate_storm(
        62.0,
        "TEC deviation: +62 % from the 27-day median, moderate ionospheric storm (W = 3)"
    )]
    #[case::a_negative_moderate_storm(
        -35.0,
        "TEC deviation: -35 % from the 27-day median, moderate ionospheric storm (W = -3)"
    )]
    #[case::an_intense_storm(
        200.0,
        "TEC deviation: +200 % from the 27-day median, intense ionospheric storm (W = 4)"
    )]
    fn the_deviation_line_states_its_share_and_grade(#[case] percent: f64, #[case] expected: &str) {
        let fixture = SeriesFixture {
            tec_deviations: vec![tec_deviation(percent)],
            ..SeriesFixture::default()
        };

        assert_eq!(evidence_of(&fixture, &[]).warning_lines(), [expected]);
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

        assert_eq!(
            warning.reassess(&[recording(id, &quiet, &[], &positions)], no_flares),
            0
        );
        assert!(warning.lines().is_empty());

        let stormy = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), None),
            ..SeriesFixture::default()
        };
        assert_eq!(
            warning.reassess(&[recording(id, &stormy, &[], &positions)], no_flares),
            1,
            "the archived day reached the loaded recording"
        );
        assert_eq!(
            warning.lines(),
            ["Geomagnetic storm: Hp30 reached 7.667 (G3)"]
        );

        let stormier = SeriesFixture {
            geomagnetic: geomagnetic_points(Some(7.667), Some(9.0)),
            ..SeriesFixture::default()
        };
        assert_eq!(
            warning.reassess(&[recording(id, &stormier, &[], &positions)], no_flares),
            0,
            "a second archived day does not toast the same recording again"
        );
    }

    /// Every disturbed recording is toasted, and only the disturbed ones.
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
            .map(|(index, &id)| recording(id, series(index), &[], &positions))
            .collect();

        assert_eq!(warning.reassess(&recordings, no_flares), 2);
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
            &[recording(id, &quiet, &[day(11)], &positions)],
            &mut count_read,
        );
        warning.reassess(
            &[recording(id, &quiet, &[day(11)], &positions)],
            &mut count_read,
        );
        warning.reassess(
            &[recording(id, &quiet, &[day(11), day(12)], &positions)],
            &mut count_read,
        );

        assert_eq!(reads, 2);
    }

    /// A recording that is closed takes its evidence out of the map's lines.
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

        warning.reassess(&[recording(id, &stormy, &[], &positions)], no_flares);
        assert!(!warning.lines().is_empty());

        warning.reassess(&[], no_flares);
        assert!(warning.lines().is_empty());
    }
}
