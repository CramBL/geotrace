//! A naive reference implementation of the documented pipeline semantics.
//!
//! Deliberately dumb and direct: a per-point fold over plain vectors, reading
//! the loaded points through [`gt_types`] accessors and nothing from
//! `gt_query`'s evaluator. Unit factors come from the language's own table
//! ([`gt_query::Unit`]): a literal's scale is not what this oracle is reasoning
//! about.
//!
//! The semantics it implements, from the pipeline's own documentation:
//!
//! - each stage evaluates over the maximal contiguous runs of the points still
//!   visible when it runs,
//! - windows and derived metrics are run-local: a window never spans a gap, and
//!   `accel` is missing at a run's first point,
//! - a missing value anywhere in a predicate makes the point or window skipped,
//!   never matched,
//! - `keep` shrinks visibility to what it matched, `hide` to what it did not,
//!   `draw` leaves visibility alone and records a matched mask,
//! - every draw mask is finally intersected with end-state visibility, so a halo
//!   only ever sits on a point the map still draws.

use std::collections::HashMap;
use std::ops::Range;

use chrono::{DateTime, Utc};
use gt_query::Unit;
use gt_types::{LoadedFile, NavPoint, TrackRef};
use gt_ui_types::PointVisibility;
use uom::si::angle::degree;
use uom::si::velocity::meter_per_second;

use super::generate::{Agg, CmpOp, Metric, Mode, Predicate, Program, Stage, Term};

/// What the oracle expects of one track: a verdict per point, and the draw
/// layers covering each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackExpectation {
    pub visibility: Vec<PointVisibility>,
    /// Per point, the stage indices whose halo covers it.
    pub draw_stages: Vec<Vec<usize>>,
}

/// One point's metric values, in the units the evaluator works in.
#[derive(Debug, Clone, Copy)]
struct Values {
    time_secs: f64,
    velocity_mps: Option<f64>,
    eph_m: Option<f64>,
    heading_deg: Option<f64>,
}

impl Values {
    fn of(point: &NavPoint) -> Self {
        Self {
            time_secs: point.tpv.time().as_secs_f64_with_subseconds(),
            velocity_mps: point.tpv.velocity().map(|v| v.get::<meter_per_second>()),
            eph_m: point.tpv.eph_m().map(f64::from),
            heading_deg: point.tpv.heading().map(|h| h.get::<degree>()),
        }
    }
}

/// Where a predicate is being evaluated within one run: at a point, or over a
/// window of points. Both are run-local.
#[derive(Debug, Clone, Copy)]
enum Scope {
    Point(usize),
    Window(usize, usize),
}

/// The verdict the harness should report for every point of every track.
pub fn expect(
    files: &[LoadedFile],
    window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    program: &Program,
) -> HashMap<TrackRef, TrackExpectation> {
    let mut expected = HashMap::new();
    for (fi, file) in files.iter().enumerate() {
        for (ti, track) in file.tracks.iter().enumerate() {
            let track_ref = gt_query_map_harness::track(fi, ti);
            expected.insert(track_ref, expect_track(&track.points, window, program));
        }
    }
    expected
}

fn expect_track(
    points: &[NavPoint],
    window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    program: &Program,
) -> TrackExpectation {
    let count = points.len();
    let times: Vec<DateTime<Utc>> = points.iter().map(|p| p.tpv.time().utc()).collect();
    // The track-level gate: a track whose span misses the window entirely is
    // off the map altogether, points and all.
    if let Some((start, end)) = window
        && (times.last().is_some_and(|last| *last < start)
            || times.first().is_some_and(|first| *first > end))
    {
        return all_withheld(count, PointVisibility::TrackNotShown);
    }

    // The evaluator only ever sees the points inside the window, which are
    // contiguous because a track's points are time-ordered.
    let slice: Vec<usize> = (0..count)
        .filter(|&i| {
            window.is_none_or(|(start, end)| {
                times
                    .get(i)
                    .is_some_and(|time| *time >= start && *time <= end)
            })
        })
        .collect();
    let values: Vec<Values> = slice
        .iter()
        .filter_map(|&i| points.get(i))
        .map(Values::of)
        .collect();

    let (visible, draw_masks) = fold(&values, program);
    let mut visibility = vec![PointVisibility::OutsideTimeFilter; count];
    let mut draw_stages = vec![Vec::new(); count];
    for (offset, &absolute) in slice.iter().enumerate() {
        let shown = visible.get(offset).copied().unwrap_or(false);
        if let Some(slot) = visibility.get_mut(absolute) {
            *slot = if shown {
                PointVisibility::Shown
            } else {
                PointVisibility::HiddenByQuery
            };
        }
        if shown && let Some(slot) = draw_stages.get_mut(absolute) {
            *slot = draw_masks
                .iter()
                .filter(|(_, mask)| mask.get(offset).copied().unwrap_or(false))
                .map(|(stage, _)| *stage)
                .collect();
        }
    }
    TrackExpectation {
        visibility,
        draw_stages,
    }
}

fn all_withheld(count: usize, visibility: PointVisibility) -> TrackExpectation {
    TrackExpectation {
        visibility: vec![visibility; count],
        draw_stages: vec![Vec::new(); count],
    }
}

/// Fold the program over one track's in-window points: the surviving visibility,
/// and each `draw` stage's mask already narrowed to it.
fn fold(values: &[Values], program: &Program) -> (Vec<bool>, Vec<(usize, Vec<bool>)>) {
    let count = values.len();
    let mut visible = vec![true; count];
    let mut draw_masks: Vec<(usize, Vec<bool>)> = Vec::new();

    for (index, stage) in program.stages.iter().enumerate() {
        let matched = match_stage(values, stage, &visible);
        match stage.mode {
            Mode::Keep => {
                for (slot, &hit) in visible.iter_mut().zip(&matched) {
                    *slot &= hit;
                }
            }
            Mode::Hide => {
                for (slot, &hit) in visible.iter_mut().zip(&matched) {
                    *slot &= !hit;
                }
            }
            Mode::Draw => draw_masks.push((index, matched)),
        }
    }

    // A halo only sits on a point that survived to the end.
    for (_, mask) in &mut draw_masks {
        for (slot, &shown) in mask.iter_mut().zip(&visible) {
            *slot &= shown;
        }
    }
    (visible, draw_masks)
}

/// What one stage matches, evaluated over the maximal runs of the points still
/// visible when it runs.
fn match_stage(values: &[Values], stage: &Stage, visible: &[bool]) -> Vec<bool> {
    let mut matched = vec![false; values.len()];
    for run in runs(visible) {
        let run_values = values.get(run.clone()).unwrap_or_default();
        let hits: Vec<Range<usize>> = match stage.window {
            None => (0..run_values.len())
                .filter(|&local| {
                    evaluate(&stage.predicate, run_values, Scope::Point(local)) == Some(true)
                })
                .map(|local| local..local + 1)
                .collect(),
            // A run shorter than the window holds no window to evaluate.
            Some(width) if run_values.len() >= width => (0..=(run_values.len() - width))
                .filter(|&start| {
                    evaluate(
                        &stage.predicate,
                        run_values,
                        Scope::Window(start, start + width),
                    ) == Some(true)
                })
                .map(|start| start..start + width)
                .collect(),
            Some(_) => Vec::new(),
        };
        for local in hits.into_iter().flatten() {
            if let Some(slot) = matched.get_mut(run.start + local) {
                *slot = true;
            }
        }
    }
    matched
}

/// The maximal contiguous runs of `true` in `visible`.
fn runs(visible: &[bool]) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (index, &alive) in visible.iter().enumerate() {
        match (alive, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                runs.push(from..index);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        runs.push(from..visible.len());
    }
    runs
}

/// A predicate's verdict, or `None` when a value it needs is missing.
fn evaluate(predicate: &Predicate, values: &[Values], scope: Scope) -> Option<bool> {
    match predicate {
        Predicate::Cmp {
            term,
            op,
            threshold,
        } => {
            let left = term_value(*term, values, scope)?;
            let right = threshold * to_base(term.metric());
            Some(match op {
                CmpOp::Lt => left < right,
                CmpOp::Le => left <= right,
                CmpOp::Gt => left > right,
                CmpOp::Ge => left >= right,
            })
        }
        // Both sides always evaluate, so a missing value on either poisons the
        // whole connective - `or` never rescues a skipped operand. Rust's `&&`
        // and `||` short-circuit, so the operands are resolved to values first
        // and only then combined.
        Predicate::And(lhs, rhs) => {
            let left = evaluate(lhs, values, scope)?;
            let right = evaluate(rhs, values, scope)?;
            Some(left && right)
        }
        Predicate::Or(lhs, rhs) => {
            let left = evaluate(lhs, values, scope)?;
            let right = evaluate(rhs, values, scope)?;
            Some(left || right)
        }
        Predicate::Not(inner) => evaluate(inner, values, scope).map(|verdict| !verdict),
    }
}

/// The factor from a threshold's written unit to the evaluator's base unit.
fn to_base(metric: Metric) -> f64 {
    Unit::from_label(metric.unit()).map_or(1.0, |unit| unit.to_base())
}

fn term_value(term: Term, values: &[Values], scope: Scope) -> Option<f64> {
    match (term, scope) {
        (Term::Point(metric), Scope::Point(index)) => metric_at(metric, values, index),
        (Term::Agg { func, metric }, Scope::Window(start, end)) => {
            let mut gathered = Vec::with_capacity(end - start);
            for index in start..end {
                gathered.push(metric_at(metric, values, index)?);
            }
            reduce(func, &gathered)
        }
        // The checker rejects a bare metric under a window and an aggregate
        // without one, so neither pairing can reach a run.
        (Term::Point(_), Scope::Window(..)) | (Term::Agg { .. }, Scope::Point(_)) => None,
    }
}

fn metric_at(metric: Metric, values: &[Values], index: usize) -> Option<f64> {
    let point = values.get(index)?;
    match metric {
        Metric::Velocity => point.velocity_mps,
        Metric::Eph => point.eph_m,
        Metric::Heading => point.heading_deg,
        // Run-local by construction: `values` is one run, so the first point of
        // it has no predecessor to difference against.
        Metric::Accel => {
            let previous = values.get(index.checked_sub(1)?)?;
            let delta_t = point.time_secs - previous.time_secs;
            if delta_t <= 0.0 {
                return None;
            }
            Some((point.velocity_mps? - previous.velocity_mps?) / delta_t)
        }
    }
    .filter(|value| value.is_finite())
}

fn reduce(func: Agg, values: &[f64]) -> Option<f64> {
    let count = values.len();
    if count == 0 {
        return None;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let value = match func {
        Agg::Avg => values.iter().sum::<f64>() / count as f64,
        Agg::Min => min,
        Agg::Max => max,
        Agg::Spread => max - min,
    };
    value.is_finite().then_some(value)
}
