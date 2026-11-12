use crate::{BootTime, SysTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemBootTimePair {
    pub system_time: SysTime,
    pub boot_time: BootTime,
}

impl SystemBootTimePair {
    /// Positive values mean system time advanced farther than boot time since the reference.
    pub fn system_boot_delta_since(self, reference: Self) -> SystemBootDelta {
        let system_elapsed_us = i128::from(self.system_time.utc().timestamp_micros())
            - i128::from(reference.system_time.utc().timestamp_micros());
        let boot_elapsed_us =
            i128::from(self.boot_time.as_micros()) - i128::from(reference.boot_time.as_micros());
        SystemBootDelta(system_elapsed_us - boot_elapsed_us)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemBootDelta(i128);

impl SystemBootDelta {
    pub fn as_micros(self) -> i128 {
        self.0
    }

    pub fn as_millis_f64(self) -> f64 {
        self.0 as f64 / MICROS_PER_MILLISECOND
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / MICROS_PER_SECOND
    }
}

const MICROS_PER_MILLISECOND: f64 = 1_000.0;
const MICROS_PER_SECOND: f64 = 1_000_000.0;

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    #[derive(Clone, Copy)]
    struct PairFixture {
        system_us: i64,
        boot_us: u64,
    }

    impl PairFixture {
        fn pair(self) -> SystemBootTimePair {
            SystemBootTimePair {
                system_time: SysTime::from_utc(
                    DateTime::from_timestamp_micros(self.system_us).expect("valid timestamp"),
                ),
                boot_time: BootTime::from_micros(self.boot_us).expect("valid boot time"),
            }
        }
    }

    #[derive(Clone, Copy)]
    struct DeltaFixture {
        reference: PairFixture,
        sample: PairFixture,
        expected_us: i128,
    }

    #[rstest]
    #[case::matching_advancements(DeltaFixture {
        reference: PairFixture { system_us: MODERN_UTC_US, boot_us: 0 },
        sample: PairFixture { system_us: MODERN_UTC_US + 1_000_000, boot_us: 1_000_000 },
        expected_us: 0,
    })]
    #[case::system_advances_farther(DeltaFixture {
        reference: PairFixture { system_us: MODERN_UTC_US, boot_us: 0 },
        sample: PairFixture { system_us: MODERN_UTC_US + 1_001_000, boot_us: 1_000_000 },
        expected_us: 1_000,
    })]
    #[case::boot_advances_farther(DeltaFixture {
        reference: PairFixture { system_us: MODERN_UTC_US, boot_us: 0 },
        sample: PairFixture { system_us: MODERN_UTC_US + 999_000, boot_us: 1_000_000 },
        expected_us: -1_000,
    })]
    #[case::one_microsecond_at_large_uptime(DeltaFixture {
        reference: PairFixture { system_us: MODERN_UTC_US, boot_us: u64::MAX - 2 },
        sample: PairFixture { system_us: MODERN_UTC_US + 2, boot_us: u64::MAX - 1 },
        expected_us: 1,
    })]
    #[case::boot_backward_step(DeltaFixture {
        reference: PairFixture { system_us: MODERN_UTC_US, boot_us: u64::MAX - 1 },
        sample: PairFixture { system_us: MODERN_UTC_US + 1, boot_us: 0 },
        expected_us: i128::from(u64::MAX),
    })]
    #[case::system_backward_step_and_repeated_boot_reading(DeltaFixture {
        reference: PairFixture { system_us: MODERN_UTC_US, boot_us: 0 },
        sample: PairFixture { system_us: MODERN_UTC_US - 1, boot_us: 0 },
        expected_us: -1,
    })]
    fn delta_retains_signed_microseconds(#[case] fixture: DeltaFixture) {
        let delta = fixture
            .sample
            .pair()
            .system_boot_delta_since(fixture.reference.pair());
        assert_eq!(delta.as_micros(), fixture.expected_us);
        assert_eq!(delta.as_millis_f64(), fixture.expected_us as f64 / 1_000.0);
        assert_eq!(
            delta.as_secs_f64(),
            fixture.expected_us as f64 / 1_000_000.0
        );
    }

    #[test]
    fn delta_supports_the_full_timestamp_and_boot_ranges() {
        let reference = SystemBootTimePair {
            system_time: SysTime::from_utc(DateTime::<Utc>::MIN_UTC),
            boot_time: BootTime::from_micros(u64::MAX - 1).expect("valid boot time"),
        };
        let sample = SystemBootTimePair {
            system_time: SysTime::from_utc(DateTime::<Utc>::MAX_UTC),
            boot_time: BootTime::from_micros(0).expect("valid boot time"),
        };
        let delta = sample.system_boot_delta_since(reference);
        let timestamp_span = i128::from(DateTime::<Utc>::MAX_UTC.timestamp_micros())
            - i128::from(DateTime::<Utc>::MIN_UTC.timestamp_micros());
        assert_eq!(delta.as_micros(), timestamp_span + i128::from(u64::MAX - 1));
        assert!(delta.as_micros() > i128::from(u64::MAX));
    }

    proptest! {
        #[test]
        fn delta_is_antisymmetric_and_zero_at_reference(
            reference_system_us in -MAX_TIMESTAMP_US..MAX_TIMESTAMP_US,
            sample_system_us in -MAX_TIMESTAMP_US..MAX_TIMESTAMP_US,
            reference_boot_us in 0..u64::MAX,
            sample_boot_us in 0..u64::MAX,
        ) {
            let reference = PairFixture {
                system_us: reference_system_us,
                boot_us: reference_boot_us,
            }.pair();
            let sample = PairFixture {
                system_us: sample_system_us,
                boot_us: sample_boot_us,
            }.pair();
            prop_assert_eq!(sample.system_boot_delta_since(reference).as_micros(),
                -reference.system_boot_delta_since(sample).as_micros());
            prop_assert_eq!(reference.system_boot_delta_since(reference).as_micros(), 0);
        }
    }

    const MODERN_UTC_US: i64 = 1_700_000_000_000_000;
    const MAX_TIMESTAMP_US: i64 = 8_000_000_000_000_000_000;
}
