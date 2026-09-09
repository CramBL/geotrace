use rstest::rstest;
use uom::si::f64::Length;
use uom::si::length::kilometer;

#[rstest]
#[case::nothing(0.0, "0.0")]
#[case::below_the_first_step(0.04, "0.0")]
#[case::the_first_step(0.05, "0.1")]
#[case::kilometres(4.63, "4.6")]
#[case::thousands(1_234.56, "1234.6")]
fn a_kilometre_reading_keeps_one_decimal(#[case] km: f64, #[case] expected: &str) {
    assert_eq!(
        crate::format_kilometers(Length::new::<kilometer>(km)),
        expected
    );
}
