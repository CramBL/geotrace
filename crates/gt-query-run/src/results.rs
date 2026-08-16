use std::collections::HashMap;
use std::ops::Range;

use geotrace_sdk_units::ChannelUnit;
use gt_fmt::EM_DASH;
use gt_query::{ChannelTimeline, PipelineOutput, QueryMetric, RunSummary, TrackMatches};
use gt_types::{DisplayMode, NavPoint, TrackRef};
use gt_ui_types::{DrawLayer, DrawLayerMask, QueryMatches};

use crate::provider::TrackQueryData;
use crate::run::ChannelRun;

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
}

/// A points-pipeline run: its composed map effect and per-query panel rows.
pub struct PointsResults {
    /// The composed display effect for the map.
    pub matches: QueryMatches,
    /// Per query, in editor order, for the results panel.
    pub queries: Vec<PanelQuery>,
    /// Per-track derived series (only for metrics some query referenced),
    /// kept so match tables show the exact values the run used.
    track_data: HashMap<TrackRef, TrackQueryData>,
}

impl PointsResults {
    /// Project a points [`PipelineOutput`] into the panel/map result. Evaluation
    /// ran on each track's time-filtered slice, so point indices shift back to
    /// absolute positions here.
    pub(crate) fn project(
        output: &PipelineOutput,
        track_data: HashMap<TrackRef, TrackQueryData>,
    ) -> Self {
        let absolute = |tms: &[TrackMatches]| -> HashMap<TrackRef, Vec<Range<usize>>> {
            tms.iter()
                .filter(|tm| !tm.ranges.is_empty())
                .map(|tm| {
                    let start = track_data
                        .get(&tm.track)
                        .map_or(0, TrackQueryData::slice_start);
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
        if output.draws.len() > DrawLayerMask::MAX_LAYERS {
            log::warn!(
                "query has {} draw stages: only the first {} render as halos",
                output.draws.len(),
                DrawLayerMask::MAX_LAYERS
            );
        }

        // The i-th draw query gets palette color i. The map keys its halo layer
        // to the same order.
        let draw_color: HashMap<usize, usize> = output
            .draws
            .iter()
            .take(DrawLayerMask::MAX_LAYERS)
            .enumerate()
            .map(|(order, layer)| (layer.query_index, order))
            .collect();

        let matches = QueryMatches {
            hidden: absolute(&output.hidden),
            draws: output
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
        };
        let queries = output
            .queries
            .iter()
            .enumerate()
            .map(|(qi, q)| PanelQuery {
                color: draw_color.get(&qi).copied(),
                summary: summary_line(&q.summary, q.mode),
                columns: q.columns.clone(),
                matches: q
                    .matches
                    .iter()
                    .map(|tm| TrackMatches {
                        track: tm.track,
                        ranges: absolute(std::slice::from_ref(tm))
                            .remove(&tm.track)
                            .unwrap_or_default(),
                    })
                    .collect(),
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
    pub summary: String,
    pub tracks: Vec<ChannelTrackResult>,
    /// The map effect: halos over the matched track segments, honoring the
    /// query mode. Carries its own `stale` flag for the map.
    pub matches: QueryMatches,
}

impl ChannelResults {
    /// Project a channel-source run into its panel result. The matched ranges
    /// are sample indices into each track's timeline, kept as-is (no slice
    /// offset: a channel source is not sliced by the point time filter).
    pub(crate) fn project(run: ChannelRun) -> Self {
        let ChannelRun {
            channel,
            components,
            summary,
            tracks,
            matches,
        } = run;
        Self {
            channel,
            components,
            summary: channel_summary_line(&summary),
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
    /// Matched sample-index ranges into `timeline`.
    pub ranges: Vec<Range<usize>>,
    pub timeline: ChannelTimeline,
}

/// One query's result for the panel: its summary line, columns, and matches.
pub struct PanelQuery {
    /// Palette color index when this query draws, for the swatch; `None`
    /// otherwise.
    pub color: Option<usize>,
    pub summary: String,
    pub columns: Vec<QueryMetric>,
    /// Absolute point-index ranges this query matched.
    pub matches: Vec<TrackMatches>,
}

/// Map one track's matched sample ranges to enclosing nav-point index ranges,
/// so the point-halo renderer bands the track over each matched span. A span
/// `[t0, t1]` extends to the nav point at or before `t0` through the one at or
/// after `t1`, so even a sub-interval match bands the segment it sits on.
/// Returned sorted and merged (disjoint), as [`QueryMatches`] requires.
pub(crate) fn matched_point_ranges(
    points: &[NavPoint],
    timeline: &ChannelTimeline,
    ranges: &[Range<usize>],
) -> Vec<Range<usize>> {
    let Some(last) = points.len().checked_sub(1) else {
        return Vec::new();
    };
    let point_secs: Vec<f64> = points.iter().map(|p| p.tpv.time().as_secs_f64()).collect();
    let mut spans: Vec<Range<usize>> = ranges
        .iter()
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
    per_track: &HashMap<TrackRef, (Vec<Range<usize>>, usize)>,
) -> QueryMatches {
    let matched: HashMap<TrackRef, Vec<Range<usize>>> = per_track
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

/// A channel-source run's summary line: match count over tracks, plus any
/// skipped-window reporting. Matches are samples, so it never mentions the
/// points/keep/hide accounting the [`summary_line`] points path uses.
fn channel_summary_line(summary: &RunSummary) -> String {
    let mut parts = vec![format!(
        "{} {} on {} {}",
        summary.match_count,
        gt_fmt::pluralize(summary.match_count, "match", "matches"),
        summary.tracks_with_matches,
        gt_fmt::pluralize(summary.tracks_with_matches, "track", "tracks"),
    )];
    for (channel, count) in &summary.skipped_channels {
        parts.push(format!("{count} skipped (missing @{channel})"));
    }
    if summary.skipped_non_finite > 0 {
        parts.push(format!(
            "{} skipped (undefined arithmetic)",
            summary.skipped_non_finite
        ));
    }
    parts.join(&format!(" {EM_DASH} "))
}

/// One query's panel summary: match count over tracks, how many points it
/// hides, and everything it skipped.
fn summary_line(summary: &RunSummary, mode: DisplayMode) -> String {
    let mut parts = vec![format!(
        "{} {} on {} {}",
        summary.match_count,
        gt_fmt::pluralize(summary.match_count, "match", "matches"),
        summary.tracks_with_matches,
        gt_fmt::pluralize(summary.tracks_with_matches, "track", "tracks"),
    )];
    // keep/hide remove points from the map. Always say how many, so hidden
    // data stays accounted for.
    let hidden = match mode {
        DisplayMode::Draw => None,
        DisplayMode::Keep => Some(summary.total_points - summary.matched_points),
        DisplayMode::Hide => Some(summary.matched_points),
    };
    if let Some(hidden) = hidden {
        parts.push(format!(
            "{hidden} of {} points hidden",
            summary.total_points
        ));
    }
    for (metric, count) in &summary.skipped {
        parts.push(format!("{count} skipped (missing {metric})"));
    }
    // "without <metric> values", not "never ran <feature>": a track whose
    // snap run left every point unsnapped also carries no values.
    for (metric, count) in &summary.tracks_without {
        parts.push(format!(
            "{count} {} without {metric} values",
            gt_fmt::pluralize(*count, "track", "tracks"),
        ));
    }
    if summary.skipped_non_finite > 0 {
        parts.push(format!(
            "{} skipped (undefined arithmetic)",
            summary.skipped_non_finite
        ));
    }
    if summary.tracks_shorter_than_window > 0 {
        parts.push(format!(
            "{} {} shorter than window",
            summary.tracks_shorter_than_window,
            gt_fmt::pluralize(summary.tracks_shorter_than_window, "track", "tracks"),
        ));
    }
    for param in &summary.unused_params {
        parts.push(format!("{param} declared but unused"));
    }
    parts.join(&format!(" {EM_DASH} "))
}

#[cfg(test)]
mod tests {
    use gt_query::{MetricProvider, TrackInput};
    use gt_types::{FileIdx, TrackIdx};
    use rstest::rstest;

    use super::*;
    use crate::check::check_text;
    use crate::test_fixtures::{TEST_EPOCH, rng, test_points};

    #[test]
    fn summary_reports_skips_and_unused_params() {
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
        let line = summary_line(&output.summary, DisplayMode::Draw);
        assert_eq!(
            line,
            format!(
                "0 matches on 0 tracks {EM_DASH} 3 skipped (missing util_all) \
                 {EM_DASH} 1 track without util_all values {EM_DASH} snr_drop declared but unused"
            )
        );
    }

    #[test]
    fn summary_reports_hidden_count_for_keep_and_hide() {
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
        // keep hides the 3 non-matching points. hide hides the 2 matching.
        assert!(summary_line(&output.summary, DisplayMode::Keep).contains("3 of 5 points hidden"));
        assert!(summary_line(&output.summary, DisplayMode::Hide).contains("2 of 5 points hidden"));
        assert!(!summary_line(&output.summary, DisplayMode::Draw).contains("hidden"));
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

    #[test]
    fn matched_point_ranges_band_the_covering_segment() {
        // Points at 0 and 1 s; a channel sample matched at 0.5 s brackets to the
        // segment between them, banding both nav points.
        let base = TEST_EPOCH as f64;
        let points = test_points();
        let timeline = ChannelTimeline {
            times: vec![base + 0.5],
            values: vec![9.8],
            columns: 1,
        };
        assert_eq!(
            matched_point_ranges(&points, &timeline, &[rng(0, 1)]),
            vec![rng(0, 2)]
        );
        // No matched samples yields no bands.
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
        let per_track = HashMap::from([(track, (vec![rng(1, 3)], 5usize))]);
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
