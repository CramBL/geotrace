use std::collections::{BTreeMap, VecDeque};

use gt_types::nav_point::NavPoint;
use gt_types::satellites::{
    Constellation, ConstellationSet, SatSample, Satellites, Slip, SlipCause,
};
use gt_types::{
    Prn, SatelliteCoverage, SatelliteIdentity, SatelliteObservationReport, SatelliteQueryScope,
    SatelliteReportMetadata, SatelliteScope,
};

use crate::{DiagnosticRates, ObservationEpoch, SlipAnalysisConfig, SlipEpoch, SlipThresholds};

/// Detect the slips at the current report `curr` relative to the previous one
/// `prev`, under the elevation mask and SNR-drop threshold.
pub fn slips_between(
    prev: &Satellites,
    curr: &Satellites,
    mask_deg: f32,
    snr_drop_db: f32,
) -> Vec<Slip> {
    let thresholds = SlipThresholds {
        elevation_mask_deg: mask_deg,
        snr_drop_db,
    };
    prev.satellites()
        .filter_map(|previous| {
            let current = curr.satellites().find(|current| {
                current.constellation() == previous.constellation()
                    && current.prn() == previous.prn()
            });
            let transition = SatelliteTransition {
                from: SatSample::of(previous),
                to: current.map(SatSample::of),
                absence_is_known: true,
            };
            let SlipComparison::Slip(cause) = transition.compare(thresholds) else {
                return None;
            };
            Some(Slip {
                constellation: previous.constellation(),
                prn: previous.prn(),
                cause,
                from: transition.from,
                to: transition.to,
            })
        })
        .collect()
}

pub fn slip_epochs(points: &[NavPoint], config: SlipAnalysisConfig) -> Vec<SlipEpoch> {
    let mut history = ObservationHistory::default();
    let mut out = Vec::new();
    let mut previous_time = None;
    let mut previous_fix_time = None;
    for (point_index, point) in points.iter().enumerate() {
        if previous_fix_time.is_some_and(|time| point.tpv.time() < time) {
            history = ObservationHistory::default();
            previous_time = None;
        }
        previous_fix_time = Some(point.tpv.time());
        for (observation_index, report) in point.satellite_evidence.observations.iter().enumerate()
        {
            let time = report.metadata.normalized_time;
            if previous_time.is_some_and(|previous| time < previous) {
                history = ObservationHistory::default();
            }
            previous_time = Some(time);
            let (slips, rates) = history.advance(report, config);
            out.push(SlipEpoch {
                source: ObservationEpoch {
                    point_index,
                    observation_index,
                    metadata: report.metadata,
                },
                slips,
                rates,
            });
        }
    }
    out
}

pub fn detect_slip_events(
    points: &[NavPoint],
    mask_deg: f32,
    snr_drop_db: f32,
) -> Vec<(usize, Vec<Slip>)> {
    slip_epochs(
        points,
        SlipAnalysisConfig {
            thresholds: SlipThresholds {
                elevation_mask_deg: mask_deg,
                snr_drop_db,
            },
            rate_window_min: None,
        },
    )
    .into_iter()
    .filter(|epoch| !epoch.slips.is_empty())
    .map(|epoch| (epoch.source.point_index, epoch.slips))
    .collect()
}

/// Slip-rate-per-minute point series, combined and per constellation.  Each
/// series is `[unix_seconds, slips_per_minute]` at eligible report epochs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlipSeries {
    pub all: Vec<[f64; 2]>,
    pub gps: Vec<[f64; 2]>,
    pub glonass: Vec<[f64; 2]>,
    pub galileo: Vec<[f64; 2]>,
    pub beidou: Vec<[f64; 2]>,
    pub navic: Vec<[f64; 2]>,
    pub qzss: Vec<[f64; 2]>,
}

pub fn slip_rate_series(
    points: &[NavPoint],
    mask_deg: f32,
    snr_drop_db: f32,
    window_min: f32,
) -> SlipSeries {
    let mut out = SlipSeries::default();
    for epoch in slip_epochs(
        points,
        SlipAnalysisConfig {
            thresholds: SlipThresholds {
                elevation_mask_deg: mask_deg,
                snr_drop_db,
            },
            rate_window_min: Some(window_min),
        },
    ) {
        let t = epoch
            .source
            .metadata
            .normalized_time
            .as_secs_f64_with_subseconds();
        for (series, rate) in [
            (&mut out.all, epoch.rates.all),
            (
                &mut out.gps,
                epoch
                    .rates
                    .get(SatelliteQueryScope::Constellation(Constellation::Gps)),
            ),
            (
                &mut out.glonass,
                epoch
                    .rates
                    .get(SatelliteQueryScope::Constellation(Constellation::Glonass)),
            ),
            (
                &mut out.galileo,
                epoch
                    .rates
                    .get(SatelliteQueryScope::Constellation(Constellation::Galileo)),
            ),
            (
                &mut out.beidou,
                epoch
                    .rates
                    .get(SatelliteQueryScope::Constellation(Constellation::Beidou)),
            ),
            (
                &mut out.navic,
                epoch
                    .rates
                    .get(SatelliteQueryScope::Constellation(Constellation::Navic)),
            ),
            (
                &mut out.qzss,
                epoch
                    .rates
                    .get(SatelliteQueryScope::Constellation(Constellation::Qzss)),
            ),
        ] {
            if let Some(rate) = rate {
                series.push([t, rate]);
            }
        }
    }
    out
}

/// Per-point slip rates (slips per minute), aligned with `points` by index.
///
/// Insufficient comparison evidence produces `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlipRatePerPoint {
    pub all: Vec<Option<f64>>,
    pub gps: Vec<Option<f64>>,
    pub glonass: Vec<Option<f64>>,
    pub galileo: Vec<Option<f64>>,
    pub beidou: Vec<Option<f64>>,
    pub navic: Vec<Option<f64>>,
    pub qzss: Vec<Option<f64>>,
}

/// Select an eligible attached report for each point and requested scope.
pub fn slip_rate_per_point(
    points: &[NavPoint],
    mask_deg: f32,
    snr_drop_db: f32,
    window_min: f32,
) -> SlipRatePerPoint {
    let epochs = slip_epochs(
        points,
        SlipAnalysisConfig {
            thresholds: SlipThresholds {
                elevation_mask_deg: mask_deg,
                snr_drop_db,
            },
            rate_window_min: Some(window_min),
        },
    );
    let scatter = |scope| {
        let mut out = vec![None; points.len()];
        let mut selected = vec![None; points.len()];
        for epoch in &epochs {
            let index = epoch.source.point_index;
            let Some(value) = epoch.rates.get(scope) else {
                continue;
            };
            let Some(point) = points.get(index) else {
                continue;
            };
            let Some(previous) = selected.get_mut(index) else {
                continue;
            };
            if previous.is_none_or(|metadata: SatelliteReportMetadata| {
                epoch
                    .source
                    .metadata
                    .compare_for_fix(&metadata, point.tpv.time())
                    .is_lt()
            }) {
                *previous = Some(epoch.source.metadata);
                if let Some(slot) = out.get_mut(index) {
                    *slot = Some(value);
                }
            }
        }
        out
    };
    SlipRatePerPoint {
        all: scatter(SatelliteQueryScope::Global),
        gps: scatter(SatelliteQueryScope::Constellation(Constellation::Gps)),
        glonass: scatter(SatelliteQueryScope::Constellation(Constellation::Glonass)),
        galileo: scatter(SatelliteQueryScope::Constellation(Constellation::Galileo)),
        beidou: scatter(SatelliteQueryScope::Constellation(Constellation::Beidou)),
        navic: scatter(SatelliteQueryScope::Constellation(Constellation::Navic)),
        qzss: scatter(SatelliteQueryScope::Constellation(Constellation::Qzss)),
    }
}

#[derive(Default)]
struct ObservationHistory {
    satellites: BTreeMap<(Constellation, Prn), SatSample>,
    complete_scopes: Vec<SatelliteScope>,
    events: VecDeque<(f64, Constellation)>,
    event_counts: BTreeMap<Constellation, usize>,
}

impl ObservationHistory {
    fn advance(
        &mut self,
        report: &SatelliteObservationReport,
        config: SlipAnalysisConfig,
    ) -> (Vec<Slip>, DiagnosticRates) {
        let comparisons = self.compare_report(report, config.thresholds);
        let time = report
            .metadata
            .normalized_time
            .as_secs_f64_with_subseconds();
        let window_min = config
            .rate_window_min
            .filter(|window| window.is_finite() && *window > 0.0);
        if let Some(window_min) = window_min {
            self.update_event_window(&comparisons.slips, time, window_min);
        }
        let rates = DiagnosticRates::for_scopes(|scope| {
            let window_min = window_min?;
            let unavailable = match scope {
                SatelliteQueryScope::Global => !comparisons.unavailable.is_empty(),
                SatelliteQueryScope::Constellation(constellation) => {
                    comparisons.unavailable.contains(constellation)
                }
            };
            let prior_coverage = self
                .complete_scopes
                .iter()
                .any(|previous| previous.covers(scope));
            if !report.metadata.complete_for(scope) || !prior_coverage || unavailable {
                return None;
            }
            let count = match scope {
                SatelliteQueryScope::Global => self.events.len(),
                SatelliteQueryScope::Constellation(constellation) => {
                    self.event_counts.get(&constellation).copied().unwrap_or(0)
                }
            };
            Some(count as f64 / f64::from(window_min))
        });
        self.record_report(report);
        (comparisons.slips, rates)
    }

    fn update_event_window(&mut self, slips: &[Slip], time: f64, window_min: f32) {
        for slip in slips {
            self.events.push_back((time, slip.constellation));
            *self.event_counts.entry(slip.constellation).or_default() += 1;
        }
        let lower = time - f64::from(window_min) * SECS_PER_MIN;
        while self
            .events
            .front()
            .is_some_and(|(event_time, _)| *event_time <= lower)
        {
            if let Some((_, constellation)) = self.events.pop_front()
                && let Some(count) = self.event_counts.get_mut(&constellation)
            {
                debug_assert!(*count > 0);
                *count -= 1;
            }
        }
    }

    fn record_report(&mut self, report: &SatelliteObservationReport) {
        if report.metadata.coverage == SatelliteCoverage::Complete {
            self.satellites.retain(|&(constellation, prn), _| {
                !report
                    .metadata
                    .scope
                    .covers(SatelliteQueryScope::Constellation(constellation))
                    || report.satellites.iter().any(|satellite| {
                        satellite.identity == SatelliteIdentity { constellation, prn }
                    })
            });
            if !self.complete_scopes.contains(&report.metadata.scope) {
                self.complete_scopes.push(report.metadata.scope);
            }
        }
        for satellite in &report.satellites {
            self.satellites.insert(
                (satellite.identity.constellation, satellite.identity.prn),
                satellite.measurements,
            );
        }
    }

    fn compare_report(
        &self,
        report: &SatelliteObservationReport,
        thresholds: SlipThresholds,
    ) -> ReportComparisons {
        let mut slips = Vec::new();
        let mut unavailable = ConstellationSet::empty();
        for (&(constellation, prn), previous) in &self.satellites {
            let scope = SatelliteQueryScope::Constellation(constellation);
            if !report.metadata.scope.covers(scope) {
                continue;
            }
            let current = report
                .satellites
                .iter()
                .find(|satellite| satellite.identity == SatelliteIdentity { constellation, prn });
            let transition = SatelliteTransition {
                from: *previous,
                to: current.map(|satellite| satellite.measurements),
                absence_is_known: report.metadata.complete_for(scope),
            };
            let comparison = transition.compare(thresholds);
            if matches!(comparison, SlipComparison::Unavailable) {
                unavailable.insert(constellation);
            }
            if let SlipComparison::Slip(cause) = comparison {
                slips.push(Slip {
                    constellation,
                    prn,
                    cause,
                    from: transition.from,
                    to: transition.to,
                });
            }
        }
        ReportComparisons { slips, unavailable }
    }
}

struct ReportComparisons {
    slips: Vec<Slip>,
    unavailable: ConstellationSet,
}

enum SlipComparison {
    NoSlip,
    Slip(SlipCause),
    Unavailable,
}

struct SatelliteTransition {
    from: SatSample,
    to: Option<SatSample>,
    absence_is_known: bool,
}

impl SatelliteTransition {
    fn compare(&self, thresholds: SlipThresholds) -> SlipComparison {
        let Some(elevation) = self.from.elevation else {
            return SlipComparison::Unavailable;
        };
        if elevation < thresholds.elevation_mask_deg {
            return SlipComparison::NoSlip;
        }
        let Some(current) = self.to else {
            return if self.absence_is_known {
                SlipComparison::Slip(SlipCause::LostLock)
            } else {
                SlipComparison::Unavailable
            };
        };
        let Some(elevation) = current.elevation else {
            return SlipComparison::Unavailable;
        };
        if elevation < thresholds.elevation_mask_deg {
            return SlipComparison::NoSlip;
        }
        let Some((previous, current)) = self.from.measured_snr().zip(current.measured_snr()) else {
            return SlipComparison::Unavailable;
        };
        if previous.value() - current.value() > thresholds.snr_drop_db {
            SlipComparison::Slip(SlipCause::SnrDrop)
        } else {
            SlipComparison::NoSlip
        }
    }
}

/// Seconds per minute, for converting the slip window (minutes) to the seconds
/// the epoch timestamps are measured in. Public so callers converting in the
/// opposite direction share the same factor.
pub const SECS_PER_MIN: f64 = 60.0;

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::fixtures::{self, FixKind};
    use gt_types::satellites::{NO_DATA_SENTINEL_DB_HZ, Satellite, Snr};
    use rstest::rstest;

    use super::*;
    use crate::test_util;

    /// A satellite of `constellation` the receiver tracked at
    /// `elevation_deg` with `snr`, without using it in the fix. Only
    /// [`slips_between`]'s previous-report gate reads the fix flag, and it
    /// reads the elevation instead.
    fn tracked(
        constellation: Constellation,
        prn: u32,
        elevation_deg: Option<f32>,
        snr: Option<Snr>,
    ) -> Satellite {
        fixtures::satellite(constellation, prn, elevation_deg, snr, false)
    }

    #[test]
    fn lost_lock_when_above_mask_satellite_disappears() {
        let prev = test_util::report(vec![tracked(
            Constellation::Gps,
            7,
            Some(40.0),
            Some(Snr::new(45.0)),
        )]);
        let curr = test_util::report(vec![]);
        let slips = slips_between(&prev, &curr, 15.0, 10.0);
        assert_eq!(slips.len(), 1);
        let slip = slips.first().expect("one slip");
        assert_eq!(slip.cause, SlipCause::LostLock);
        assert_eq!(slip.constellation, Constellation::Gps);
        assert_eq!(slip.prn, 7);
        assert_eq!(slip.from.elevation, Some(40.0));
        assert_eq!(slip.to, None);
    }

    #[test]
    fn no_lost_lock_for_satellite_below_mask_or_unknown_elevation() {
        // A satellite setting below the mask, then gone, is a natural set.
        let prev = test_util::report(vec![
            tracked(Constellation::Gps, 7, Some(5.0), Some(Snr::new(45.0))),
            tracked(Constellation::Gps, 8, None, Some(Snr::new(45.0))),
        ]);
        let curr = test_util::report(vec![]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    #[test]
    fn snr_drop_above_threshold_is_a_slip_and_records_from_to() {
        let prev = test_util::report(vec![tracked(
            Constellation::Galileo,
            3,
            Some(30.0),
            Some(Snr::new(45.0)),
        )]);
        let curr = test_util::report(vec![tracked(
            Constellation::Galileo,
            3,
            Some(28.0),
            Some(Snr::new(30.0)),
        )]);
        let slips = slips_between(&prev, &curr, 15.0, 10.0);
        assert_eq!(slips.len(), 1);
        let slip = slips.first().expect("one slip");
        assert_eq!(slip.cause, SlipCause::SnrDrop);
        assert_eq!(slip.from.snr.map(|s| s.value()), Some(45.0));
        assert_eq!(slip.to.and_then(|t| t.snr).map(|s| s.value()), Some(30.0));
        assert_eq!(slip.to.map(|t| t.elevation), Some(Some(28.0)));
    }

    #[test]
    fn snr_drop_at_or_below_threshold_is_not_a_slip() {
        // Exactly the threshold does not count (strictly greater required).
        let prev = test_util::report(vec![tracked(
            Constellation::Gps,
            1,
            Some(30.0),
            Some(Snr::new(45.0)),
        )]);
        let curr = test_util::report(vec![tracked(
            Constellation::Gps,
            1,
            Some(30.0),
            Some(Snr::new(35.0)),
        )]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    #[test]
    fn snr_drop_ignored_when_satellite_falls_below_mask() {
        let prev = test_util::report(vec![tracked(
            Constellation::Gps,
            1,
            Some(20.0),
            Some(Snr::new(45.0)),
        )]);
        let curr = test_util::report(vec![tracked(
            Constellation::Gps,
            1,
            Some(5.0),
            Some(Snr::new(20.0)),
        )]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    /// The fall from the no-data value to a measured reading is no signal
    /// loss: firmware reports about 99 dB-Hz for "no measurement".
    #[rstest]
    #[case::the_previous_epoch_holds_the_no_data_value(
        Some(Snr::new(NO_DATA_SENTINEL_DB_HZ)),
        Some(Snr::new(30.0))
    )]
    #[case::the_current_epoch_holds_the_no_data_value(
        Some(Snr::new(45.0)),
        Some(Snr::new(NO_DATA_SENTINEL_DB_HZ))
    )]
    fn the_no_data_snr_value_is_no_snr_drop(
        #[case] previous_snr: Option<Snr>,
        #[case] current_snr: Option<Snr>,
    ) {
        let prev = test_util::report(vec![tracked(
            Constellation::Gps,
            1,
            Some(30.0),
            previous_snr,
        )]);
        let curr = test_util::report(vec![tracked(
            Constellation::Gps,
            1,
            Some(30.0),
            current_snr,
        )]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    #[test]
    fn rising_snr_and_steady_lock_yield_no_slips() {
        let prev = test_util::report(vec![tracked(
            Constellation::Beidou,
            5,
            Some(40.0),
            Some(Snr::new(30.0)),
        )]);
        let curr = test_util::report(vec![tracked(
            Constellation::Beidou,
            5,
            Some(41.0),
            Some(Snr::new(48.0)),
        )]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    /// A GPS satellite in the fix at `elevation_deg` and `snr_db`.
    fn gps(prn: u32, elevation_deg: f32, snr_db: f32) -> Satellite {
        fixtures::satellite(
            Constellation::Gps,
            prn,
            Some(elevation_deg),
            Some(Snr::new(snr_db)),
            true,
        )
    }

    #[test]
    fn detect_slip_events_groups_all_slips_at_one_epoch() {
        // Sat 1 stays above the mask throughout. Sats 2 and 3 are both lost at
        // index 1 and produce one grouped event at that epoch.
        let points = vec![
            test_util::point_at(
                0,
                vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0), gps(3, 25.0, 38.0)],
            ),
            test_util::point_at(1_000, vec![gps(1, 40.0, 45.0)]),
        ];
        let events = detect_slip_events(&points, MASK_DEG, SNR_DROP_DB);
        assert_eq!(events.len(), 1, "one grouped event, not one per satellite");
        let (index, slips) = events.first().expect("one event");
        assert_eq!(*index, 1);
        assert_eq!(slips.len(), 2);
        let mut prns: Vec<u32> = slips.iter().map(|s| s.prn.value()).collect();
        prns.sort_unstable();
        assert_eq!(prns, vec![2, 3]);
        assert!(slips.iter().all(|s| s.cause == SlipCause::LostLock));
    }

    #[rstest]
    #[case::a_second_between_the_fixes(1_000, vec![[1.0, 1.0]])]
    #[case::half_a_second_between_the_fixes(500, vec![[0.5, 1.0]])]
    fn slip_rate_series_counts_one_slip_per_minute_window(
        #[case] second_fix_millis: i64,
        #[case] expected: Vec<[f64; 2]>,
    ) {
        let points = vec![
            test_util::point_at(0, vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0)]),
            test_util::point_at(second_fix_millis, vec![gps(1, 40.0, 45.0)]),
        ];
        let s = slip_rate_series(&points, MASK_DEG, SNR_DROP_DB, WINDOW_MIN);
        assert_eq!(s.all, expected);
        assert_eq!(s.gps, expected);
        assert!(s.glonass.iter().all(|p| p[1] == 0.0));
    }

    #[test]
    fn backward_time_steps_reset_comparisons_and_rates() {
        let points = vec![
            test_util::point_at(120_000, vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0)]),
            test_util::point_at(60_000, vec![gps(1, 40.0, 45.0)]),
        ];
        assert!(detect_slip_events(&points, MASK_DEG, SNR_DROP_DB).is_empty());
        assert!(
            slip_rate_series(&points, MASK_DEG, SNR_DROP_DB, WINDOW_MIN)
                .all
                .is_empty()
        );
    }

    /// The per-point form is index-aligned: reportless points hold `None`,
    /// and every value matches the time-keyed series.
    #[test]
    fn slip_rate_per_point_aligns_with_series() {
        let reportless = fixtures::nav_point(
            DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(1),
            Latitude::new(55.0),
            Longitude::new(12.0),
            FixKind::GhostWithoutHeading,
        );
        let points = vec![
            test_util::point_at(0, vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0)]),
            reportless,
            test_util::point_at(2_000, vec![gps(1, 40.0, 45.0)]),
        ];

        let per_point = slip_rate_per_point(&points, MASK_DEG, SNR_DROP_DB, WINDOW_MIN);
        assert_eq!(per_point.all, vec![None, None, Some(1.0)]);
        assert_eq!(per_point.gps, vec![None, None, Some(1.0)]);

        let series = slip_rate_series(&points, MASK_DEG, SNR_DROP_DB, WINDOW_MIN);
        let series_values: Vec<f64> = series.all.iter().map(|[_, v]| *v).collect();
        let aligned_values: Vec<f64> = per_point.all.iter().copied().flatten().collect();
        assert_eq!(series_values, aligned_values);
    }

    #[test]
    fn slip_rate_per_point_with_zero_window_is_all_none() {
        let points = vec![test_util::point_at(0, vec![gps(1, 40.0, 45.0)])];
        let per_point = slip_rate_per_point(&points, MASK_DEG, SNR_DROP_DB, 0.0);
        assert_eq!(per_point.all, vec![None]);
    }

    /// A NavIC slip lands in `s.navic`, a QZSS slip in `s.qzss`, and neither
    /// leaks into the GPS series - the new constellations get their own buckets.
    #[test]
    fn navic_and_qzss_slips_route_into_their_own_series() {
        let in_fix = |constellation, prn, elevation_deg, snr_db: f32| {
            fixtures::satellite(
                constellation,
                prn,
                Some(elevation_deg),
                Some(Snr::new(snr_db)),
                true,
            )
        };
        let points = vec![
            test_util::point_at(
                0,
                vec![
                    in_fix(Constellation::Navic, 3, 40.0, 45.0),
                    in_fix(Constellation::Qzss, 5, 35.0, 44.0),
                ],
            ),
            // Both lost at t=1 while above the mask -> one slip each.
            test_util::point_at(1_000, vec![]),
        ];
        let s = slip_rate_series(&points, MASK_DEG, SNR_DROP_DB, WINDOW_MIN);
        assert_eq!(s.navic, vec![[1.0, 1.0]]);
        assert_eq!(s.qzss, vec![[1.0, 1.0]]);
        assert!(s.gps.iter().all(|p| p[1] == 0.0));
        // Two slips total at the same epoch.
        assert_eq!(s.all, vec![[1.0, 2.0]]);
    }

    /// Elevation mask, in degrees.
    const MASK_DEG: f32 = 15.0;

    /// SNR fall that counts as a slip, in dB-Hz.
    const SNR_DROP_DB: f32 = 10.0;

    /// Trailing window of the rate, in minutes.
    const WINDOW_MIN: f32 = 1.0;
}
