use chrono::TimeDelta;
use gt_types::FixStats;
use rstest::rstest;

#[rstest]
#[case::rounded_down(4_800, 900, 0, 0, Some(84), "84% availability")]
#[case::rounded_up(85, 15, 0, 0, Some(85), "85% availability")]
#[case::empty(0, 0, 0, 0, None, "— availability")]
#[case::unknown(0, 0, 0, 60, None, "— availability")]
#[case::estimated_is_known_absence(20, 20, 10, 10, Some(40), "40% availability")]
fn navigation_availability_uses_known_duration(
    #[case] fix: i64,
    #[case] no_fix: i64,
    #[case] estimated: i64,
    #[case] unknown: i64,
    #[case] percent: Option<u32>,
    #[case] text: &str,
) {
    let stats = FixStats {
        time_with_fix: TimeDelta::seconds(fix),
        time_without_fix: TimeDelta::seconds(no_fix),
        time_estimated_only: TimeDelta::seconds(estimated),
        time_unknown: TimeDelta::seconds(unknown),
        ..FixStats::default()
    };
    assert_eq!(
        crate::NavigationSummary(stats).availability_percentage(),
        percent
    );
    assert_eq!(crate::NavigationSummary(stats).availability_label(), text);
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
fn navigation_summary_reports_coverage_duration_buckets_and_absence() {
    let stats = FixStats {
        time_with_fix: TimeDelta::seconds(20),
        time_without_fix: TimeDelta::seconds(20),
        time_estimated_only: TimeDelta::seconds(10),
        time_unknown: TimeDelta::seconds(10),
        fix_loss_count: 1,
        max_continuous_no_fix: TimeDelta::seconds(20),
    };
    assert_eq!(
        crate::NavigationSummary(stats).details(),
        "83% known-state coverage  ·  20s no fix  ·  10s estimated only  ·  10s unknown  ·  1 loss  ·  longest absence 20s"
    );
    let mut aggregate = stats;
    aggregate += FixStats {
        time_with_fix: TimeDelta::seconds(50),
        ..FixStats::default()
    };
    assert_eq!(
        crate::NavigationSummary(aggregate).availability_label(),
        "70% availability"
    );
    assert!(
        crate::NavigationSummary(aggregate)
            .details()
            .starts_with("91% known-state coverage")
    );
}
