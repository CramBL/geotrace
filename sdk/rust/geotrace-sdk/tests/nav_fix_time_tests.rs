use geotrace_sdk::{DateTime, Duration, NavFixTime, RecordedFixTimestamps, Utc};
use rstest::rstest;

#[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
fn receiver_stamp() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn host_stamp() -> DateTime<Utc> {
    receiver_stamp() + Duration::milliseconds(250)
}

#[rstest]
#[case::receiver(
    NavFixTime::Receiver(receiver_stamp()),
    Some(receiver_stamp()),
    None,
    receiver_stamp()
)]
#[case::host(NavFixTime::Host(host_stamp()), None, Some(host_stamp()), host_stamp())]
#[case::both(
    NavFixTime::Both { gps: receiver_stamp(), sys: host_stamp() },
    Some(receiver_stamp()),
    Some(host_stamp()),
    receiver_stamp()
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
    RecordedFixTimestamps { gps: Some(receiver_stamp()), sys: None },
    Some(NavFixTime::Receiver(receiver_stamp()))
)]
#[case::host(
    RecordedFixTimestamps { gps: None, sys: Some(host_stamp()) },
    Some(NavFixTime::Host(host_stamp()))
)]
#[case::both(
    RecordedFixTimestamps { gps: Some(receiver_stamp()), sys: Some(host_stamp()) },
    Some(NavFixTime::Both { gps: receiver_stamp(), sys: host_stamp() })
)]
#[case::neither(RecordedFixTimestamps { gps: None, sys: None }, None)]
fn a_recorded_pair_resolves_to_the_clocks_it_holds(
    #[case] recorded: RecordedFixTimestamps,
    #[case] expected: Option<NavFixTime>,
) {
    assert_eq!(NavFixTime::from_recorded(recorded), expected);
}
