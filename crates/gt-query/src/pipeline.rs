//! Running several queries as one sequential pipeline.
//!
//! Each query is a full `points | ... | mode` pipeline. They compose by
//! folding a per-track visibility set: `hide`/`keep` shrink it, `draw` records
//! a colored layer over what is still visible. A later query never sees a point
//! an earlier one hid. Each query evaluates over the maximal runs of contiguous
//! survivors (via [`RunView`]), so a window never spans a hidden point and
//! derived metrics reset at each gap.

use std::collections::BTreeMap;
use std::ops::Range;

use gt_types::{DisplayMode, TrackRef};

use crate::check::{CheckedQuery, TableColumn};
use crate::eval::{
    CANCEL_CHECK_INTERVAL, ChannelSamples, ChannelTimeline, MetricProvider, RunSummary, TrackInput,
    TrackMatches, evaluate_track, ranges_from,
};
use crate::metric::QueryMetric;

/// Feeds the per-query results panel.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryOutput {
    pub mode: DisplayMode,
    pub matches: Vec<TrackMatches>,
    /// The match table's columns, as [`crate::RunOutput::columns`].
    pub columns: Vec<TableColumn>,
    pub summary: RunSummary,
}

/// A `draw` query's halos: the points it matched that are still visible in the
/// final composed result. Contributions are returned in draw order, which the
/// caller maps to distinct colors (the map-facing `gt_ui_types::DrawLayer`).
#[derive(Debug, Clone, PartialEq)]
pub struct DrawContribution {
    /// Index into [`PipelineOutput::queries`] of the drawing query.
    pub query_index: usize,
    pub matches: Vec<TrackMatches>,
}

/// The composed result of running several queries in order.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineOutput {
    /// Per query, in editor order.
    pub queries: Vec<QueryOutput>,
    /// Final hidden point ranges per track: where the polyline breaks.
    pub hidden: Vec<TrackMatches>,
    /// Halo layers, one per `draw` query, in draw order.
    pub draws: Vec<DrawContribution>,
}

/// A window onto a contiguous run of one provider's points, reindexed to
/// `0..len`. Evaluating a query over it makes windows and derived metrics
/// run-local: `accel` at run-local `0` is missing, and a window cannot reach
/// past the run's edges into a hidden gap.
struct RunView<'a, P: MetricProvider> {
    inner: &'a P,
    start: usize,
    len: usize,
}

impl<P: MetricProvider> MetricProvider for RunView<'_, P> {
    fn len(&self) -> usize {
        self.len
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        if index >= self.len {
            return None;
        }
        self.inner.value(metric, self.start + index)
    }

    fn point_can_match(&self, index: usize) -> bool {
        index < self.len && self.inner.point_can_match(self.start + index)
    }

    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        // Channel samples are keyed by absolute time, so the run's time window
        // selects them directly from the underlying provider - no index offset.
        self.inner.channel_span(name, t_lo, t_hi)
    }

    fn channel_timeline(&self, name: &str) -> ChannelTimeline {
        // A channel's timeline is its own sample clock, independent of the point
        // run this view slices, so it forwards whole.
        self.inner.channel_timeline(name)
    }
}

/// Run `queries` in order over `tracks`. `None` only on cancellation.
pub fn run_pipeline<P: MetricProvider>(
    queries: &[CheckedQuery],
    tracks: &[TrackInput<'_, P>],
    should_cancel: &impl Fn() -> bool,
) -> Option<PipelineOutput> {
    run_pipeline_with_interval(queries, tracks, should_cancel, CANCEL_CHECK_INTERVAL)
}

fn run_pipeline_with_interval<P: MetricProvider>(
    queries: &[CheckedQuery],
    tracks: &[TrackInput<'_, P>],
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<PipelineOutput> {
    let mut query_accums: Vec<QueryAccum> = queries.iter().map(QueryAccum::new).collect();
    let draw_indices: Vec<usize> = queries
        .iter()
        .enumerate()
        .filter(|(_, q)| q.mode() == DisplayMode::Draw)
        .map(|(i, _)| i)
        .collect();
    let mut draw_layers: Vec<DrawContribution> = draw_indices
        .iter()
        .map(|&query_index| DrawContribution {
            query_index,
            matches: Vec::new(),
        })
        .collect();
    let mut hidden = Vec::new();

    for input in tracks {
        if should_cancel() {
            return None;
        }
        let fold = fold_track(queries, input, should_cancel, check_interval)?;

        for (accum, contrib) in query_accums.iter_mut().zip(fold.per_query) {
            accum.absorb(input.track, contrib);
        }
        for (layer, ranges) in draw_layers.iter_mut().zip(fold.draw_ranges) {
            if !ranges.is_empty() {
                layer.matches.push(TrackMatches {
                    track: input.track,
                    ranges,
                });
            }
        }
        if !fold.hidden.is_empty() {
            hidden.push(TrackMatches {
                track: input.track,
                ranges: fold.hidden,
            });
        }
    }

    Some(PipelineOutput {
        queries: query_accums.into_iter().map(QueryAccum::finish).collect(),
        hidden,
        draws: draw_layers,
    })
}

/// One track's fold: each query's contribution (in query order), each draw
/// query's final-visible ranges (in draw order), and the final hidden ranges.
struct TrackFold {
    per_query: Vec<QueryContribution>,
    draw_ranges: Vec<Vec<Range<usize>>>,
    hidden: Vec<Range<usize>>,
}

/// One query's effect on one track at its evaluation step.
struct QueryContribution {
    ranges: Vec<Range<usize>>,
    matched_points: usize,
    visible_points: usize,
    skipped: BTreeMap<QueryMetric, usize>,
    skipped_non_finite: usize,
    /// Referenced metrics the whole track carried no value for, probed
    /// against the full provider (not the visibility-narrowed views): a
    /// track without a snap run lacks `snap_error` regardless of what an
    /// earlier stage hid.
    absent: Vec<QueryMetric>,
    shorter_than_window: bool,
}

fn fold_track<P: MetricProvider>(
    queries: &[CheckedQuery],
    input: &TrackInput<'_, P>,
    should_cancel: &impl Fn() -> bool,
    check_interval: usize,
) -> Option<TrackFold> {
    let len = input.provider.len();
    let mut visible = vec![true; len];
    let mut per_query = Vec::with_capacity(queries.len());
    let mut draw_ranges = Vec::new();
    // A draw query's matched mask is kept until the fold ends, then
    // intersected with the final visibility so a later hide removes its halo.
    let mut draw_matched: Vec<Vec<bool>> = Vec::new();

    for query in queries {
        let runs = ranges_from(&visible);
        let visible_points = runs.iter().map(Range::len).sum();
        let mut matched = vec![false; len];
        let mut skipped: BTreeMap<QueryMetric, usize> = BTreeMap::new();
        let mut skipped_non_finite = 0;
        let mut run_long_enough = false;

        for run in &runs {
            let view = RunView {
                inner: input.provider,
                start: run.start,
                len: run.len(),
            };
            let eval = evaluate_track(query, &view, should_cancel, check_interval)?;
            if let Some(dest) = matched.get_mut(run.clone()) {
                for (slot, hit) in dest.iter_mut().zip(&eval.matched) {
                    *slot |= *hit;
                }
            }
            for (metric, count) in eval.skipped {
                *skipped.entry(metric).or_insert(0) += count;
            }
            skipped_non_finite += eval.skipped_non_finite;
            run_long_enough |= !eval.shorter_than_window;
        }

        let matched_points = matched.iter().filter(|m| **m).count();
        per_query.push(QueryContribution {
            ranges: ranges_from(&matched),
            matched_points,
            visible_points,
            skipped,
            skipped_non_finite,
            absent: crate::eval::absent_metrics(query.referenced_metrics(), input.provider),
            shorter_than_window: query.window().is_some() && !run_long_enough,
        });

        // `shows` is the shared keep/hide/draw truth table. Draw shows every
        // point, so the fold leaves visibility untouched and only records the
        // matched mask for the final halo pass.
        for (vis, hit) in visible.iter_mut().zip(&matched) {
            *vis &= query.mode().shows(*hit);
        }
        if query.mode() == DisplayMode::Draw {
            draw_matched.push(matched);
        }
    }

    // Halos only on points visible in the final result.
    for matched in draw_matched {
        let shown: Vec<bool> = matched
            .iter()
            .zip(&visible)
            .map(|(&hit, &vis)| hit && vis)
            .collect();
        draw_ranges.push(ranges_from(&shown));
    }
    let hidden_mask: Vec<bool> = visible.iter().map(|&v| !v).collect();

    Some(TrackFold {
        per_query,
        draw_ranges,
        hidden: ranges_from(&hidden_mask),
    })
}

/// Accumulates one query's contributions across tracks into a [`QueryOutput`].
struct QueryAccum {
    mode: DisplayMode,
    columns: Vec<TableColumn>,
    matches: Vec<TrackMatches>,
    summary: RunSummary,
}

impl QueryAccum {
    fn new(query: &CheckedQuery) -> Self {
        Self {
            mode: query.mode(),
            columns: query.columns().to_vec(),
            matches: Vec::new(),
            summary: RunSummary {
                unused_params: query.unused_params().to_vec(),
                ..RunSummary::default()
            },
        }
    }

    fn absorb(&mut self, track: TrackRef, contrib: QueryContribution) {
        self.summary.total_points += contrib.visible_points;
        self.summary.matched_points += contrib.matched_points;
        self.summary.skipped_non_finite += contrib.skipped_non_finite;
        self.summary.tracks_shorter_than_window += usize::from(contrib.shorter_than_window);
        for (metric, count) in contrib.skipped {
            *self.summary.skipped.entry(metric).or_insert(0) += count;
        }
        for metric in contrib.absent {
            *self.summary.tracks_without.entry(metric).or_insert(0) += 1;
        }
        if !contrib.ranges.is_empty() {
            self.summary.tracks_with_matches += 1;
            self.summary.match_count += contrib.ranges.len();
            self.matches.push(TrackMatches {
                track,
                ranges: contrib.ranges,
            });
        }
    }

    fn finish(self) -> QueryOutput {
        QueryOutput {
            mode: self.mode,
            matches: self.matches,
            columns: self.columns,
            summary: self.summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use gt_types::{FileIdx, TrackIdx};

    use super::*;
    use crate::{ChannelSchema, check, parse};

    fn track() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    /// Velocity in m/s and a 1 s-per-point clock; other metrics are missing.
    struct Speeds(Vec<f64>);

    impl MetricProvider for Speeds {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
            match metric {
                QueryMetric::Velocity => self.0.get(index).copied(),
                QueryMetric::Time => (index < self.0.len()).then_some(index as f64),
                _ => None,
            }
        }
    }

    /// Each stage's summary counts the track once per metric it references
    /// and the track lacks - a `snap_error` stage names the run-less track,
    /// the velocity stage next to it does not.
    #[test]
    fn stages_count_tracks_without_their_own_metrics() {
        let provider = Speeds(vec![5.0, 10.0, 9.0]);
        let output = compose(
            &[
                "points | where snap_error > 1 m",
                "points | where velocity > 1 km/h",
            ],
            &provider,
        );
        let summaries: Vec<_> = output
            .queries
            .iter()
            .map(|q| q.summary.tracks_without.clone())
            .collect();
        assert_eq!(
            summaries,
            vec![
                BTreeMap::from([(QueryMetric::SnapError, 1)]),
                BTreeMap::new(),
            ]
        );
    }

    fn compose(srcs: &[&str], provider: &impl MetricProvider) -> PipelineOutput {
        let queries: Vec<CheckedQuery> = srcs
            .iter()
            .map(|s| check(&parse(s).expect(s), &ChannelSchema::new()).expect(s))
            .collect();
        let inputs = [TrackInput {
            track: track(),
            provider,
        }];
        run_pipeline(&queries, &inputs, &|| false).expect("not cancelled")
    }

    fn hidden_ranges(out: &PipelineOutput) -> Vec<Range<usize>> {
        out.hidden
            .first()
            .map(|h| h.ranges.clone())
            .unwrap_or_default()
    }

    fn draw_ranges(out: &PipelineOutput, layer: usize) -> Vec<Range<usize>> {
        out.draws
            .get(layer)
            .and_then(|l| l.matches.first())
            .map(|m| m.ranges.clone())
            .unwrap_or_default()
    }

    #[test]
    fn hide_then_draw_evaluates_over_survivors() {
        // Slow at the ends, fast in the middle.
        let provider = Speeds(vec![5.0, 5.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 5.0, 5.0]);
        let out = compose(
            &[
                "points | where velocity < 10 m/s | hide",
                "points | window 3 | where avg(velocity) > 10 m/s | draw",
            ],
            &provider,
        );
        // The slow ends are hidden.
        assert_eq!(hidden_ranges(&out), vec![0..2, 8..10]);
        // The draw windows only cover the fast survivors.
        assert_eq!(draw_ranges(&out, 0), vec![2..8]);
    }

    #[test]
    fn a_window_never_spans_a_hidden_point() {
        // Every fifth point is slow, leaving survivor runs of four.
        let speeds: Vec<f64> = (0..20)
            .map(|i| if i % 5 == 4 { 0.0 } else { 20.0 })
            .collect();
        let provider = Speeds(speeds);
        let out = compose(
            &[
                "points | where velocity < 10 m/s | hide",
                "points | window 10 | where avg(velocity) > 5 m/s | draw",
            ],
            &provider,
        );
        assert_eq!(hidden_ranges(&out), vec![4..5, 9..10, 14..15, 19..20]);
        // No run reaches ten points, so the window matches nothing.
        assert!(draw_ranges(&out, 0).is_empty());
    }

    #[test]
    fn a_later_hide_removes_an_earlier_halo() {
        let provider = Speeds(vec![20.0, 40.0, 20.0, 40.0]);
        let out = compose(
            &[
                "points | where velocity > 10 m/s | draw",
                "points | where velocity < 30 m/s | hide",
            ],
            &provider,
        );
        // Every point was drawn, but the two slow ones are then hidden.
        assert_eq!(hidden_ranges(&out), vec![0..1, 2..3]);
        assert_eq!(draw_ranges(&out, 0), vec![1..2, 3..4]);
    }

    #[test]
    fn keep_hides_the_non_matching_points() {
        // 30 km/h is 8.33 m/s. Points 1 and 3 exceed it.
        let provider = Speeds(vec![5.0, 20.0, 5.0, 20.0]);
        let out = compose(&["points | where velocity > 30 km/h | keep"], &provider);
        assert_eq!(hidden_ranges(&out), vec![0..1, 2..3]);
        assert_eq!(out.queries.first().map(|q| q.mode), Some(DisplayMode::Keep));
        assert!(out.draws.is_empty());
    }

    #[test]
    fn one_draw_query_matches_a_plain_run() {
        // A single draw query behaves like a lone run: halos, nothing hidden.
        let provider = Speeds(vec![5.0, 20.0, 20.0, 5.0]);
        let out = compose(&["points | where velocity > 10 m/s | draw"], &provider);
        assert!(hidden_ranges(&out).is_empty());
        assert_eq!(draw_ranges(&out, 0), vec![1..3]);
    }

    #[test]
    fn accel_resets_across_a_hidden_gap() {
        // A slow point in the middle is hidden, splitting the track into two
        // runs. `accel` differences velocity over time within a run only.
        let provider = Speeds(vec![10.0, 20.0, 1.0, 100.0, 100.0, 100.0]);
        let out = compose(
            &[
                "points | where velocity < 5 m/s | hide",
                "points | where accel > 2 m/s2 | draw",
            ],
            &provider,
        );
        assert_eq!(hidden_ranges(&out), vec![2..3]);
        // The 10->20 step inside the first run is a real acceleration. The
        // 1->100 jump is not matched: accel is missing at the start of the
        // second run.
        assert_eq!(draw_ranges(&out, 0), vec![1..2]);
    }

    #[test]
    fn cancellation_stops_the_pipeline_without_partial_results() {
        let provider = Speeds(vec![20.0; 8]);
        let queries: Vec<CheckedQuery> = [
            "points | where velocity > 0 m/s | hide",
            "points | where velocity > 0 m/s | draw",
        ]
        .iter()
        .map(|s| check(&parse(s).expect(s), &ChannelSchema::new()).expect(s))
        .collect();
        let inputs = [TrackInput {
            track: track(),
            provider: &provider,
        }];

        // Interval 1 checks every point. Cancel after a few so the stop lands
        // inside a run's scan, not only at the per-track entry.
        let calls = std::cell::Cell::new(0_u32);
        let cancel_after_three = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        assert_eq!(
            run_pipeline_with_interval(&queries, &inputs, &cancel_after_three, 1),
            None
        );

        // The same pipeline completes when never cancelled.
        assert!(run_pipeline_with_interval(&queries, &inputs, &|| false, 1).is_some());
    }
}
