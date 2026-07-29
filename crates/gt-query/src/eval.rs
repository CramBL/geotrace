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
use crate::check::{AggSource, ArithOp, CExpr, CheckedQuery, CheckedSource, CmpOp, Window};
use crate::metric::QueryMetric;

const FULL_TURN_DEG: f64 = 360.0;

/// Points evaluated between cancellation checks. Small enough to stop within
/// a frame or two, large enough that the check never shows up in a profile.
pub(crate) const CANCEL_CHECK_INTERVAL: usize = 4096;

/// A channel's samples over a time span: row-major values in the evaluator's
/// base units, `columns` per row. A scalar channel has one column; a vector
/// channel one per component (`@accel.x` is column 0). Empty when the channel
/// is unknown or has no samples in the span.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelSamples {
    /// Row-major values, `columns` per row.
    pub values: Vec<f64>,
    /// Values per row: 1 for a scalar channel, the component count otherwise.
    pub columns: usize,
}

impl ChannelSamples {
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The rows, each a slice of `columns` component values.
    fn rows(&self) -> impl Iterator<Item = &[f64]> {
        // `chunks_exact` panics on a zero size and silently drops a trailing
        // partial row, so guard the size and assert the row-major contract
        // rather than lose data (mirrors `Channel::slice_time_range`).
        let columns = self.columns.max(1);
        debug_assert_eq!(
            self.values.len() % columns,
            0,
            "channel samples must be a whole number of rows"
        );
        self.values.chunks_exact(columns)
    }
}

/// A channel's full sample timeline, for a query whose source is that channel:
/// each sample's time (seconds) and its row of component values, in base units.
/// `times.len()` is the sample count; `values` is row-major with `columns` per
/// row (one for a scalar channel). Empty when the channel is unknown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelTimeline {
    pub times: Vec<f64>,
    pub values: Vec<f64>,
    pub columns: usize,
}

impl ChannelTimeline {
    /// The `index`-th sample's row of component values, or `None` past the end.
    fn row(&self, index: usize) -> Option<&[f64]> {
        let columns = self.columns.max(1);
        self.values.get(index * columns..(index + 1) * columns)
    }
}

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

    /// A channel's native samples whose timestamp lands in the closed span
    /// `[t_lo, t_hi]` (seconds), in the channel's native order, as row-major
    /// values with one column per component (one column for a scalar channel).
    /// Values are in the evaluator's base units, like [`value`](Self::value) -
    /// the provider converts from the channel's stored unit. An unknown channel
    /// or a span with no samples yields empty samples. Providers with no
    /// channels use the default.
    fn channel_span(&self, _name: &str, _t_lo: f64, _t_hi: f64) -> ChannelSamples {
        ChannelSamples::default()
    }

    /// The full sample timeline of `name`, for a query whose source is that
    /// channel. Values are in base units. An unknown channel yields an empty
    /// timeline. Providers with no channels use the default.
    fn channel_timeline(&self, _name: &str) -> ChannelTimeline {
        ChannelTimeline::default()
    }
}

/// One track to run a query over.
pub struct TrackInput<'a, P: MetricProvider> {
    pub track: TrackRef,
    pub provider: &'a P,
}

// Hand-written so `Clone`/`Copy` hold for any `P`: the provider is a shared
// reference (always `Copy`), so neither needs `P: Clone`/`P: Copy`.
impl<P: MetricProvider> Clone for TrackInput<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: MetricProvider> Copy for TrackInput<'_, P> {}

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
    /// Timeline entries evaluated, summed over tracks: nav points for a points
    /// source, samples for a channel source - the denominator for the keep/hide
    /// "N of M points" summary.
    pub total_points: usize,
    /// Windows (or points, without a window) skipped per missing metric.
    pub skipped: BTreeMap<QueryMetric, usize>,
    /// Windows skipped per channel that had no samples in the window's span.
    pub skipped_channels: BTreeMap<String, usize>,
    /// Skips from non-finite arithmetic (e.g. division by zero) - kept
    /// separate so they stay visible without a metric to blame.
    pub skipped_non_finite: usize,
    /// Tracks that carried no value at all for a referenced metric - a
    /// track without a snap run (or one whose run left every point
    /// unsnapped), an eph-less receiver. Point-level skips land in
    /// [`Self::skipped`]; this names whole tracks the metric could never
    /// match on. Deliberately about values, not causes: the summary must
    /// not claim "never snapped" for a run that merely produced nothing.
    pub tracks_without: BTreeMap<QueryMetric, usize>,
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

pub fn run<P: MetricProvider>(query: &CheckedQuery, tracks: &[TrackInput<'_, P>]) -> RunOutput {
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
pub fn run_cancellable<P: MetricProvider>(
    query: &CheckedQuery,
    tracks: &[TrackInput<'_, P>],
    should_cancel: &impl Fn() -> bool,
) -> Option<RunOutput> {
    run_with_interval(query, tracks, should_cancel, CANCEL_CHECK_INTERVAL)
}

/// [`run_cancellable`] with the check interval exposed, so tests can cross
/// the interval boundary without a 4096-point fixture.
pub(crate) fn run_with_interval<P: MetricProvider>(
    query: &CheckedQuery,
    tracks: &[TrackInput<'_, P>],
    should_cancel: &impl Fn() -> bool,
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
        let eval = evaluate_track(query, input.provider, should_cancel, check_interval)?;
        for metric in absent_metrics(query.referenced_metrics(), input.provider) {
            *summary.tracks_without.entry(metric).or_insert(0) += 1;
        }
        // The timeline length is the matched mask's length: nav points for a
        // points source, samples for a channel source.
        summary.total_points += eval.matched.len();
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
    /// Windows skipped per channel with no samples in the span.
    pub skipped_channels: BTreeMap<String, usize>,
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
        for (channel, count) in &eval.skipped_channels {
            *self.skipped_channels.entry(channel.clone()).or_insert(0) += count;
        }
        self.skipped_non_finite += eval.skipped_non_finite;
        self.tracks_shorter_than_window += usize::from(eval.shorter_than_window);
    }
}

/// The referenced metrics `provider` carries no value for at any point -
/// whole-track absences like a missing snap run, a run that left every
/// point unsnapped, or an eph-less receiver. Absence is about values, not
/// causes: both no-run and all-unsnapped tracks count, and the summary
/// wording stays truthful for both.
/// `accel` derives from velocity, so its presence is probed through
/// velocity; channel-source queries reference no nav metrics, so they get
/// an empty list for free. An empty provider reports nothing: there are no
/// points the metric could have been missing on.
pub(crate) fn absent_metrics<P: MetricProvider>(
    metrics: &[QueryMetric],
    provider: &P,
) -> Vec<QueryMetric> {
    metrics
        .iter()
        .filter(|&&metric| {
            let probe = match metric {
                QueryMetric::Accel => QueryMetric::Velocity,
                other => other,
            };
            provider.len() > 0
                && (0..provider.len()).all(|index| provider.value(probe, index).is_none())
        })
        .copied()
        .collect()
}

/// Evaluate `query` over one provider, returning the matched mask and skips.
/// `None` only on cancellation. Windows and derived metrics are relative to
/// this provider, so running it over a [`crate::RunView`] gives gap-aware
/// evaluation for the pipeline.
pub(crate) fn evaluate_track<P: MetricProvider>(
    query: &CheckedQuery,
    provider: &P,
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<TrackEval> {
    match query.source() {
        // The nav points are the timeline: iterate points, windows group points.
        CheckedSource::Points => evaluate_points(query, provider, should_cancel, check_interval),
        // A channel's own samples are the timeline: iterate samples, windows
        // group samples, and a match is a range of sample indices.
        CheckedSource::Channel(name) => {
            evaluate_channel_source(query, provider, name, should_cancel, check_interval)
        }
    }
}

/// Evaluate a points-source query: the timeline is the provider's nav points.
fn evaluate_points<P: MetricProvider>(
    query: &CheckedQuery,
    provider: &P,
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<TrackEval> {
    let len = provider.len();
    let mut ctx = Ctx {
        provider,
        missing: BTreeSet::new(),
        missing_channels: BTreeSet::new(),
        non_finite: false,
    };
    let mut matched = vec![false; len];
    let mut skips = Skips::default();
    let mut shorter_than_window = false;

    match query.window() {
        // A count window at anchor `start` spans the points `[start, start+n)`.
        // Too few points for even one full window means nothing can match.
        Some(Window::Count(n)) if len < n => {
            shorter_than_window = true;
        }
        Some(Window::Count(n)) => {
            for start in 0..=(len - n) {
                if start % check_interval == 0 && should_cancel() {
                    return None;
                }
                let end = start + n;
                // A count window's channel span is the closed time extent of
                // its points, `[t(start), t(end-1)]`. A boundary point with no
                // timestamp (never, for nav points) leaves the span absent.
                let span = ctx
                    .raw(QueryMetric::Time, start)
                    .zip(ctx.raw(QueryMetric::Time, end - 1))
                    .map(|(lo, hi)| TimeSpan { lo, hi });
                let window = WindowScope { start, end, span };
                apply_window(query, &mut ctx, &mut matched, &mut skips, window);
            }
        }
        Some(Window::Duration(secs)) => {
            shorter_than_window = duration_windows(
                query,
                &mut ctx,
                &mut matched,
                &mut skips,
                secs,
                should_cancel,
                check_interval,
            )?;
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
        skipped_channels: skips.per_channel,
        skipped_non_finite: skips.non_finite,
        shorter_than_window,
    })
}

/// Evaluate one window: mark its points on a match, record a skip on a poisoned
/// window, do nothing on no-match.
fn apply_window<P: MetricProvider>(
    query: &CheckedQuery,
    ctx: &mut Ctx<'_, P>,
    matched: &mut [bool],
    skips: &mut Skips,
    window: WindowScope,
) {
    match verdict(query, ctx, Scope::Window(window)) {
        Some(true) => {
            for slot in matched
                .iter_mut()
                .skip(window.start)
                .take(window.end - window.start)
            {
                *slot = true;
            }
        }
        Some(false) => {}
        None => skips.record(ctx),
    }
}

/// Evaluate every `secs`-long duration window: at each anchor the contiguous
/// points whose time lands in `[t, t + secs)`, requiring the full duration to
/// fit within the data. Returns `None` on cancellation, else whether the track
/// was too short for any window to fit (the shorter-than-window flag).
fn duration_windows<P: MetricProvider>(
    query: &CheckedQuery,
    ctx: &mut Ctx<'_, P>,
    matched: &mut [bool],
    skips: &mut Skips,
    secs: f64,
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<bool> {
    let len = matched.len();
    // Each point's time places a window's span; a nav point always has one, so
    // in practice none are missing.
    let times: Vec<Option<f64>> = (0..len).map(|i| ctx.raw(QueryMetric::Time, i)).collect();
    // How far the data reaches, which bounds where a full window fits. Taken as
    // the max rather than the last value: this crate does not assume per-track
    // time is monotonic (see `derived_accel`, which flags backward steps), so a
    // clock jump must not make later anchors vanish.
    let max_time = times
        .iter()
        .flatten()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let mut any_fit = false;
    for start in 0..len {
        if start % check_interval == 0 && should_cancel() {
            return None;
        }
        let Some(t_start) = times.get(start).copied().flatten() else {
            continue;
        };
        // The full duration must fit: the data has to reach `t_start + secs`.
        // Skip (not break) an anchor whose window overruns, since without a
        // monotonic-time assumption a later anchor may still fit.
        if t_start + secs > max_time {
            continue;
        }
        any_fit = true;
        // The contiguous points with time in `[t_start, t_start + secs)`; the
        // anchor itself always fits.
        let end_time = t_start + secs;
        let mut end = start;
        while end < len
            && times
                .get(end)
                .copied()
                .flatten()
                .is_some_and(|t| t < end_time)
        {
            end += 1;
        }
        // The channel span is the window's declared time extent `[t_start,
        // t_start + secs]`; gathering is closed at the top, differing from the
        // half-open point rule only at an exact-float boundary.
        let window = WindowScope {
            start,
            end,
            span: Some(TimeSpan {
                lo: t_start,
                hi: end_time,
            }),
        };
        apply_window(query, ctx, matched, skips, window);
    }
    Some(!any_fit)
}

/// Evaluate a channel-source query: `name`'s own samples are the timeline, so a
/// match is a range of sample indices. Without a window each sample is judged on
/// its own row; a count window groups consecutive samples and a duration window
/// a time span, for the aggregates to reduce over.
fn evaluate_channel_source<P: MetricProvider>(
    query: &CheckedQuery,
    provider: &P,
    name: &str,
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<TrackEval> {
    let timeline = provider.channel_timeline(name);
    let len = timeline.times.len();
    let mut ctx = Ctx {
        provider,
        missing: BTreeSet::new(),
        missing_channels: BTreeSet::new(),
        non_finite: false,
    };
    let mut matched = vec![false; len];
    let mut skips = Skips::default();
    let mut shorter_than_window = false;

    match query.window() {
        Some(Window::Count(n)) if len < n => shorter_than_window = true,
        Some(Window::Count(n)) => {
            for start in 0..=(len - n) {
                if start % check_interval == 0 && should_cancel() {
                    return None;
                }
                apply_sample_window(
                    query,
                    &mut ctx,
                    &mut matched,
                    &mut skips,
                    &timeline,
                    start..start + n,
                );
            }
        }
        Some(Window::Duration(secs)) => {
            shorter_than_window = sample_duration_windows(
                query,
                &mut ctx,
                &mut matched,
                &mut skips,
                &timeline,
                secs,
                should_cancel,
                check_interval,
            )?;
        }
        None => {
            for start in 0..len {
                if start % check_interval == 0 && should_cancel() {
                    return None;
                }
                let Some(row) = timeline.row(start) else {
                    continue;
                };
                match verdict(query, &mut ctx, Scope::Sample(row)) {
                    Some(true) => {
                        if let Some(slot) = matched.get_mut(start) {
                            *slot = true;
                        }
                    }
                    Some(false) => {}
                    None => skips.record(&ctx),
                }
            }
        }
    }
    Some(TrackEval {
        matched,
        skipped: skips.per_metric,
        skipped_channels: skips.per_channel,
        skipped_non_finite: skips.non_finite,
        shorter_than_window,
    })
}

/// Evaluate one channel-source window over the `samples` index range: mark them
/// on a match, record a skip on a poisoned window, do nothing on no-match.
fn apply_sample_window<P: MetricProvider>(
    query: &CheckedQuery,
    ctx: &mut Ctx<'_, P>,
    matched: &mut [bool],
    skips: &mut Skips,
    timeline: &ChannelTimeline,
    samples: Range<usize>,
) {
    let columns = timeline.columns.max(1);
    let Some(rows) = timeline
        .values
        .get(samples.start * columns..samples.end * columns)
    else {
        return;
    };
    match verdict(query, ctx, Scope::SampleWindow { rows, columns }) {
        Some(true) => {
            for slot in matched.iter_mut().skip(samples.start).take(samples.len()) {
                *slot = true;
            }
        }
        Some(false) => {}
        None => skips.record(ctx),
    }
}

/// Every `secs`-long duration window over a channel-source timeline: at each
/// anchor the contiguous samples whose time lands in `[t, t + secs)`, requiring
/// the full duration to fit. Mirrors [`duration_windows`] over the channel's
/// sample times. Returns `None` on cancellation, else the shorter-than-window
/// flag.
#[expect(
    clippy::too_many_arguments,
    reason = "the channel-source window loop threads the same evaluation state as duration_windows, plus the sample timeline; a struct would not group anything meaningfully reusable"
)]
fn sample_duration_windows<P: MetricProvider>(
    query: &CheckedQuery,
    ctx: &mut Ctx<'_, P>,
    matched: &mut [bool],
    skips: &mut Skips,
    timeline: &ChannelTimeline,
    secs: f64,
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<bool> {
    let len = timeline.times.len();
    // As in duration_windows, the reach is the max time, not the last, since
    // sample time is not assumed monotonic.
    let max_time = timeline
        .times
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let mut any_fit = false;
    for start in 0..len {
        if start % check_interval == 0 && should_cancel() {
            return None;
        }
        let Some(&t_start) = timeline.times.get(start) else {
            continue;
        };
        if t_start + secs > max_time {
            continue;
        }
        any_fit = true;
        let end_time = t_start + secs;
        let mut end = start;
        while end < len && timeline.times.get(end).is_some_and(|&t| t < end_time) {
            end += 1;
        }
        apply_sample_window(query, ctx, matched, skips, timeline, start..end);
    }
    Some(!any_fit)
}

/// Skip tallies accumulated during one track's evaluation, kept apart from the
/// matched mask so both can be mutated in the same loop.
#[derive(Default)]
struct Skips {
    per_metric: BTreeMap<QueryMetric, usize>,
    per_channel: BTreeMap<String, usize>,
    non_finite: usize,
}

impl Skips {
    fn record<P: MetricProvider>(&mut self, ctx: &Ctx<'_, P>) {
        for metric in &ctx.missing {
            *self.per_metric.entry(*metric).or_insert(0) += 1;
        }
        for channel in &ctx.missing_channels {
            *self.per_channel.entry(channel.clone()).or_insert(0) += 1;
        }
        if ctx.non_finite {
            self.non_finite += 1;
        }
    }
}

/// All `where` stages must hold; a missing value in any of them poisons the
/// whole point or window to "skipped".
fn verdict<P: MetricProvider>(
    query: &CheckedQuery,
    ctx: &mut Ctx<'_, P>,
    scope: Scope,
) -> Option<bool> {
    ctx.missing.clear();
    ctx.missing_channels.clear();
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

/// The closed time span `[lo, hi]` (seconds) a window's channel samples are
/// gathered from.
#[derive(Clone, Copy)]
struct TimeSpan {
    lo: f64,
    hi: f64,
}

/// A window over the points `start..end`, plus the time span its channel
/// samples come from. The span is absent only when a boundary point has no
/// timestamp (never for nav points); a channel aggregate then reports a missing
/// time rather than blaming the channel.
#[derive(Clone, Copy)]
struct WindowScope {
    start: usize,
    end: usize,
    span: Option<TimeSpan>,
}

#[derive(Clone, Copy)]
enum Scope<'a> {
    Point(usize),
    /// A point aggregate reduces its metric over the window's points; a channel
    /// aggregate reduces the channel's samples over the window's time span.
    Window(WindowScope),
    /// One native channel sample: the row of component values (one for a scalar
    /// channel). A `@name.x` node reads its column; `norm` reads the whole row.
    Sample(&'a [f64]),
    /// A window of a channel-source timeline: the samples' rows, row-major with
    /// `columns` per row. An aggregate reduces its argument over these rows.
    SampleWindow {
        rows: &'a [f64],
        columns: usize,
    },
}

struct Ctx<'a, P: MetricProvider> {
    provider: &'a P,
    /// Metrics that came up missing in the current point/window evaluation.
    missing: BTreeSet<QueryMetric>,
    /// Channels with no samples in the current window's span.
    missing_channels: BTreeSet<String>,
    non_finite: bool,
}

impl<P: MetricProvider> Ctx<'_, P> {
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
fn raw_value<P: MetricProvider>(provider: &P, metric: QueryMetric, index: usize) -> Option<f64> {
    provider.value(metric, index).filter(|v| v.is_finite())
}

/// The derived `accel` metric: backward difference of velocity over time, in
/// m/s2. Missing on the first point of a track, wherever velocity is missing,
/// and on non-increasing timestamps (a clock anomaly cannot yield a
/// meaningful accel).
///
/// Public so the UI can show the same value in match tables that the
/// evaluator used in predicates - this is the single definition of `accel`.
pub fn derived_accel<P: MetricProvider>(provider: &P, index: usize) -> Option<f64> {
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
fn eval_bool<P: MetricProvider>(ctx: &mut Ctx<'_, P>, expr: &CExpr, scope: Scope) -> Option<bool> {
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
        | CExpr::Channel(_)
        | CExpr::Norm(_)
        | CExpr::Agg { .. }
        | CExpr::Abs(_)
        | CExpr::Sqrt(_)
        | CExpr::Neg(_)
        | CExpr::Arith { .. }
        | CExpr::Power { .. } => None,
    }
}

/// Value nodes, in base units.
fn eval_num<P: MetricProvider>(ctx: &mut Ctx<'_, P>, expr: &CExpr, scope: Scope) -> Option<f64> {
    match expr {
        CExpr::Const(v) => Some(*v),
        CExpr::Metric(metric) => match scope {
            Scope::Point(index) => ctx.metric_at(*metric, index),
            // The checker forbids a bare metric in a windowed predicate, rejects
            // a nav-point metric inside a channel aggregate, and rejects metrics
            // on a channel source, so no window or sample scope reaches here.
            Scope::Window(_) | Scope::Sample(_) | Scope::SampleWindow { .. } => None,
        },
        // A channel node reads its component's column of the current sample row
        // inside a channel aggregate (or per sample on a channel source); in any
        // other scope it has no value. The checker keeps the column in range.
        CExpr::Channel(key) => match scope {
            Scope::Sample(row) => row.get(key.component.unwrap_or(0)).copied(),
            Scope::Point(_) | Scope::Window(_) | Scope::SampleWindow { .. } => None,
        },
        // norm is the Euclidean magnitude of the whole sample row. A non-finite
        // component (an absent sample) poisons it like any undefined arithmetic.
        CExpr::Norm(_) => match scope {
            Scope::Sample(row) => {
                let result = row.iter().map(|v| v * v).sum::<f64>().sqrt();
                if !result.is_finite() {
                    ctx.non_finite = true;
                    return None;
                }
                Some(result)
            }
            Scope::Point(_) | Scope::Window(_) | Scope::SampleWindow { .. } => None,
        },
        CExpr::Agg {
            func,
            circular,
            source,
            arg,
        } => aggregate(ctx, *func, *circular, source, arg, scope),
        CExpr::Abs(inner) => eval_num(ctx, inner, scope).map(f64::abs),
        CExpr::Sqrt(inner) => {
            // A negative radicand roots to NaN; poison it like any undefined
            // arithmetic rather than comparing a NaN.
            let result = eval_num(ctx, inner, scope)?.sqrt();
            if !result.is_finite() {
                ctx.non_finite = true;
                return None;
            }
            Some(result)
        }
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
        CExpr::Power { base, exponent } => {
            let result = eval_num(ctx, base, scope)?.powi(i32::from(*exponent));
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

fn both_nums<P: MetricProvider>(
    ctx: &mut Ctx<'_, P>,
    lhs: &CExpr,
    rhs: &CExpr,
    scope: Scope,
) -> Option<(f64, f64)> {
    let l = eval_num(ctx, lhs, scope);
    let r = eval_num(ctx, rhs, scope);
    Some((l?, r?))
}

/// Evaluate the aggregate argument once per native sample of channel `name` in
/// the window's time span, each argument seeing that sample's whole row (so a
/// `@name.x`/`@name.y`/`norm` reads aligned columns). An absent span (a boundary
/// point had no timestamp) reports a missing time; a span with no samples
/// reports the missing channel. Either way the aggregate poisons rather than
/// matching silently.
fn reduce_channel<P: MetricProvider>(
    ctx: &mut Ctx<'_, P>,
    name: &str,
    arg: &CExpr,
    window: WindowScope,
) -> Option<Vec<f64>> {
    let Some(span) = window.span else {
        ctx.missing.insert(QueryMetric::Time);
        return None;
    };
    let samples = ctx.provider.channel_span(name, span.lo, span.hi);
    if samples.is_empty() {
        ctx.missing_channels.insert(name.to_owned());
        return None;
    }
    let mut values = Vec::with_capacity(samples.values.len() / samples.columns.max(1));
    for row in samples.rows() {
        values.push(eval_num(ctx, arg, Scope::Sample(row))?);
    }
    Some(values)
}

/// Reduce the aggregate argument over the window: a channel's native samples in
/// the time span, or the metric's values over the window's points, per the
/// source the checker resolved. Any missing value poisons the whole aggregate.
fn aggregate<P: MetricProvider>(
    ctx: &mut Ctx<'_, P>,
    func: Func,
    circular: bool,
    source: &AggSource,
    arg: &CExpr,
    scope: Scope,
) -> Option<f64> {
    let values = match scope {
        // A points-source window: reduce the metric over its points, or the
        // channel's samples over the window's time span.
        Scope::Window(window) => match source {
            AggSource::Channel(name) => reduce_channel(ctx, name, arg, window)?,
            AggSource::Points => {
                let mut values = Vec::with_capacity(window.end - window.start);
                for index in window.start..window.end {
                    values.push(eval_num(ctx, arg, Scope::Point(index))?);
                }
                values
            }
        },
        // A channel-source window: reduce over the window's own sample rows.
        Scope::SampleWindow { rows, columns } => {
            let mut values = Vec::with_capacity(rows.len() / columns.max(1));
            for row in rows.chunks_exact(columns.max(1)) {
                values.push(eval_num(ctx, arg, Scope::Sample(row))?);
            }
            values
        }
        Scope::Point(_) | Scope::Sample(_) => return None,
    };
    reduce_values(func, circular, values, ctx)
}

/// Reduce a window's gathered per-sample `values` by `func`. A non-finite
/// result (overflow, or the circular-std singularity) poisons rather than
/// comparing as a bare infinity.
fn reduce_values<P: MetricProvider>(
    func: Func,
    circular: bool,
    mut values: Vec<f64>,
    ctx: &mut Ctx<'_, P>,
) -> Option<f64> {
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
        // The checker rejects var on a direction, so it is always linear.
        Func::Var => population_variance(&values),
        // The checker never emits abs, sqrt, or norm as an aggregate; they are
        // their own scalar `CExpr` nodes.
        Func::Abs | Func::Sqrt | Func::Norm => return None,
    };
    // Like arithmetic (see `eval_num`), an aggregate never hands a non-finite
    // value to a comparison: an overflowing sum (in `avg` or `population_std`),
    // or the circular-std singularity of a window with no resultant direction,
    // poisons and is reported as skipped rather than comparing as a bare
    // infinity.
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
    population_variance(values).sqrt()
}

/// Population variance (divided by N): the mean squared deviation. Its unit is
/// the square of the values' unit, so a query compares it only to another
/// squared quantity or feeds it to `sqrt` (which is `std`).
fn population_variance(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n
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
        assert!((population_variance(&values) - 4.0).abs() < 1e-12);
        // A single value has no spread.
        assert!(population_std(&[42.0]).abs() < 1e-12);
        assert!(population_variance(&[42.0]).abs() < 1e-12);
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
