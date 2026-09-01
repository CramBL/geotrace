use std::fmt;
use std::ops::Range;

use geotrace_sdk_units::ChannelUnit;
use gt_fmt::EM_DASH;
use gt_query::{AggregateColumn, ChannelTimeline, RunSummary, TableColumn, TrackMatches};
use gt_types::{DisplayMode, NavPoint, TrackRef};
use gt_ui_types::{DrawLayer, DrawLayerMask, QueryMatches, TrackRanges};
use rustc_hash::FxHashMap;

use crate::provider::TrackQueryData;
use crate::run::{ChannelRun, PointsQueryRun, PointsRun, RunTrackData};

/// What one run produced, dispatched on the source of its queries: either a
/// points pipeline (map halos plus point match tables) or a channel-source run
/// (sample match tables plus halos over the matched track segments).
pub enum RunResults {
    Points(PointsResults),
    Channel(ChannelResults),
}

impl RunResults {
    /// The map effect: halos and hidden ranges, with the staleness flag the
    /// renderer grays out on.
    pub fn matches(&self) -> &QueryMatches {
        match self {
            Self::Points(points) => &points.matches,
            Self::Channel(channel) => &channel.matches,
        }
    }

    /// Whether anything the run depended on has changed since it ran.
    pub fn stale(&self) -> bool {
        self.matches().stale
    }

    pub(crate) fn set_stale(&mut self, stale: bool) {
        match self {
            Self::Points(points) => points.matches.stale = stale,
            Self::Channel(channel) => channel.matches.stale = stale,
        }
    }

    /// Stamp the sequence number of the run that produced these results, which
    /// the map reveals a new run's halos by.
    pub(crate) fn set_run(&mut self, run: u64) {
        match self {
            Self::Points(points) => points.matches.run = run,
            Self::Channel(channel) => channel.matches.run = run,
        }
    }
}

/// A points-pipeline run: its composed map effect and per-query panel rows.
pub struct PointsResults {
    /// The composed display effect for the map.
    pub matches: QueryMatches,
    /// Per query, in editor order, for the results panel.
    pub queries: Vec<PanelQuery>,
    /// Per-track derived series (only for metrics some query referenced),
    /// kept so match tables show the exact values the run used.
    track_data: RunTrackData,
}

impl PointsResults {
    /// Project a points run into the panel/map result. Point indices shift back
    /// to absolute positions here: evaluation ran on each track's time-filtered
    /// slice.
    pub(crate) fn project(run: PointsRun, track_data: RunTrackData) -> Self {
        let slice_start = |track: TrackRef| {
            track_data
                .get(&track)
                .map_or(0, |data| data.filtered_points().start())
        };
        let absolute = |tms: &[TrackMatches]| -> TrackRanges {
            tms.iter()
                .filter(|tm| !tm.ranges.is_empty())
                .map(|tm| {
                    let start = slice_start(tm.track);
                    let ranges = tm
                        .ranges
                        .iter()
                        .map(|r| r.start + start..r.end + start)
                        .collect();
                    (tm.track, ranges)
                })
                .collect()
        };

        // The point-key mask distinguishes only so many halo layers. Extra draw
        // queries beyond that cannot be rendered distinctly.
        if run.draws.len() > DrawLayerMask::MAX_LAYERS {
            log::warn!(
                "query has {} draw stages: only the first {} render as halos",
                run.draws.len(),
                DrawLayerMask::MAX_LAYERS
            );
        }

        // The i-th draw query gets palette color i. The map keys its halo layer
        // to the same order.
        let draw_color: FxHashMap<usize, usize> = run
            .draws
            .iter()
            .take(DrawLayerMask::MAX_LAYERS)
            .enumerate()
            .map(|(order, layer)| (layer.query_index, order))
            .collect();

        let matches = QueryMatches {
            hidden: absolute(&run.hidden),
            draws: run
                .draws
                .iter()
                .take(DrawLayerMask::MAX_LAYERS)
                .enumerate()
                .map(|(color, layer)| DrawLayer {
                    color,
                    ranges: absolute(&layer.matches),
                })
                .collect(),
            stale: false,
            run: 0,
        };
        let queries = run
            .queries
            .into_iter()
            .enumerate()
            .map(|(query_index, query)| {
                PanelQuery::project(query, draw_color.get(&query_index).copied(), slice_start)
            })
            .collect();

        Self {
            matches,
            queries,
            track_data,
        }
    }

    /// The derived series the run computed for `track`, absent for a track that
    /// contributed none. Match tables read values through this so they show
    /// exactly what the evaluator saw.
    pub fn track_data(&self, track: TrackRef) -> Option<&TrackQueryData> {
        self.track_data.get(&track)
    }
}

/// A channel-source run: matched sample ranges per track over the source
/// channel's own timeline. Renders as sample tables, and as halos over the
/// track segments the matched spans cover.
pub struct ChannelResults {
    /// The source channel's name, for the panel header.
    pub channel: String,
    /// Component labels for a vector channel, empty for a scalar.
    pub components: Vec<String>,
    /// The query's aggregate `table` columns, in table order, valued once per
    /// match. The sample time and the component columns are the channel's own.
    pub aggregate_columns: Vec<AggregateColumn>,
    pub summary: QuerySummary,
    pub tracks: Vec<ChannelTrackResult>,
    /// The map effect: halos over the matched track segments, honoring the
    /// query mode. Carries its own `stale` flag for the map.
    pub matches: QueryMatches,
}

impl ChannelResults {
    /// Project a channel-source run into its panel result. The matched ranges
    /// are sample indices into each track's timeline, kept as-is (no slice
    /// offset: a channel source is not sliced by the point time filter).
    pub(crate) fn project(
        ChannelRun {
            channel,
            components,
            aggregate_columns,
            summary,
            tracks,
            matches,
        }: ChannelRun,
    ) -> Self {
        Self {
            channel,
            components,
            aggregate_columns,
            summary: QuerySummary::of_channel(&summary),
            tracks,
            matches,
        }
    }
}

/// One track's channel-source matches and the timeline they index into.
pub struct ChannelTrackResult {
    pub track: TrackRef,
    /// This track's declared display unit. The evaluator timeline below is in
    /// base units; tables convert it back through this metadata.
    pub unit: Option<ChannelUnit>,
    /// Matched stretches of `timeline`, indexed by sample.
    pub matches: Vec<MatchValues>,
    pub timeline: ChannelTimeline,
}

/// One query's result for the panel: its summary line, columns, and matches.
pub struct PanelQuery {
    /// Palette color index when this query draws, for the swatch; `None`
    /// otherwise.
    pub color: Option<usize>,
    pub summary: QuerySummary,
    /// The match table's columns, in table order.
    pub columns: Vec<TableColumn>,
    /// This query's matches, at absolute point indices.
    pub matches: Vec<TrackMatchValues>,
}

impl PanelQuery {
    /// Project one query of a points run into its panel result, shifting each
    /// matched range by `slice_start` from the evaluated slice to absolute
    /// point indices.
    fn project(
        PointsQueryRun {
            mode,
            columns,
            summary,
            matches,
        }: PointsQueryRun,
        color: Option<usize>,
        slice_start: impl Fn(TrackRef) -> usize,
    ) -> Self {
        Self {
            color,
            summary: QuerySummary::of_points(&summary, mode),
            columns,
            matches: matches
                .into_iter()
                .map(|TrackMatchValues { track, matches }| {
                    let start = slice_start(track);
                    TrackMatchValues {
                        track,
                        matches: matches
                            .into_iter()
                            .map(|MatchValues { rows, aggregates }| MatchValues {
                                rows: rows.start + start..rows.end + start,
                                aggregates,
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }
}

/// One track's matches of one query.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackMatchValues {
    pub track: TrackRef,
    pub matches: Vec<MatchValues>,
}

/// One match of a query: its extent in the source, and its aggregate columns'
/// values over that extent.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchValues {
    /// Indices into the match's source: a track's nav points, or a channel
    /// timeline's samples.
    pub rows: Range<usize>,
    /// One value per aggregate column of the query, in table order. `None`
    /// where the aggregate has no value over the match.
    pub aggregates: Vec<Option<f64>>,
}

/// Map one track's matched sample ranges to enclosing nav-point index ranges,
/// so the point-halo renderer bands the track over each matched span. A span
/// `[t0, t1]` extends to the nav point at or before `t0` through the one at or
/// after `t1`, so even a sub-interval match bands the segment it sits on.
/// Returned sorted and merged (disjoint), as [`QueryMatches`] requires.
///
/// Fix and sample timestamps are compared as Unix seconds with the sub-second
/// fraction, the unit of [`ChannelTimeline::times`].
pub(crate) fn matched_point_ranges(
    points: &[NavPoint],
    timeline: &ChannelTimeline,
    matches: &[MatchValues],
) -> Vec<Range<usize>> {
    let Some(last) = points.len().checked_sub(1) else {
        return Vec::new();
    };
    let point_secs: Vec<f64> = points
        .iter()
        .map(|p| p.tpv.time().as_secs_f64_with_subseconds())
        .collect();
    let mut spans: Vec<Range<usize>> = matches
        .iter()
        .map(|matched| &matched.rows)
        .filter_map(|r| {
            let t0 = *timeline.times.get(r.start)?;
            let t1 = *timeline.times.get(r.end.checked_sub(1)?)?;
            // Last point at or before t0 (or the first point); first point at or
            // after t1 (or the last).
            let lo = point_secs.partition_point(|&t| t <= t0).saturating_sub(1);
            let hi = point_secs.partition_point(|&t| t < t1).min(last);
            Some(lo..hi + 1)
        })
        .collect();
    spans.sort_by_key(|r| r.start);
    merge_ranges(spans)
}

/// Merge overlapping or touching ranges (assumes `ranges` sorted by start),
/// yielding sorted, disjoint, non-empty ranges.
pub(crate) fn merge_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.into_iter().filter(|r| !r.is_empty()) {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// The complement of sorted, disjoint `ranges` within `0..len`: the gaps not
/// covered by any range.
pub(crate) fn complement_ranges(ranges: &[Range<usize>], len: usize) -> Vec<Range<usize>> {
    let mut gaps = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if range.start > cursor {
            gaps.push(cursor..range.start);
        }
        cursor = cursor.max(range.end);
    }
    if cursor < len {
        gaps.push(cursor..len);
    }
    gaps
}

/// Build the map effect for a channel-source run from each track's matched
/// nav-point ranges and point count, honoring the query's display mode:
/// `draw` halos the matched segments, `hide` breaks the polyline there, and
/// `keep` breaks it everywhere else.
pub(crate) fn channel_query_matches(
    mode: DisplayMode,
    per_track: &FxHashMap<TrackRef, (Vec<Range<usize>>, usize)>,
) -> QueryMatches {
    let matched: TrackRanges = per_track
        .iter()
        .map(|(track, (ranges, _))| (*track, ranges.clone()))
        .collect();
    match mode {
        DisplayMode::Draw => QueryMatches {
            draws: vec![DrawLayer {
                color: 0,
                ranges: matched,
            }],
            ..QueryMatches::default()
        },
        DisplayMode::Hide => QueryMatches {
            hidden: matched,
            ..QueryMatches::default()
        },
        DisplayMode::Keep => QueryMatches {
            hidden: per_track
                .iter()
                .map(|(track, (ranges, len))| (*track, complement_ranges(ranges, *len)))
                .filter(|(_, gaps)| !gaps.is_empty())
                .collect(),
            ..QueryMatches::default()
        },
    }
}

/// One query's counts for the results summary strip, and the lines behind them
/// stating what the run left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QuerySummary {
    pub match_count: usize,
    pub tracks_with_matches: usize,
    /// Points `keep` and `hide` remove from the map, of the total the query
    /// counted them against. `draw` removes none.
    pub hidden_points: Option<HiddenPoints>,
    /// Rows the run could not value, over every reason it could not.
    pub skipped: usize,
    /// One line per reason rows or tracks were left out.
    pub notes: Vec<String>,
}

/// Points a query removes from the map, of the total it counted them against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenPoints {
    pub hidden: usize,
    pub total: usize,
}

impl QuerySummary {
    /// One points query's summary: how much it matched, how many points it
    /// removes from the map, and everything it skipped.
    fn of_points(summary: &RunSummary, mode: DisplayMode) -> Self {
        let mut notes = Vec::new();
        for (metric, count) in &summary.skipped {
            notes.push(format!("{count} skipped (missing {metric})"));
        }
        // "without <metric> values", not "never ran <feature>": a track whose
        // snap run left every point unsnapped also carries no values.
        for (metric, count) in &summary.tracks_without {
            notes.push(format!(
                "{count} {} without {metric} values",
                gt_fmt::pluralize(*count, "track", "tracks"),
            ));
        }
        push_non_finite_note(&mut notes, summary.skipped_non_finite);
        if summary.tracks_shorter_than_window > 0 {
            notes.push(format!(
                "{} {} shorter than window",
                summary.tracks_shorter_than_window,
                gt_fmt::pluralize(summary.tracks_shorter_than_window, "track", "tracks"),
            ));
        }
        for param in &summary.unused_params {
            notes.push(format!("{param} declared but unused"));
        }
        // keep/hide remove points from the map. Always say how many, so hidden
        // data stays accounted for.
        let hidden = match mode {
            DisplayMode::Draw => None,
            DisplayMode::Keep => Some(summary.total_points.saturating_sub(summary.matched_points)),
            DisplayMode::Hide => Some(summary.matched_points),
        };
        Self {
            match_count: summary.match_count,
            tracks_with_matches: summary.tracks_with_matches,
            hidden_points: hidden.map(|hidden| HiddenPoints {
                hidden,
                total: summary.total_points,
            }),
            skipped: summary.skipped.values().sum::<usize>() + summary.skipped_non_finite,
            notes,
        }
    }

    /// A channel-source run's summary: never carries the points/keep/hide
    /// accounting [`Self::of_points`] states, since its matches are samples.
    fn of_channel(summary: &RunSummary) -> Self {
        let mut notes = Vec::new();
        for (channel, count) in &summary.skipped_channels {
            notes.push(format!("{count} skipped (missing @{channel})"));
        }
        push_non_finite_note(&mut notes, summary.skipped_non_finite);
        Self {
            match_count: summary.match_count,
            tracks_with_matches: summary.tracks_with_matches,
            hidden_points: None,
            skipped: summary.skipped_channels.values().sum::<usize>() + summary.skipped_non_finite,
            notes,
        }
    }
}

/// The note for the rows the evaluator skipped on a value that is NaN or
/// infinite. No metric is named for these skips: such a value comes from
/// undefined arithmetic or from a channel sample the file recorded that way.
fn push_non_finite_note(notes: &mut Vec<String>, skipped_non_finite: usize) {
    if skipped_non_finite > 0 {
        notes.push(format!("{skipped_non_finite} skipped (non-finite value)"));
    }
}

impl fmt::Display for QuerySummary {
    /// The whole summary as one line, for the hover naming the query a match
    /// came from.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = vec![format!(
            "{} {} on {} {}",
            self.match_count,
            gt_fmt::pluralize(self.match_count, "match", "matches"),
            self.tracks_with_matches,
            gt_fmt::pluralize(self.tracks_with_matches, "track", "tracks"),
        )];
        if let Some(HiddenPoints { hidden, total }) = self.hidden_points {
            parts.push(format!("{hidden} of {total} points hidden"));
        }
        parts.extend(self.notes.iter().cloned());
        write!(f, "{}", parts.join(&format!(" {EM_DASH} ")))
    }
}

#[cfg(test)]
mod tests {
    use gt_query::{MetricProvider, QueryMetric, TrackInput};
    use gt_types::{FileIdx, TrackIdx};
    use rstest::rstest;

    use super::*;
    use crate::check::check_text;
    use crate::test_fixtures::{TEST_EPOCH, matched_rows, points_at_millis, rng};

    #[test]
    fn summary_notes_every_skip_and_unused_param() {
        let query = check_text(
            "points | with mask 15 deg, snr_drop 10 | where util_all < 50 %",
            &gt_query::ChannelSchema::new(),
        )
        .expect("checks");
        let provider = EmptyProvider { len: 3 };
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        let summary = QuerySummary::of_points(&output.summary, DisplayMode::Draw);
        assert_eq!(
            summary.to_string(),
            format!(
                "0 matches on 0 tracks {EM_DASH} 3 skipped (missing util_all) \
                 {EM_DASH} 1 track without util_all values {EM_DASH} snr_drop declared but unused"
            )
        );
        assert_eq!(summary.skipped, 3);
        assert_eq!(
            summary.notes,
            [
                "3 skipped (missing util_all)",
                "1 track without util_all values",
                "snr_drop declared but unused",
            ]
        );
    }

    #[test]
    fn summary_notes_a_non_finite_skip() {
        let summary = RunSummary {
            skipped_non_finite: 2,
            ..RunSummary::default()
        };
        let points = QuerySummary::of_points(&summary, DisplayMode::Draw);
        assert_eq!(points.notes, ["2 skipped (non-finite value)"]);
        assert_eq!(points.skipped, 2);
    }

    #[test]
    fn summary_counts_the_points_keep_and_hide_remove() {
        // 5 points, 2 matched.
        let query = check_text(
            "points | where velocity > 30 km/h",
            &gt_query::ChannelSchema::new(),
        )
        .expect("checks");
        let provider = TestSpeeds(vec![
            Some(40.0),
            Some(40.0),
            Some(1.0),
            Some(1.0),
            Some(1.0),
        ]);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        let hidden = |mode| QuerySummary::of_points(&output.summary, mode).hidden_points;
        // keep hides the 3 non-matching points. hide hides the 2 matching.
        assert_eq!(
            hidden(DisplayMode::Keep),
            Some(HiddenPoints {
                hidden: 3,
                total: 5
            })
        );
        assert_eq!(
            hidden(DisplayMode::Hide),
            Some(HiddenPoints {
                hidden: 2,
                total: 5
            })
        );
        assert_eq!(hidden(DisplayMode::Draw), None);
    }

    /// Velocity in m/s per point, everything else missing.
    struct TestSpeeds(Vec<Option<f64>>);

    impl MetricProvider for TestSpeeds {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
            match metric {
                QueryMetric::Velocity => self.0.get(index).copied().flatten(),
                _ => None,
            }
        }
    }

    struct EmptyProvider {
        len: usize,
    }

    impl MetricProvider for EmptyProvider {
        fn len(&self) -> usize {
            self.len
        }

        fn value(&self, _metric: QueryMetric, _index: usize) -> Option<f64> {
            None
        }
    }

    /// A sample between two fixes bands both of them. A sample on a fix's own
    /// timestamp bands that fix alone.
    #[rstest]
    #[case(250, rng(0, 2))]
    #[case(500, rng(1, 2))]
    fn matched_point_ranges_band_the_fixes_around_a_matched_sample(
        #[case] sample_millis: i64,
        #[case] expected: Range<usize>,
    ) {
        let points = points_at_millis(&[0, 500]);
        let timeline = ChannelTimeline {
            times: vec![TEST_EPOCH as f64 + sample_millis as f64 / 1_000.0],
            values: vec![9.8],
            columns: 1,
        };

        assert_eq!(
            matched_point_ranges(&points, &timeline, &[matched_rows(rng(0, 1))]),
            vec![expected]
        );
    }

    #[test]
    fn matched_point_ranges_are_empty_without_a_matched_sample() {
        let points = points_at_millis(&[0, 500]);
        let timeline = ChannelTimeline {
            times: vec![TEST_EPOCH as f64 + 0.25],
            values: vec![9.8],
            columns: 1,
        };

        assert!(matched_point_ranges(&points, &timeline, &[]).is_empty());
    }

    #[test]
    fn merge_ranges_merges_touching_and_overlapping() {
        assert_eq!(merge_ranges(vec![0..2, 2..4, 5..6]), vec![0..4, 5..6]);
        assert_eq!(merge_ranges(vec![0..3, 1..2]), vec![rng(0, 3)]);
        assert!(merge_ranges(vec![]).is_empty());
    }

    #[test]
    fn complement_ranges_returns_the_gaps() {
        assert_eq!(complement_ranges(&[1..3, 5..6], 8), vec![0..1, 3..5, 6..8]);
        assert!(complement_ranges(&[rng(0, 4)], 4).is_empty());
        assert_eq!(complement_ranges(&[], 3), vec![rng(0, 3)]);
    }

    #[rstest]
    #[case(DisplayMode::Draw)]
    #[case(DisplayMode::Hide)]
    #[case(DisplayMode::Keep)]
    fn channel_query_matches_honors_the_mode(#[case] mode: DisplayMode) {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let per_track = FxHashMap::from_iter([(track, (vec![rng(1, 3)], 5usize))]);
        let matches = channel_query_matches(mode, &per_track);
        match mode {
            // Draw halos the matched segments.
            DisplayMode::Draw => {
                assert_eq!(matches.draws[0].ranges_for(track), [rng(1, 3)]);
                assert!(matches.hidden.is_empty());
            }
            // Hide breaks the polyline at the matched segments.
            DisplayMode::Hide => {
                assert_eq!(matches.hidden_ranges(track), [rng(1, 3)]);
                assert!(matches.draws.is_empty());
            }
            // Keep breaks the polyline everywhere else (the complement).
            DisplayMode::Keep => {
                assert_eq!(matches.hidden_ranges(track), &[0..1, 3..5]);
                assert!(matches.draws.is_empty());
            }
        }
    }
}
