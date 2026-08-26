use std::sync::Arc;

use chrono::{DateTime, Utc};
use gt_analysis::loss_of_lock::{self, SECS_PER_MIN, SlipRatePerPoint};
use gt_analysis::satellite_utilization::{self, UtilPerPoint};
use gt_query::{ChannelSamples, ChannelTimeline, MetricProvider, Params, QueryMetric, Unit};
use gt_types::satellites::Constellation;
use gt_types::{Channel, NavPoint};
use gt_ui_types::{GeomagneticPoint, TecPoint};
use uom::si::angle::degree;
use uom::si::velocity::meter_per_second;

use crate::MICROS_PER_SEC;

/// The per-track series a run reads ready-made: the snap run's errors and
/// the two archives' per-fix values.
#[derive(Default, Clone)]
pub(crate) struct CapturedTrackValues {
    pub(crate) snap_error: Option<Arc<Vec<Option<f64>>>>,
    pub(crate) jamming: Option<Arc<Vec<Option<f64>>>>,
    pub(crate) geomagnetic: Option<Arc<Vec<GeomagneticPoint>>>,
    pub(crate) tec: Option<Arc<Vec<TecPoint>>>,
}

/// Owned per-track inputs for [`TrackProvider`], computed once per run.
#[derive(Default)]
pub struct TrackQueryData {
    util: Option<UtilPerPoint>,
    slip: Option<SlipRatePerPoint>,
    /// Dense per-point snap error values from the track's latest completed
    /// snap run at spawn time, shared with the app's per-run cache. `None`
    /// for tracks without a run.
    snap_error: Option<Arc<Vec<Option<f64>>>>,
    /// Dense per-point interference percentages at spawn time.
    jamming: Option<Arc<Vec<Option<f64>>>>,
    /// Dense per-point geomagnetic index values at spawn time, one entry per
    /// fix carrying both indices.
    geomagnetic: Option<Arc<Vec<GeomagneticPoint>>>,
    /// Dense per-point TEC values at spawn time.
    tec: Option<Arc<Vec<TecPoint>>>,
    /// Index of the first point inside the global time filter - the offset
    /// between slice-relative evaluation indices and absolute point indices.
    slice_start: usize,
}

impl TrackQueryData {
    /// Derive the series `params` calls for, over the whole track.
    ///
    /// `slice_start` is the first point inside the global time filter, kept so
    /// results can shift evaluation indices back to absolute positions.
    pub(crate) fn derive(
        points: &[NavPoint],
        params: Params,
        uses_util: bool,
        uses_slip: bool,
        slice_start: usize,
        captured: CapturedTrackValues,
    ) -> Self {
        // gt_query::check::require_params guarantees these parameters whenever
        // the corresponding metrics are referenced - defaulting below is for the
        // Option unwrap only, never a real fallback.
        debug_assert!(
            !(uses_util || uses_slip) || params.mask_deg.is_some(),
            "checker must reject util/slip metrics without a mask"
        );
        debug_assert!(
            !uses_slip || (params.snr_drop_db_hz.is_some() && params.slip_window_s.is_some()),
            "checker must reject slip metrics without snr_drop and slip_window"
        );
        let mask_deg = params.mask_deg.unwrap_or_default() as f32;
        let util = uses_util.then(|| satellite_utilization::util_per_point(points, mask_deg));
        let slip = uses_slip.then(|| {
            loss_of_lock::slip_rate_per_point(
                points,
                mask_deg,
                params.snr_drop_db_hz.unwrap_or_default() as f32,
                (params.slip_window_s.unwrap_or_default() / SECS_PER_MIN) as f32,
            )
        });
        Self {
            jamming: captured.jamming,
            geomagnetic: captured.geomagnetic,
            tec: captured.tec,
            util,
            slip,
            snap_error: captured.snap_error,
            slice_start,
        }
    }

    /// Index of the first point inside the global time filter: the offset from
    /// the evaluator's slice-relative indices to absolute point indices.
    pub fn slice_start(&self) -> usize {
        self.slice_start
    }
}

/// Provider over one track's points plus the run's derived series, in the
/// evaluator's base units (m/s, degrees, seconds, 0-1 ratios, per minute).
#[derive(Clone, Copy)]
pub struct TrackProvider<'a> {
    points: &'a [NavPoint],
    channels: &'a [Channel],
    util: Option<&'a UtilPerPoint>,
    slip: Option<&'a SlipRatePerPoint>,
    /// Dense per-point snap error values (one slot per track point), from
    /// the track's latest completed snap run. `None` for tracks without a
    /// run - the metric then resolves no values and points are skipped.
    snap_error: Option<&'a [Option<f64>]>,
    /// Dense per-point interference percentages. `None` for tracks whose
    /// days are not archived.
    jamming: Option<&'a [Option<f64>]>,
    /// Dense per-point geomagnetic index values, missing under the same
    /// conditions as [`Self::jamming`].
    geomagnetic: Option<&'a [GeomagneticPoint]>,
    /// Dense per-point TEC values, missing under the same conditions as
    /// [`Self::jamming`].
    tec: Option<&'a [TecPoint]>,
}

impl<'a> TrackProvider<'a> {
    /// The provider both a run and the match tables read through - one code
    /// path, so tables always show the values the evaluator saw.
    pub fn new(
        points: &'a [NavPoint],
        channels: &'a [Channel],
        data: Option<&'a TrackQueryData>,
    ) -> Self {
        Self {
            points,
            channels,
            util: data.and_then(|d| d.util.as_ref()),
            slip: data.and_then(|d| d.slip.as_ref()),
            snap_error: data.and_then(|d| d.snap_error.as_deref().map(Vec::as_slice)),
            jamming: data.and_then(|d| d.jamming.as_deref().map(Vec::as_slice)),
            geomagnetic: data.and_then(|d| d.geomagnetic.as_deref().map(Vec::as_slice)),
            tec: data.and_then(|d| d.tec.as_deref().map(Vec::as_slice)),
        }
    }

    /// One index's value at `index`, as the service published it.
    fn geomagnetic_value(
        &self,
        index: usize,
        value: impl Fn(&GeomagneticPoint) -> Option<f64>,
    ) -> Option<f64> {
        self.geomagnetic
            .and_then(|points| points.get(index))
            .and_then(value)
    }

    fn util_value(
        &self,
        index: usize,
        series: impl Fn(&UtilPerPoint) -> &[Option<f64>],
    ) -> Option<f64> {
        let percent = self
            .util
            .and_then(|u| series(u).get(index).copied().flatten())?;
        // gt-analysis reports percent. The evaluator's ratio base is the 0-1
        // fraction, converted through the language's canonical % factor.
        Some(percent * Unit::PERCENT.to_base())
    }

    fn slip_value(
        &self,
        index: usize,
        series: impl Fn(&SlipRatePerPoint) -> &[Option<f64>],
    ) -> Option<f64> {
        self.slip
            .and_then(|s| series(s).get(index).copied().flatten())
    }

    fn counts(&self, index: usize, constellation: Constellation) -> SatCounts {
        self.points
            .get(index)
            .and_then(|p| p.satellites.as_ref())
            .map_or(SatCounts::default(), |sats| {
                sats.by_constellation(constellation)
                    .fold(SatCounts::default(), |acc, sat| SatCounts {
                        seen: acc.seen + 1,
                        fix: acc.fix + usize::from(sat.in_fix()),
                    })
            })
    }

    /// Locate channel `name` and the two things both channel readers need: its
    /// column count and the factor converting its stored unit to base units. An
    /// unknown or absent unit leaves values a bare number (factor 1.0), matching
    /// how the checker types such a channel; components share the channel unit.
    fn resolve_channel(&self, name: &str) -> Option<(&Channel, usize, f64)> {
        let channel = self.channels.iter().find(|c| c.name == name)?;
        let to_base = channel
            .unit
            .as_ref()
            .and_then(|unit| unit.as_recognized())
            .map_or(1.0, Unit::to_base);
        Some((channel, channel.component_count(), to_base))
    }
}

/// Seen/in-fix satellite counts of one constellation at one point.
#[derive(Debug, Clone, Copy, Default)]
struct SatCounts {
    seen: usize,
    fix: usize,
}

impl MetricProvider for TrackProvider<'_> {
    fn len(&self) -> usize {
        self.points.len()
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        let point = self.points.get(index)?;
        let sats = point.satellites.as_ref();
        match metric {
            QueryMetric::Time => Some(point.tpv.time().as_secs_f64()),
            QueryMetric::SysTime => point
                .tpv
                .sys_time()
                .map(|s| s.utc().timestamp_millis() as f64 / 1_000.0),
            QueryMetric::Lat => Some(point.tpv.lat().as_degrees()),
            QueryMetric::Lon => Some(point.tpv.lon().as_degrees()),
            QueryMetric::Velocity => point.tpv.velocity().map(|v| v.get::<meter_per_second>()),
            QueryMetric::Heading => point.tpv.heading().map(|h| h.get::<degree>()),
            // Derived by the evaluator (`gt_query::derived_accel`), never
            // requested from providers.
            QueryMetric::Accel => None,
            QueryMetric::Eph => point.tpv.eph_m().map(f64::from),
            QueryMetric::ClockDelta => point
                .tpv
                .gps_system_clock_offset()
                .map(|offset| offset.num_milliseconds() as f64 / 1_000.0),
            QueryMetric::SatsSeen => sats.map(|s| f64::from(s.satellite_count())),
            QueryMetric::SatsFix => sats.map(|s| f64::from(s.fix_count())),
            QueryMetric::GpsSeen => {
                sats.map(|_| self.counts(index, Constellation::Gps).seen as f64)
            }
            QueryMetric::GpsFix => sats.map(|_| self.counts(index, Constellation::Gps).fix as f64),
            QueryMetric::GlonassSeen => {
                sats.map(|_| self.counts(index, Constellation::Glonass).seen as f64)
            }
            QueryMetric::GlonassFix => {
                sats.map(|_| self.counts(index, Constellation::Glonass).fix as f64)
            }
            QueryMetric::GalileoSeen => {
                sats.map(|_| self.counts(index, Constellation::Galileo).seen as f64)
            }
            QueryMetric::GalileoFix => {
                sats.map(|_| self.counts(index, Constellation::Galileo).fix as f64)
            }
            QueryMetric::BeidouSeen => {
                sats.map(|_| self.counts(index, Constellation::Beidou).seen as f64)
            }
            QueryMetric::BeidouFix => {
                sats.map(|_| self.counts(index, Constellation::Beidou).fix as f64)
            }
            QueryMetric::NavicSeen => {
                sats.map(|_| self.counts(index, Constellation::Navic).seen as f64)
            }
            QueryMetric::NavicFix => {
                sats.map(|_| self.counts(index, Constellation::Navic).fix as f64)
            }
            QueryMetric::QzssSeen => {
                sats.map(|_| self.counts(index, Constellation::Qzss).seen as f64)
            }
            QueryMetric::QzssFix => {
                sats.map(|_| self.counts(index, Constellation::Qzss).fix as f64)
            }
            QueryMetric::UtilAll => self.util_value(index, |u| &u.all),
            QueryMetric::UtilGps => self.util_value(index, |u| &u.gps),
            QueryMetric::UtilGlonass => self.util_value(index, |u| &u.glonass),
            QueryMetric::UtilGalileo => self.util_value(index, |u| &u.galileo),
            QueryMetric::UtilBeidou => self.util_value(index, |u| &u.beidou),
            QueryMetric::UtilNavic => self.util_value(index, |u| &u.navic),
            QueryMetric::UtilQzss => self.util_value(index, |u| &u.qzss),
            QueryMetric::SlipAll => self.slip_value(index, |s| &s.all),
            QueryMetric::SlipGps => self.slip_value(index, |s| &s.gps),
            QueryMetric::SlipGlonass => self.slip_value(index, |s| &s.glonass),
            QueryMetric::SlipGalileo => self.slip_value(index, |s| &s.galileo),
            QueryMetric::SlipBeidou => self.slip_value(index, |s| &s.beidou),
            QueryMetric::SlipNavic => self.slip_value(index, |s| &s.navic),
            QueryMetric::SlipQzss => self.slip_value(index, |s| &s.qzss),
            QueryMetric::SnapError => self
                .snap_error
                .and_then(|values| values.get(index).copied().flatten()),
            QueryMetric::Jamming => self
                .jamming
                .and_then(|values| values.get(index).copied().flatten())
                // A percentage, converted to the 0-1 ratio base like the util
                // metrics.
                .map(|percent| percent * Unit::PERCENT.to_base()),
            QueryMetric::Hp30 => self.geomagnetic_value(index, |point| point.hp30),
            QueryMetric::Kp => self.geomagnetic_value(index, |point| point.kp),
            QueryMetric::Tec => self
                .tec
                .and_then(|points| points.get(index))
                .and_then(|point| point.tecu),
        }
    }

    /// A channel's samples whose timestamp lands in `[t_lo, t_hi]`, as row-major
    /// rows (one column per component, one for a scalar channel), converted from
    /// the channel's stored unit to the evaluator's base units.
    ///
    /// `t_lo`/`t_hi` arrive floored to whole seconds (the query engine's time
    /// resolution, since nav-point time floors to whole seconds); the sub-second
    /// precision of a sample's own timestamp only refines placement within that
    /// grid.
    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        let Some((channel, columns, to_base)) = self.resolve_channel(name) else {
            return ChannelSamples::default();
        };
        // `times` is sorted ascending, so the samples in the closed span are a
        // contiguous row range found by binary search. An inverted span (`t_lo >
        // t_hi`, possible when the track's time is non-monotonic) makes the range
        // empty rather than panicking. Values are row-major `[rows, columns]`, so
        // row `r`'s columns are `r*columns .. (r+1)*columns`.
        let secs = |time: &DateTime<Utc>| time.timestamp_micros() as f64 / MICROS_PER_SEC;
        let lo = channel.times.partition_point(|time| secs(time) < t_lo);
        let hi = channel.times.partition_point(|time| secs(time) <= t_hi);
        let values = channel
            .values
            .get(lo * columns..hi * columns)
            .unwrap_or_default()
            .iter()
            .map(|value| value * to_base)
            .collect();
        ChannelSamples { values, columns }
    }

    /// The whole sample timeline of `name` in base units, for a query whose
    /// source is that channel. Each sample keeps its own (sub-second) time: the
    /// channel is the timeline here.
    fn channel_timeline(&self, name: &str) -> ChannelTimeline {
        let Some((channel, columns, to_base)) = self.resolve_channel(name) else {
            return ChannelTimeline::default();
        };
        ChannelTimeline {
            times: channel
                .times
                .iter()
                .map(|t| t.timestamp_micros() as f64 / MICROS_PER_SEC)
                .collect(),
            values: channel.values.iter().map(|value| value * to_base).collect(),
            columns,
        }
    }
}

/// A window onto another provider: the evaluator sees only the points inside
/// the global time filter, while the inner [`TrackProvider`] (and the derived
/// series it carries) stays indexed by absolute track position.
pub struct SliceProvider<'a> {
    inner: TrackProvider<'a>,
    start: usize,
    len: usize,
}

impl<'a> SliceProvider<'a> {
    /// A view of `inner` covering `len` points from absolute index `start`.
    pub fn new(inner: TrackProvider<'a>, start: usize, len: usize) -> Self {
        Self { inner, start, len }
    }
}

impl MetricProvider for SliceProvider<'_> {
    fn len(&self) -> usize {
        self.len
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        if index >= self.len {
            return None;
        }
        self.inner.value(metric, self.start + index)
    }

    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        // Channel samples are keyed by absolute time, so the slice's time span
        // selects them directly from the inner provider - no index offset.
        self.inner.channel_span(name, t_lo, t_hi)
    }

    fn channel_timeline(&self, name: &str) -> ChannelTimeline {
        // A channel's own sample clock is independent of the point slice, so it
        // forwards whole.
        self.inner.channel_timeline(name)
    }
}

#[cfg(test)]
mod tests {
    use gt_types::{FileIdx, TrackIdx, TrackRef};

    use super::*;
    use crate::check::check_text;
    use crate::schema::schema_from_files;
    use crate::test_fixtures::{
        TEST_EPOCH, file_with_channels, scalar_channel, test_points, vector_channel,
    };

    #[test]
    fn provider_maps_metrics_to_base_units() {
        let points = test_points();
        let util = UtilPerPoint {
            gps: vec![Some(50.0), None],
            ..UtilPerPoint::default()
        };
        let slip = SlipRatePerPoint {
            all: vec![Some(2.0), None],
            ..SlipRatePerPoint::default()
        };
        let data = TrackQueryData {
            // Point 0 in a cell where 10 % of aircraft reported low
            // accuracy. Point 1's day is not archived.
            jamming: Some(Arc::new(vec![Some(10.0), None])),
            // Point 0 is in an archived storm period, Hp30 above the Kp
            // ceiling. Point 1's day is not archived.
            geomagnetic: Some(Arc::new(vec![
                GeomagneticPoint {
                    x_secs: 0.0,
                    hp30: Some(11.333),
                    kp: Some(9.0),
                },
                GeomagneticPoint {
                    x_secs: 1.0,
                    hp30: None,
                    kp: None,
                },
            ])),
            // Point 0 sits under an archived storm-day map. Point 1's day is
            // not archived.
            tec: Some(Arc::new(vec![
                TecPoint {
                    x_secs: 0.0,
                    tecu: Some(112.5),
                },
                TecPoint {
                    x_secs: 1.0,
                    tecu: None,
                },
            ])),
            util: Some(util),
            slip: Some(slip),
            // Point 0 snapped with a 3.5 m error; point 1 carries no value
            // (unsnapped, thinned, or simply beyond the sent range).
            snap_error: Some(Arc::new(vec![Some(3.5), None])),
            slice_start: 0,
        };
        let provider = TrackProvider::new(&points, &[], Some(&data));

        // (metric, point index, expected base-unit value)
        let cases = [
            (QueryMetric::Lat, 0, Some(55.5)),
            (QueryMetric::Lon, 0, Some(12.25)),
            (QueryMetric::Velocity, 0, Some(10.0)), // 36 km/h in m/s
            (QueryMetric::Heading, 0, Some(90.0)),
            (QueryMetric::Eph, 0, Some(2.5)),
            (QueryMetric::SatsSeen, 0, Some(3.0)),
            (QueryMetric::SatsFix, 0, Some(2.0)),
            (QueryMetric::GpsSeen, 0, Some(2.0)),
            (QueryMetric::GpsFix, 0, Some(1.0)),
            (QueryMetric::GalileoFix, 0, Some(1.0)),
            (QueryMetric::BeidouSeen, 0, Some(0.0)),
            (QueryMetric::UtilGps, 0, Some(0.5)), // 50 % as a fraction
            (QueryMetric::SlipAll, 0, Some(2.0)), // already per minute
            (QueryMetric::SnapError, 0, Some(3.5)), // already metres
            (QueryMetric::Jamming, 0, Some(0.1)), // 10 % as a fraction
            (QueryMetric::Hp30, 0, Some(11.333)), // the published index value
            (QueryMetric::Kp, 0, Some(9.0)),
            (QueryMetric::Tec, 0, Some(112.5)), // already TEC units
            // The reportless point: counts and derived series are missing,
            // never zero.
            (QueryMetric::Velocity, 1, None),
            (QueryMetric::SatsSeen, 1, None),
            (QueryMetric::GpsSeen, 1, None),
            (QueryMetric::UtilGps, 1, None),
            (QueryMetric::SlipAll, 1, None),
            (QueryMetric::SnapError, 1, None),
            (QueryMetric::Jamming, 1, None),
            (QueryMetric::Hp30, 1, None),
            (QueryMetric::Kp, 1, None),
            (QueryMetric::Tec, 1, None),
        ];
        for (metric, index, expected) in cases {
            let value = provider.value(metric, index);
            match expected {
                Some(want) => {
                    let got = value.unwrap_or_else(|| panic!("{metric} at {index} missing"));
                    assert!(
                        (got - want).abs() < 1e-9,
                        "{metric} at {index}: {got} != {want}"
                    );
                }
                None => assert_eq!(value, None, "{metric} at {index}"),
            }
        }
        assert_eq!(provider.len(), 2);
    }

    /// A track without a snap run resolves no `snap_error` values - the
    /// metric never invents data (and never triggers an upload; providers
    /// only read what the app captured).
    #[test]
    fn snap_error_is_absent_without_a_run() {
        let points = test_points();
        let provider = TrackProvider::new(&points, &[], None);
        assert_eq!(provider.value(QueryMetric::SnapError, 0), None);
        assert_eq!(provider.value(QueryMetric::SnapError, 1), None);
    }

    /// The correlation query the geomagnetic metrics exist for: a fix under
    /// a storm that also lost lock. Only the fix inside the archived storm
    /// period matches, so the index is what narrows the result.
    #[test]
    fn a_storm_and_slip_correlation_checks_and_runs_end_to_end() {
        let points = test_points();
        let data = TrackQueryData {
            geomagnetic: Some(Arc::new(vec![
                GeomagneticPoint {
                    x_secs: 0.0,
                    hp30: Some(6.333),
                    kp: Some(5.0),
                },
                GeomagneticPoint {
                    x_secs: 1.0,
                    hp30: Some(2.667),
                    kp: Some(2.667),
                },
            ])),
            slip: Some(SlipRatePerPoint {
                all: vec![Some(3.0), Some(3.0)],
                ..SlipRatePerPoint::default()
            }),
            ..TrackQueryData::default()
        };
        let schema = schema_from_files(&[]);
        let query = check_text(
            "points | with mask 15 deg, snr_drop 10, slip_window 5 min | \
             where hp30 > 5 and slip_all > 2 per min",
            &schema,
        )
        .expect("the index compares against a bare number");

        let provider = TrackProvider::new(&points, &[], Some(&data));
        let output = gt_query::run(
            &query,
            &[gt_query::TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(
            output.summary.match_count, 1,
            "only the fix under the storm"
        );
    }

    /// The correlation query the TEC metric exists for: a fix under a storm
    /// ionosphere that also lost lock. Only the fix under the enhanced TEC
    /// matches, so the metric is what narrows the result.
    #[test]
    fn a_tec_and_slip_correlation_checks_and_runs_end_to_end() {
        let points = test_points();
        let data = TrackQueryData {
            tec: Some(Arc::new(vec![
                TecPoint {
                    x_secs: 0.0,
                    tecu: Some(112.5),
                },
                TecPoint {
                    x_secs: 1.0,
                    tecu: Some(18.4),
                },
            ])),
            slip: Some(SlipRatePerPoint {
                all: vec![Some(3.0), Some(3.0)],
                ..SlipRatePerPoint::default()
            }),
            ..TrackQueryData::default()
        };
        let schema = schema_from_files(&[]);
        let query = check_text(
            "points | with mask 15 deg, snr_drop 10, slip_window 5 min | \
             where tec > 100 and slip_all > 2 per min",
            &schema,
        )
        .expect("TEC compares against a bare number");

        let provider = TrackProvider::new(&points, &[], Some(&data));
        let output = gt_query::run(
            &query,
            &[gt_query::TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(
            output.summary.match_count, 1,
            "only the fix under the enhanced ionosphere"
        );
    }

    #[test]
    fn slice_provider_offsets_and_bounds() {
        let points = test_points();
        let slice = SliceProvider::new(TrackProvider::new(&points, &[], None), 1, 1);
        assert_eq!(slice.len(), 1);
        // Index 0 of the slice is point 1 of the track (the bare point).
        assert_eq!(slice.value(QueryMetric::Lat, 0), Some(55.6));
        assert_eq!(slice.value(QueryMetric::Lat, 1), None, "out of the slice");
    }

    #[test]
    fn channel_span_converts_units_and_filters_time() {
        // A g-valued scalar accel channel: channel_span converts each sample to
        // base m/s2 and keeps only those whose absolute time lands in the span.
        let base = TEST_EPOCH as f64;
        let accel = scalar_channel(
            "accel",
            Some("g"),
            &[(0, 1.0), (1, 1.5), (2, 2.0), (3, 0.5)],
        );
        let channels = [accel];
        let points = test_points();
        let provider = TrackProvider::new(&points, &channels, None);

        let got = provider.channel_span("accel", base, base + 2.0);
        // The first three samples (the fourth is past t_hi), each g -> m/s2, one
        // column (scalar).
        let g = Unit::G.to_base();
        let want = [1.0 * g, 1.5 * g, 2.0 * g];
        assert_eq!(got.columns, 1);
        assert_eq!(got.values.len(), want.len());
        for (a, b) in got.values.iter().zip(want) {
            assert!((a - b).abs() < 1e-9, "{a} != {b}");
        }
    }

    #[test]
    fn channel_span_reads_vector_rows() {
        // A vector channel returns row-major values, all columns per row, each
        // converted to base; an unknown channel yields nothing.
        let base = TEST_EPOCH as f64;
        let accel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.0, 2.0, 3.0]), (1, [1.1, 2.2, 3.3])],
        );
        let channels = [accel];
        let points = test_points();
        let provider = TrackProvider::new(&points, &channels, None);

        let g = Unit::G.to_base();
        let got = provider.channel_span("accel", base, base + 1.0);
        assert_eq!(got.columns, 3);
        let want = [1.0, 2.0, 3.0, 1.1, 2.2, 3.3];
        assert_eq!(got.values.len(), want.len());
        for (a, raw) in got.values.iter().zip(want) {
            assert!((a - raw * g).abs() < 1e-9, "{a}");
        }
        assert!(
            provider
                .channel_span("missing", 0.0, f64::MAX)
                .values
                .is_empty()
        );
    }

    #[test]
    fn slice_provider_channel_span_ignores_the_index_offset() {
        // Channels are absolute-time-keyed, so a SliceProvider selects the same
        // samples as its inner provider regardless of the point-index start.
        let base = TEST_EPOCH as f64;
        let accel = scalar_channel("accel", Some("g"), &[(0, 1.0), (1, 1.5), (2, 2.0)]);
        let channels = [accel];
        let points = test_points();
        let inner = TrackProvider::new(&points, &channels, None);
        let slice = SliceProvider::new(inner, 1, 1);

        // The span [base, base+1] holds the first two samples through either
        // provider; the slice's start must not shift the time window.
        assert_eq!(
            slice.channel_span("accel", base, base + 1.0),
            inner.channel_span("accel", base, base + 1.0),
        );
        let g = Unit::G.to_base();
        let want = [1.0 * g, 1.5 * g];
        let got = slice.channel_span("accel", base, base + 1.0);
        assert_eq!(got.values.len(), want.len());
        for (a, b) in got.values.iter().zip(want) {
            assert!((a - b).abs() < 1e-9, "{a} != {b}");
        }
    }

    #[test]
    fn channel_timeline_serves_the_whole_channel_in_base_units() {
        // The channel-source timeline carries every sample's time and value,
        // converted to base units (g -> m/s2), independent of the point slice.
        let base = TEST_EPOCH as f64;
        let accel = scalar_channel("accel", Some("g"), &[(0, 1.0), (1, 2.0)]);
        let channels = [accel];
        let points = test_points();
        let provider = TrackProvider::new(&points, &channels, None);

        let timeline = provider.channel_timeline("accel");
        assert_eq!(timeline.columns, 1);
        assert_eq!(timeline.times.len(), 2);
        assert!((timeline.times[0] - base).abs() < 1e-6);
        let g = Unit::G.to_base();
        assert!((timeline.values[0] - g).abs() < 1e-9);
        assert!((timeline.values[1] - 2.0 * g).abs() < 1e-9);
        // Unknown channel yields an empty timeline.
        assert!(provider.channel_timeline("missing").times.is_empty());
    }

    #[test]
    fn a_loaded_channel_checks_and_runs_end_to_end() {
        // The whole app path: build the editor schema from the file, check a
        // channel query against it, then run it over a provider carrying the
        // same channel. The peak sample (1.5 g) clears the 1 g threshold.
        let channel = scalar_channel("accel", Some("g"), &[(0, 0.9), (1, 1.5)]);
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("points | window 2 | where max(@accel) > 1 g", &schema)
            .expect("checks against the loaded schema");

        let points = test_points();
        let channels = [channel];
        let provider = TrackProvider::new(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[gt_query::TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.matches.len(), 1, "the window matches");
        assert_eq!(output.summary.match_count, 1);
    }

    #[test]
    fn a_vector_component_checks_and_runs_end_to_end() {
        // The whole app path for a vector component: build the schema, check
        // @accel.y, then run over a provider carrying the vector. Only the y
        // column (peak 1.5 g) clears the threshold; x (0.9) would not.
        let channel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [0.9, 0.9, 0.9]), (1, [0.9, 1.5, 0.9])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("points | window 2 | where max(@accel.y) > 1 g", &schema)
            .expect("a component checks against the loaded schema");

        let points = test_points();
        let channels = [channel];
        let provider = TrackProvider::new(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[gt_query::TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.matches.len(), 1, "the y column clears the threshold");
    }

    #[test]
    fn an_si_prefixed_channel_unit_checks_and_runs_end_to_end() {
        // The whole app path for SI prefixes on both sides: a channel spec'd
        // in mg (the usual IMU datasheet unit) against an mg literal. Sample
        // 1 (80 mg) clears the 50 mg threshold; sample 0 (20 mg) does not,
        // pinning that the channel values scale by the prefixed label too.
        let channel = vector_channel(
            "accel",
            Some("mg"),
            &["x", "y", "z"],
            &[(0, [20.0, 0.0, 0.0]), (1, [80.0, 0.0, 0.0])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("points | window 2 | where max(@accel.x) > 50 mg", &schema)
            .expect("an mg channel compares to an mg literal");
        // The same channel against a g literal: the units share the quantity.
        check_text("points | window 2 | where max(@accel.x) > 0.05 g", &schema)
            .expect("an mg channel compares to a g literal");

        let points = test_points();
        let channels = [channel];
        let provider = TrackProvider::new(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[gt_query::TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.summary.match_count, 1, "only the 80 mg sample");
    }

    #[test]
    fn norm_of_a_loaded_vector_checks_and_runs_end_to_end() {
        // norm(@accel) over a loaded vector: row 0 is (3,4,0) -> 5 m/s2, well
        // over 0.1 g (0.981 m/s2), so the window matches.
        let channel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [3.0, 4.0, 0.0]), (1, [0.1, 0.0, 0.0])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text(
            "points | window 2 | where max(norm(@accel)) > 0.1 g",
            &schema,
        )
        .expect("norm checks against the loaded schema");

        let points = test_points();
        let channels = [channel];
        let provider = TrackProvider::new(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[gt_query::TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(
            output.matches.len(),
            1,
            "the magnitude clears the threshold"
        );
    }
}
