use chrono::Duration;
use proptest::prelude::*;
use rstest::rstest;

use super::{AssociationWindow, InvalidAssociationWindow};

#[rstest]
#[case::zero(Duration::zero(), Ok(AssociationWindow::ZERO))]
#[case::maximum(Duration::days(365), Ok(AssociationWindow::MAX))]
#[case::negative(Duration::nanoseconds(-1), Err(InvalidAssociationWindow))]
#[case::above_maximum(Duration::days(365) + Duration::nanoseconds(1), Err(InvalidAssociationWindow))]
#[case::minimum_duration(Duration::MIN, Err(InvalidAssociationWindow))]
#[case::maximum_duration(Duration::MAX, Err(InvalidAssociationWindow))]
fn construction_enforces_inclusive_bounds(
    #[case] duration: Duration,
    #[case] expected: Result<AssociationWindow, InvalidAssociationWindow>,
) {
    assert_eq!(AssociationWindow::try_from(duration), expected);
}

#[rstest]
#[case::zero(0, Ok(AssociationWindow::ZERO))]
#[case::maximum(31_536_000, Ok(AssociationWindow::MAX))]
#[case::above_maximum(31_536_001, Err(InvalidAssociationWindow))]
#[case::signed_overflow(u64::MAX, Err(InvalidAssociationWindow))]
#[case::duration_overflow(i64::MAX as u64, Err(InvalidAssociationWindow))]
fn seconds_input_enforces_bounds_without_overflow(
    #[case] seconds: u64,
    #[case] expected: Result<AssociationWindow, InvalidAssociationWindow>,
) {
    assert_eq!(AssociationWindow::try_from_seconds(seconds), expected);
}

proptest! {
    #[test]
    fn construction_preserves_every_valid_nanosecond(nanoseconds in 0i64..=31_536_000_000_000_000) {
        let duration = Duration::nanoseconds(nanoseconds);
        let window = AssociationWindow::try_from(duration).expect("valid duration");
        prop_assert_eq!(Duration::from(window), duration);
    }
}
