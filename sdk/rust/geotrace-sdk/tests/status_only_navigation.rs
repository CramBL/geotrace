use geotrace_sdk::{NavFileBuilder, NavFix, NavFixTime, SolutionState};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

#[rstest]
#[case::fix(SolutionState::Fix)]
#[case::loss(SolutionState::NoFix)]
#[case::estimated(SolutionState::EstimatedOnly)]
#[case::unknown(SolutionState::Unknown)]
fn status_only_navigation_round_trips_without_position_or_optional_measurements(
    #[case] state: SolutionState,
) {
    let times = [
        NavFixTime::Receiver(test_util::base()),
        NavFixTime::Host(test_util::t_ms(2000)),
        NavFixTime::Both {
            gps: test_util::t_ms(5000),
            sys: test_util::t_ms(5100),
        },
    ];
    let mut recorder = NavFileBuilder::new().open();
    for time in times {
        recorder.add(NavFix::status_only(time, state));
    }
    let source = recorder.finish().unwrap();
    let file = test_util::round_trip(&source).unwrap();
    assert!(source.equals_ignoring_build_provenance(&file));
    assert_eq!(file.nav_points().len(), times.len());
    for (point, time) in file.nav_points().iter().zip(times) {
        let fix = point.fix;
        assert_eq!(fix.time, time);
        assert_eq!(fix.state, Some(state));
        assert!(fix.lat.as_degrees().is_nan());
        assert!(fix.lon.as_degrees().is_nan());
        assert_eq!(fix.position_provenance, None);
        assert_eq!(fix.heading, None);
        assert_eq!(fix.speed, None);
        assert_eq!(fix.eph_m, None);
        assert_eq!(fix.used_count, None);
        assert_eq!(fix.method, None);
        assert_eq!(fix.dimension, None);
        assert_eq!((fix.hdop, fix.vdop, fix.pdop), (None, None, None));
        assert!(point.satellites().is_none());
        assert!(point.satellite_evidence().usage.is_empty());
        assert!(point.satellite_evidence().observations.is_empty());
    }
}
