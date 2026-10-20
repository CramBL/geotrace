use geotrace_sdk::{
    Angle, BuildError, Constellation, DateTime, Duration, NavFile, NavFileBuilder, NavFix,
    NavFixTime, Satellite, SatelliteCoverage, SatelliteObservationReport, SatelliteReport,
    SatelliteScope, SatelliteUsageReport, SolutionState,
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
