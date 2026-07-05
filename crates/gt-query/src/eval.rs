//! Query evaluation over per-point metric providers.
//!
//! Values flow in base units (see `crate::unit`). Missing values poison the
//! surrounding point or window to "no match" and are reported per metric in
//! the run summary - nothing is interpolated or silently skipped.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use gt_types::TrackRef;
use nalgebra::{Complex, UnitComplex};

use crate::ast::{Func, ParamName};
use crate::check::{ArithOp, CExpr, CheckedQuery, CmpOp};
use crate::metric::QueryMetric;

const FULL_TURN_DEG: f64 = 360.0;

/// Points evaluated between cancellation checks. Small enough to stop within
/// a frame or two, large enough that the check never shows up in a profile.
pub(crate) const CANCEL_CHECK_INTERVAL: usize = 4096;

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
    /// Points that fell inside a match, summed over tracks.
    pub matched_points: usize,
    /// Points evaluated, summed over tracks - the denominator for the
    /// keep/hide "N of M points" summary.
    pub total_points: usize,
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
    let never = || false;
    run_cancellable(query, tracks, &never).unwrap_or_else(|| RunOutput {
        // Unreachable: the cancel check above never fires. Constructed rather
        // than unwrapped to keep the no-panic guarantee.
        matches: Vec::new(),
        columns: query.columns().to_vec(),
        summary: RunSummary::default(),
    })
}

/// Like [`run`], stopping early when `should_cancel` returns true. Returns
/// `None` on cancellation - partial results are never surfaced, so a
/// cancelled run cannot masquerade as a complete one.
pub fn run_cancellable(
    query: &CheckedQuery,
    tracks: &[TrackInput<'_>],
    should_cancel: &dyn Fn() -> bool,
) -> Option<RunOutput> {
    run_with_interval(query, tracks, should_cancel, CANCEL_CHECK_INTERVAL)
}

/// [`run_cancellable`] with the check interval exposed, so tests can cross
/// the interval boundary without a 4096-point fixture.
pub(crate) fn run_with_interval(
    query: &CheckedQuery,
    tracks: &[TrackInput<'_>],
    should_cancel: &dyn Fn() -> bool,
    check_interval: usize,
) -> Option<RunOutput> {
    let mut summary = RunSummary {
        unused_params: query.unused_params().to_vec(),
        ..RunSummary::default()
    };
    let mut matches = Vec::new();

    for input in tracks {
        if should_cancel() {
            return None;
        }
        summary.total_points += input.provider.len();
        let eval = evaluate_track(query, input.provider, should_cancel, check_interval)?;
        summary.absorb(&eval);
        let ranges = ranges_from(&eval.matched);
        if !ranges.is_empty() {
            summary.tracks_with_matches += 1;
            summary.match_count += ranges.len();
            matches.push(TrackMatches {
                track: input.track,
                ranges,
            });
        }
    }

    Some(RunOutput {
        matches,
        columns: query.columns().to_vec(),
        summary,
    })
}

/// One track's evaluation: which points matched, plus the skip counts. The
/// shared core of both the single-query [`run`] and the multi-query
/// [`crate::run_pipeline`], which runs it over each visible run.
pub(crate) struct TrackEval {
    /// One flag per provider point.
    pub matched: Vec<bool>,
    /// Windows (or points) skipped per missing metric.
    pub skipped: BTreeMap<QueryMetric, usize>,
    pub skipped_non_finite: usize,
    /// The provider was shorter than the window, so nothing could match.
    pub shorter_than_window: bool,
}

impl RunSummary {
    /// Fold one track's evaluation into the running totals.
    fn absorb(&mut self, eval: &TrackEval) {
        self.matched_points += eval.matched.iter().filter(|m| **m).count();
        for (metric, count) in &eval.skipped {
            *self.skipped.entry(*metric).or_insert(0) += count;
        }
        self.skipped_non_finite += eval.skipped_non_finite;
        self.tracks_shorter_than_window += usize::from(eval.shorter_than_window);
    }
}

/// Evaluate `query` over one provider, returning the matched mask and skips.
/// `None` only on cancellation. Windows and derived metrics are relative to
/// this provider, so running it over a [`crate::RunView`] gives gap-aware
/// evaluation for the pipeline.
pub(crate) fn evaluate_track(
    query: &CheckedQuery,
    provider: &dyn MetricProvider,
    should_cancel: &dyn Fn() -> bool,
    check_interval: usize,
) -> Option<TrackEval> {
    let len = provider.len();
    let mut ctx = Ctx {
        provider,
        missing: BTreeSet::new(),
        non_finite: false,
    };
    let mut matched = vec![false; len];
    let mut skips = Skips::default();
    let mut shorter_than_window = false;

    match query.window() {
        Some(window) if len < window => {
            shorter_than_window = true;
        }
        Some(window) => {
            let last_start = len - window;
            for start in 0..=last_start {
                if start % check_interval == 0 && should_cancel() {
                    return None;
                }
                let scope = Scope::Window { start, len: window };
                match verdict(query, &mut ctx, scope) {
                    Some(true) => {
                        for slot in matched.iter_mut().skip(start).take(window) {
                            *slot = true;
                        }
                    }
                    Some(false) => {}
                    None => skips.record(&ctx),
                }
            }
        }
        None => {
            for (index, slot) in matched.iter_mut().enumerate() {
                if index % check_interval == 0 && should_cancel() {
                    return None;
                }
                match verdict(query, &mut ctx, Scope::Point(index)) {
                    Some(true) => *slot = true,
                    Some(false) => {}
                    None => skips.record(&ctx),
                }
            }
        }
    }
    Some(TrackEval {
        matched,
        skipped: skips.per_metric,
        skipped_non_finite: skips.non_finite,
        shorter_than_window,
    })
}

/// Skip tallies accumulated during one track's evaluation, kept apart from the
/// matched mask so both can be mutated in the same loop.
#[derive(Default)]
struct Skips {
    per_metric: BTreeMap<QueryMetric, usize>,
    non_finite: usize,
}

impl Skips {
    fn record(&mut self, ctx: &Ctx<'_>) {
        for metric in &ctx.missing {
            *self.per_metric.entry(*metric).or_insert(0) += 1;
        }
        if ctx.non_finite {
            self.non_finite += 1;
        }
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
        raw_value(self.provider, metric, index)
    }

    fn metric_at(&mut self, metric: QueryMetric, index: usize) -> Option<f64> {
        let value = if metric == QueryMetric::Accel {
            derived_accel(self.provider, index)
        } else {
            self.raw(metric, index)
        };
        if value.is_none() {
            self.missing.insert(metric);
        }
        value
    }
}

/// Provider value with NaN/inf treated as missing.
fn raw_value(provider: &dyn MetricProvider, metric: QueryMetric, index: usize) -> Option<f64> {
    provider.value(metric, index).filter(|v| v.is_finite())
}

/// The derived `accel` metric: backward difference of velocity over time, in
/// m/s2. Missing on the first point of a track, wherever velocity is missing,
/// and on non-increasing timestamps (a clock anomaly cannot yield a
/// meaningful accel).
///
/// Public so the UI can show the same value in match tables that the
/// evaluator used in predicates - this is the single definition of `accel`.
pub fn derived_accel(provider: &dyn MetricProvider, index: usize) -> Option<f64> {
    let prev = index.checked_sub(1)?;
    let v1 = raw_value(provider, QueryMetric::Velocity, index)?;
    let v0 = raw_value(provider, QueryMetric::Velocity, prev)?;
    let t1 = raw_value(provider, QueryMetric::Time, index)?;
    let t0 = raw_value(provider, QueryMetric::Time, prev)?;
    let dt = t1 - t0;
    if dt <= 0.0 {
        return None;
    }
    Some((v1 - v0) / dt)
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
        Func::Std => {
            if circular {
                circular_std(&values)
            } else {
                population_std(&values)
            }
        }
        // The checker never emits abs as an aggregate.
        Func::Abs => return None,
    };
    // Like arithmetic (see `eval_num`), an aggregate never hands a non-finite
    // value to a comparison: an overflowing spread, or the circular-std
    // singularity of a window with no resultant direction, poisons and is
    // reported as skipped rather than comparing as a bare infinity.
    if !value.is_finite() {
        ctx.non_finite = true;
        return None;
    }
    Some(value)
}

/// Signed shortest angular difference from `first` to `last`, in degrees,
/// approximately in (-180, 180].
///
/// Expressed as the rotation carrying `first` onto `last`, so the wrap is the
/// rotation group's job rather than hand-rolled modular arithmetic. At the
/// exact antipode the sign is implementation-defined: the turn is equally short
/// either way, and `angle()` decides it by floating-point rounding.
fn circular_delta(first: f64, last: f64) -> f64 {
    let rotation =
        UnitComplex::new(last.to_radians()) * UnitComplex::new(first.to_radians()).inverse();
    rotation.angle().to_degrees()
}

/// Population standard deviation (divided by N) of the window's values.
///
/// N is the whole window, so this describes the data in hand rather than
/// estimating a larger population, the same descriptive stance as
/// `avg`/`min`/`max`/`spread`. A single value has a deviation of 0.
fn population_std(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    variance.sqrt()
}

/// Circular (population) standard deviation of directions given in degrees,
/// returned in degrees.
///
/// Built from the mean resultant length R of the directions as unit vectors:
/// `sqrt(-2 ln R)` (Mardia), which is robust across the 0/360 wrap where a
/// linear standard deviation of the degrees is not. Identical directions give
/// 0; as they spread toward uniform, R falls to 0 and the deviation grows
/// without bound, reaching a non-finite value at the R = 0 singularity that the
/// [`aggregate`] boundary turns into a reported skip.
fn circular_std(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let resultant = values.iter().fold(Complex::new(0.0, 0.0), |acc, deg| {
        acc + UnitComplex::new(deg.to_radians()).into_inner()
    });
    // Clamp guards a floating-point overshoot above 1 when every direction is
    // identical, which would make the log positive and the sqrt NaN.
    let mean_resultant = (resultant.norm() / n).min(1.0);
    (-2.0 * mean_resultant.ln()).sqrt().to_degrees()
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
pub(crate) fn ranges_from(matched: &[bool]) -> Vec<Range<usize>> {
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
    fn population_std_divides_by_n() {
        // 2,4,4,4,5,5,7,9: mean 5, population variance 4, so std 2.
        let values = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert!((population_std(&values) - 2.0).abs() < 1e-12);
        // A single value has no spread.
        assert!(population_std(&[42.0]).abs() < 1e-12);
    }

    #[test]
    fn circular_std_stays_small_across_the_wrap() {
        // Headings clustered around north stay small, unlike a linear std of
        // the raw degrees (which 359, 0, 1 would blow up).
        assert!(circular_std(&[359.0, 0.0, 1.0]) < 2.0);
        // Identical directions collapse to zero (within float noise).
        assert!(circular_std(&[123.0, 123.0, 123.0]) < 1e-6);
        assert!(circular_std(&[42.0]) < 1e-6);
        // A wide scatter is a large deviation.
        assert!(circular_std(&[0.0, 90.0, 180.0]) > 45.0);
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

    mod properties {
        use proptest::prelude::*;

        use super::super::circular_std;

        proptest! {
            /// Circular std is invariant under a rigid rotation of all the
            /// directions, including across the 0/360 wrap - the property that
            /// justifies the whole helper. Tested on tight clusters (within
            /// +/-30 deg) where the statistic is well conditioned, so the
            /// invariant holds to a fine tolerance.
            #[test]
            fn circular_std_is_rotation_invariant(
                deltas in proptest::collection::vec(-30.0f64..30.0, 1..20),
                center in 0.0f64..360.0,
                offset in 0.0f64..360.0,
            ) {
                let cluster = |base: f64| -> Vec<f64> {
                    deltas.iter().map(|d| (base + d).rem_euclid(360.0)).collect()
                };
                let here = circular_std(&cluster(center));
                let there = circular_std(&cluster(center + offset));
                prop_assert!(here.is_finite() && here >= 0.0);
                prop_assert!((here - there).abs() < 1e-6, "here {here} there {there}");
            }

            /// The R clamp keeps the statistic real over arbitrary directions:
            /// never NaN, never negative.
            #[test]
            fn circular_std_is_never_nan(
                angles in proptest::collection::vec(0.0f64..360.0, 1..50),
            ) {
                let std = circular_std(&angles);
                prop_assert!(!std.is_nan() && std >= 0.0);
            }
        }
    }
}
