#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use geotrace_sdk::{
    Angle, BuildError, Channel, ChannelError, ChannelUnit, NavFileBuilder, Unit, UnitParseError,
};
use geotrace_sdk_test_util as test_util;

#[test]
fn a_channel_name_must_be_a_lowercase_identifier() {
    for bad in ["Accel Fwd", "accel-fwd", "", "1accel", "Accel"] {
        assert!(
            matches!(
                Channel::builder()
                    .name(bad)
                    .times(vec![test_util::base()])
                    .values(vec![1.0])
                    .build(),
                Err(ChannelError::InvalidName { .. })
            ),
            "expected {bad:?} to be rejected"
        );
    }
    Channel::builder()
        .name("accel_fwd2")
        .times(vec![test_util::base()])
        .values(vec![1.0])
        .build()
        .expect("a lowercase identifier with digits and underscores is valid");
}

#[test]
fn an_invalid_custom_unit_is_rejected() {
    assert!(matches!(
        ChannelUnit::custom("   "),
        Err(UnitParseError::EmptyCustom)
    ));
}

#[test]
fn legacy_invalid_unit_metadata_cannot_be_new_writer_input() {
    let result = Channel::builder()
        .name("legacy")
        .unit(ChannelUnit::from_file_label("bad\nunit"))
        .times(vec![test_util::base()])
        .values(vec![1.0])
        .build();

    assert!(matches!(result, Err(ChannelError::UnwritableUnit { .. })));
}

#[test]
fn channel_period_requires_a_positive_angular_unit() {
    let build = |unit: Option<ChannelUnit>, period: Option<Angle>| {
        Channel::builder()
            .name("bearing")
            .maybe_unit(unit)
            .maybe_period(period)
            .times(vec![test_util::base()])
            .values(vec![10.0])
            .build()
    };

    assert!(matches!(
        build(Some(Unit::G.into()), Some(Angle::degrees(360.0))),
        Err(ChannelError::PeriodNeedsAngularUnit { .. })
    ));
    assert!(matches!(
        build(Some(Unit::DEG.into()), Some(Angle::degrees(0.0))),
        Err(ChannelError::InvalidPeriod { .. })
    ));
    build(Some(Unit::DEG.into()), Some(Angle::degrees(360.0)))
        .expect("positive angular period is valid");
}

#[test]
fn duplicate_channel_names_are_rejected() {
    let mut recorder = NavFileBuilder::new().open();
    for value in [1.0, 2.0] {
        recorder.add_channel(
            Channel::builder()
                .name("accel")
                .times(vec![test_util::base()])
                .values(vec![value])
                .build()
                .expect("valid channel"),
        );
    }
    assert!(matches!(
        recorder.finish(),
        Err(BuildError::DuplicateChannelName { name }) if name == "accel"
    ));
}

#[test]
fn a_channel_rejects_mismatched_lengths() {
    let err = Channel::builder()
        .name("accel")
        .times(vec![test_util::base()])
        .values(vec![1.0, 2.0])
        .build()
        .expect_err("two values but one timestamp");
    assert!(matches!(
        err,
        ChannelError::LengthMismatch {
            expected: 1,
            actual: 2,
            ..
        }
    ));
}

#[test]
fn a_malformed_vector_channel_is_rejected() {
    let times = vec![test_util::base()];
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(Vec::<String>::new())
            .times(times.clone())
            .values(vec![1.0])
            .build(),
        Err(ChannelError::EmptyComponents { .. })
    ));
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(["x", "Y"])
            .times(times.clone())
            .values(vec![1.0, 2.0])
            .build(),
        Err(ChannelError::InvalidComponent { .. })
    ));
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(["x", "x"])
            .times(times.clone())
            .values(vec![1.0, 2.0])
            .build(),
        Err(ChannelError::DuplicateComponent { .. })
    ));
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(["x", "y", "z"])
            .times(times)
            .values(vec![1.0, 2.0])
            .build(),
        Err(ChannelError::LengthMismatch {
            expected: 3,
            actual: 2,
            ..
        })
    ));
}

#[test]
fn channel_components_accept_any_stringlike_iterable() -> Result<(), Box<dyn std::error::Error>> {
    let times = vec![test_util::base()];
    let values = vec![1.0, 2.0, 3.0];

    let from_array = Channel::builder()
        .name("accel")
        .components(["x", "y", "z"])
        .times(times.clone())
        .values(values.clone())
        .build()?;
    let from_vec_of_str = Channel::builder()
        .name("accel")
        .components(vec!["x", "y", "z"])
        .times(times.clone())
        .values(values.clone())
        .build()?;
    let from_owned = Channel::builder()
        .name("accel")
        .components(vec!["x".to_owned(), "y".to_owned(), "z".to_owned()])
        .times(times)
        .values(values)
        .build()?;

    assert_eq!(from_array, from_vec_of_str);
    assert_eq!(from_array, from_owned);
    assert_eq!(from_array.components(), &["x", "y", "z"]);
    Ok(())
}
