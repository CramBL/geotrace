use gt_ui_components::{
    ActionPresentation, ColumnBudget, ColumnFallback, ColumnPlan, ColumnRole, ColumnSpec,
};
use rstest::rstest;

#[rstest]
#[case::wide(400.0, &[0, 1, 2, 3, 4], 170.0, ActionPresentation::Full, ColumnFallback::Fits)]
#[case::compact_actions(300.0, &[0, 1, 2, 3, 4], 120.0, ActionPresentation::Compact, ColumnFallback::Fits)]
#[case::hide_low_priority(270.0, &[0, 1, 2, 4], 150.0, ActionPresentation::Compact, ColumnFallback::Fits)]
#[case::core(230.0, &[0, 1, 4], 140.0, ActionPresentation::Compact, ColumnFallback::Fits)]
#[case::scroll(200.0, &[0, 1, 4], 120.0, ActionPresentation::Compact, ColumnFallback::HorizontalScroll)]
fn responsive_allocation_preserves_primary_and_required_columns(
    #[case] width: f32,
    #[case] selected: &[u8],
    #[case] primary_width: f32,
    #[case] actions: ActionPresentation,
    #[case] fallback: ColumnFallback,
) {
    let plan = ColumnPlan::allocate(
        &COLUMNS,
        ColumnBudget {
            available_width: width,
            gap: 10.0,
            pixels_per_point: 1.0,
        },
    );
    assert_eq!(
        plan.columns
            .iter()
            .map(|column| column.key)
            .collect::<Vec<_>>(),
        selected
    );
    assert_eq!(
        plan.columns.first().expect("primary").width.to_bits(),
        primary_width.to_bits()
    );
    assert_eq!(plan.actions, actions);
    assert_eq!(plan.fallback, fallback);
    assert!(plan.columns.iter().all(|column| column.width > 0.0));
}

#[rstest]
#[case::normal(1.0)]
#[case::scaled(1.5)]
#[case::larger(2.0)]
fn oversized_optional_values_preserve_readable_primary_width(#[case] scale: f32) {
    let mut columns = COLUMNS;
    columns.get_mut(2).expect("optional column").minimum_width = 10_000.0;
    let plan = ColumnPlan::allocate(
        &columns,
        ColumnBudget {
            available_width: 350.1,
            gap: 10.0,
            pixels_per_point: scale,
        },
    );
    assert_eq!(
        plan.columns
            .iter()
            .map(|column| column.key)
            .collect::<Vec<_>>(),
        [0, 1, 3, 4]
    );
    assert_eq!(plan.actions, ActionPresentation::Full);
    assert_eq!(plan.fallback, ColumnFallback::Fits);
    assert!(plan.columns.first().expect("primary").width >= 120.0);
    assert!(plan.width <= 350.1);
}

#[test]
#[should_panic(expected = "column plans support at most one action column")]
fn multiple_action_columns_violate_the_shared_presentation_invariant() {
    let mut columns = COLUMNS;
    columns.get_mut(2).expect("optional column").role = ColumnRole::Actions {
        compact_width: 20.0,
    };
    ColumnPlan::allocate(
        &columns,
        ColumnBudget {
            available_width: 220.0,
            gap: 0.0,
            pixels_per_point: 1.0,
        },
    );
}

const COLUMNS: [ColumnSpec<u8>; 5] = [
    ColumnSpec {
        key: 0,
        role: ColumnRole::Primary,
        minimum_width: 120.0,
    },
    ColumnSpec {
        key: 1,
        role: ColumnRole::Required,
        minimum_width: 40.0,
    },
    ColumnSpec {
        key: 2,
        role: ColumnRole::Optional { priority: 0 },
        minimum_width: 20.0,
    },
    ColumnSpec {
        key: 3,
        role: ColumnRole::Optional { priority: 1 },
        minimum_width: 50.0,
    },
    ColumnSpec {
        key: 4,
        role: ColumnRole::Actions {
            compact_width: 30.0,
        },
        minimum_width: 80.0,
    },
];
