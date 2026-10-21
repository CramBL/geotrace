#![expect(clippy::panic_in_result_fn, reason = "test assertions")]

use geotrace_sdk::{
    Angle, Constellation, DebugTimeRepair, Duration, NavFile, NavFileBuilder, NavFileOpenMode,
    NavFix, NavFixTime, PositionProvenance, SatelliteCoverage, SatelliteIdentity,
    SatelliteMembership, SatelliteObservation, SatelliteObservationReport, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport, SolutionState,
};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

fn fix(time: NavFixTime) -> NavFix {
    NavFix::builder()
        .time(time)
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .state(SolutionState::Fix)
        .used_count(8)
        .hdop(0.9)
        .build()
}

fn usage(time: NavFixTime) -> SatelliteUsageReport {
    SatelliteUsageReport::builder()
        .time(time)
        .scope(SatelliteScope::Global)
        .coverage(SatelliteCoverage::Partial)
        .reported_count(8)
        .satellites(vec![SatelliteUsage::new(
            SatelliteIdentity::new(Constellation::Gps, 1),
            SatelliteMembership::Unknown,
        )])
        .build()
}

fn observations(time: NavFixTime) -> SatelliteObservationReport {
    SatelliteObservationReport::builder()
        .time(time)
        .scope(SatelliteScope::Global)
        .coverage(SatelliteCoverage::Partial)
        .satellites(vec![
            SatelliteObservation::builder()
                .identity(SatelliteIdentity::new(Constellation::Gps, 1))
                .snr(0.0)
                .build(),
        ])
        .build()
}

#[rstest]
#[case::inclusive_boundary(500, 1)]
#[case::outside_boundary(501, 2)]
fn independent_reports_use_the_configured_window(
    #[case] offset_ms: i64,
    #[case] point_count: usize,
    #[values(false, true)] host_clock: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let time = |offset_ms| {
        if host_clock {
            NavFixTime::Host(test_util::t_ms(offset_ms))
        } else {
            NavFixTime::Receiver(test_util::t_ms(offset_ms))
        }
    };
    let source = fix(time(0));
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(source);
    recorder.add_satellite_usage_report(usage(time(offset_ms)));
    recorder.add_satellite_observation_report(observations(time(offset_ms)));
    let file = recorder.finish()?;
    assert_eq!(file.nav_points().len(), point_count);
    assert_eq!(file.nav_points()[0].fix, source);
    let evidence = file
        .nav_points()
        .last()
        .expect("point")
        .satellite_evidence();
    assert_eq!(evidence.usage.len(), 1);
    assert_eq!(evidence.observations.len(), 1);
    assert_eq!(
        evidence.usage[0]
            .placement
            .expect("placement")
            .normalized_time,
        test_util::t_ms(offset_ms)
    );
    assert_eq!(
        evidence.usage[0].placement.expect("placement").input_order,
        0
    );
    assert_eq!(
        evidence.observations[0]
            .placement
            .expect("placement")
            .input_order,
        1
    );
    assert_eq!(test_util::round_trip(&file)?, file);
    Ok(())
}

#[test]
fn sparse_polling_preserves_source_fixes_and_report_times() -> Result<(), Box<dyn std::error::Error>>
{
    let mut recorder = NavFileBuilder::new().open();
    let fixes: Vec<_> = (0..=10)
        .map(|second| fix(NavFixTime::Receiver(test_util::t_ms(second * 1000))))
        .collect();
    for source in &fixes {
        recorder.add(*source);
    }
    for second in [10, 0, 5] {
        recorder.add(observations(NavFixTime::Receiver(test_util::t_ms(
            second * 1000 + 400,
        ))));
    }
    let file = recorder.finish()?;
    assert_eq!(
        file.nav_points()
            .iter()
            .map(|point| point.fix)
            .collect::<Vec<_>>(),
        fixes
    );
    for (index, point) in file.nav_points().iter().enumerate() {
        let reports = &point.satellite_evidence().observations;
        assert_eq!(reports.len(), usize::from(index % 5 == 0));
        assert!(point.satellite_evidence().usage.is_empty());
        if let Some(report) = reports.first() {
            assert_eq!(
                report.time,
                NavFixTime::Receiver(test_util::t_ms(index as i64 * 1000 + 400))
            );
            assert_eq!(
                report.placement.expect("placement").normalized_time,
                test_util::t_ms(index as i64 * 1000 + 400)
            );
        }
    }
    assert_eq!(test_util::round_trip(&file)?, file);
    Ok(())
}

struct HostPolls {
    usage_ms: &'static [i64],
    observation_ms: &'static [i64],
}

#[rstest]
#[case::usage_only(HostPolls { usage_ms: &[2000], observation_ms: &[] })]
#[case::observation_only(HostPolls { usage_ms: &[], observation_ms: &[2000] })]
#[case::equal_time_kinds(HostPolls { usage_ms: &[2000], observation_ms: &[2000] })]
#[case::distinct_times(HostPolls { usage_ms: &[9000], observation_ms: &[1000] })]
fn host_only_orphans_preserve_times_and_interpolation_fractions(
    #[case] polls: HostPolls,
) -> Result<(), Box<dyn std::error::Error>> {
    let sources = [0, 10_000].map(|time_ms| {
        NavFix::builder()
            .time(NavFixTime::Host(test_util::t_ms(time_ms)))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(time_ms as f64 / 1000.0))
            .build()
    });
    let mut recorder = NavFileBuilder::new().open();
    for source in sources {
        recorder.add(source);
    }
    for &time_ms in polls.usage_ms {
        recorder.add(usage(NavFixTime::Host(test_util::t_ms(time_ms))));
    }
    for &time_ms in polls.observation_ms {
        recorder.add(observations(NavFixTime::Host(test_util::t_ms(time_ms))));
    }
    let recorded = recorder.finish()?;
    let decoded = test_util::round_trip(&recorded)?;
    assert_eq!(decoded, recorded);
    let points = decoded.nav_points();
    assert_eq!(points.first().expect("first source").fix, sources[0]);
    assert_eq!(points.last().expect("last source").fix, sources[1]);
    let mut carrier_times: Vec<_> = polls
        .usage_ms
        .iter()
        .chain(polls.observation_ms)
        .copied()
        .collect();
    carrier_times.sort_unstable();
    carrier_times.dedup();
    assert_eq!(points.len(), carrier_times.len() + sources.len());
    for (carrier, time_ms) in points[1..points.len() - 1].iter().zip(carrier_times) {
        let expected_time = test_util::t_ms(time_ms);
        assert_eq!(carrier.fix.time, NavFixTime::Receiver(expected_time));
        assert_eq!(carrier.fix.lat, Angle::degrees(0.0));
        assert!((carrier.fix.lon.as_degrees() - time_ms as f64 / 1000.0).abs() < 1e-12);
        assert_eq!(
            carrier.fix.position_provenance,
            Some(PositionProvenance::SdkSynthesized)
        );
        let evidence = carrier.satellite_evidence();
        assert_eq!(
            evidence.usage.len(),
            usize::from(polls.usage_ms.contains(&time_ms))
        );
        assert_eq!(
            evidence.observations.len(),
            usize::from(polls.observation_ms.contains(&time_ms))
        );
        for (report_time, placement) in evidence
            .usage
            .iter()
            .map(|report| (report.time, report.placement))
            .chain(
                evidence
                    .observations
                    .iter()
                    .map(|report| (report.time, report.placement)),
            )
        {
            assert_eq!(report_time, NavFixTime::Host(expected_time));
            assert_eq!(placement.expect("placement").normalized_time, expected_time);
        }
        for report in &evidence.usage {
            assert_eq!(
                report.placement.expect("placement").input_order,
                polls
                    .usage_ms
                    .iter()
                    .position(|&time| time == time_ms)
                    .expect("usage input") as u64
            );
        }
        for report in &evidence.observations {
            assert_eq!(
                report.placement.expect("placement").input_order,
                (polls.usage_ms.len()
                    + polls
                        .observation_ms
                        .iter()
                        .position(|&time| time == time_ms)
                        .expect("observation input")) as u64
            );
        }
    }
    Ok(())
}

#[test]
fn all_nearby_reports_remain_in_input_order() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(fix(NavFixTime::Receiver(test_util::t_ms(1000))));
    recorder.add(fix(NavFixTime::Receiver(test_util::t_ms(0))));
    recorder.add(observations(NavFixTime::Receiver(test_util::t_ms(500))));
    recorder.add(usage(NavFixTime::Receiver(test_util::t_ms(400))));
    recorder.add(usage(NavFixTime::Receiver(test_util::t_ms(100))));
    recorder.add(observations(NavFixTime::Receiver(test_util::t_ms(200))));
    let file = recorder.finish()?;
    assert_eq!(file.nav_points().len(), 2);
    let evidence = file.nav_points()[0].satellite_evidence();
    assert_eq!(
        evidence
            .usage
            .iter()
            .map(|report| (
                report.time,
                report.placement.expect("placement").input_order
            ))
            .collect::<Vec<_>>(),
        vec![
            (NavFixTime::Receiver(test_util::t_ms(400)), 1),
            (NavFixTime::Receiver(test_util::t_ms(100)), 2)
        ]
    );
    assert_eq!(
        evidence
            .observations
            .iter()
            .map(|report| (
                report.time,
                report.placement.expect("placement").input_order
            ))
            .collect::<Vec<_>>(),
        vec![
            (NavFixTime::Receiver(test_util::t_ms(500)), 0),
            (NavFixTime::Receiver(test_util::t_ms(200)), 3)
        ]
    );
    assert!(
        file.nav_points()[1]
            .satellite_evidence()
            .observations
            .is_empty()
    );
    assert_eq!(test_util::round_trip(&file)?, file);
    Ok(())
}

#[test]
fn host_polling_uses_the_nearest_host_fix_across_changing_clock_offsets()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    for (gps_ms, host_ms) in [(0, 10_000), (100, 0), (200, 100)] {
        recorder.add(fix(NavFixTime::Both {
            gps: test_util::t_ms(gps_ms),
            sys: test_util::t_ms(host_ms),
        }));
    }
    recorder.add(observations(NavFixTime::Host(test_util::t_ms(10_300))));
    let file = recorder.finish()?;
    assert_eq!(file.nav_points().len(), 3);
    let report = &file.nav_points()[0].satellite_evidence().observations[0];
    assert_eq!(report.time, NavFixTime::Host(test_util::t_ms(10_300)));
    assert_eq!(
        report.placement.expect("placement").normalized_time,
        test_util::t_ms(300)
    );
    assert!(
        file.nav_points()[1..]
            .iter()
            .all(|point| point.satellite_evidence().observations.is_empty())
    );
    assert_eq!(test_util::round_trip(&file)?, file);
    Ok(())
}

#[test]
fn host_polling_can_use_a_delta_for_a_receiver_only_fix() -> Result<(), Box<dyn std::error::Error>>
{
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(fix(NavFixTime::Receiver(test_util::t_ms(0))));
    recorder.add(fix(NavFixTime::Both {
        gps: test_util::t_ms(1000),
        sys: test_util::t_ms(11_000),
    }));
    recorder.add(usage(NavFixTime::Host(test_util::t_ms(10_400))));
    let file = recorder.finish()?;
    assert_eq!(file.nav_points().len(), 2);
    let report = &file.nav_points()[0].satellite_evidence().usage[0];
    assert_eq!(report.time, NavFixTime::Host(test_util::t_ms(10_400)));
    assert_eq!(
        report.placement.expect("placement").normalized_time,
        test_util::t_ms(400)
    );
    Ok(())
}

#[test]
fn equal_normalized_orphan_times_share_one_synthesized_position()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    let before = fix(NavFixTime::Both {
        gps: test_util::t_ms(0),
        sys: test_util::t_ms(10_000),
    });
    let after = NavFix::builder()
        .time(NavFixTime::Both {
            gps: test_util::t_ms(10_000),
            sys: test_util::t_ms(20_000),
        })
        .lat(Angle::degrees(56.0))
        .lon(Angle::degrees(13.0))
        .build();
    recorder.add(before).add(after);
    recorder.add(observations(NavFixTime::Host(test_util::t_ms(13_000))));
    recorder.add(usage(NavFixTime::Receiver(test_util::t_ms(4000))));
    recorder.add(usage(NavFixTime::Receiver(test_util::t_ms(3000))));
    let file = recorder.finish()?;
    assert_eq!(file.nav_points().len(), 4);
    assert_eq!(file.nav_points()[0].fix, before);
    assert_eq!(file.nav_points()[3].fix, after);
    let first_carrier = &file.nav_points()[1];
    assert_eq!(
        first_carrier.fix.time,
        NavFixTime::Receiver(test_util::t_ms(3000))
    );
    assert_eq!(first_carrier.fix.lat, Angle::degrees(55.3));
    assert!((first_carrier.fix.lon.as_degrees() - 12.3).abs() < 1e-12);
    assert_eq!(
        first_carrier.fix.position_provenance,
        Some(PositionProvenance::SdkSynthesized)
    );
    assert!(first_carrier.fix.heading.is_some());
    assert_eq!(first_carrier.fix.state, None);
    assert_eq!(first_carrier.satellite_evidence().usage.len(), 1);
    assert_eq!(first_carrier.satellite_evidence().observations.len(), 1);
    assert_eq!(
        first_carrier.satellite_evidence().observations[0].time,
        NavFixTime::Host(test_util::t_ms(13_000))
    );
    assert_eq!(
        file.nav_points()[2].fix.time,
        NavFixTime::Receiver(test_util::t_ms(4000))
    );
    assert_eq!(test_util::round_trip(&file)?, file);
    Ok(())
}

#[test]
fn observation_only_orphans_supply_no_state_observation() -> Result<(), Box<dyn std::error::Error>>
{
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(fix(NavFixTime::Receiver(test_util::t_ms(0))));
    recorder.add(observations(NavFixTime::Receiver(test_util::t_ms(2000))));
    let file = recorder.finish()?;
    let carrier = &file.nav_points()[1];
    assert_eq!(carrier.fix.state, None);
    assert_eq!(carrier.fix.used_count, None);
    assert_eq!(
        carrier.fix.position_provenance,
        Some(PositionProvenance::SdkSynthesized)
    );
    assert_eq!(carrier.satellites(), &None);
    assert!(carrier.satellite_evidence().usage.is_empty());
    Ok(())
}

#[test]
fn debug_repair_shifts_all_report_clocks_and_placements_once()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    for offset in [0, 3000, 6000] {
        recorder.add(fix(NavFixTime::Receiver(test_util::t_ms(offset))));
        recorder.add(usage(NavFixTime::Receiver(test_util::t_ms(offset + 400))));
        recorder.add(observations(NavFixTime::Both {
            gps: test_util::t_ms(offset + 300),
            sys: test_util::t_ms(offset + 10_300),
        }));
    }
    let mut file = recorder.finish()?;
    let mut repeated = file.nav_points()[0].clone();
    repeated.fix.time = NavFixTime::Receiver(test_util::t_ms(0));
    let mut evidence = repeated.satellite_evidence().clone();
    evidence
        .usage
        .push(usage(NavFixTime::Host(test_util::t_ms(200))));
    repeated.set_satellite_evidence(evidence);
    file.nav_points_mut()[2] = repeated;
    let mut bytes = Vec::new();
    file.write(&mut bytes)?;
    let unchanged = NavFile::read(bytes.as_slice())?;
    assert_eq!(
        unchanged.nav_points()[2].fix.time,
        NavFixTime::Receiver(test_util::t_ms(0))
    );
    let repair = DebugTimeRepair::new(Duration::seconds(2))?;
    let repaired =
        NavFile::read_with_mode(bytes.as_slice(), NavFileOpenMode::DebugTimeRepair(repair))?;
    let point = &repaired.nav_points()[2];
    assert_eq!(point.fix.time, NavFixTime::Receiver(test_util::t_ms(5000)));
    let evidence = point.satellite_evidence();
    assert_eq!(
        evidence.usage[0].time,
        NavFixTime::Receiver(test_util::t_ms(5400))
    );
    assert_eq!(
        evidence.usage[0]
            .placement
            .expect("placement")
            .normalized_time,
        test_util::t_ms(5400)
    );
    assert_eq!(
        evidence.usage[0].placement.expect("placement").input_order,
        0
    );
    assert_eq!(
        evidence.usage[1].time,
        NavFixTime::Host(test_util::t_ms(5200))
    );
    assert_eq!(evidence.usage[1].placement, None);
    assert_eq!(
        evidence.observations[0].time,
        NavFixTime::Both {
            gps: test_util::t_ms(5300),
            sys: test_util::t_ms(15_300)
        }
    );
    assert_eq!(
        evidence.observations[0]
            .placement
            .expect("placement")
            .normalized_time,
        test_util::t_ms(5300)
    );
    file.set_debug_time_repair_tag(repair);
    let mut tagged_bytes = Vec::new();
    file.write(&mut tagged_bytes)?;
    assert_eq!(
        NavFile::read(tagged_bytes.as_slice())?.nav_points(),
        repaired.nav_points()
    );
    Ok(())
}
