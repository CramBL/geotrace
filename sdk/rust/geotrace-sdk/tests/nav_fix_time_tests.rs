use geotrace_sdk::{DateTime, NavFixTime, RecordedFixTimestamps, Utc};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

#[rstest]
#[case::receiver(
    NavFixTime::Receiver(test_util::base()),
    Some(test_util::base()),
    None,
    test_util::base()
)]
#[case::host(
    NavFixTime::Host(test_util::t_ms(250)),
    None,
    Some(test_util::t_ms(250)),
    test_util::t_ms(250)
)]
#[case::both(
    NavFixTime::Both { gps: test_util::base(), sys: test_util::t_ms(250) },
    Some(test_util::base()),
    Some(test_util::t_ms(250)),
    test_util::base()
)]
fn a_fix_time_reads_back_the_clocks_that_stamped_it(
    #[case] time: NavFixTime,
    #[case] gps_time: Option<DateTime<Utc>>,
    #[case] sys_time: Option<DateTime<Utc>>,
    #[case] effective: DateTime<Utc>,
) {
    assert_eq!(time.gps_time(), gps_time);
    assert_eq!(time.sys_time(), sys_time);
    assert_eq!(time.effective(), effective);
}

#[rstest]
#[case::receiver(
    RecordedFixTimestamps { gps: Some(test_util::base()), sys: None },
    Some(NavFixTime::Receiver(test_util::base()))
)]
#[case::host(
    RecordedFixTimestamps { gps: None, sys: Some(test_util::t_ms(250)) },
    Some(NavFixTime::Host(test_util::t_ms(250)))
)]
#[case::both(
    RecordedFixTimestamps { gps: Some(test_util::base()), sys: Some(test_util::t_ms(250)) },
    Some(NavFixTime::Both { gps: test_util::base(), sys: test_util::t_ms(250) })
)]
#[case::neither(RecordedFixTimestamps { gps: None, sys: None }, None)]
fn a_recorded_pair_resolves_to_the_clocks_it_holds(
    #[case] recorded: RecordedFixTimestamps,
    #[case] expected: Option<NavFixTime>,
) {
    assert_eq!(NavFixTime::from_recorded(recorded), expected);
}
