use std::num::NonZeroU64;

use proptest::prelude::*;
use strum::IntoEnumIterator as _;

use gt_types::DisplayMode;

use crate::ast::{
    BinaryOp, ChannelRef, Expr, Func, MetricRef, ModeStage, NumberLit, ParamDecl, ParamName, Query,
    Source, Span, TableSpec, UnaryOp, Window,
};
use crate::unit::Unit;
use crate::{QueryMetric, parser};

fn span() -> Span {
    Span::new(0, 0)
}

fn metric_strategy() -> impl Strategy<Value = QueryMetric> {
    proptest::sample::select(QueryMetric::iter().collect::<Vec<_>>())
}

fn unit_strategy() -> impl Strategy<Value = Unit> {
    // The canonical catalog, which is the set the formatter emits.
    proptest::sample::select(Unit::CANONICAL.to_vec())
}

fn number_strategy() -> impl Strategy<Value = NumberLit> {
    (0.0..1e9f64, proptest::option::of(unit_strategy())).prop_map(|(value, unit)| NumberLit {
        value,
        unit,
        span: span(),
    })
}

fn expr_strategy() -> impl Strategy<Value = Expr> {
    let ident = "[a-z_][a-z0-9_]*";
    let leaf = prop_oneof![
        number_strategy().prop_map(Expr::Number),
        metric_strategy().prop_map(|metric| Expr::Metric(MetricRef {
            metric,
            span: span(),
        })),
        (ident, proptest::option::of(ident)).prop_map(|(name, component)| Expr::Channel(
            ChannelRef {
                name,
                component,
                span: span(),
            }
        )),
    ];
    leaf.prop_recursive(4, 24, 2, |inner| {
        let funcs = proptest::sample::select(Func::iter().collect::<Vec<_>>());
        let ops = proptest::sample::select(vec![
            BinaryOp::Or,
            BinaryOp::And,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
        ]);
        prop_oneof![
            (inner.clone(), inner.clone(), ops).prop_map(|(lhs, rhs, op)| Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span: span(),
            }),
            (inner.clone(), funcs).prop_map(|(arg, func)| Expr::Call {
                func,
                arg: Box::new(arg),
                span: span(),
            }),
            inner.clone().prop_map(|operand| Expr::Unary {
                op: UnaryOp::Not,
                operand: Box::new(operand),
                span: span(),
            }),
            inner.clone().prop_map(|operand| Expr::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(operand),
                span: span(),
            }),
            (inner, any::<i8>()).prop_map(|(base, exponent)| Expr::Power {
                base: Box::new(base),
                exponent,
                span: span(),
            }),
        ]
    })
}

/// One `table` column as the parser accepts it: a metric name, a channel
/// reference, or a call around either. A column never nests further, so
/// this strategy does not recurse.
fn table_column_strategy() -> impl Strategy<Value = Expr> {
    let ident = "[a-z_][a-z0-9_]*";
    let name = prop_oneof![
        metric_strategy().prop_map(|metric| Expr::Metric(MetricRef {
            metric,
            span: span(),
        })),
        (ident, proptest::option::of(ident)).prop_map(|(name, component)| Expr::Channel(
            ChannelRef {
                name,
                component,
                span: span(),
            }
        )),
    ];
    let funcs = proptest::sample::select(Func::iter().collect::<Vec<_>>());
    prop_oneof![
        name.clone(),
        (name, funcs).prop_map(|(arg, func)| Expr::Call {
            func,
            arg: Box::new(arg),
            span: span(),
        }),
    ]
}

fn query_strategy() -> impl Strategy<Value = Query> {
    let params = proptest::collection::vec(
        (
            proptest::sample::select(vec![
                ParamName::Mask,
                ParamName::SnrDrop,
                ParamName::SlipWindow,
            ]),
            number_strategy(),
        ),
        0..3,
    );
    let window = proptest::option::of(prop_oneof![
        (1u64..1000)
            .prop_filter_map("a window count is at least 1 point", NonZeroU64::new)
            .prop_map(|len| Window::Count { len, span: span() }),
        (
            1.0f64..1000.0,
            prop_oneof![
                Just(Unit::MS),
                Just(Unit::S),
                Just(Unit::MIN),
                Just(Unit::H),
            ],
        )
            .prop_map(|(value, unit)| Window::Duration {
                value,
                unit,
                span: span(),
            }),
    ]);
    let predicates = proptest::collection::vec(expr_strategy(), 0..3);
    let mode = proptest::option::of(proptest::sample::select(
        DisplayMode::iter().collect::<Vec<_>>(),
    ));
    let table = proptest::option::of(proptest::collection::vec(table_column_strategy(), 1..4));
    (params, window, predicates, mode, table).prop_map(
        |(params, window, predicates, mode, table)| Query {
            // The round-trip fuzz targets stage formatting. Channel-source
            // formatting is pinned by a dedicated test.
            source: Source::Points,
            params: params
                .into_iter()
                .map(|(name, value)| ParamDecl {
                    name,
                    value,
                    span: span(),
                })
                .collect(),
            window,
            predicates,
            mode: mode.map(|mode| ModeStage { mode, span: span() }),
            table: table.map(|columns| TableSpec {
                columns,
                span: span(),
            }),
        },
    )
}

proptest! {
    #[test]
    fn parse_never_panics(src in ".*") {
        let _outcome = parser::parse(&src);
    }

    #[test]
    fn format_is_a_fixed_point(query in query_strategy()) {
        let printed = query.to_string();
        let reparsed = parser::parse(&printed).expect(&printed);
        prop_assert_eq!(reparsed.to_string(), printed);
    }
}
