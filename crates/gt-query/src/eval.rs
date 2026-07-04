//! Query evaluation over per-point metric providers.
//!
//! Values flow in base units (see `crate::unit`). Missing values poison the
//! surrounding point or window to "no match" and are reported per metric in
//! the run summary - nothing is interpolated or silently skipped.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use gt_types::TrackRef;

use crate::ast::{Func, ParamName};
use crate::check::{ArithOp, CExpr, CheckedQuery, CmpOp};
use crate::metric::QueryMetric;

const FULL_TURN_DEG: f64 = 360.0;
const HALF_TURN_DEG: f64 = 180.0;

/// Per-point metric access for one track.
///
/// Values are in the evaluator's base units: degrees, meters, m/s, m/s2,
/// seconds (timestamps as Unix seconds), 0-1 fractions for ratios, and
/// events per minute for rates. `None` means the point has no value for the
/// metric. `accel` is derived by the evaluator itself and never requested
/// from a provider.
pub trait MetricProvider {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64>;
}

/// One track to run a query over.
#[derive(Clone, Copy)]
pub struct TrackInput<'a> {
    pub track: TrackRef,
    pub provider: &'a dyn MetricProvider,
}

/// Matches of one track: maximal runs of consecutive matched point indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackMatches {
    pub track: TrackRef,
    pub ranges: Vec<Range<usize>>,
}

/// Counts reported after a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub match_count: usize,
    pub tracks_with_matches: usize,
    /// Windows (or points, without a window) skipped per missing metric.
    pub skipped: BTreeMap<QueryMetric, usize>,
    /// Skips from non-finite arithmetic (e.g. division by zero) - kept
    /// separate so they stay visible without a metric to blame.
    pub skipped_non_finite: usize,
    pub tracks_shorter_than_window: usize,
    /// Declared `with` parameters no referenced metric needs.
    pub unused_params: Vec<ParamName>,
}

/// Everything a run produces. Rendering is the caller's job.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub matches: Vec<TrackMatches>,
    pub columns: Vec<QueryMetric>,
    pub summary: RunSummary,
}

pub fn run(query: &CheckedQuery, tracks: &[TrackInput<'_>]) -> RunOutput {
    let mut summary = RunSummary {
        unused_params: query.unused_params().to_vec(),
        ..RunSummary::default()
    };
    let mut matches = Vec::new();

    for input in tracks {
        let len = input.provider.len();
        let mut ctx = Ctx {
            provider: input.provider,
            missing: BTreeSet::new(),
            non_finite: false,
        };
        let mut matched = vec![false; len];

        match query.window() {
            Some(window) if len < window => {
                summary.tracks_shorter_than_window += 1;
            }
            Some(window) => {
                let last_start = len - window;
                for start in 0..=last_start {
                    let scope = Scope::Window { start, len: window };
                    match verdict(query, &mut ctx, scope) {
                        Some(true) => {
                            for slot in matched.iter_mut().skip(start).take(window) {
                                *slot = true;
                            }
                        }
                        Some(false) => {}
                        None => record_skip(&mut summary, &ctx),
                    }
                }
            }
            None => {
                for (index, slot) in matched.iter_mut().enumerate() {
                    match verdict(query, &mut ctx, Scope::Point(index)) {
                        Some(true) => *slot = true,
                        Some(false) => {}
                        None => record_skip(&mut summary, &ctx),
                    }
                }
            }
        }

        let ranges = ranges_from(&matched);
        if !ranges.is_empty() {
            summary.tracks_with_matches += 1;
            summary.match_count += ranges.len();
            matches.push(TrackMatches {
                track: input.track,
                ranges,
            });
        }
    }

    RunOutput {
        matches,
        columns: query.columns().to_vec(),
        summary,
    }
}

fn record_skip(summary: &mut RunSummary, ctx: &Ctx<'_>) {
    for metric in &ctx.missing {
        *summary.skipped.entry(*metric).or_insert(0) += 1;
    }
    if ctx.non_finite {
        summary.skipped_non_finite += 1;
    }
}

/// All `where` stages must hold; a missing value in any of them poisons the
/// whole point or window to "skipped".
fn verdict(query: &CheckedQuery, ctx: &mut Ctx<'_>, scope: Scope) -> Option<bool> {
    ctx.missing.clear();
    ctx.non_finite = false;
    let mut all = true;
    let mut poisoned = false;
    for predicate in &query.predicates {
        match eval_bool(ctx, predicate, scope) {
            Some(b) => all &= b,
            None => poisoned = true,
        }
    }
    if poisoned { None } else { Some(all) }
}

#[derive(Clone, Copy)]
enum Scope {
    Point(usize),
    Window { start: usize, len: usize },
}

struct Ctx<'a> {
    provider: &'a dyn MetricProvider,
    /// Metrics that came up missing in the current point/window evaluation.
    missing: BTreeSet<QueryMetric>,
    non_finite: bool,
}

impl Ctx<'_> {
    /// Provider value with NaN/inf treated as missing, without attribution.
    fn raw(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        self.provider.value(metric, index).filter(|v| v.is_finite())
    }

    fn metric_at(&mut self, metric: QueryMetric, index: usize) -> Option<f64> {
        let value = if metric == QueryMetric::Accel {
            self.accel_at(index)
        } else {
            self.raw(metric, index)
        };
        if value.is_none() {
            self.missing.insert(metric);
        }
        value
    }

    /// Backward difference of velocity over time. Missing on the first point
    /// of a track, wherever velocity is missing, and on non-increasing
    /// timestamps (a clock anomaly cannot yield a meaningful accel).
    fn accel_at(&self, index: usize) -> Option<f64> {
        let prev = index.checked_sub(1)?;
        let v1 = self.raw(QueryMetric::Velocity, index)?;
        let v0 = self.raw(QueryMetric::Velocity, prev)?;
        let t1 = self.raw(QueryMetric::Time, index)?;
        let t0 = self.raw(QueryMetric::Time, prev)?;
        let dt = t1 - t0;
        if dt <= 0.0 {
            return None;
        }
        Some((v1 - v0) / dt)
    }
}

/// Condition nodes. The checker guarantees which nodes are conditions, so a
/// value node here is a checker bug and poisons rather than lies.
fn eval_bool(ctx: &mut Ctx<'_>, expr: &CExpr, scope: Scope) -> Option<bool> {
    match expr {
        CExpr::Not(inner) => eval_bool(ctx, inner, scope).map(|b| !b),
        CExpr::Cmp { op, lhs, rhs } => {
            let (l, r) = both_nums(ctx, lhs, rhs, scope)?;
            let ordering = l.total_cmp(&r);
            Some(match op {
                CmpOp::Lt => ordering == Ordering::Less,
                CmpOp::Le => ordering != Ordering::Greater,
                CmpOp::Gt => ordering == Ordering::Greater,
                CmpOp::Ge => ordering != Ordering::Less,
                CmpOp::Eq => ordering == Ordering::Equal,
                CmpOp::Ne => ordering != Ordering::Equal,
            })
        }
        CExpr::Logic { and, lhs, rhs } => {
            // Both sides always evaluate: short-circuiting would make the
            // missing-value skip counts depend on operand order.
            let l = eval_bool(ctx, lhs, scope);
            let r = eval_bool(ctx, rhs, scope);
            let (a, b) = (l?, r?);
            Some(if *and { a && b } else { a || b })
        }
        CExpr::Const(_)
        | CExpr::Metric(_)
        | CExpr::Agg { .. }
        | CExpr::Abs(_)
        | CExpr::Neg(_)
        | CExpr::Arith { .. } => None,
    }
}

/// Value nodes, in base units.
fn eval_num(ctx: &mut Ctx<'_>, expr: &CExpr, scope: Scope) -> Option<f64> {
    match expr {
        CExpr::Const(v) => Some(*v),
        CExpr::Metric(metric) => match scope {
            Scope::Point(index) => ctx.metric_at(*metric, index),
            // The checker forbids bare metrics in windowed predicates.
            Scope::Window { .. } => None,
        },
        CExpr::Agg {
            func,
            circular,
            arg,
        } => aggregate(ctx, *func, *circular, arg, scope),
        CExpr::Abs(inner) => eval_num(ctx, inner, scope).map(f64::abs),
        CExpr::Neg(inner) => eval_num(ctx, inner, scope).map(|v| -v),
        CExpr::Arith { op, lhs, rhs } => {
            let (l, r) = both_nums(ctx, lhs, rhs, scope)?;
            let result = match op {
                ArithOp::Add => l + r,
                ArithOp::Sub => l - r,
                ArithOp::Mul => l * r,
                ArithOp::Div => l / r,
            };
            if !result.is_finite() {
                ctx.non_finite = true;
                return None;
            }
            Some(result)
        }
        // Condition nodes never appear in value position.
        CExpr::Not(_) | CExpr::Cmp { .. } | CExpr::Logic { .. } => None,
    }
}

fn both_nums(ctx: &mut Ctx<'_>, lhs: &CExpr, rhs: &CExpr, scope: Scope) -> Option<(f64, f64)> {
    let l = eval_num(ctx, lhs, scope);
    let r = eval_num(ctx, rhs, scope);
    Some((l?, r?))
}

/// Evaluate the aggregate argument at every point of the window; any missing
/// point poisons the whole aggregate.
fn aggregate(
    ctx: &mut Ctx<'_>,
    func: Func,
    circular: bool,
    arg: &CExpr,
    scope: Scope,
) -> Option<f64> {
    let Scope::Window { start, len } = scope else {
        return None;
    };
    let mut values = Vec::with_capacity(len);
    for index in start..start + len {
        values.push(eval_num(ctx, arg, Scope::Point(index))?);
    }
    let (first, last) = (values.first().copied()?, values.last().copied()?);
    let value = match func {
        Func::Avg => values.iter().sum::<f64>() / values.len() as f64,
        Func::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
        Func::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Func::First => first,
        Func::Last => last,
        Func::Delta => {
            if circular {
                circular_delta(first, last)
            } else {
                last - first
            }
        }
        Func::Spread => {
            if circular {
                circular_spread(&mut values)
            } else {
                let min = values.iter().copied().fold(f64::INFINITY, f64::min);
                let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                max - min
            }
        }
        // The checker never emits abs as an aggregate.
        Func::Abs => return None,
    };
    Some(value)
}

/// Signed shortest angular difference from `first` to `last`, in (-180, 180].
fn circular_delta(first: f64, last: f64) -> f64 {
    let diff = (last - first + HALF_TURN_DEG).rem_euclid(FULL_TURN_DEG) - HALF_TURN_DEG;
    if diff <= -HALF_TURN_DEG {
        diff + FULL_TURN_DEG
    } else {
        diff
    }
}

/// Size of the smallest arc containing all directions: 360 minus the largest
/// gap between neighboring values on the circle.
fn circular_spread(values: &mut [f64]) -> f64 {
    for value in values.iter_mut() {
        *value = value.rem_euclid(FULL_TURN_DEG);
    }
    values.sort_unstable_by(f64::total_cmp);
    let (Some(first), Some(last)) = (values.first().copied(), values.last().copied()) else {
        return 0.0;
    };
    let wrap_gap = first + FULL_TURN_DEG - last;
    let max_gap = values
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some(b - a),
            _ => None,
        })
        .fold(wrap_gap, f64::max);
    FULL_TURN_DEG - max_gap
}

/// Maximal runs of `true`.
fn ranges_from(matched: &[bool]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut run_start = None;
    for (index, &hit) in matched.iter().enumerate() {
        match (hit, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                ranges.push(start..index);
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        ranges.push(start..matched.len());
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_delta_takes_the_short_way() {
        assert!((circular_delta(350.0, 10.0) - 20.0).abs() < 1e-12);
        assert!((circular_delta(10.0, 350.0) + 20.0).abs() < 1e-12);
        assert!((circular_delta(0.0, 180.0) - 180.0).abs() < 1e-12);
    }

    #[test]
    fn circular_spread_hugs_the_wrap() {
        let mut wrapped = vec![350.0, 10.0, 0.0];
        assert!((circular_spread(&mut wrapped) - 20.0).abs() < 1e-12);
        let mut plain = vec![10.0, 40.0];
        assert!((circular_spread(&mut plain) - 30.0).abs() < 1e-12);
        let mut single = vec![123.0];
        assert!((circular_spread(&mut single)).abs() < 1e-12);
    }

    #[test]
    fn ranges_merge_consecutive_points() {
        assert_eq!(
            ranges_from(&[false, true, true, false, true]),
            vec![1..3, 4..5]
        );
        assert_eq!(ranges_from(&[true]), vec![0..1]);
        assert!(ranges_from(&[false, false]).is_empty());
    }
}
