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
pub mod construct;
mod dimension;
mod eval;
mod fmt;
pub mod lexer;
mod metric;
mod parser;
mod pipeline;
mod position;
mod unit;

pub use ast::{ParamName, Query, Span};
pub use check::{ChannelConflict, ChannelInfo, ChannelSchema, CheckedQuery, Params, Window, check};
pub use construct::{Construct, ConstructKind, catalog};
pub use dimension::Dimension;
pub use eval::{
    ChannelSamples, ChannelTimeline, MetricProvider, RunOutput, RunSummary, TrackInput,
    TrackMatches, derived_accel, run, run_cancellable,
};
pub use metric::{Quantity, QueryMetric};
pub use parser::parse;
pub use pipeline::{DrawContribution, PipelineOutput, QueryOutput, run_pipeline};
pub use position::{
    ChannelCompletions, ChannelSuggestion, CompletionTrigger, Completions, channel_at,
    channel_completions_at, completions_at, construct_at,
};
pub use unit::Unit;

/// A parse or type error: what went wrong, where, and optionally how to fix
/// it. Rendered by the editor as an underline plus message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

impl Diagnostic {
    /// An error with no suggestion.
    pub(crate) fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            help: None,
        }
    }

    /// An error whose fix goes in `help` (shown as a separate "Hint:" line)
    /// rather than tacked onto the message.
    pub(crate) fn with_hint(
        span: Span,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> Self {
        Self {
            span,
            message: message.into(),
            help: Some(help.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use geotrace_sdk_units::ChannelUnit;
    use gt_types::{DisplayMode, FileIdx, TrackIdx, TrackRef};
    use rstest::rstest;

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

    /// Per-metric series in base units; anything absent is missing. Channels
    /// carry their own `(time, row)` samples, keyed by name, where each row
    /// holds one value per component (one for a scalar channel).
    #[derive(Default)]
    struct TestProvider {
        len: usize,
        series: BTreeMap<QueryMetric, Vec<Option<f64>>>,
        channels: BTreeMap<String, Vec<(f64, Vec<f64>)>>,
    }

    impl TestProvider {
        fn new(len: usize) -> Self {
            Self {
                len,
                series: BTreeMap::new(),
                channels: BTreeMap::new(),
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

        /// Attach a scalar channel's native `(time_secs, value)` samples.
        fn with_channel(mut self, name: &str, samples: Vec<(f64, f64)>) -> Self {
            let rows = samples.into_iter().map(|(t, v)| (t, vec![v])).collect();
            self.channels.insert(name.to_owned(), rows);
            self
        }

        /// Attach a vector channel's native `(time_secs, row)` samples, each row
        /// one value per component.
        fn with_vector_channel(mut self, name: &str, samples: Vec<(f64, Vec<f64>)>) -> Self {
            self.channels.insert(name.to_owned(), samples);
            self
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

        fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
            let Some(rows) = self.channels.get(name) else {
                return ChannelSamples::default();
            };
            let columns = rows.first().map_or(1, |(_, row)| row.len());
            let values = rows
                .iter()
                .filter(|(t, _)| *t >= t_lo && *t <= t_hi)
                .flat_map(|(_, row)| row.iter().copied())
                .collect();
            ChannelSamples { values, columns }
        }

        fn channel_timeline(&self, name: &str) -> ChannelTimeline {
            let Some(rows) = self.channels.get(name) else {
                return ChannelTimeline::default();
            };
            let columns = rows.first().map_or(1, |(_, row)| row.len());
            ChannelTimeline {
                times: rows.iter().map(|(t, _)| *t).collect(),
                values: rows
                    .iter()
                    .flat_map(|(_, row)| row.iter().copied())
                    .collect(),
                columns,
            }
        }
    }

    fn checked(src: &str) -> CheckedQuery {
        check(&parse(src).unwrap(), &ChannelSchema::new()).unwrap()
    }

    /// Check with an empty channel schema, for the many tests that reference no
    /// channels. Tests that need channels build their own schema.
    fn chk(query: &Query) -> Result<CheckedQuery, Diagnostic> {
        check(query, &ChannelSchema::new())
    }

    #[test]
    fn a_count_window_checks_to_window_count() {
        assert_eq!(
            checked("points | window 5 | where avg(velocity) > 30 km/h").window(),
            Some(Window::Count(5))
        );
        assert_eq!(checked("points | where velocity > 30 km/h").window(), None);
    }

    #[test]
    fn a_duration_window_checks_to_seconds() {
        assert_eq!(
            checked("points | window 15 s | where avg(velocity) > 30 km/h").window(),
            Some(Window::Duration(15.0))
        );
        // Units convert to seconds: 2 min = 120 s.
        assert_eq!(
            checked("points | window 2 min | where avg(velocity) > 30 km/h").window(),
            Some(Window::Duration(120.0))
        );
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

    /// A track that carries no value at all for a referenced metric is
    /// counted per metric in `tracks_without` - whole-track absences (no
    /// snap run, an eph-less receiver) get named, not just point skips. A
    /// track with even one value stays out; `accel` derives from velocity,
    /// so its absence is probed through velocity.
    #[test]
    fn summary_counts_tracks_without_a_referenced_metric() {
        let with_values =
            TestProvider::new(3).with(QueryMetric::SnapError, vec![Some(2.0), None, Some(4.0)]);
        let without = TestProvider::new(3);
        let output = run(
            &checked("points | where snap_error > 1 m"),
            &[
                TrackInput {
                    track: track_ref(),
                    provider: &with_values,
                },
                TrackInput {
                    track: TrackRef::new(FileIdx::new(0), TrackIdx::new(1)),
                    provider: &without,
                },
            ],
        );
        assert_eq!(
            output.summary.tracks_without,
            BTreeMap::from([(QueryMetric::SnapError, 1)]),
            "only the run-less track counts"
        );
        // The valued track's single missing point stays a point skip; the
        // absent track contributes its full length.
        assert_eq!(
            output.summary.skipped,
            BTreeMap::from([(QueryMetric::SnapError, 4)])
        );

        // A run that produced values for no point (every point unsnapped)
        // counts too: the summary reports missing values, and its wording
        // deliberately does not claim the track was never snapped.
        let all_unsnapped =
            TestProvider::new(3).with(QueryMetric::SnapError, vec![None, None, None]);
        let output = run(
            &checked("points | where snap_error > 1 m"),
            &[TrackInput {
                track: track_ref(),
                provider: &all_unsnapped,
            }],
        );
        assert_eq!(
            output.summary.tracks_without,
            BTreeMap::from([(QueryMetric::SnapError, 1)])
        );

        let velocity_only =
            TestProvider::new(2).with(QueryMetric::Velocity, vec![Some(5.0), Some(6.0)]);
        let output = run(
            &checked("points | where accel > 0 m/s2"),
            &[TrackInput {
                track: track_ref(),
                provider: &velocity_only,
            }],
        );
        assert!(
            output.summary.tracks_without.is_empty(),
            "accel derives from velocity - a velocity-carrying track is not without it"
        );
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
        // 3 of the 5 points matched - the counts the keep/hide summary uses.
        assert_eq!(output.summary.matched_points, 3);
        assert_eq!(output.summary.total_points, 5);
        assert!(output.summary.skipped.is_empty());
    }

    #[test]
    fn matched_points_count_points_not_windows() {
        // Windows [1,3) and [2,4) both pass, so points 1..=4 (four points)
        // match even though only two windows did.
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
        assert_eq!(output.summary.matched_points, 4);
        assert_eq!(output.summary.total_points, 6);
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
    fn a_duration_window_reduces_the_points_in_its_time_span() {
        // Points at 0..5 s. A `window 2 s` at anchor i spans [t[i], t[i]+2), so
        // two points, and needs the full 2 s to fit (last anchor is point 2).
        let provider = TestProvider::new(5).indexed_time().with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(10.0), Some(0.0), Some(0.0), Some(0.0)],
        );
        let output = run_one(
            "points | window 2 s | where avg(velocity) > 5 km/h",
            &provider,
        );
        // Anchor 0 (pts 0,1 avg 10) and anchor 1 (pts 1,2 avg 5 m/s) clear the
        // 5 km/h bar; anchor 2 (pts 2,3 avg 0) does not; anchor 3 doesn't fit.
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn a_duration_window_longer_than_the_track_matches_nothing() {
        // Track spans only 2 s (points at 0, 1, 2), so a 10 s window never fits.
        let provider = TestProvider::new(3).indexed_time().with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(10.0), Some(10.0)],
        );
        let output = run_one(
            "points | window 10 s | where avg(velocity) > 0 km/h",
            &provider,
        );
        assert!(output.matches.is_empty());
        assert_eq!(output.summary.tracks_shorter_than_window, 1);
    }

    #[test]
    fn a_fractional_duration_window_spans_sub_second() {
        // Points at 0, 0.4, 0.8, 1.2 s. A 0.5 s window at anchor 0 spans [0, 0.5),
        // holding points 0 and 1; the last anchor that fits is at 0.8 s (0.8+0.5
        // = 1.3 > 1.2, so it does not fit — only anchors 0 and 1 fit).
        let provider = TestProvider::new(4)
            .with(
                QueryMetric::Time,
                vec![Some(0.0), Some(0.4), Some(0.8), Some(1.2)],
            )
            .with(
                QueryMetric::Velocity,
                vec![Some(10.0), Some(10.0), Some(0.0), Some(0.0)],
            );
        let output = run_one(
            "points | window 0.5 s | where avg(velocity) > 5 km/h",
            &provider,
        );
        // Anchor 0 (pts 0,1 avg 10, match); anchor 1 (pts 1,2 avg 5 m/s, match).
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn a_duration_window_survives_a_backward_time_step() {
        // A backward clock jump at point 2 (times 0, 1, 0, 1 s): the crate does
        // not assume monotonic time, so anchors after the jump must still be
        // evaluated, not silently dropped.
        let provider = TestProvider::new(4)
            .with(
                QueryMetric::Time,
                vec![Some(0.0), Some(1.0), Some(0.0), Some(1.0)],
            )
            .with(
                QueryMetric::Velocity,
                vec![Some(0.0), Some(0.0), Some(10.0), Some(10.0)],
            );
        // window 2 s never fits (max time is 1 s), but the fast pair after the
        // jump proves the loop reaches them: with window 1 s, anchor 2 spans
        // [0,1) = point 2 (avg 10, match), which a `break` would have skipped.
        let output = run_one(
            "points | window 1 s | where avg(velocity) > 5 km/h",
            &provider,
        );
        assert!(output.matches.iter().any(|m| m.ranges.contains(&(2..3))));
    }

    #[test]
    fn a_duration_window_spans_real_time_not_point_count() {
        // Uneven spacing: points at 0, 1, 5, 6 s. A 2 s window at point 0 holds
        // only points 0 and 1 (point at 5 s is outside [0, 2)); the sparse
        // stretch is not force-filled to a fixed count.
        let provider = TestProvider::new(4)
            .with(
                QueryMetric::Time,
                vec![Some(0.0), Some(1.0), Some(5.0), Some(6.0)],
            )
            .with(
                QueryMetric::Velocity,
                vec![Some(10.0), Some(10.0), Some(0.0), Some(0.0)],
            );
        // window 2 s: anchor 0 → pts 0,1 (avg 10, match); anchor 1 → t=1, 1+2=3
        // <= 6, pts with time in [1,3) = just point 1 (avg 10, match); anchor 2
        // → t=5, 5+2=7 > 6, doesn't fit → break.
        let output = run_one(
            "points | window 2 s | where avg(velocity) > 5 km/h",
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..2]);
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
    fn std_over_a_window_uses_population_deviation() {
        // Steady speed has zero std; the last window jumps, so only the steady
        // stretch matches. 2 km/h is 0.556 m/s, well above the 0 of a flat run.
        let provider = TestProvider::new(4).with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(10.0), Some(10.0), Some(20.0)],
        );
        let output = run_one(
            "points | window 2 | where std(velocity) < 2 km/h",
            &provider,
        );
        // Windows [0,2) and [1,3) are flat; [2,4) has a 5 m/s std and fails.
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn circular_std_flags_a_steady_heading() {
        // A heading tight around north stays under the threshold; the scattered
        // windows do not, so only the steady stretch matches.
        let provider = TestProvider::new(6).with(
            QueryMetric::Heading,
            vec![
                Some(359.0),
                Some(1.0),
                Some(0.0),
                Some(90.0),
                Some(200.0),
                Some(300.0),
            ],
        );
        let output = run_one("points | window 3 | where std(heading) <= 5 deg", &provider);
        // Only the first window [0,3) around north is steady.
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
    fn cancellation_stops_the_run_without_partial_results() {
        let provider = TestProvider::new(5).with(
            QueryMetric::Velocity,
            vec![Some(9.0), Some(9.0), Some(9.0), Some(9.0), Some(9.0)],
        );
        let query = checked("points | where velocity > 0 km/h");
        let inputs = [TrackInput {
            track: track_ref(),
            provider: &provider,
        }];

        let cancelled = || true;
        assert_eq!(run_cancellable(&query, &inputs, &cancelled), None);

        let live = || false;
        let output = run_cancellable(&query, &inputs, &live).expect("not cancelled");
        assert_eq!(output.summary.match_count, 1);
    }

    /// The interval-gated check inside a track's scan loop must also stop
    /// the run - not only the per-track check at its entry.
    #[test]
    fn cancellation_fires_mid_scan() {
        let provider = TestProvider::new(6).with(QueryMetric::Velocity, vec![Some(9.0); 6]);
        let query = checked("points | where velocity > 0 km/h");
        let inputs = [TrackInput {
            track: track_ref(),
            provider: &provider,
        }];

        // First call is the per-track check; cancel on a later one so the
        // stop happens inside the point loop.
        let calls = std::cell::Cell::new(0_u32);
        let cancel_after_two = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert_eq!(
            crate::eval::run_with_interval(&query, &inputs, &cancel_after_two, 1),
            None,
            "mid-scan cancellation must not surface partial matches"
        );
        assert!(calls.get() > 2, "the loop reached the mid-scan checks");
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
        assert_eq!(query.mode(), DisplayMode::Draw);
    }

    #[test]
    fn display_mode_defaults_to_draw() {
        // No display stage, and a table without one, both mean draw.
        assert_eq!(
            checked("points | where velocity > 0 km/h").mode(),
            DisplayMode::Draw
        );
        assert_eq!(
            checked("points | where velocity > 0 km/h | table time").mode(),
            DisplayMode::Draw
        );
        assert_eq!(
            checked("points | where velocity > 0 km/h | keep").mode(),
            DisplayMode::Keep
        );
        assert_eq!(
            checked("points | where velocity > 0 km/h | hide | table time").mode(),
            DisplayMode::Hide
        );
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

    #[rstest]
    #[case("points | where velocity > 30", "velocity needs a unit, e.g. 30 km/h")]
    #[case(
        "points | where velocity > 30 deg",
        "expected a speed unit (km/h, m/s, kn), found deg"
    )]
    #[case(
        "points | where velocity == 30 km/h",
        "use a range, e.g. 29 km/h < velocity and velocity < 31 km/h"
    )]
    #[case(
        "points | window 10 | where velocity > 30 km/h",
        "velocity is per point"
    )]
    #[case("points | where avg(velocity) > 30 km/h", "avg needs a window")]
    #[case("points | where util_gps < 50 %", "util_gps needs an elevation mask")]
    #[case(
        "points | where slip_all > 2 per min",
        "slip_all needs mask, snr_drop, and slip_window"
    )]
    #[case(
        "points | where eph > 20 m | window 3",
        "window must come before where"
    )]
    fn pinned_error_messages(#[case] src: &str, #[case] expected: &str) {
        // The error may come from either the parse or the check stage.
        let message = parse(src)
            .and_then(|q| chk(&q).map(|_| ()))
            .expect_err(src)
            .message;
        assert_eq!(message, expected, "for {src}");
    }

    /// `jamming` is a ratio, so it takes what the util metrics take: a `%`
    /// literal or another ratio metric. A bare number or a literal of any
    /// other dimension is rejected.
    #[rstest]
    #[case::percent_literal("points | where jamming > 10 %", true)]
    #[case::a_range("points | where 2 % < jamming and jamming < 10 %", true)]
    // The unit is required, exactly as it is for the util metrics.
    #[case::bare_number("points | where jamming > 0.1", false)]
    #[case::zero("points | where jamming > 0", false)]
    #[case::against_another_ratio("points | with mask 15 deg | where jamming > util_all", true)]
    #[case::compared_to_a_length("points | where jamming > 10 m", false)]
    #[case::compared_to_a_speed("points | where jamming > 10 km/h", false)]
    #[case::compared_to_a_duration("points | where jamming > 10 s", false)]
    fn jamming_accepts_ratios_and_rejects_other_dimensions(
        #[case] src: &str,
        #[case] accepted: bool,
    ) {
        let checked = parse(src).and_then(|query| chk(&query).map(|_| ()));
        assert_eq!(checked.is_ok(), accepted, "for {src}: {checked:?}");
    }

    /// The metric needs no `with` parameters, unlike the util and slip
    /// metrics: its values come from the archive, not from a derivation.
    /// The rejection names the metric and shows the form that works.
    #[test]
    fn a_jamming_comparison_without_a_unit_says_so() {
        let err = chk(&parse("points | where jamming > 0.1").unwrap()).unwrap_err();
        assert_eq!(err.message, "jamming needs a unit, e.g. 50 %");
    }

    #[test]
    fn jamming_needs_no_parameters() {
        let checked =
            parse("points | where jamming > 10 %").and_then(|query| chk(&query).map(|_| ()));
        assert_eq!(checked, Ok(()), "no `with` stage is required");
    }

    #[test]
    fn util_mask_error_has_the_add_help() {
        let err = chk(&parse("points | where util_gps < 50 %").unwrap()).unwrap_err();
        assert_eq!(err.help.as_deref(), Some("add: | with mask 15 deg"));
    }

    #[test]
    fn suggestion_lives_in_help_not_the_message() {
        // A diagnostic's fix goes in the structured `help`, not appended to
        // `message`, so the editor shows it as a separate "Hint:" line without
        // parsing the message.
        let err =
            chk(&parse("points | window 10 | where velocity > 30 km/h").unwrap()).unwrap_err();
        assert_eq!(err.message, "velocity is per point");
        assert_eq!(
            err.help.as_deref(),
            Some("wrap it in an aggregate like avg(velocity)")
        );
    }

    /// `g`, `km/h/s`, and the `kmh` alias are accepted where their quantity
    /// fits, and rejected (with a message) where it does not - `g` is an
    /// acceleration, not a speed.
    #[rstest]
    #[case("points | window 3 | where avg(accel) >= 0.3 g", None)]
    #[case("points | window 3 | where avg(accel) >= 5 km/h/s", None)]
    #[case("points | where velocity > 30 kmh", None)]
    #[case(
        "points | where velocity > 30 g",
        Some("expected a speed unit (km/h, m/s, kn), found g")
    )]
    fn acceleration_units_and_kmh_alias(#[case] src: &str, #[case] error: Option<&str>) {
        match error {
            None => {
                chk(&parse(src).expect(src)).expect(src);
            }
            Some(message) => {
                assert_eq!(chk(&parse(src).unwrap()).unwrap_err().message, message);
            }
        }
    }

    #[test]
    fn deep_nesting_errors_instead_of_overflowing() {
        // The MAX_DEPTH guard must fire well before the stack gives out.
        let src = format!("points | where {}velocity > 0 km/h", "not ".repeat(70));
        let err = parse(&src).unwrap_err();
        assert_eq!(err.message, "expression is too deeply nested");
    }

    /// Arithmetic is dimensional algebra: `*` adds dimensions, `/` subtracts,
    /// and a dimensionless result is a bare number. A product or quotient of
    /// dimensioned values is always well-formed - a wrong combination surfaces
    /// at the comparison, not the arithmetic (see the rejected cases below).
    #[rstest]
    #[case("points | where velocity + 3 km/h > 30 km/h")]
    #[case("points | where eph - 3 m > 10 m")]
    #[case("points | where velocity * 2 > 30 km/h")]
    #[case("points | where 2 * velocity > 30 km/h")]
    #[case("points | where sats_fix * 2 > 6")]
    #[case("points | where velocity / 2 > 15 km/h")]
    // length / length and speed / speed are dimensionless bare numbers.
    #[case("points | where eph / eph > 0.5")]
    #[case("points | where eph / clock_delta > 1 m/s")]
    #[case("points | where velocity / clock_delta > 0.1 m/s2")]
    // speed * duration is a length; speed / length is a rate. Both are new:
    // the old table rejected any product or quotient of two dimensioned values.
    #[case("points | where velocity * clock_delta > eph")]
    #[case("points | where velocity / eph > 2 per min")]
    fn arithmetic_accepts_well_formed_dimensions(#[case] src: &str) {
        chk(&parse(src).expect(src)).expect(src);
    }

    /// The rejected side of the algebra. A product or quotient with an exotic
    /// dimension type-checks but cannot compare to a bare number; addition
    /// still needs a shared dimension; timestamps, directions, and conditions
    /// reject arithmetic outright.
    #[rstest]
    #[case(
        "points | where velocity * eph > 3",
        "cannot compare length²/time with number"
    )]
    #[case(
        "points | where sats_fix / velocity > 3",
        "cannot compare time/length with number"
    )]
    #[case(
        "points | where velocity + eph > 3 m",
        "unsupported arithmetic between speed and length"
    )]
    #[case(
        "points | where time - clock_delta > 3 s",
        "timestamps do not support + and -"
    )]
    #[case(
        "points | where heading + 10 deg < 30 deg",
        "directions do not support + and -"
    )]
    #[case(
        "points | where (sats_fix == 1) + 1 > 1",
        "conditions do not support arithmetic"
    )]
    fn arithmetic_rejects_with_message(#[case] src: &str, #[case] expected: &str) {
        let message = chk(&parse(src).expect(src)).expect_err(src).message;
        assert_eq!(message, expected, "for {src}");
    }

    #[test]
    fn min_unit_and_min_aggregate_coexist() {
        // Position disambiguates: after a number `min` is the minute unit,
        // before `(` it is the aggregate.
        checked("points | window 3 | where delta(time) <= 15 min and min(velocity) > 5 km/h");
    }

    #[test]
    fn division_by_a_call_named_like_a_unit_round_trips() {
        // `min` names both the minute unit and the aggregate. After a unit,
        // `/ min(...)` is division by a call, not a `deg/min` compound unit -
        // the shape the printer emits for `<length> / min(x)`. Regression for
        // a format/reparse round-trip the property test surfaced.
        let query = parse("points | where avg(1 deg / min(heading)) > 0")
            .expect("division by a call parses");
        let printed = query.to_string();
        assert_eq!(
            parse(&printed).expect("re-parses").to_string(),
            printed,
            "the canonical form round-trips"
        );
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

    /// `==`/`!=` accept a discrete count (`sats_fix == 6`) but not a continuous
    /// quantity (`velocity == 30 km/h`), which would be a float-equality trap.
    #[rstest]
    #[case("points | where sats_fix == 6", true)]
    #[case("points | where velocity == 30 km/h", false)]
    fn equality_is_allowed_only_on_counts(#[case] src: &str, #[case] accepted: bool) {
        assert_eq!(chk(&parse(src).unwrap()).is_ok(), accepted, "for {src}");
    }

    /// A ratio compares against `%`, never a bare number - a bare number is the
    /// neutral kind and does not stand in for a percentage.
    #[rstest]
    #[case("points | with mask 15 deg | where util_all < 50 %", true)]
    #[case("points | with mask 15 deg | where util_all < 50", false)]
    fn a_ratio_metric_needs_a_percent_literal(#[case] src: &str, #[case] accepted: bool) {
        assert_eq!(chk(&parse(src).unwrap()).is_ok(), accepted, "for {src}");
    }

    /// `var` squares the argument's dimension: `var(velocity)` is a squared
    /// speed with no matching literal, while `var(sats_fix)` is a plain number.
    #[rstest]
    #[case("points | window 3 | where var(sats_fix) < 4", None)]
    // Two squared speeds share a dimension, so they compare.
    #[case("points | window 3 | where var(velocity) > var(velocity)", None)]
    // A squared ratio is a bare number; a squared timestamp is a squared
    // duration; a squared angle is exotic - none has a matching literal.
    #[case(
        "points | with mask 15 deg | window 3 | where var(util_all) < 0.1",
        None
    )]
    #[case(
        "points | window 3 | where var(velocity) > 30 km/h",
        Some("cannot compare speed² with speed")
    )]
    #[case(
        "points | window 3 | where var(velocity) > var(eph)",
        Some("cannot compare speed² with length²")
    )]
    #[case(
        "points | window 3 | where var(time) > 5 s",
        Some("cannot compare duration² with duration")
    )]
    #[case(
        "points | window 3 | where var(lat) > 3 deg",
        Some("cannot compare angle² with angle")
    )]
    fn var_squares_the_dimension(#[case] src: &str, #[case] error: Option<&str>) {
        match error {
            None => {
                chk(&parse(src).expect(src)).expect(src);
            }
            Some(message) => {
                assert_eq!(
                    chk(&parse(src).unwrap()).unwrap_err().message,
                    message,
                    "for {src}"
                );
            }
        }
    }

    #[test]
    fn var_on_a_direction_suggests_std() {
        let err =
            chk(&parse("points | window 3 | where var(heading) < 1 deg").unwrap()).unwrap_err();
        assert_eq!(err.message, "var is not defined for a direction");
        assert_eq!(
            err.help.as_deref(),
            Some("circular variance is unitless, not a squared angle - use std")
        );
    }

    #[test]
    fn var_matches_low_variance_windows() {
        // window 2 var(sats_fix) over [6,6,6,9]: windows [0,2) and [1,3) have
        // variance 0; [2,4) has variance 2.25. So only the steady points match.
        let provider = TestProvider::new(4).with(
            QueryMetric::SatsFix,
            vec![Some(6.0), Some(6.0), Some(6.0), Some(9.0)],
        );
        let output = run_one("points | window 2 | where var(sats_fix) < 1", &provider);
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn caret_and_superscript_powers_agree() {
        // The caret form is a convenience for the canonical superscript; both
        // parse to the same tree, so they print identically.
        let same = |caret: &str, superscript: &str| {
            assert_eq!(
                parse(caret).unwrap().to_string(),
                parse(superscript).unwrap().to_string()
            );
        };
        same(
            "points | where velocity^2 > velocity^2",
            "points | where velocity² > velocity²",
        );
        same(
            "points | where sats_fix^-1 < 0.5",
            "points | where sats_fix⁻¹ < 0.5",
        );
    }

    /// A power binds tighter than unary minus and than `*`/`/`, and its base can
    /// be a parenthesized expression.
    #[rstest]
    #[case("points | where -accel² < 0", "(-(accel²))")]
    #[case("points | where velocity² * eph > 0", "((velocity²) * eph)")]
    #[case("points | where (velocity + eph)² > 0", "((velocity + eph)²)")]
    fn power_binds_tighter_than_minus_and_mul(#[case] src: &str, #[case] fragment: &str) {
        let canonical = parse(src).expect(src).to_string();
        assert!(
            canonical.contains(fragment),
            "{canonical} should contain {fragment}"
        );
    }

    /// The exponent is a whole number in `i8` range; fractional and oversized
    /// powers are rejected while parsing.
    #[rstest]
    #[case("points | where velocity^2.5 > 0", "a power must be a whole number")]
    #[case(
        "points | where velocity^999 > 0",
        "a power must be a whole number between -128 and 127"
    )]
    #[case(
        "points | where velocity⁹⁹⁹ > 0",
        "a power must be a whole number between -128 and 127"
    )]
    #[case("points | where velocity⁻ > 0", "a power must be a whole number")]
    fn power_rejects_non_integer_and_out_of_range(#[case] src: &str, #[case] expected: &str) {
        assert_eq!(parse(src).expect_err(src).message, expected, "for {src}");
    }

    /// A power scales the base's dimension: `velocity²` is a squared speed
    /// (comparable only to another squared speed), while any power of a
    /// dimensionless value is a bare number.
    #[rstest]
    #[case("points | where sats_fix² < 100", None)]
    #[case("points | where sats_fix⁻¹ < 0.5", None)]
    #[case("points | where velocity² > velocity²", None)]
    #[case(
        "points | where velocity² > 30 km/h",
        Some("cannot compare speed² with speed")
    )]
    fn power_scales_the_dimension(#[case] src: &str, #[case] error: Option<&str>) {
        match error {
            None => {
                chk(&parse(src).expect(src)).expect(src);
            }
            Some(message) => {
                assert_eq!(
                    chk(&parse(src).unwrap()).unwrap_err().message,
                    message,
                    "for {src}"
                );
            }
        }
    }

    #[test]
    fn power_squares_a_point_value() {
        // sats_fix squared: 3² = 9 < 16 matches, 5² = 25 does not.
        let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(3.0), Some(5.0)]);
        let output = run_one("points | where sats_fix² < 16", &provider);
        assert_eq!(output.matches[0].ranges, vec![0..1]);
    }

    #[test]
    fn power_with_a_negative_exponent_inverts() {
        // sats_fix⁻¹: 1/2 = 0.5 > 0.4 matches, 1/4 = 0.25 does not.
        let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(2.0), Some(4.0)]);
        let output = run_one("points | where sats_fix⁻¹ > 0.4", &provider);
        assert_eq!(output.matches[0].ranges, vec![0..1]);
    }

    #[test]
    fn power_with_a_zero_exponent_is_one() {
        // Every value to the zeroth power is 1, so all points clear the bar.
        let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(3.0), Some(7.0)]);
        let output = run_one("points | where sats_fix⁰ > 0.5", &provider);
        assert_eq!(output.matches[0].ranges, vec![0..2]);
    }

    #[test]
    fn a_negative_power_of_zero_poisons_the_point() {
        // 0⁻¹ is infinite, so that point is skipped like any undefined
        // arithmetic; the finite inverse still matches.
        let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(0.0), Some(2.0)]);
        let output = run_one("points | where sats_fix⁻¹ < 1", &provider);
        assert_eq!(output.matches[0].ranges, vec![1..2]);
        assert_eq!(output.summary.skipped_non_finite, 1);
    }

    /// `sqrt` halves the dimension, so its argument must be a perfect square (or
    /// dimensionless): `sqrt(velocity²)` is a speed, and squaring components
    /// then rooting the sum is a magnitude. A non-square is a pointed error.
    #[rstest]
    #[case("points | where sqrt(velocity²) > 30 km/h", None)]
    #[case("points | where sqrt(lat² + lon²) > 0 deg", None)]
    #[case("points | where sqrt(velocity² + velocity²) > 30 km/h", None)]
    #[case("points | where sqrt(sats_fix) < 5", None)]
    // sqrt nested inside an aggregate (it works in a window too).
    #[case("points | window 3 | where avg(sqrt(velocity²)) > 30 km/h", None)]
    // A squared ratio roots to a bare number, compared without a unit.
    #[case("points | with mask 15 deg | where sqrt(util_gps) > 0.7", None)]
    #[case(
        "points | with mask 15 deg | where sqrt(util_gps) > 50 %",
        Some("cannot compare number with ratio")
    )]
    #[case("points | where sqrt(velocity) > 0", Some("sqrt needs a square"))]
    #[case(
        "points | where sqrt(time) > 5 s",
        Some("cannot take the square root of a timestamp")
    )]
    fn sqrt_needs_a_perfect_square(#[case] src: &str, #[case] error: Option<&str>) {
        match error {
            None => {
                chk(&parse(src).expect(src)).expect(src);
            }
            Some(message) => {
                assert_eq!(
                    chk(&parse(src).unwrap()).unwrap_err().message,
                    message,
                    "for {src}"
                );
            }
        }
    }

    #[test]
    fn sqrt_on_a_non_square_suggests_squaring_first() {
        let err = chk(&parse("points | where sqrt(velocity) > 0").unwrap()).unwrap_err();
        assert_eq!(
            err.help.as_deref(),
            Some("square the values first, e.g. sqrt(x² + y²)")
        );
    }

    #[test]
    fn a_squared_comparison_suggests_sqrt() {
        // The squared side has a matching root, so the fix is to take it.
        let err = chk(&parse("points | where velocity² > 30 km/h").unwrap()).unwrap_err();
        assert_eq!(err.message, "cannot compare speed² with speed");
        assert_eq!(err.help.as_deref(), Some("take its square root with sqrt"));
    }

    #[rstest]
    #[case("points | where @accel > 0", "accel", None)]
    #[case("points | where @accel.x > 0", "accel", Some("x"))]
    fn a_channel_reference_parses_and_formats(
        #[case] src: &str,
        #[case] name: &str,
        #[case] component: Option<&str>,
    ) {
        use crate::ast::Expr;
        let query = parse(src).expect(src);
        // The predicate is `<channel> > 0`; the channel is the comparison's LHS.
        let Some(Expr::Binary { lhs, .. }) = query.predicates.first() else {
            panic!("expected a comparison in {src}");
        };
        let Expr::Channel(c) = lhs.as_ref() else {
            panic!("expected a channel reference as the lhs in {src}");
        };
        assert_eq!(c.name, name);
        assert_eq!(c.component.as_deref(), component);
        // The canonical form round-trips through parse → format unchanged.
        let expected = match component {
            Some(comp) => format!("@{name}.{comp}"),
            None => format!("@{name}"),
        };
        assert!(query.to_string().contains(&expected));
    }

    /// A scalar channel schema entry for `@name` with the given unit/period.
    fn schema_with(name: &str, unit: Option<&str>, period_deg: Option<f64>) -> ChannelSchema {
        let mut schema = ChannelSchema::new();
        schema.insert(
            name,
            ChannelInfo {
                unit: unit.map(ChannelUnit::from_file_label),
                period_deg,
                components: vec![],
                conflicts: Vec::new(),
            },
        );
        schema
    }

    /// A single vector channel with the given unit and component labels.
    fn vector_schema(name: &str, unit: Option<&str>, components: &[&str]) -> ChannelSchema {
        let mut schema = ChannelSchema::new();
        schema.insert(
            name,
            ChannelInfo {
                unit: unit.map(ChannelUnit::from_file_label),
                period_deg: None,
                components: components.iter().map(|c| (*c).to_owned()).collect(),
                conflicts: Vec::new(),
            },
        );
        schema
    }

    #[test]
    fn a_channel_absent_from_the_schema_is_no_such_channel() {
        // An empty schema knows no channels.
        let err = check(
            &parse("points | window 10 | where max(@accel) > 0.1 g").unwrap(),
            &ChannelSchema::new(),
        )
        .unwrap_err();
        assert_eq!(err.message, "no such channel @accel");
    }

    #[test]
    fn a_scalar_channel_resolves_to_its_unit_dimension() {
        // @accel (unit g) is an acceleration, so it compares to an acceleration
        // literal and rejects a speed.
        let schema = schema_with("accel", Some("g"), None);
        let ok = "points | window 10 | where max(@accel) > 0.1 g";
        check(&parse(ok).unwrap(), &schema).expect("checks with the schema");

        let bad = "points | window 10 | where max(@accel) > 30 km/h";
        let err = check(&parse(bad).unwrap(), &schema).unwrap_err();
        assert_eq!(
            err.message,
            "expected a acceleration unit (m/s2, g, km/h/s), found km/h"
        );
    }

    #[test]
    fn a_channel_with_a_period_is_circular_and_accepts_spread() {
        // @heading (deg, period 360) is a direction, so spread accepts it.
        let schema = schema_with("heading", Some("deg"), Some(360.0));
        let ok = "points | window 10 | where spread(@heading) < 10 deg";
        check(&parse(ok).unwrap(), &schema).expect("checks with the schema");
    }

    #[rstest]
    #[case("avg")]
    #[case("min")]
    #[case("max")]
    fn a_direction_channel_rejects_ambiguous_aggregates(#[case] func: &str) {
        // avg/min/max collapse a direction ambiguously, the same rule the
        // Direction nav metric follows.
        let schema = schema_with("heading", Some("deg"), Some(360.0));
        let src = format!("points | window 10 | where {func}(@heading) < 10 deg");
        let err = check(&parse(&src).unwrap(), &schema).unwrap_err();
        assert_eq!(err.message, format!("{func} on a direction is ambiguous"));
    }

    #[rstest]
    // No unit and an unrecognised unit both resolve to a bare number: it
    // compares to a plain number but not to a dimensioned literal.
    #[case(None)]
    #[case(Some("furlong"))]
    fn a_channel_without_a_known_unit_is_a_bare_number(#[case] unit: Option<&str>) {
        let schema = schema_with("x", unit, None);
        let ok = "points | window 10 | where max(@x) > 5";
        check(&parse(ok).unwrap(), &schema).expect("a bare number compares to a number");

        let bad = "points | window 10 | where max(@x) > 5 g";
        check(&parse(bad).unwrap(), &schema).unwrap_err();
    }

    #[test]
    fn a_bare_channel_must_be_aggregated() {
        // Like a nav-point metric, a channel has no per-point value; used raw it
        // errors with a hint to wrap it in an aggregate.
        let schema = schema_with("accel", Some("g"), None);
        let err = check(
            &parse("points | window 10 | where @accel > 0.1 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "@accel is per sample");
        assert_eq!(
            err.help.as_deref(),
            Some("wrap it in an aggregate like max(@accel)")
        );
    }

    #[test]
    fn an_aggregate_over_two_channels_is_rejected() {
        // An aggregate reduces one timeline; two channels are on separate clocks
        // and cannot be combined per sample, so mixing them is a category error.
        let mut schema = schema_with("ax", Some("g"), None);
        schema.insert(
            "ay",
            ChannelInfo {
                unit: Some(Unit::G.into()),
                period_deg: None,
                components: vec![],
                conflicts: Vec::new(),
            },
        );
        let err = check(
            &parse("points | window 10 | where max(@ax + @ay) > 1.0 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "an aggregate reduces one channel at a time");
    }

    #[test]
    fn an_aggregate_mixing_a_channel_and_a_metric_is_rejected() {
        // @accel (a channel) and accel (the derived nav metric) share a dimension
        // but not a clock, so an aggregate cannot combine them per element.
        let schema = schema_with("accel", Some("g"), None);
        let err = check(
            &parse("points | window 10 | where max(@accel + accel) > 1.0 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "cannot mix @accel with a per-point metric");
    }

    #[test]
    fn a_vector_component_resolves_to_the_channel_dimension() {
        // @accel.x is one column of the g-unit vector, so it is an acceleration:
        // it compares to an acceleration literal and rejects a speed.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let ok = "points | window 10 | where max(@accel.x) > 0.1 g";
        check(&parse(ok).unwrap(), &schema).expect("a component checks like a scalar");

        let bad = "points | window 10 | where max(@accel.x) > 30 km/h";
        let err = check(&parse(bad).unwrap(), &schema).unwrap_err();
        assert!(err.message.contains("acceleration"), "{}", err.message);
    }

    #[test]
    fn a_bare_vector_channel_needs_a_component() {
        // A whole vector has no scalar value; the error points at a component.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let err = check(
            &parse("points | window 10 | where max(@accel) > 0.1 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "@accel is a vector channel");
        assert_eq!(
            err.help.as_deref(),
            Some("reference a component like @accel.x")
        );
    }

    #[test]
    fn an_unknown_component_is_rejected() {
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let err = check(
            &parse("points | window 10 | where max(@accel.w) > 0.1 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "@accel has no component w");
        assert_eq!(err.help.as_deref(), Some("its components are x, y, z"));
    }

    #[test]
    fn a_component_on_a_scalar_channel_is_rejected() {
        let schema = schema_with("incline", Some("deg"), None);
        let err = check(
            &parse("points | window 10 | where max(@incline.x) > 1 deg").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "@incline is not a vector channel");
    }

    #[test]
    fn a_bare_component_must_be_aggregated() {
        // Like a nav-point metric, a component has no per-point value; used raw
        // it hints at wrapping it in an aggregate, keeping the `.x`.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let err = check(
            &parse("points | window 10 | where @accel.x > 0.1 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "@accel.x is per sample");
        assert_eq!(
            err.help.as_deref(),
            Some("wrap it in an aggregate like max(@accel.x)")
        );
    }

    #[test]
    fn a_windowless_points_channel_points_at_the_working_forms() {
        // On the points source with no window, wrapping the channel in an
        // aggregate alone dead-ends on "needs a window", so the hint points at
        // both forms that work: an aggregate over a window, or the channel as
        // its own source. (Regression: the old hint sent the user to
        // max(@accel.x), which then failed with "max needs a window".)
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let err = check(&parse("points | where @accel.x > 0.2 mg").unwrap(), &schema).unwrap_err();
        assert_eq!(err.message, "@accel.x is per sample");
        assert_eq!(
            err.help.as_deref(),
            Some("aggregate it over a window like max(@accel.x), or query @accel as the source")
        );
    }

    #[test]
    fn an_aggregate_without_a_window_hints_at_adding_one() {
        // Following the hint above to an aggregate still needs a window; the
        // error now says how instead of leaving the user stuck.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let err = check(
            &parse("points | where max(@accel.x) > 0.2 mg").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "max needs a window");
        assert_eq!(
            err.help.as_deref(),
            Some("add a window before the where, e.g. window 10")
        );
    }

    #[test]
    fn a_windowless_points_norm_points_at_the_working_forms() {
        // norm takes the same unwindowed-points branch, labelled as it reads.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let err = check(
            &parse("points | where norm(@accel) > 0.2 mg").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "norm(@accel) is per sample");
        assert_eq!(
            err.help.as_deref(),
            Some(
                "aggregate it over a window like max(norm(@accel)), or query @accel as the source"
            )
        );
    }

    #[test]
    fn components_of_one_channel_combine_per_sample() {
        // Components of one vector share a clock, so per-sample math across them
        // is a single timeline: sqrt(x² + y²) type-checks as an acceleration.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let src = "points | window 10 | where max(sqrt(@accel.x² + @accel.y²)) > 0.1 g";
        check(&parse(src).unwrap(), &schema).expect("shared-clock components combine");
    }

    #[test]
    fn two_different_channels_cannot_combine() {
        // Distinct channels are on independent clocks, even at the same
        // dimension (both acceleration here, so the timeline rule is what bites,
        // not a unit mismatch).
        let mut schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        schema.insert(
            "accel2",
            ChannelInfo {
                unit: Some(Unit::G.into()),
                period_deg: None,
                components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
                conflicts: Vec::new(),
            },
        );
        let err = check(
            &parse("points | window 10 | where max(@accel.x + @accel2.x) > 0.1 g").unwrap(),
            &schema,
        )
        .unwrap_err();
        assert_eq!(err.message, "an aggregate reduces one channel at a time");
    }

    #[test]
    fn norm_is_the_magnitude_of_a_vector_channel() {
        // norm(@accel) is an acceleration, so it compares to a g literal.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let ok = "points | window 10 | where max(norm(@accel)) > 1 g";
        check(&parse(ok).unwrap(), &schema).expect("norm of a vector is its dimension");
    }

    /// accel and accel2 (g-unit vectors) plus the scalar incline, for exercising
    /// norm's rejections and cross-timeline mixing.
    fn norm_schema() -> ChannelSchema {
        let mut schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        schema.insert(
            "accel2",
            ChannelInfo {
                unit: Some(Unit::G.into()),
                period_deg: None,
                components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
                conflicts: Vec::new(),
            },
        );
        schema.insert(
            "incline",
            ChannelInfo {
                unit: Some(Unit::DEG.into()),
                period_deg: None,
                components: vec![],
                conflicts: Vec::new(),
            },
        );
        schema
    }

    #[rstest]
    // A scalar channel has no vector to take the magnitude of.
    #[case(
        "max(norm(@incline)) > 1 deg",
        "@incline is not a vector channel",
        Some("norm needs a vector like @accel")
    )]
    // A single component is not a whole vector.
    #[case(
        "max(norm(@accel.x)) > 1 g",
        "norm takes a whole vector, not a component",
        Some("use norm(@accel)")
    )]
    // norm is per sample, so a bare use needs an aggregate.
    #[case(
        "norm(@accel) > 1 g",
        "norm(@accel) is per sample",
        Some("wrap it in an aggregate like max(norm(@accel))")
    )]
    // norm's channel counts as a timeline: two are on separate clocks.
    #[case(
        "max(norm(@accel) + norm(@accel2)) > 1 g",
        "an aggregate reduces one channel at a time",
        Some("split it into separate aggregates, one per channel")
    )]
    // And a channel cannot mix with a per-point metric (accel is acceleration,
    // matching norm's dimension, so the timeline rule is what bites).
    #[case(
        "max(norm(@accel) + accel) > 1 g",
        "cannot mix @accel with a per-point metric",
        Some("a channel and a nav-point metric are on separate clocks")
    )]
    fn norm_and_channel_mixing_rejections(
        #[case] predicate: &str,
        #[case] message: &str,
        #[case] help: Option<&str>,
    ) {
        let src = format!("points | window 10 | where {predicate}");
        let err = check(&parse(&src).unwrap(), &norm_schema()).unwrap_err();
        assert_eq!(err.message, message);
        assert_eq!(err.help.as_deref(), help);
    }

    #[test]
    fn a_channel_can_be_the_source() {
        // `@accel | ...` iterates the channel's own samples. A bare per-sample
        // predicate is fine here: the sample is the match granularity.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        check(
            &parse("@accel | where norm(@accel) > 1 g").unwrap(),
            &schema,
        )
        .expect("a channel source with a per-sample predicate checks");
        // Aggregated over a window of samples, too.
        check(
            &parse("@accel | window 5 | where std(norm(@accel)) < 0.02 g").unwrap(),
            &schema,
        )
        .expect("a windowed channel-source aggregate checks");
    }

    #[test]
    fn a_channel_source_round_trips_through_the_formatter() {
        let src = "@accel\n| window 5\n| where (max(norm(@accel)) > 1 g)";
        let query = parse(src).unwrap();
        assert_eq!(query.to_string(), src);
    }

    #[rstest]
    // A nav-point metric on a channel source has no interpolation yet.
    #[case(
        "@accel | where velocity > 1 km/h",
        "velocity is not available on a channel source"
    )]
    // A table column is a nav metric too, rejected like a predicate one.
    #[case(
        "@accel | where norm(@accel) > 1 g | table velocity",
        "velocity is not available on a channel source"
    )]
    // Only the source channel is on the timeline.
    #[case(
        "@accel | window 3 | where max(@gyro.x) > 1 deg",
        "@gyro is not the source channel"
    )]
    // Windowed, the source channel must be aggregated like any other.
    #[case("@accel | window 3 | where @accel.x > 1 g", "@accel.x is per sample")]
    #[case(
        "@accel | window 3 | where norm(@accel) > 1 g",
        "norm(@accel) is per sample"
    )]
    // A component is not a whole-channel source.
    #[case(
        "@accel.x | where @accel.x > 1 g",
        "a channel source is a whole channel"
    )]
    // An unknown source channel.
    #[case("@nope | where @nope > 1", "no such channel @nope")]
    fn a_channel_source_rejects(#[case] src: &str, #[case] message: &str) {
        let mut schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        schema.insert(
            "gyro",
            ChannelInfo {
                unit: Some(Unit::DEG.into()),
                period_deg: None,
                components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
                conflicts: Vec::new(),
            },
        );
        let err = check(&parse(src).unwrap(), &schema).unwrap_err();
        assert_eq!(err.message, message);
    }

    /// Run a channel query against a provider, checking with the given schema.
    fn run_channel(src: &str, schema: &ChannelSchema, provider: &TestProvider) -> RunOutput {
        let query = check(&parse(src).unwrap(), schema).expect(src);
        run(
            &query,
            &[TrackInput {
                track: track_ref(),
                provider,
            }],
        )
    }

    #[test]
    fn a_channel_aggregate_reduces_native_samples_in_the_window_span() {
        // Points at 0,1,2 s; @accel sampled finer than points. A count `window 3`
        // spans the closed point extent [t(0), t(2)] = [0, 2], holding all five
        // accel samples. Sample values are base units (m/s2) per the channel_span
        // contract, near 1g here; the peak 10.8 clears 1.0 g (9.81 m/s2), so the
        // whole track matches.
        let schema = schema_with("accel", Some("g"), None);
        let provider = TestProvider::new(3).indexed_time().with_channel(
            "accel",
            vec![
                (0.0, 9.6),
                (0.5, 10.3),
                (1.0, 10.8),
                (1.5, 10.0),
                (2.0, 9.7),
            ],
        );
        let output = run_channel(
            "points | window 3 | where max(@accel) > 1.0 g",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn a_channel_reduces_more_samples_than_points() {
        // std over a `window 2` (span [0, 1]) reduces all five native accel
        // samples, not the 2 points. Values are base units (m/s2). A flat channel
        // has std 0 (< 0.02 g = 0.196 m/s2); a jumpy one does not.
        let schema = schema_with("accel", Some("g"), None);
        let flat = TestProvider::new(2).indexed_time().with_channel(
            "accel",
            vec![(0.0, 9.8), (0.25, 9.8), (0.5, 9.8), (0.75, 9.8), (1.0, 9.8)],
        );
        let calm = run_channel(
            "points | window 2 | where std(@accel) < 0.02 g",
            &schema,
            &flat,
        );
        assert_eq!(calm.matches[0].ranges, vec![0..2]);

        let jumpy = TestProvider::new(2).indexed_time().with_channel(
            "accel",
            vec![
                (0.0, 9.0),
                (0.25, 10.5),
                (0.5, 9.0),
                (0.75, 10.5),
                (1.0, 9.0),
            ],
        );
        let shaky = run_channel(
            "points | window 2 | where std(@accel) < 0.02 g",
            &schema,
            &jumpy,
        );
        assert!(shaky.matches.is_empty());
    }

    #[test]
    fn a_window_with_no_channel_samples_is_reported_as_skipped() {
        // The channel has no samples in the window's span, so the aggregate is
        // missing: the window is skipped, attributed to the channel by name.
        let schema = schema_with("accel", Some("g"), None);
        let provider = TestProvider::new(2)
            .indexed_time()
            .with_channel("accel", vec![(100.0, 9.8)]); // far outside [0, 1]
        let output = run_channel(
            "points | window 2 | where max(@accel) > 0.5 g",
            &schema,
            &provider,
        );
        assert!(output.matches.is_empty());
        assert_eq!(output.summary.skipped_channels.get("accel"), Some(&1));
    }

    #[test]
    fn channel_per_sample_arithmetic_reduces_within_the_timeline() {
        // `@accel * 2` is per-sample math within the channel's own clock: each
        // base-unit sample doubled, then reduced. max(2*5.2) = 10.4 clears
        // 1.0 g (9.81 m/s2).
        let schema = schema_with("accel", Some("g"), None);
        let provider = TestProvider::new(2)
            .indexed_time()
            .with_channel("accel", vec![(0.0, 4.0), (0.5, 5.2), (1.0, 4.5)]);
        let output = run_channel(
            "points | window 2 | where max(@accel * 2) > 1.0 g",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..2]);
    }

    #[test]
    fn a_vector_component_reduces_its_own_column() {
        // @accel.y reduces the middle column, not x or z. A `window 3` spans the
        // three points' closed time extent [0, 2], holding all three samples.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let provider = TestProvider::new(3).indexed_time().with_vector_channel(
            "accel",
            vec![
                (0.0, vec![9.0, 10.5, 9.5]),
                (1.0, vec![9.1, 11.0, 9.4]),
                (2.0, vec![9.2, 10.8, 9.6]),
            ],
        );
        // The y column peaks at 11.0, clearing 1.0 g (9.81 m/s2), so all match.
        let y = run_channel(
            "points | window 3 | where max(@accel.y) > 1.0 g",
            &schema,
            &provider,
        );
        assert_eq!(y.matches[0].ranges, vec![0..3]);
        // The x column peaks at 9.2, below 1.0 g, so nothing matches - proving
        // the reduction reads x's column, not whichever column happens to clear.
        let x = run_channel(
            "points | window 3 | where max(@accel.x) > 1.0 g",
            &schema,
            &provider,
        );
        assert!(x.matches.is_empty());
    }

    #[test]
    fn norm_reduces_the_per_sample_vector_magnitude() {
        // norm(@accel) is sqrt(x²+y²+z²) per sample, on base-unit rows. Row 0 is
        // (3,4,0) -> 5 m/s2; the rest are near zero. 0.1 g is 0.981 m/s2, so
        // max(norm) = 5 clears it and min does not. Unit "g", which the schema
        // types as an acceleration.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let provider = TestProvider::new(2).indexed_time().with_vector_channel(
            "accel",
            vec![(0.0, vec![3.0, 4.0, 0.0]), (1.0, vec![0.1, 0.0, 0.0])],
        );
        let hit = run_channel(
            "points | window 2 | where max(norm(@accel)) > 0.1 g",
            &schema,
            &provider,
        );
        assert_eq!(hit.matches[0].ranges, vec![0..2]);
        let miss = run_channel(
            "points | window 2 | where min(norm(@accel)) > 0.1 g",
            &schema,
            &provider,
        );
        assert!(miss.matches.is_empty());
    }

    #[test]
    fn norm_of_a_non_finite_sample_poisons_the_window() {
        // A component of f64::MAX squares to inf, so norm is non-finite: the
        // window poisons rather than matching, and the skip is counted.
        let schema = vector_schema("accel", Some("g"), &["x", "y"]);
        let provider = TestProvider::new(1)
            .indexed_time()
            .with_vector_channel("accel", vec![(0.0, vec![f64::MAX, 0.0])]);
        let output = run_channel(
            "points | window 1 | where max(norm(@accel)) > 0.1 g",
            &schema,
            &provider,
        );
        assert!(output.matches.is_empty());
        assert_eq!(output.summary.skipped_non_finite, 1);
    }

    #[test]
    fn components_combine_per_sample_within_the_row() {
        // sqrt(x² + y²) is per-sample math across two columns of the same row.
        // Row 0 is (3, 4) -> 5 m/s2, clearing 0.1 g (0.981 m/s2); the explicit
        // form matches what norm computes over those columns.
        let schema = vector_schema("accel", Some("g"), &["x", "y"]);
        let provider = TestProvider::new(2)
            .indexed_time()
            .with_vector_channel("accel", vec![(0.0, vec![3.0, 4.0]), (1.0, vec![0.1, 0.0])]);
        let output = run_channel(
            "points | window 2 | where max(sqrt(@accel.x² + @accel.y²)) > 0.1 g",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..2]);
    }

    #[test]
    fn a_channel_source_matches_per_sample() {
        // `@accel | where ...` judges each sample on its own; the match ranges
        // are sample indices, and the total is the sample count, not nav points.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let provider = TestProvider::new(2).indexed_time().with_vector_channel(
            "accel",
            vec![
                (0.0, vec![9.8, 0.0, 0.0]),  // norm 9.8, over 0.1 g
                (1.0, vec![0.1, 0.0, 0.0]),  // norm 0.1, under
                (2.0, vec![10.0, 0.0, 0.0]), // over
            ],
        );
        let output = run_channel("@accel | where norm(@accel) > 0.1 g", &schema, &provider);
        assert_eq!(output.matches[0].ranges, vec![0..1, 2..3]);
        // Three channel samples, not the provider's two nav points.
        assert_eq!(output.summary.total_points, 3);
    }

    #[test]
    fn a_channel_source_window_reduces_its_samples() {
        // `@accel | window 3 | where max(norm(@accel)) > 0.1 g`: the window over
        // all three samples has peak norm 10, so every sample matches.
        let schema = vector_schema("accel", Some("g"), &["x", "y", "z"]);
        let provider = TestProvider::new(2).indexed_time().with_vector_channel(
            "accel",
            vec![
                (0.0, vec![0.1, 0.0, 0.0]),
                (1.0, vec![0.1, 0.0, 0.0]),
                (2.0, vec![10.0, 0.0, 0.0]),
            ],
        );
        let output = run_channel(
            "@accel | window 3 | where max(norm(@accel)) > 0.1 g",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn a_channel_source_duration_window_groups_by_sample_time() {
        // `@accel | window 2 s`: at anchor 0 the samples in [0, 2) s are indices
        // 0 and 1 (the sample at 2.0 s starts a window that overruns the data).
        // Their std over the x column is 0, so a calm channel matches [0, 2).
        let schema = vector_schema("accel", Some("g"), &["x"]);
        let provider = TestProvider::new(2).indexed_time().with_vector_channel(
            "accel",
            vec![(0.0, vec![9.8]), (1.0, vec![9.8]), (2.0, vec![9.8])],
        );
        let output = run_channel(
            "@accel | window 2 s | where std(@accel.x) < 0.02 g",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..2]);
    }

    #[test]
    fn a_duration_window_gathers_channel_samples_over_its_span() {
        // A `window 2 s` at anchor 0 spans [0, 2] s (the declared extent, closed
        // at the top), the only anchor whose full duration fits. The sample at
        // exactly 2.0 s is gathered though point 2 (at t=2) is not in the
        // half-open [0, 2) point window - the boundary difference the code flags.
        // The span's peak 10.8 clears 1.0 g, so the window's points 0 and 1 match.
        let schema = schema_with("accel", Some("g"), None);
        let provider = TestProvider::new(3).indexed_time().with_channel(
            "accel",
            vec![(0.0, 9.6), (0.7, 10.8), (1.4, 10.0), (2.0, 9.7)],
        );
        let output = run_channel(
            "points | window 2 s | where max(@accel) > 1.0 g",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..2]);
    }

    #[test]
    fn a_channel_span_includes_samples_on_both_endpoints() {
        // The span is closed at both ends, so samples at exactly t_lo and t_hi
        // both count. A count `window 3` spans [0, 2]; the endpoints 9 and 11 are
        // the min and max, so spread = 2 clears the threshold. Dropping either
        // endpoint drops the spread to 1 and the window no longer matches, so the
        // assertion pins both bounds. A unitless channel keeps the math plain.
        let schema = schema_with("sensor", None, None);
        let provider = TestProvider::new(3)
            .indexed_time()
            .with_channel("sensor", vec![(0.0, 9.0), (1.0, 10.0), (2.0, 11.0)]);
        let output = run_channel(
            "points | window 3 | where spread(@sensor) > 1.5",
            &schema,
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![0..3]);
    }

    #[test]
    fn sqrt_computes_a_magnitude() {
        // sqrt(lat² + lon²): sqrt(3² + 4²) = 5 clears 4.5, sqrt(0) does not.
        let provider = TestProvider::new(2)
            .with(QueryMetric::Lat, vec![Some(3.0), Some(0.0)])
            .with(QueryMetric::Lon, vec![Some(4.0), Some(0.0)]);
        let output = run_one("points | where sqrt(lat² + lon²) > 4.5 deg", &provider);
        assert_eq!(output.matches[0].ranges, vec![0..1]);
    }

    #[test]
    fn sqrt_wraps_a_windowed_aggregate() {
        // sqrt(avg(velocity)²) over window 2: avg 15 m/s (54 km/h) misses the
        // 70 km/h bar, avg 25 m/s (90 km/h) clears it.
        let provider = TestProvider::new(3).with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(20.0), Some(30.0)],
        );
        let output = run_one(
            "points | window 2 | where sqrt(avg(velocity)²) > 70 km/h",
            &provider,
        );
        assert_eq!(output.matches[0].ranges, vec![1..3]);
    }

    #[test]
    fn sqrt_of_a_negative_poisons_the_point() {
        // sqrt(sats_fix - 10): 4 - 10 = -6 roots to NaN and is skipped; 20 - 10
        // roots to a finite value that matches.
        let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(4.0), Some(20.0)]);
        let output = run_one("points | where sqrt(sats_fix - 10) < 5", &provider);
        assert_eq!(output.matches[0].ranges, vec![1..2]);
        assert_eq!(output.summary.skipped_non_finite, 1);
    }

    #[test]
    fn long_arithmetic_chain_checks_without_panicking() {
        // A long `*` chain folds many exponent additions in the checker; the
        // dimension arithmetic saturates rather than overflowing i8 (which once
        // panicked in dev builds). The exotic dimension has no matching literal,
        // so the query is rejected at the comparison rather than crashing.
        let chain = std::iter::repeat_n("velocity", 200)
            .collect::<Vec<_>>()
            .join(" * ");
        let src = format!("points | where {chain} > 0");
        let result = chk(&parse(&src).expect("a long product chain parses"));
        assert!(
            result.is_err(),
            "an exotic dimension cannot compare to a bare number"
        );
    }

    #[test]
    fn uc1_parses_checks_and_formats() {
        let query = parse(UC1).unwrap();
        chk(&query).unwrap();
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
            "points | window 10 km/h",
            "points | window 0 s",
            "points | window 3 | window 4",
            "points | draw | where velocity > 0 km/h",
            "points | draw | draw",
            "points | keep | hide",
            "points | where velocity > 0 km/h | keep | table time",
            "points | where velocity > 30 mph",
            "points | where velocity > 30 km/s",
            "points | where accel > 1 g/s",
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
                    Ok(q) => match chk(&q) {
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

        use gt_types::DisplayMode;

        use super::super::ast::{
            BinaryOp, ChannelRef, Expr, Func, MetricRef, ModeStage, NumberLit, ParamDecl,
            ParamName, Query, Source, Span, TableSpec, UnaryOp, Window,
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
            // Built from the enum's own iterator so a new variant is covered
            // by the format/reparse round-trip automatically.
            proptest::sample::select(Unit::CANONICAL.to_vec())
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
                (1u64..1000).prop_map(|len| Window::Count { len, span: span() }),
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
            let table = proptest::option::of(proptest::collection::vec(metric_strategy(), 1..4));
            (params, window, predicates, mode, table).prop_map(
                |(params, window, predicates, mode, table)| Query {
                    // The round-trip fuzz targets stage formatting; channel-source
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
