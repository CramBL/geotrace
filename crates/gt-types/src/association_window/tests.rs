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

proptest! {
    #[test]
    fn construction_preserves_every_valid_nanosecond(nanoseconds in 0i64..=31_536_000_000_000_000) {
        let duration = Duration::nanoseconds(nanoseconds);
        let window = AssociationWindow::try_from(duration).expect("valid duration");
        prop_assert_eq!(Duration::from(window), duration);
    }
}
