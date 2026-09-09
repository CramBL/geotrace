use chrono::TimeDelta;
use gt_types::track::FixStats;
use rstest::rstest;

#[rstest]
#[case::rounded_down(4_800, 900, 84)]
#[case::rounded_up(85, 15, 85)]
#[case::a_fix_the_whole_time(4_800, 0, 100)]
#[case::no_time_recorded(0, 0, 0)]
fn a_fix_percentage_rounds_to_the_nearest_whole_percent(
    #[case] seconds_with_fix: i64,
    #[case] seconds_without_fix: i64,
    #[case] expected: u32,
) {
    let stats = FixStats {
        time_with_fix: TimeDelta::seconds(seconds_with_fix),
        time_without_fix: TimeDelta::seconds(seconds_without_fix),
        fix_loss_count: 0,
        max_continuous_no_fix: TimeDelta::zero(),
    };

    assert_eq!(crate::fix_percentage(stats), expected);
}

#[test]
fn a_fix_percentage_reading_ends_in_the_word_fix() {
    let stats = FixStats {
        time_with_fix: TimeDelta::seconds(85),
        time_without_fix: TimeDelta::seconds(15),
        fix_loss_count: 0,
        max_continuous_no_fix: TimeDelta::seconds(15),
    };

    assert_eq!(crate::format_fix_percentage(stats), "85% fix");
}

#[rstest]
#[case::a_whole_percent(0.87, "87%")]
#[case::below_half_a_percent(0.874, "87%")]
#[case::exactly_half_a_percent(0.875, "88%")]
#[case::nothing(0.0, "0%")]
#[case::everything(1.0, "100%")]
#[case::below_zero_clamps(-0.5, "0%")]
#[case::above_one_clamps(1.5, "100%")]
#[case::not_a_number(f64::NAN, "0%")]
fn a_fraction_reads_as_a_whole_percent_rounded_half_up_and_clamped(
    #[case] fraction: f64,
    #[case] expected: &str,
) {
    assert_eq!(crate::format_fraction_percent(fraction), expected);
}

#[test]
fn a_fix_held_the_whole_time_adds_no_tooltip_details() {
    let stats = FixStats {
        time_with_fix: TimeDelta::seconds(4_800),
        time_without_fix: TimeDelta::zero(),
        fix_loss_count: 0,
        max_continuous_no_fix: TimeDelta::zero(),
    };

    assert_eq!(crate::format_fix_tooltip_details(stats), "");
}

#[test]
fn the_tooltip_details_state_the_time_without_fix_the_losses_and_the_longest_gap() {
    let stats = FixStats {
        time_with_fix: TimeDelta::seconds(4_800),
        time_without_fix: TimeDelta::seconds(900),
        fix_loss_count: 3,
        max_continuous_no_fix: TimeDelta::seconds(480),
    };

    assert_eq!(
        crate::format_fix_tooltip_details(stats),
        "  ·  15m w/o fix  ·  3 losses  ·  max gap 8m"
    );
}
