use geotrace_sdk::{
    Angle, BuildError, Constellation, DateTime, Duration, NavFile, NavFileBuilder, NavFix,
    NavFixTime, Satellite, SatelliteCoverage, SatelliteIdentity, SatelliteMembership,
    SatelliteObservation, SatelliteObservationReport, SatelliteReport, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport, SolutionState,
};

pub fn headingless_navigation_recording() -> Result<NavFile, BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    for (second, latitude, longitude) in [(0, 0.0, 0.0), (5, 10.0, 10.0), (10, 0.0, 1.0)] {
        let time = DateTime::UNIX_EPOCH + Duration::seconds(second);
        let state = if second == 5 {
            SolutionState::Unknown
        } else {
            SolutionState::Fix
        };
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(latitude))
                .lon(Angle::degrees(longitude))
                .state(state)
                .hdop(0.9)
                .build(),
        );
        if second != 5 {
            recorder.add_satellite_report(
                SatelliteReport::builder()
                    .time(NavFixTime::Receiver(time))
                    .tracked(vec![
                        Satellite::builder()
                            .constellation(Constellation::Gps)
                            .prn(1)
                            .in_fix(true)
                            .build(),
                    ])
                    .build(),
            );
        }
    }
    for offset in [100, 400] {
        recorder.add_satellite_usage_report(
            SatelliteUsageReport::builder()
                .time(NavFixTime::Receiver(
                    DateTime::UNIX_EPOCH + Duration::milliseconds(5_000 + offset),
                ))
                .scope(SatelliteScope::Global)
                .coverage(SatelliteCoverage::Partial)
                .reported_count(8)
                .satellites(Vec::new())
                .build(),
        );
    }
    recorder.add_satellite_observation_report(
        SatelliteObservationReport::builder()
            .time(NavFixTime::Receiver(
                DateTime::UNIX_EPOCH + Duration::milliseconds(5_200),
            ))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .reported_count(12)
            .satellites(Vec::new())
            .build(),
    );
    recorder.finish()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationDetail {
    FullObservations,
    Summaries,
    UsageIdentities,
}

pub fn progressive_navigation_recording(detail: NavigationDetail) -> Result<NavFile, BuildError> {
    let mut recorder = NavFileBuilder::new().with_scrubbed_provenance().open();
    for (index, (second, state, used_count)) in [
        (0, SolutionState::Fix, 8),
        (10, SolutionState::NoFix, 12),
        (30, SolutionState::Fix, 8),
        (40, SolutionState::Unknown, 0),
        (50, SolutionState::EstimatedOnly, 8),
        (60, SolutionState::Fix, 8),
    ]
    .into_iter()
    .enumerate()
    {
        let time = NavFixTime::Receiver(DateTime::UNIX_EPOCH + Duration::seconds(second));
        recorder.add(
            NavFix::builder()
                .time(time)
                .lat(Angle::degrees(55.0 + index as f64 * 0.001))
                .lon(Angle::degrees(12.0 + (index % 2) as f64 * 0.001))
                .state(state)
                .used_count(used_count)
                .hdop(0.9)
                .build(),
        );
        if detail != NavigationDetail::Summaries {
            recorder.add(
                SatelliteUsageReport::builder()
                    .time(time)
                    .scope(SatelliteScope::Global)
                    .coverage(SatelliteCoverage::Complete)
                    .satellites(
                        (1..=12)
                            .map(|prn| {
                                SatelliteUsage::new(
                                    SatelliteIdentity::new(Constellation::Gps, prn),
                                    if prn <= 8 {
                                        SatelliteMembership::Used
                                    } else {
                                        SatelliteMembership::Unused
                                    },
                                )
                            })
                            .collect(),
                    )
                    .build(),
            );
        }
        if detail == NavigationDetail::FullObservations {
            recorder.add(
                SatelliteObservationReport::builder()
                    .time(time)
                    .scope(SatelliteScope::Global)
                    .coverage(SatelliteCoverage::Complete)
                    .satellites(
                        (1..=12)
                            .map(|prn| {
                                SatelliteObservation::builder()
                                    .identity(SatelliteIdentity::new(Constellation::Gps, prn))
                                    .elevation(35.0)
                                    .azimuth(prn as f32 * 20.0)
                                    .snr(42.0)
                                    .build()
                            })
                            .collect(),
                    )
                    .build(),
            );
        }
    }
    recorder.finish()
}
