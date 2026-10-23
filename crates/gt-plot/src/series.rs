use chrono::{DateTime, TimeDelta, Utc};
use gt_analysis::clock_offset::ClockOffsetPlacement;
use gt_analysis::satellite_utilization::UtilAnomaly;
use gt_analysis::{SlipAnalysisConfig, SlipThresholds};
use gt_egui_mipmap::{MipMap, WrapPeriod};
use gt_types::satellites::{Constellation, ConstellationSet};
use gt_types::{
    LoadedFile, MetricKind, NavPoint, NavigationEvidenceSource, PositionProvenance,
    SatelliteQueryScope, SatelliteScope,
};
use rustc_hash::FxHashMap;
use strum::IntoEnumIterator as _;
use uom::si::angle::degree;

use crate::AnalysisConfig;
use crate::metric_series::{MetricSampleSource, MetricSeries, TimedMetricSample};

/// A track's mipmap series together with the index of the file it belongs to,
/// which the plot learns only once that file joins the loaded-files list.
#[derive(Debug, Clone)]
pub(crate) struct PlacedTrackSeries {
    pub fi: usize,
    pub series: TrackSeries,
}

impl PlacedTrackSeries {
    pub(crate) fn track_ref(&self) -> gt_types::TrackRef {
        gt_types::TrackRef::new(
            gt_types::FileIdx::new(self.fi),
            gt_types::TrackIdx::new(self.series.ti),
        )
    }
}

/// Mipmap series for a single track.
///
/// Built from all points in the track regardless of current visibility or filter.
/// Visibility and time-range clamping are applied at render time in
/// [`super::plot_widget`] so the cache stays valid across filter changes.
#[derive(Debug, Clone)]
pub(crate) struct TrackSeries {
    /// Track index within its file.
    pub ti: usize,
    pub metrics: FxHashMap<MetricKind, MetricSeries>,
    /// `(x_min, x_max)` in Unix seconds, or `None` when the track has no
    /// points.
    pub x_range: Option<(f64, f64)>,
    /// Constellations that appear at least once in this track's satellite
    /// reports.  The plot uses the union across tracks to select which
    /// per-constellation chips and lines to show, so a constellation with no
    /// data never clutters the UI.
    pub present: ConstellationSet,
    pub velocity_kmh: MipMap,
    pub eph_m: MipMap,
    pub heading_deg: MipMap,
    /// GPS-clock lead over the host system clock, in milliseconds.
    /// Positive = GPS clock ahead, negative = system clock ahead.
    /// Only present when the TPV record carries a system timestamp.
    ///
    /// Excludes the samples [`Self::clock_offset_placement`] holds back: an
    /// offset the shared y-axis cannot show would otherwise set its
    /// auto-bounds, flattening every other metric.  Those samples are drawn by
    /// the off-scale indicator instead, never dropped.
    pub clock_delta_ms: MipMap,
    /// What is kept off [`Self::clock_delta_ms`] and marked on its own: the
    /// departures from this track's baseline that returned to it, or every
    /// sample of a track whose baseline the shared y-axis cannot show.
    /// Threshold-dependent: recomputed by [`TrackSeries::apply_analysis`].
    pub clock_offset_placement: ClockOffsetPlacement,
    /// Observation epochs with known usage below the elevation mask.
    pub util_anomalies: Vec<UtilAnomaly>,
    pub channels: Vec<ChannelSeries>,
}

/// One ad-hoc channel's plot series: a line per component, on the channel's
/// own sample clock (channels are correlated with the track by time, not
/// resampled onto the fixes).
#[derive(Debug, Clone)]
pub(crate) struct ChannelSeries {
    pub name: String,
    /// The producer's unit label (`g`, `deg`), shown in the chip and the
    /// line names. Purely presentational here - the plot draws raw values.
    pub unit: Option<String>,
    /// One line per component, in channel order. A scalar channel is a
    /// single component labelled by the channel name alone.
    pub components: Vec<ChannelComponentSeries>,
    /// The channel's backward time steps, ascending by the x each step is
    /// marked at.
    pub backward_time_steps: Vec<PlacedBackwardTimeStep>,
}

/// One backward time step of a channel: the two timestamps its hover text
/// reads, and the plot x the overlay marks it at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedBackwardTimeStep {
    /// Timestamp of the sample stored before the stepping one.
    pub previous_time: DateTime<Utc>,
    /// Timestamp of the stepping sample.
    pub time: DateTime<Utc>,
    /// [`Self::time`] in Unix seconds.
    pub x_secs: f64,
}

impl PlacedBackwardTimeStep {
    /// How far the clock stepped back, a positive duration.
    pub fn step_back(self) -> TimeDelta {
        self.previous_time - self.time
    }
}

/// One channel component's line: its display label and its mipmapped samples,
/// one entry per chronological run of the channel.
#[derive(Debug, Clone)]
pub(crate) struct ChannelComponentSeries {
    /// `accel.x` for a vector component, `incline` for a scalar channel.
    pub label: String,
    /// The runs of [`gt_types::Channel::chronological_runs`], in stored order,
    /// each mipmapped on its own: a mipmap level is searched by a binary search
    /// over x.
    pub runs: Vec<MipMap>,
}

/// Build the per-component plot series of one channel.
fn build_channel_series(channel: &gt_types::Channel) -> ChannelSeries {
    let columns = channel.component_count();
    let period = channel
        .period
        .and_then(|period| WrapPeriod::new(period.get::<degree>()));
    let times: Vec<f64> = channel
        .times
        .iter()
        .map(|t| t.timestamp_micros() as f64 / MICROS_PER_SEC)
        .collect();
    let runs = channel.chronological_runs();
    let components = (0..columns)
        .map(|col| {
            let label = match channel.components.get(col) {
                Some(component) => format!("{}.{component}", channel.name),
                None => channel.name.clone(),
            };
            let runs = runs
                .iter()
                .map(|run| {
                    let pts: Vec<[f64; 2]> = times
                        .get(run.clone())
                        .unwrap_or_default()
                        .iter()
                        .zip(channel.values.chunks(columns).skip(run.start))
                        .filter_map(|(&t, row)| row.get(col).map(|&v| [t, v]))
                        .collect();
                    match period {
                        Some(period) => MipMap::build_wrapping(pts, period),
                        None => MipMap::build(pts),
                    }
                })
                .collect();
            ChannelComponentSeries { label, runs }
        })
        .collect();
    let mut backward_time_steps: Vec<PlacedBackwardTimeStep> = channel
        .backward_time_steps()
        .into_iter()
        .filter_map(|step| {
            let time = *channel.times.get(step.position)?;
            Some(PlacedBackwardTimeStep {
                previous_time: time + step.step_back,
                time,
                x_secs: *times.get(step.position)?,
            })
        })
        .collect();
    backward_time_steps.sort_by(|a, b| a.x_secs.total_cmp(&b.x_secs));
    ChannelSeries {
        name: channel.name.clone(),
        unit: channel.unit.as_ref().map(ToString::to_string),
        components,
        backward_time_steps,
    }
}

impl TrackSeries {
    /// Recompute only the analysis-dependent series (utilization rate +
    /// anomalies and slip rate) for `track` under `analysis`, leaving the
    /// analysis-independent mipmaps intact.
    pub(crate) fn apply_analysis(
        &mut self,
        track: &gt_types::LoadedTrack,
        analysis: AnalysisConfig,
    ) {
        let diagnostics = build_diagnostic_series(&track.points, analysis);
        self.util_anomalies = diagnostics.anomalies;
        self.metrics
            .retain(|kind, _| !diagnostics.metrics.contains_key(kind));
        self.metrics.extend(diagnostics.metrics);

        let (clock_delta_pts, placement) =
            clock_delta_series(track, analysis.clock_excursion_threshold_s);
        self.clock_delta_ms = MipMap::build(clock_delta_pts);
        self.clock_offset_placement = placement;
    }
}

/// The clock offset line points and what is held back from them.
///
/// The plot marks each held-back sample at the edge of the view, with its true
/// offset on hover.  A track whose baseline the shared y-axis cannot show has
/// no line at all: every one of its samples is held back.
fn clock_delta_series(
    track: &gt_types::LoadedTrack,
    threshold_s: f32,
) -> (Vec<[f64; 2]>, ClockOffsetPlacement) {
    let placement = gt_analysis::clock_offset::detect_placement(&track.points, threshold_s);
    let ClockOffsetPlacement::BaselineOnScale(excursions) = &placement else {
        return (Vec::new(), placement);
    };
    let excluded = gt_analysis::clock_offset::excursion_indices(excursions);
    let points = track
        .points
        .iter()
        .enumerate()
        .filter(|(i, _)| excluded.binary_search(i).is_err())
        .filter_map(|(_, point)| {
            let delta_ms = point.tpv.gps_system_clock_offset()?.num_milliseconds();
            Some([point.tpv.time().as_secs_f64(), delta_ms as f64])
        })
        .collect();
    (points, placement)
}

/// Build mipmap series for every track in a single file, using `fi` as the file
/// index (for cache keying).
///
/// No visibility check or time filter is applied - that is done at render time
/// so the cache stays valid across filter changes without a rebuild.
pub(crate) fn build_file_series(file: &LoadedFile, analysis: AnalysisConfig) -> Vec<TrackSeries> {
    file.tracks
        .iter()
        .enumerate()
        .map(|(ti, track)| build_track_series(ti, track, analysis))
        .collect()
}

/// Build mipmap series for every track in every file.
pub(crate) fn build_all_series(
    files: &[LoadedFile],
    analysis: AnalysisConfig,
) -> Vec<PlacedTrackSeries> {
    files
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            build_file_series(file, analysis)
                .into_iter()
                .map(move |series| PlacedTrackSeries { fi, series })
        })
        .collect()
}

fn build_track_series(
    ti: usize,
    track: &gt_types::LoadedTrack,
    analysis: AnalysisConfig,
) -> TrackSeries {
    let mut present = ConstellationSet::empty();
    for point in &track.points {
        for constellation in point
            .satellite_evidence
            .usage
            .iter()
            .flat_map(|report| &report.satellites)
            .map(|satellite| satellite.identity.constellation)
            .chain(
                point
                    .satellite_evidence
                    .observations
                    .iter()
                    .flat_map(|report| &report.satellites)
                    .map(|satellite| satellite.identity.constellation),
            )
        {
            present.insert(constellation);
        }
        for scope in point
            .satellite_evidence
            .usage
            .iter()
            .map(|report| report.metadata.scope)
            .chain(
                point
                    .satellite_evidence
                    .observations
                    .iter()
                    .map(|report| report.metadata.scope),
            )
        {
            if let SatelliteScope::Constellations(set) = scope {
                for constellation in
                    Constellation::iter().filter(|constellation| set.contains(*constellation))
                {
                    present.insert(constellation);
                }
            }
        }
    }
    let mut metrics = build_count_series(&track.points);
    let diagnostics = build_diagnostic_series(&track.points, analysis);
    metrics.extend(diagnostics.metrics);
    for kind in [MetricKind::Hdop, MetricKind::Vdop, MetricKind::Pdop] {
        let samples = track.points.iter().enumerate().map(|(point_index, point)| {
            let time = point.tpv.time();
            let value = match kind {
                MetricKind::Hdop => point.navigation.hdop,
                MetricKind::Vdop => point.navigation.vdop,
                _ => point.navigation.pdop,
            };
            if value.is_none()
                && point.navigation.position_provenance == Some(PositionProvenance::SdkSynthesized)
            {
                return Vec::new();
            }
            vec![TimedMetricSample {
                time,
                value,
                source: MetricSampleSource {
                    point_index,
                    evidence: None,
                },
                fix_time: time,
                report: None,
            }]
        });
        metrics.insert(kind, build_metric_runs(&track.points, samples));
    }
    let mut velocity_kmh_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut eph_m_pts: Vec<[f64; 2]> = Vec::new();
    let mut heading_deg_pts: Vec<[f64; 2]> = Vec::new();

    for point in &track.points {
        let t = point.tpv.time().as_secs_f64_with_subseconds();

        if let Some(v) = point.tpv.velocity_kmh() {
            velocity_kmh_pts.push([t, v]);
        }

        if let Some(eph) = point.tpv.eph_m() {
            eph_m_pts.push([t, eph as f64]);
        }

        if let Some(h) = point.tpv.heading() {
            heading_deg_pts.push([t, h.get::<degree>()]);
        }
    }

    let (clock_delta_ms_pts, clock_offset_placement) =
        clock_delta_series(track, analysis.clock_excursion_threshold_s);

    let x_range = track
        .points
        .iter()
        .flat_map(|point| {
            std::iter::once(point.tpv.time())
                .chain(
                    point
                        .satellite_evidence
                        .usage
                        .iter()
                        .map(|report| report.metadata.normalized_time),
                )
                .chain(
                    point
                        .satellite_evidence
                        .observations
                        .iter()
                        .map(|report| report.metadata.normalized_time),
                )
        })
        .map(|time| time.as_secs_f64_with_subseconds())
        .fold(None, |range, time| {
            Some(range.map_or((time, time), |(lo, hi): (f64, f64)| {
                (lo.min(time), hi.max(time))
            }))
        });

    TrackSeries {
        ti,
        metrics,
        x_range,
        present,
        velocity_kmh: MipMap::build(velocity_kmh_pts),
        eph_m: MipMap::build(eph_m_pts),
        heading_deg: MipMap::build_wrapping(heading_deg_pts, WrapPeriod::full_turn_degrees()),
        clock_delta_ms: MipMap::build(clock_delta_ms_pts),
        clock_offset_placement,
        util_anomalies: diagnostics.anomalies,
        channels: track.channels.iter().map(build_channel_series).collect(),
    }
}

#[derive(Clone, Copy)]
enum CountQuantity {
    Used,
    Seen,
}

#[derive(Clone, Copy)]
struct CountMetric {
    scope: SatelliteQueryScope,
    quantity: CountQuantity,
}

impl CountMetric {
    fn from_kind(kind: MetricKind) -> Option<Self> {
        let (scope, quantity) = match kind {
            MetricKind::SatsFix => (SatelliteQueryScope::Global, CountQuantity::Used),
            MetricKind::SatsSeen => (SatelliteQueryScope::Global, CountQuantity::Seen),
            MetricKind::GpsFix => (
                SatelliteQueryScope::Constellation(Constellation::Gps),
                CountQuantity::Used,
            ),
            MetricKind::GpsSeen => (
                SatelliteQueryScope::Constellation(Constellation::Gps),
                CountQuantity::Seen,
            ),
            MetricKind::GlonassFix => (
                SatelliteQueryScope::Constellation(Constellation::Glonass),
                CountQuantity::Used,
            ),
            MetricKind::GlonassSeen => (
                SatelliteQueryScope::Constellation(Constellation::Glonass),
                CountQuantity::Seen,
            ),
            MetricKind::GalileoFix => (
                SatelliteQueryScope::Constellation(Constellation::Galileo),
                CountQuantity::Used,
            ),
            MetricKind::GalileoSeen => (
                SatelliteQueryScope::Constellation(Constellation::Galileo),
                CountQuantity::Seen,
            ),
            MetricKind::BeidouFix => (
                SatelliteQueryScope::Constellation(Constellation::Beidou),
                CountQuantity::Used,
            ),
            MetricKind::BeidouSeen => (
                SatelliteQueryScope::Constellation(Constellation::Beidou),
                CountQuantity::Seen,
            ),
            MetricKind::NavicFix => (
                SatelliteQueryScope::Constellation(Constellation::Navic),
                CountQuantity::Used,
            ),
            MetricKind::NavicSeen => (
                SatelliteQueryScope::Constellation(Constellation::Navic),
                CountQuantity::Seen,
            ),
            MetricKind::QzssFix => (
                SatelliteQueryScope::Constellation(Constellation::Qzss),
                CountQuantity::Used,
            ),
            MetricKind::QzssSeen => (
                SatelliteQueryScope::Constellation(Constellation::Qzss),
                CountQuantity::Seen,
            ),
            _ => return None,
        };
        Some(Self { scope, quantity })
    }

    fn point_samples(self, point_index: usize, point: &NavPoint) -> Vec<TimedMetricSample> {
        let mut samples = Vec::new();
        let resolved = match self.quantity {
            CountQuantity::Used => point.resolved_used_count(self.scope),
            CountQuantity::Seen => point.resolved_seen_count(self.scope),
        };
        let navigation_count =
            resolved.filter(|count| count.evidence == NavigationEvidenceSource::NavigationCount);
        if let Some(count) = navigation_count {
            samples.push(TimedMetricSample {
                time: point.tpv.time(),
                fix_time: point.tpv.time(),
                report: None,
                value: Some(f64::from(count.count)),
                source: MetricSampleSource {
                    point_index,
                    evidence: Some(count.evidence),
                },
            });
        } else if resolved.is_none()
            && point.navigation.position_provenance != Some(PositionProvenance::SdkSynthesized)
        {
            samples.push(TimedMetricSample {
                time: point.tpv.time(),
                fix_time: point.tpv.time(),
                report: None,
                value: None,
                source: MetricSampleSource {
                    point_index,
                    evidence: Some(NavigationEvidenceSource::NavigationCount),
                },
            });
        }
        match self.quantity {
            CountQuantity::Used => {
                for (index, report) in point.satellite_evidence.usage.iter().enumerate() {
                    if !report.metadata.scope.covers(self.scope)
                        && self.scope != SatelliteQueryScope::Global
                    {
                        continue;
                    }
                    let count = report.resolved_used_count(self.scope);
                    samples.push(TimedMetricSample {
                        time: report.metadata.normalized_time,
                        fix_time: point.tpv.time(),
                        report: Some(report.metadata),
                        value: count.map(|count| f64::from(count.count)),
                        source: MetricSampleSource {
                            point_index,
                            evidence: Some(if count.is_some_and(|count| count.is_aggregate) {
                                NavigationEvidenceSource::UsageAggregate(index)
                            } else {
                                NavigationEvidenceSource::UsageList(index)
                            }),
                        },
                    });
                }
            }
            CountQuantity::Seen => {
                for (index, report) in point.satellite_evidence.observations.iter().enumerate() {
                    if !report.metadata.scope.covers(self.scope)
                        && self.scope != SatelliteQueryScope::Global
                    {
                        continue;
                    }
                    let count = report.resolved_seen_count(self.scope);
                    samples.push(TimedMetricSample {
                        time: report.metadata.normalized_time,
                        fix_time: point.tpv.time(),
                        report: Some(report.metadata),
                        value: count.map(|count| f64::from(count.count)),
                        source: MetricSampleSource {
                            point_index,
                            evidence: Some(if count.is_some_and(|count| count.is_aggregate) {
                                NavigationEvidenceSource::ObservationAggregate(index)
                            } else {
                                NavigationEvidenceSource::ObservationList(index)
                            }),
                        },
                    });
                }
            }
        }
        samples
    }
}

fn build_count_series(points: &[NavPoint]) -> FxHashMap<MetricKind, MetricSeries> {
    MetricKind::iter()
        .filter_map(|kind| {
            let metric = CountMetric::from_kind(kind)?;
            let samples = points
                .iter()
                .enumerate()
                .map(|(index, point)| metric.point_samples(index, point));
            Some((kind, build_metric_runs(points, samples)))
        })
        .collect()
}

fn build_metric_runs(
    points: &[NavPoint],
    samples: impl ExactSizeIterator<Item = Vec<TimedMetricSample>>,
) -> MetricSeries {
    debug_assert_eq!(points.len(), samples.len());
    let mut runs = Vec::new();
    let mut run = Vec::new();
    let mut previous_fix_time = None;
    let mut previous_report_time = None;
    for (point, mut samples) in points.iter().zip(samples) {
        if previous_fix_time.is_some_and(|time| point.tpv.time() < time) {
            runs.push(std::mem::take(&mut run));
            previous_report_time = None;
        }
        previous_fix_time = Some(point.tpv.time());
        samples.sort_by_key(|sample| sample.report.map(|report| report.input_order));
        for sample in samples {
            if sample.report.is_some() {
                if previous_report_time.is_some_and(|time| sample.time < time) {
                    runs.push(std::mem::take(&mut run));
                }
                previous_report_time = Some(sample.time);
            }
            run.push(sample);
        }
    }
    runs.push(run);
    MetricSeries::from_timed_runs(runs)
}

struct DiagnosticSeries {
    metrics: FxHashMap<MetricKind, MetricSeries>,
    anomalies: Vec<UtilAnomaly>,
}

fn build_diagnostic_series(points: &[NavPoint], analysis: AnalysisConfig) -> DiagnosticSeries {
    let utilization =
        gt_analysis::satellite_utilization::utilization_epochs(points, analysis.elevation_mask_deg);
    let slips = gt_analysis::loss_of_lock::slip_epochs(
        points,
        SlipAnalysisConfig {
            thresholds: SlipThresholds {
                elevation_mask_deg: analysis.elevation_mask_deg,
                snr_drop_db: analysis.snr_drop_db,
            },
            rate_window_min: Some(analysis.slip_window_min),
        },
    );
    let definitions = [
        (MetricKind::UtilAll, SatelliteQueryScope::Global, false),
        (
            MetricKind::UtilGps,
            SatelliteQueryScope::Constellation(Constellation::Gps),
            false,
        ),
        (
            MetricKind::UtilGlonass,
            SatelliteQueryScope::Constellation(Constellation::Glonass),
            false,
        ),
        (
            MetricKind::UtilGalileo,
            SatelliteQueryScope::Constellation(Constellation::Galileo),
            false,
        ),
        (
            MetricKind::UtilBeidou,
            SatelliteQueryScope::Constellation(Constellation::Beidou),
            false,
        ),
        (
            MetricKind::UtilNavic,
            SatelliteQueryScope::Constellation(Constellation::Navic),
            false,
        ),
        (
            MetricKind::UtilQzss,
            SatelliteQueryScope::Constellation(Constellation::Qzss),
            false,
        ),
        (MetricKind::SlipAll, SatelliteQueryScope::Global, true),
        (
            MetricKind::SlipGps,
            SatelliteQueryScope::Constellation(Constellation::Gps),
            true,
        ),
        (
            MetricKind::SlipGlonass,
            SatelliteQueryScope::Constellation(Constellation::Glonass),
            true,
        ),
        (
            MetricKind::SlipGalileo,
            SatelliteQueryScope::Constellation(Constellation::Galileo),
            true,
        ),
        (
            MetricKind::SlipBeidou,
            SatelliteQueryScope::Constellation(Constellation::Beidou),
            true,
        ),
        (
            MetricKind::SlipNavic,
            SatelliteQueryScope::Constellation(Constellation::Navic),
            true,
        ),
        (
            MetricKind::SlipQzss,
            SatelliteQueryScope::Constellation(Constellation::Qzss),
            true,
        ),
    ];
    let metrics = definitions
        .into_iter()
        .map(|(kind, scope, slip)| {
            let mut per_point = vec![Vec::new(); points.len()];
            let epochs: Vec<_> = if slip {
                slips
                    .iter()
                    .map(|epoch| (epoch.source, &epoch.rates))
                    .collect()
            } else {
                utilization
                    .iter()
                    .map(|epoch| (epoch.source, &epoch.rates))
                    .collect()
            };
            for (source, rates) in epochs {
                if !source.metadata.scope.covers(scope) && scope != SatelliteQueryScope::Global {
                    continue;
                }
                let Some(point) = points.get(source.point_index) else {
                    continue;
                };
                if let Some(samples) = per_point.get_mut(source.point_index) {
                    samples.push(TimedMetricSample {
                        time: source.metadata.normalized_time,
                        fix_time: point.tpv.time(),
                        report: Some(source.metadata),
                        value: rates.get(scope),
                        source: MetricSampleSource {
                            point_index: source.point_index,
                            evidence: Some(NavigationEvidenceSource::ObservationList(
                                source.observation_index,
                            )),
                        },
                    });
                }
            }
            (kind, build_metric_runs(points, per_point.into_iter()))
        })
        .collect();
    DiagnosticSeries {
        metrics,
        anomalies: utilization
            .into_iter()
            .filter_map(|epoch| epoch.into_anomaly())
            .collect(),
    }
}

const MICROS_PER_SEC: f64 = 1_000_000.0;

#[cfg(test)]
mod tests {
    use geotrace_sdk_units::Unit;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::nav_point::NavPoint;
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use rstest::rstest;
    use uom::si::f64::Angle;

    use super::*;

    /// The first sample of every fixture in this module: 2023-11-14 22:13:20
    /// UTC, `secs` seconds on.
    fn at_second(secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("valid timestamp")
    }

    /// The points one line draws at the level a view wanting `target` samples
    /// selects, in the order they are drawn.
    fn drawn_points(mipmap: &MipMap, target: usize) -> Vec<(f64, f64)> {
        let level = mipmap.select_indices(
            gt_egui_mipmap::SelectionRange::within_viewport(f64::NEG_INFINITY..=f64::INFINITY),
            target,
        );
        mipmap
            .slice_at(level)
            .iter()
            .map(|point| (point.x, point.y))
            .collect()
    }

    /// A vector channel becomes one line per component, on the channel's own
    /// sample clock - not resampled onto the nav points.
    #[test]
    fn a_vector_channel_builds_one_mipmap_per_component() {
        let channel = gt_types::Channel {
            name: "accel".to_owned(),
            unit: Some(Unit::G.into()),
            period: None,
            description: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            times: vec![at_second(0), at_second(1)],
            values: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
        };
        let series = build_channel_series(&channel);
        assert_eq!(series.name, "accel");
        assert_eq!(series.unit.as_deref(), Some("g"));
        let labels: Vec<&str> = series.components.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["accel.x", "accel.y", "accel.z"]);
        // Each component line carries its column at the channel's timestamps.
        let [run] = series.components[1].runs.as_slice() else {
            panic!("a channel whose timestamps never step backwards is one run");
        };
        let pts = drawn_points(run, usize::MAX);
        assert_eq!(
            pts,
            [(1_700_000_000.0, 0.2), (1_700_000_001.0, 0.5)],
            "the y column at the channel's timestamps"
        );
    }

    /// Each stretch between two backward steps is mipmapped on its own and
    /// drawn as its own line: mipmap level selection is a binary search over x,
    /// and a recorder that restarts its clock stamps the samples after the
    /// restart at times the samples before it already cover.
    #[test]
    fn a_channel_whose_timestamps_step_backwards_becomes_one_run_per_stretch() {
        let channel = gt_types::Channel {
            name: "accel".to_owned(),
            unit: Some(Unit::G.into()),
            period: None,
            description: None,
            components: vec!["x".to_owned()],
            times: vec![
                at_second(0),
                at_second(1),
                at_second(2),
                at_second(0),
                at_second(1),
            ],
            values: vec![0.0, 1.0, 2.0, 3.0, 4.0],
        };
        let series = build_channel_series(&channel);
        let [component] = series.components.as_slice() else {
            panic!("a one-component channel has one component series");
        };
        let drawn: Vec<Vec<(f64, f64)>> = component
            .runs
            .iter()
            .map(|run| drawn_points(run, usize::MAX))
            .collect();
        assert_eq!(
            drawn,
            [
                vec![
                    (1_700_000_000.0, 0.0),
                    (1_700_000_001.0, 1.0),
                    (1_700_000_002.0, 2.0),
                ],
                vec![(1_700_000_000.0, 3.0), (1_700_000_001.0, 4.0)],
            ]
        );
    }

    /// A track at 1 Hz whose clock offset holds near −234 ms, with the sample
    /// at `spike_at` carrying a 1 h 09 m recording gap - the `gnss.h5.gtd`
    /// case, where the receiver reported its pre-gap GPS epoch after resuming.
    fn track_with_a_clock_spike(count: i64, spike_at: i64) -> gt_types::LoadedTrack {
        let points = (0..count)
            .map(|i| {
                let gps = at_second(i);
                let ahead_ms = if i == spike_at { 4_127_054 } else { 234 };
                let tpv = gt_types::tpv::TimePositionVelocity::builder()
                    .time(gt_types::time_types::GpsTime::from_utc(gps))
                    .lat(gt_types::coordinates::Latitude::new(55.0))
                    .lon(gt_types::coordinates::Longitude::new(12.0))
                    .sys_time(gt_types::time_types::SysTime::from_utc(
                        gps + chrono::Duration::milliseconds(ahead_ms),
                    ))
                    .build();
                gt_types::nav_point::NavPoint::new(tpv, None)
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    /// The y-extent of the clock offset line, over every point it carries.
    fn clock_delta_extent(series: &TrackSeries) -> (f64, f64) {
        drawn_points(&series.clock_delta_ms, usize::MAX)
            .into_iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, y)| {
                (lo.min(y), hi.max(y))
            })
    }

    /// The excursions of a series whose baseline the shared y-axis can show.
    fn excursions(series: &TrackSeries) -> &[gt_analysis::clock_offset::ClockOffsetExcursion] {
        match &series.clock_offset_placement {
            ClockOffsetPlacement::BaselineOnScale(excursions) => excursions,
            ClockOffsetPlacement::BaselineOffScale(off_scale) => {
                panic!("expected a baseline on the axis, got {off_scale:?}")
            }
        }
    }

    /// The whole point of the excursion split: one sample carrying a recording
    /// gap must not set the auto-bounds of the y-axis every metric shares.
    #[test]
    fn a_clock_spike_stays_off_the_line_and_out_of_its_extent() {
        let track = track_with_a_clock_spike(8, 4);
        let series = build_track_series(0, &track, AnalysisConfig::default());

        let (lo, hi) = clock_delta_extent(&series);
        assert!(
            lo >= -1000.0 && hi <= 0.0,
            "the line keeps the track's own scale, got {lo}..{hi}"
        );
        let [excursion] = excursions(&series) else {
            panic!("expected one excursion, got {:?}", excursions(&series));
        };
        assert_eq!(excursion.peak().offset_ms, -4_127_054, "value is not lost");
    }

    /// The series leaves out the excursion's samples and keeps every other
    /// one. The plot cuts the line into stretches at draw time, which takes
    /// nothing further out of the mipmap.
    #[test]
    fn the_clock_offset_series_keeps_every_sample_outside_the_excursion() {
        let track = track_with_a_clock_spike(8, 4);
        let series = build_track_series(0, &track, AnalysisConfig::default());

        let seconds: Vec<i64> = drawn_points(&series.clock_delta_ms, usize::MAX)
            .into_iter()
            .map(|(x, _)| x as i64 - at_second(0).timestamp())
            .collect();

        assert_eq!(seconds, [0, 1, 2, 3, 5, 6, 7]);
    }

    /// Raising the threshold past the departure puts the sample back on the
    /// line: the split is the user's call, not a fixed rule.
    #[test]
    fn the_threshold_determines_what_leaves_the_line() {
        let track = track_with_a_clock_spike(8, 4);
        let analysis = AnalysisConfig {
            clock_excursion_threshold_s: 4200.0,
            ..AnalysisConfig::default()
        };
        let series = build_track_series(0, &track, analysis);

        assert!(excursions(&series).is_empty());
        let (lo, _) = clock_delta_extent(&series);
        assert_eq!(lo, -4_127_054.0, "the sample is back on the line");
    }

    /// `apply_analysis` re-derives the split, so changing the threshold in
    /// Settings lands without a reload.
    #[test]
    fn apply_analysis_re_derives_the_excursion_split() {
        let track = track_with_a_clock_spike(8, 4);
        let mut series = build_track_series(0, &track, AnalysisConfig::default());
        assert_eq!(excursions(&series).len(), 1);

        series.apply_analysis(
            &track,
            AnalysisConfig {
                clock_excursion_threshold_s: 4200.0,
                ..AnalysisConfig::default()
            },
        );
        assert!(excursions(&series).is_empty());
        assert_eq!(clock_delta_extent(&series).0, -4_127_054.0);
    }

    /// The steps leave the stored order for ascending x here: the overlay
    /// clips its marks to the view with a binary search over x.
    #[test]
    fn build_channel_series_orders_the_backward_time_steps_by_the_timestamp_they_are_marked_at() {
        let channel = gt_types::Channel {
            name: "incline".to_owned(),
            unit: None,
            period: None,
            description: None,
            components: vec![],
            times: vec![
                at_second(0),
                at_second(30),
                at_second(12),
                at_second(20),
                at_second(4),
            ],
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0],
        };

        let series = build_channel_series(&channel);

        let stepped: Vec<(i64, i64)> = series
            .backward_time_steps
            .iter()
            .map(|placed| (placed.previous_time.timestamp(), placed.time.timestamp()))
            .collect();
        assert_eq!(
            stepped,
            [
                (at_second(20).timestamp(), at_second(4).timestamp()),
                (at_second(30).timestamp(), at_second(12).timestamp()),
            ]
        );
    }

    /// A recording from a tracker whose clock restarts at every boot, loaded
    /// the way the app loads one.
    fn recording_whose_clock_restarts_at_every_boot() -> LoadedFile {
        gt_loader::load_bytes(
            &gt_test_utils::recording_whose_clock_restarts_at_every_boot(),
            "clock_reset.gtd".to_owned(),
        )
        .expect("the fixture loads")
    }

    #[test]
    fn every_stretch_between_two_backward_steps_is_its_own_run() {
        let file = recording_whose_clock_restarts_at_every_boot();
        let series = build_file_series(&file, AnalysisConfig::default());
        let with_channels: Vec<&TrackSeries> =
            series.iter().filter(|s| !s.channels.is_empty()).collect();
        let [track] = with_channels.as_slice() else {
            panic!("the fixture's channel samples all fall in one track");
        };
        let [channel] = track.channels.as_slice() else {
            panic!("the fixture holds one channel");
        };
        for component in &channel.components {
            assert_eq!(
                component
                    .runs
                    .iter()
                    .map(MipMap::original_len)
                    .collect::<Vec<_>>(),
                [250, 200, 300, 250],
                "{}",
                component.label
            );
        }
    }

    /// Each run is drawn on its own, at every mipmap level. A line spanning two
    /// runs would double back at the wrong place: mipmap level selection is a
    /// binary search over x, and two runs of a recorder that restarted its
    /// clock cover the same times.
    #[test]
    fn no_drawn_channel_line_steps_back_in_time() {
        let file = recording_whose_clock_restarts_at_every_boot();
        let series = build_file_series(&file, AnalysisConfig::default());
        for track in &series {
            for channel in &track.channels {
                for component in &channel.components {
                    for run in &component.runs {
                        for target in [COARSE_TARGET_FOR_A_RUN_OF_HUNDREDS, usize::MAX] {
                            let drawn = drawn_points(run, target);
                            assert!(
                                drawn.windows(2).all(|pair| match pair {
                                    [before, after] => before.0 <= after.0,
                                    _ => true,
                                }),
                                "{} drew {drawn:?} for a target of {target}",
                                component.label
                            );
                        }
                    }
                }
            }
        }
    }

    /// The tracker holds its RTC default for the fixes at the head of each
    /// boot, which puts the whole offset between the two clock epochs on those
    /// samples. The clock-offset line keeps to the track's baseline of a fifth
    /// of a second: the excursions hold the departures.
    #[test]
    fn the_clock_offset_line_keeps_to_the_baseline_over_a_boot_clock_reset() {
        let file = recording_whose_clock_restarts_at_every_boot();
        let series = build_file_series(&file, AnalysisConfig::default());
        for track in &series {
            let drawn = drawn_points(&track.clock_delta_ms, usize::MAX);
            assert!(
                drawn
                    .iter()
                    .all(|&(_, offset_ms)| offset_ms.abs() <= 1_000.0),
                "the clock offset line of track {} reaches {drawn:?}",
                track.ti
            );
        }
        let peaks: Vec<i64> = series
            .iter()
            .flat_map(excursions)
            .map(|excursion| excursion.peak().offset_ms)
            .collect();
        assert_eq!(peaks.len(), 4, "one excursion per boot");
        let epoch_gap_ms =
            gt_test_utils::clock_reset_fixtures::gap_between_the_clock_epochs().num_milliseconds();
        assert!(
            peaks.iter().all(|&offset_ms| offset_ms >= epoch_gap_ms),
            "each excursion holds a boot's whole clock offset of at least \
             {epoch_gap_ms} ms, peaks are {peaks:?}"
        );
    }

    /// A 1 Hz track whose fixes carry `headings`, in degrees. `None` is a ghost
    /// fix: the receiver reported a position but no direction.
    fn track_with_headings(headings: &[Option<f64>]) -> gt_types::LoadedTrack {
        let points = headings
            .iter()
            .enumerate()
            .map(|(i, heading)| {
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(at_second(i as i64)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .maybe_heading(heading.map(Angle::new::<degree>))
                    .build();
                NavPoint::new(tpv, None)
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    fn heading_series(headings: &[Option<f64>]) -> MipMap {
        let track = track_with_headings(headings);
        build_track_series(0, &track, AnalysisConfig::default()).heading_deg
    }

    /// A scalar channel of `values` in degrees, sampled at 1 Hz, declaring
    /// `period_deg` as its wrap period.
    fn degree_channel(period_deg: Option<f64>, values: &[f64]) -> gt_types::Channel {
        gt_types::Channel {
            name: "compass".to_owned(),
            unit: Some(Unit::DEG.into()),
            period: period_deg.map(Angle::new::<degree>),
            description: None,
            components: vec![],
            times: (0..values.len() as i64).map(at_second).collect(),
            values: values.to_vec(),
        }
    }

    /// The y values the plot draws for one series at the level a view wanting
    /// `target` samples selects, in the order they are drawn.
    fn drawn_values(mipmap: &MipMap, target: usize) -> Vec<f64> {
        drawn_points(mipmap, target)
            .into_iter()
            .map(|(_, y)| y)
            .collect()
    }

    /// The one component of a scalar channel's series.
    fn scalar_channel_values(channel: &gt_types::Channel, target: usize) -> Vec<f64> {
        let series = build_channel_series(channel);
        let [component] = series.components.as_slice() else {
            panic!("a scalar channel has one component");
        };
        let [run] = component.runs.as_slice() else {
            panic!("a channel whose timestamps never step backwards is one run");
        };
        drawn_values(run, target)
    }

    /// Around north a bucket's linear minimum and maximum are ~0° and ~359°,
    /// and the U-turn between them is the outlier the mip-map exists to keep.
    #[test]
    fn a_southward_swing_survives_a_bucket_of_northward_headings() {
        let headings = [359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0].map(Some);
        let drawn = drawn_values(
            &heading_series(&headings),
            COARSE_TARGET_FOR_AN_EIGHT_FIX_TRACK,
        );
        assert!(
            drawn.contains(&180.0),
            "the southward fix must survive downsampling, drawn values are {drawn:?}"
        );
    }

    /// A channel states its own wrap period, which is the period its samples
    /// are downsampled over.
    #[rstest]
    #[case::full_turn(360.0, [359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0], 180.0)]
    #[case::half_turn(180.0, [179.0, 1.0, 90.0, 2.0, 178.0, 0.0, 179.0, 1.0], 90.0)]
    fn a_swing_survives_a_bucket_of_a_channel_declaring_a_wrap_period(
        #[case] period_deg: f64,
        #[case] values: [f64; 8],
        #[case] swing: f64,
    ) {
        let channel = degree_channel(Some(period_deg), &values);
        let drawn = scalar_channel_values(&channel, COARSE_TARGET_FOR_AN_EIGHT_FIX_TRACK);
        assert!(
            drawn.contains(&swing),
            "the {swing}° sample must survive downsampling, drawn values are {drawn:?}"
        );
    }

    /// A channel that declares no period keeps each bucket's linear minimum and
    /// maximum, however angular its unit reads.
    #[test]
    fn a_degree_channel_without_a_declared_period_is_downsampled_linearly() {
        let channel = degree_channel(None, &[359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0]);
        assert_eq!(
            scalar_channel_values(&channel, COARSE_TARGET_FOR_AN_EIGHT_FIX_TRACK),
            [359.0, 1.0, 0.0, 359.0]
        );
    }

    /// At full detail the plot draws the headings the receiver reported, in
    /// the values it reported them.
    #[test]
    fn every_recorded_heading_is_drawn_at_full_detail() {
        let headings = [359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0];
        assert_eq!(
            drawn_values(&heading_series(&headings.map(Some)), usize::MAX),
            headings
        );
    }

    /// A ghost fix has no heading, which is not a heading of north.
    #[test]
    fn a_ghost_fixs_missing_heading_contributes_no_sample() {
        let series = heading_series(&[Some(10.0), None, None, Some(20.0)]);
        assert_eq!(drawn_values(&series, usize::MAX), [10.0, 20.0]);
    }

    /// Sample target that selects a downsampled level of a run of a few
    /// hundred samples.
    const COARSE_TARGET_FOR_A_RUN_OF_HUNDREDS: usize = 8;

    /// Target sample count that selects the first downsampled level of an
    /// eight-fix track.
    const COARSE_TARGET_FOR_AN_EIGHT_FIX_TRACK: usize = 4;

    #[test]
    fn navigation_only_series_preserve_source_measurements_and_missing_diagnostics() {
        let recording =
            gt_test_utils::navigation_evidence_fixtures::sparse_navigation_recording().unwrap();
        let mut bytes = Vec::new();
        recording.write(&mut bytes).unwrap();
        let file = gt_loader::load_bytes(&bytes, "navigation_only.gtd".to_owned()).unwrap();
        let track = file.tracks.first().unwrap();
        let series = build_track_series(0, track, AnalysisConfig::default());
        let time = track
            .points
            .get(1)
            .expect("motion epoch")
            .tpv
            .time()
            .as_secs_f64();
        assert_eq!(
            drawn_points(&series.velocity_kmh, usize::MAX),
            vec![(time, 15.12)]
        );
        assert_eq!(
            drawn_points(&series.heading_deg, usize::MAX),
            vec![(time, 75.0), (time + 10.0, 75.0)]
        );
        let hdop = series.metrics.get(&MetricKind::Hdop).expect("HDOP series");
        assert_eq!(
            drawn_points(hdop.runs.first().unwrap(), usize::MAX),
            vec![(time, 0.9)]
        );
        assert!(series.eph_m.finest_level().is_empty());
        for kind in [MetricKind::SatsSeen, MetricKind::UtilAll] {
            assert!(
                series
                    .metrics
                    .get(&kind)
                    .expect("diagnostic series")
                    .runs
                    .is_empty()
            );
        }
        assert!(series.present.is_empty());
        assert!(track.points.iter().all(|point| {
            point
                .utilization_evidence(SatelliteQueryScope::Global, 10.0)
                .is_none()
        }));
    }
}

#[cfg(test)]
mod count_tests;
