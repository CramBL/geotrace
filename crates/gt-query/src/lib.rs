//! The GeoTrace query language: a small declarative pipeline for ad-hoc
//! analysis of loaded navigation data.
//!
//! ```text
//! points
//! | window 10
//! | where spread(heading) <= 10 deg
//!     and avg(accel) >= 0.3 m/s2
//!     and avg(velocity) > 30 km/h
//! | draw
//! | table time, velocity, heading, accel
//! ```
//!
//! The flow is [`parse`] → [`check`] → [`run`]. Parsing and checking report a
//! [`Diagnostic`] with a byte span for the editor to underline; a checked
//! query evaluates over [`MetricProvider`]s supplied by the caller and
//! returns matches as point-index ranges per track, plus a run summary.
//!
//! This crate is pure language and evaluation - no data loading, no UI, no
//! rendering.

pub mod ast;
mod check;
mod eval;
mod fmt;
pub mod lexer;
mod metric;
mod parser;
mod unit;

pub use ast::{ParamName, Query, Span};
pub use check::{CheckedQuery, Params, check};
pub use eval::{
    MetricProvider, RunOutput, RunSummary, TrackInput, TrackMatches, derived_accel, run,
};
pub use metric::{Quantity, QueryMetric};
pub use parser::parse;
pub use unit::Unit;

/// A parse or type error: what went wrong, where, and optionally how to fix
/// it. Rendered by the editor as an underline plus message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gt_types::{FileIdx, TrackIdx, TrackRef};

    use super::*;

    const UC1: &str = "points
| window 10
| where spread(heading) <= 10 deg
    and avg(accel) >= 0.3 m/s2
    and avg(velocity) > 30 km/h
| draw
| table time, velocity, heading, accel";

    fn track_ref() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    /// Per-metric series in base units; anything absent is missing.
    #[derive(Default)]
    struct TestProvider {
        len: usize,
        series: BTreeMap<QueryMetric, Vec<Option<f64>>>,
    }

    impl TestProvider {
        fn new(len: usize) -> Self {
            Self {
                len,
                series: BTreeMap::new(),
            }
        }

        fn with(mut self, metric: QueryMetric, values: Vec<Option<f64>>) -> Self {
            assert_eq!(values.len(), self.len);
            self.series.insert(metric, values);
            self
        }

        fn indexed_time(self) -> Self {
            let len = self.len;
            self.with(
                QueryMetric::Time,
                (0..len).map(|i| Some(i as f64)).collect(),
            )
        }
    }

    impl MetricProvider for TestProvider {
        fn len(&self) -> usize {
            self.len
        }

        fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
            self.series
                .get(&metric)
                .and_then(|values| values.get(index).copied().flatten())
        }
    }

    fn checked(src: &str) -> CheckedQuery {
        check(&parse(src).unwrap()).unwrap()
    }

    fn run_one(src: &str, provider: &TestProvider) -> RunOutput {
        let query = checked(src);
        run(
            &query,
            &[TrackInput {
                track: track_ref(),
                provider,
            }],
        )
    }

    #[test]
    fn point_predicate_matches_consecutive_runs() {
        // 30 km/h is 8.33 m/s; points 1, 2, and 4 exceed it.
        let provider = TestProvider::new(5).with(
            QueryMetric::Velocity,
            vec![Some(5.0), Some(10.0), Some(9.0), Some(3.0), Some(12.0)],
        );
        let output = run_one("points | where velocity > 30 km/h", &provider);
        assert_eq!(output.matches.len(), 1);
        assert_eq!(output.matches[0].ranges, vec![1..3, 4..5]);
        assert_eq!(output.summary.match_count, 2);
        assert_eq!(output.summary.tracks_with_matches, 1);
        assert!(output.summary.skipped.is_empty());
    }

    #[test]
    fn overlapping_windows_merge_into_one_match() {
        // Windows starting at 1 and 2 pass (avg > 10), so points 1..=4 merge.
        let provider = TestProvider::new(6).with(
            QueryMetric::Velocity,
            vec![
                Some(0.0),
                Some(11.0),
                Some(12.0),
                Some(13.0),
                Some(11.0),
                Some(0.0),
            ],
        );
        let output = run_one(
            "points | window 3 | where avg(velocity) > 36 km/h",
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![1..5]);
        assert_eq!(output.summary.match_count, 1);
    }

    #[test]
    fn missing_values_poison_and_are_counted() {
        let provider = TestProvider::new(5).with(
            QueryMetric::Heading,
            vec![Some(10.0), Some(12.0), None, Some(11.0), Some(13.0)],
        );
        let output = run_one(
            "points | window 2 | where spread(heading) <= 10 deg",
            &provider,
        );
        // Windows [1,2] and [2,3] touch the hole and are skipped.
        assert_eq!(output.summary.skipped.get(&QueryMetric::Heading), Some(&2));
        assert_eq!(output.matches[0].ranges, vec![0..2, 3..5]);
    }

    #[test]
    fn accel_derives_from_velocity_and_time() {
        let provider = TestProvider::new(4)
            .with(
                QueryMetric::Velocity,
                vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
            )
            .indexed_time();
        let output = run_one("points | where accel >= 0.5 m/s2", &provider);
        // Point 0 has no accel (no predecessor) and counts as skipped.
        assert_eq!(output.matches[0].ranges, vec![1..4]);
        assert_eq!(output.summary.skipped.get(&QueryMetric::Accel), Some(&1));
    }

    #[test]
    fn circular_spread_matches_across_north() {
        let provider = TestProvider::new(3).with(
            QueryMetric::Heading,
            vec![Some(350.0), Some(0.0), Some(10.0)],
        );
        let output = run_one(
            "points | window 3 | where spread(heading) <= 25 deg",
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn short_track_is_reported_not_dropped() {
        let provider =
            TestProvider::new(3).with(QueryMetric::Velocity, vec![Some(1.0), Some(2.0), Some(3.0)]);
        let output = run_one(
            "points | window 10 | where avg(velocity) > 0 km/h",
            &provider,
        );
        assert!(output.matches.is_empty());
        assert_eq!(output.summary.tracks_shorter_than_window, 1);
    }

    #[test]
    fn unused_params_flow_into_the_summary() {
        let provider = TestProvider::new(1).with(QueryMetric::Velocity, vec![Some(1.0)]);
        let output = run_one(
            "points | with mask 15 deg | where velocity > 0 km/h",
            &provider,
        );
        assert_eq!(output.summary.unused_params, vec![ParamName::Mask]);
    }

    #[test]
    fn columns_default_to_time_plus_referenced_metrics() {
        let query = checked(
            "points | window 5 | where spread(heading) <= 10 deg and avg(velocity) > 30 km/h",
        );
        assert_eq!(
            query.columns(),
            &[
                QueryMetric::Time,
                QueryMetric::Heading,
                QueryMetric::Velocity
            ]
        );
    }

    #[test]
    fn explicit_table_controls_columns_time_stays_first() {
        let query = checked(UC1);
        assert_eq!(
            query.columns(),
            &[
                QueryMetric::Time,
                QueryMetric::Velocity,
                QueryMetric::Heading,
                QueryMetric::Accel,
            ]
        );
        assert!(query.draw());
    }

    #[test]
    fn implicit_draw_without_output_stage() {
        assert!(checked("points | where velocity > 0 km/h").draw());
        assert!(!checked("points | where velocity > 0 km/h | table time").draw());
        assert!(checked("points | where velocity > 0 km/h | draw | table time").draw());
    }

    #[test]
    fn with_params_resolve_to_base_units() {
        let query = checked(
            "points | with mask 15 deg, snr_drop 10, slip_window 5 min | where slip_all > 2 per min",
        );
        let params = query.params();
        assert_eq!(params.mask_deg, Some(15.0));
        assert_eq!(params.snr_drop_db_hz, Some(10.0));
        assert_eq!(params.slip_window_s, Some(300.0));
        assert!(query.unused_params().is_empty());
    }

    #[test]
    fn pinned_error_messages() {
        let cases = [
            (
                "points | where velocity > 30",
                "velocity needs a unit, e.g. 30 km/h",
            ),
            (
                "points | where velocity > 30 deg",
                "expected a speed unit (km/h, m/s, kn), found deg",
            ),
            (
                "points | where velocity == 30 km/h",
                "use a range, e.g. 29 km/h < velocity and velocity < 31 km/h",
            ),
            (
                "points | window 10 | where velocity > 30 km/h",
                "velocity is per point - wrap it in an aggregate like avg(velocity)",
            ),
            (
                "points | where avg(velocity) > 30 km/h",
                "avg needs a window",
            ),
            (
                "points | where util_gps < 50 %",
                "util_gps needs an elevation mask",
            ),
            (
                "points | where slip_all > 2 per min",
                "slip_all needs mask, snr_drop, and slip_window",
            ),
            (
                "points | where eph > 20 m | window 3",
                "window must come before where - windows always see consecutive points",
            ),
        ];
        for (src, expected) in cases {
            let message = parse(src)
                .and_then(|q| check(&q).map(|_| ()))
                .expect_err(src)
                .message;
            assert_eq!(message, expected, "for {src}");
        }
    }

    #[test]
    fn util_mask_error_has_the_add_help() {
        let err = check(&parse("points | where util_gps < 50 %").unwrap()).unwrap_err();
        assert_eq!(err.help.as_deref(), Some("add: | with mask 15 deg"));
    }

    #[test]
    fn deep_nesting_errors_instead_of_overflowing() {
        // The MAX_DEPTH guard must fire well before the stack gives out.
        let src = format!("points | where {}velocity > 0 km/h", "not ".repeat(70));
        let err = parse(&src).unwrap_err();
        assert_eq!(err.message, "expression is too deeply nested");
    }

    #[test]
    fn arithmetic_quantity_table() {
        // The dimensional truth table of check::arith_quantity, both sides.
        let accepted = [
            "points | where velocity + 3 km/h > 30 km/h",
            "points | where eph - 3 m > 10 m",
            "points | where velocity * 2 > 30 km/h",
            "points | where 2 * velocity > 30 km/h",
            "points | where sats_fix * 2 > 6",
            "points | where velocity / 2 > 15 km/h",
            "points | where eph / eph > 50 %",
            "points | where eph / clock_delta > 1 m/s",
            "points | where velocity / clock_delta > 0.1 m/s2",
        ];
        for src in accepted {
            check(&parse(src).expect(src)).expect(src);
        }
        let rejected = [
            (
                "points | where velocity * eph > 3",
                "unsupported arithmetic between speed and length",
            ),
            (
                "points | where sats_fix / velocity > 3",
                "unsupported arithmetic between count and speed",
            ),
            (
                "points | where velocity + eph > 3 m",
                "unsupported arithmetic between speed and length",
            ),
            (
                "points | where time - clock_delta > 3 s",
                "timestamps do not support + and -",
            ),
            (
                "points | where heading + 10 deg < 30 deg",
                "directions do not support + and -",
            ),
            (
                "points | where (sats_fix == 1) + 1 > 1",
                "conditions do not support arithmetic",
            ),
        ];
        for (src, expected) in rejected {
            let message = check(&parse(src).expect(src)).expect_err(src).message;
            assert_eq!(message, expected, "for {src}");
        }
    }

    #[test]
    fn min_unit_and_min_aggregate_coexist() {
        // Position disambiguates: after a number `min` is the minute unit,
        // before `(` it is the aggregate.
        checked("points | window 3 | where delta(time) <= 15 min and min(velocity) > 5 km/h");
    }

    #[test]
    fn negative_thresholds_parse_and_check() {
        let provider = TestProvider::new(3)
            .with(
                QueryMetric::Velocity,
                vec![Some(10.0), Some(5.0), Some(1.0)],
            )
            .indexed_time();
        let output = run_one("points | where accel < -2 m/s2", &provider);
        assert_eq!(output.matches[0].ranges, vec![1..3]);
    }

    #[test]
    fn sats_equality_is_allowed_velocity_equality_is_not() {
        check(&parse("points | where sats_fix == 6").unwrap()).unwrap();
        check(&parse("points | where velocity == 30 km/h").unwrap()).unwrap_err();
    }

    #[test]
    fn uc1_parses_checks_and_formats() {
        let query = parse(UC1).unwrap();
        check(&query).unwrap();
        insta::assert_debug_snapshot!("uc1_ast", query);
        insta::assert_snapshot!("uc1_canonical", query.to_string());
    }

    #[test]
    fn error_catalog() {
        // One snapshot over every distinct diagnostic, so any wording or span
        // change shows up as a reviewable diff.
        let sources = [
            "",
            "where velocity > 0",
            "points | window",
            "points | window 0",
            "points | window 2.5",
            "points | window 15 s",
            "points | window 3 | window 4",
            "points | draw | where velocity > 0 km/h",
            "points | draw | draw",
            "points | where velocity > 30 kmh",
            "points | where velocity > 30 km/s",
            "points | where speed > 30 km/h",
            "points | where avg > 3",
            "points | where blah(velocity) > 3",
            "points | where velocity + 3 s > 30 km/h",
            "points | where not velocity",
            "points | where -heading < 10 deg",
            "points | window 3 | where avg(avg(velocity)) > 0 km/h",
            "points | window 3 | where avg(heading) < 10 deg",
            "points | window 3 | where spread(velocity) > 3 km/h and avg(velocity)",
            "points | where time > 100",
            "points | where sats_fix > 6 m",
            "points | where eph > velocity",
            "points | where velocity / eph > 1",
            "points | with mask 15 | where util_all < 50 %",
            "points | with snr_drop 10 db | where velocity > 0 km/h",
            "points | with slip_window 5 | where velocity > 0 km/h",
            "points | with mask 1 deg, mask 2 deg | where util_all < 50 %",
            "points | with speed 3 | where velocity > 0 km/h",
            "points | Draw",
            "points | table",
            "points | table velocity, | draw",
            "points | where (velocity > 0 km/h",
            "points | where velocity > 2 per day",
            "points draw",
            "points |",
            "points | 5",
            "points | where > 3",
            "points | with mask deg",
            "points | with mask 1 deg | with mask 2 deg",
            "points | table time | table time",
            "points | window 5 | with mask 1 deg",
            "points | draw | window 5",
            "points | window 2 | where avg(velocity)",
            "points | where velocity",
            "points | window 2 | where avg(velocity > 0 km/h)",
            "points | where abs(velocity > 0 km/h)",
            "points | where (velocity > 0 km/h) < (eph > 1 m)",
            "points | window 2 | where first(velocity) == last(velocity)",
            "points | with snr_drop 10 s | where velocity > 0 km/h",
        ];
        let catalog: Vec<(&str, String)> = sources
            .iter()
            .map(|src| {
                let outcome = match parse(src) {
                    Err(e) => diag_line(&e),
                    Ok(q) => match check(&q) {
                        Err(e) => diag_line(&e),
                        Ok(_) => "(no error)".to_owned(),
                    },
                };
                (*src, outcome)
            })
            .collect();
        insta::assert_debug_snapshot!("error_catalog", catalog);
    }

    fn diag_line(diagnostic: &Diagnostic) -> String {
        let mut line = format!(
            "{}..{}: {}",
            diagnostic.span.start, diagnostic.span.end, diagnostic.message
        );
        if let Some(help) = &diagnostic.help {
            line.push_str(" | help: ");
            line.push_str(help);
        }
        line
    }

    mod properties {
        use proptest::prelude::*;
        use strum::IntoEnumIterator as _;

        use super::super::ast::{
            BinaryOp, Expr, Func, MetricRef, NumberLit, ParamDecl, ParamName, Query, Span,
            TableSpec, UnaryOp, Window,
        };
        use super::super::unit::Unit;
        use super::super::{QueryMetric, parse};

        fn span() -> Span {
            Span::new(0, 0)
        }

        fn metric_strategy() -> impl Strategy<Value = QueryMetric> {
            proptest::sample::select(QueryMetric::iter().collect::<Vec<_>>())
        }

        fn unit_strategy() -> impl Strategy<Value = Unit> {
            proptest::sample::select(vec![
                Unit::Deg,
                Unit::M,
                Unit::Km,
                Unit::KmPerH,
                Unit::MPerS,
                Unit::Kn,
                Unit::MPerS2,
                Unit::Ms,
                Unit::S,
                Unit::Min,
                Unit::H,
                Unit::Percent,
                Unit::PerS,
                Unit::PerMin,
                Unit::PerH,
            ])
        }

        fn number_strategy() -> impl Strategy<Value = NumberLit> {
            (0.0..1e9f64, proptest::option::of(unit_strategy())).prop_map(|(value, unit)| {
                NumberLit {
                    value,
                    unit,
                    span: span(),
                }
            })
        }

        fn expr_strategy() -> impl Strategy<Value = Expr> {
            let leaf = prop_oneof![
                number_strategy().prop_map(Expr::Number),
                metric_strategy().prop_map(|metric| Expr::Metric(MetricRef {
                    metric,
                    span: span(),
                })),
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
                    inner.prop_map(|operand| Expr::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(operand),
                        span: span(),
                    }),
                ]
            })
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
            let window = proptest::option::of(1u64..1000);
            let predicates = proptest::collection::vec(expr_strategy(), 0..3);
            let table = proptest::option::of(proptest::collection::vec(metric_strategy(), 1..4));
            (params, window, predicates, proptest::bool::ANY, table).prop_map(
                |(params, window, predicates, draw, table)| Query {
                    params: params
                        .into_iter()
                        .map(|(name, value)| ParamDecl {
                            name,
                            value,
                            span: span(),
                        })
                        .collect(),
                    window: window.map(|len| Window { len, span: span() }),
                    predicates,
                    draw: draw.then(span),
                    table: table.map(|metrics| TableSpec {
                        columns: metrics
                            .into_iter()
                            .map(|metric| MetricRef {
                                metric,
                                span: span(),
                            })
                            .collect(),
                        span: span(),
                    }),
                },
            )
        }

        proptest! {
            #[test]
            fn parse_never_panics(src in ".*") {
                let _outcome = parse(&src);
            }

            #[test]
            fn format_is_a_fixed_point(query in query_strategy()) {
                let printed = query.to_string();
                let reparsed = parse(&printed).expect(&printed);
                prop_assert_eq!(reparsed.to_string(), printed);
            }
        }
    }
}
