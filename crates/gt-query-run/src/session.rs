use gt_loaded_files::LoadedFilesView;
use gt_query::{ChannelSchema, CheckedQuery, Diagnostic, Span};
use gt_ui_types::QueryMatches;

use crate::check::{self, QueryChunk};
use crate::dependencies::RunDependencies;
use crate::fingerprint::{RunFingerprint, RunInputs};
use crate::results::{ChannelResults, PointsResults, RunResults};
use crate::run::{PreparedRun, RunHandle, RunKind, RunOutcome, RunProduct};

pub(super) mod retained;

pub const MIXED_CHANNEL_MESSAGE: &str =
    "A channel-source query must be the only query in the editor";

/// What [`QuerySession::sync_checks`] found changed since the last check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckRefresh {
    /// The channel schema changed - a file load or unload - but the text did
    /// not, so a `@name` error may have resolved on its own.
    SchemaChanged,
    /// The text changed, possibly along with the schema.
    TextChanged,
    /// Text and schema both unchanged, so the chunks were kept.
    Unchanged,
}

pub enum QueryTextCheck {
    Empty,
    Invalid,
    Valid(CheckedQueryText),
}

pub struct CheckedQueryText {
    text: String,
    schema: ChannelSchema,
    chunks: Vec<QueryChunk>,
}

impl CheckedQueryText {
    /// Report diagnostics with byte spans in the full text. An error from the
    /// callback stops validation before the next chunk.
    pub fn check<E>(
        text: String,
        schema: ChannelSchema,
        diagnose: impl FnMut(Diagnostic) -> Result<(), E>,
    ) -> Result<QueryTextCheck, E> {
        let Some(chunks) =
            check::runnable_chunks(&text, check::check_chunks(&text, &schema), diagnose)?
        else {
            return Ok(QueryTextCheck::Invalid);
        };
        if chunks.is_empty() {
            return Ok(QueryTextCheck::Empty);
        }
        Ok(QueryTextCheck::Valid(Self {
            text,
            schema,
            chunks,
        }))
    }
}

/// How far a run in flight has come, for the progress line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryProgress {
    /// Tracks whose derived series are ready.
    pub tracks_prepared: usize,
    /// Tracks the run evaluates in total.
    pub track_total: usize,
}

/// A run in flight somewhere else: what it takes to watch, cancel, and later
/// receive it. The caller transports the outcome back.
struct InFlightRun {
    handle: RunHandle,
    track_total: usize,
    /// Snapshot taken when the run started, attached to its results.
    fingerprint: RunFingerprint,
}

/// The results of the last completed run and the inputs it saw.
struct CompletedRun {
    results: RunResults,
    fingerprint: RunFingerprint,
}

/// The query run lifecycle: the editor's text, that text checked chunk by
/// chunk, the run in flight, and the results of the last one.
///
/// Everything here is synchronous and free of UI. A caller drives a whole run
/// in four steps - [`set_text`](Self::set_text),
/// [`sync_checks`](Self::sync_checks), [`start_run`](Self::start_run),
/// [`finish_run`](Self::finish_run) - and evaluates the prepared run wherever
/// it likes: the app hands it to a worker thread, a test calls
/// [`PreparedRun::execute`] inline.
pub struct QuerySession {
    text: String,
    /// The blank-line-separated queries of `text`, each parsed and checked.
    chunks: Vec<QueryChunk>,
    /// The text `chunks` was computed from.
    checked_text: String,
    /// The channel schema `chunks` was checked against. Kept so a file load or
    /// unload that changes the channels re-checks even when the text is
    /// unchanged (a `@name` error resolves once its channel appears).
    checked_schema: ChannelSchema,
    checked_dependencies: RunDependencies,
    in_flight: Option<InFlightRun>,
    completed: Option<CompletedRun>,
    /// Sequence number the next completed run's matches carry. Starts at 1, so
    /// the map can read 0 as "no run".
    next_run: u64,
}

impl QuerySession {
    /// An empty session: no text, no results.
    pub fn new() -> Self {
        let text = String::new();
        Self {
            chunks: check::check_all(&text, &ChannelSchema::new()),
            checked_text: text.clone(),
            checked_schema: ChannelSchema::new(),
            checked_dependencies: RunDependencies::default(),
            text,
            in_flight: None,
            completed: None,
            next_run: 1,
        }
    }

    /// The current editor text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The editor text for in-place editing, as a text widget needs it. The
    /// checks go stale until the next [`sync_checks`](Self::sync_checks).
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.text
    }

    /// Replace the editor text, e.g. when loading a history entry or an
    /// example. Never runs - running stays an explicit step.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// Re-check the text against `schema` when either changed since the last
    /// call, and report what that was, so a caller can react to an edit (arming
    /// its diagnostic grace period, invalidating completions).
    pub fn sync_checks(&mut self, schema: &ChannelSchema) -> CheckRefresh {
        let text_changed = self.checked_text != self.text;
        let schema_changed = self.checked_schema != *schema;
        if !text_changed && !schema_changed {
            return CheckRefresh::Unchanged;
        }
        self.chunks = check::check_all(&self.text, schema);
        self.checked_dependencies = RunDependencies::of(&self.chunks);
        self.checked_text = self.text.clone();
        self.checked_schema = schema.clone();
        if text_changed {
            CheckRefresh::TextChanged
        } else {
            CheckRefresh::SchemaChanged
        }
    }

    /// The checked queries of the editor text, in editor order.
    pub fn chunks(&self) -> &[QueryChunk] {
        &self.chunks
    }

    pub fn checked_dependencies(&self) -> RunDependencies {
        self.checked_dependencies
    }

    /// Whether every query in the editor checks and there is at least one, so
    /// a run is possible.
    pub fn all_ok(&self) -> bool {
        !self.chunks.is_empty() && self.chunks.iter().all(|c| c.result.is_ok())
    }

    /// How a run of the current (checked) queries would dispatch, or why it
    /// cannot run. Only meaningful when [`all_ok`](Self::all_ok).
    pub fn run_kind(&self) -> RunKind {
        RunKind::of_checked_chunks(&self.chunks)
    }

    /// The editor-coordinate span of each channel-source chunk's `@name` source
    /// token, for the mixed-channel underline.
    pub fn channel_source_spans(&self) -> Vec<Span> {
        self.chunks
            .iter()
            .filter_map(|chunk| chunk.channel_source_span(&self.text))
            .collect()
    }

    /// Prepare a run of the checked queries over `inputs`, or `None` when
    /// nothing can run: a failing (or empty) editor, a channel source mixed
    /// with other queries, or a run already in flight.
    ///
    /// The prepared run owns its data, so the caller can evaluate it anywhere
    /// and hand the outcome back to [`finish_run`](Self::finish_run).
    pub fn start_run(&mut self, inputs: RunInputs<'_>) -> Option<PreparedRun> {
        if !self.all_ok() || self.run_kind() == RunKind::MixedChannel || self.in_flight.is_some() {
            return None;
        }
        let queries: Vec<CheckedQuery> = self
            .chunks
            .iter()
            .filter_map(|c| c.result.as_ref().ok().cloned())
            .collect();
        let fingerprint = RunFingerprint::of(inputs, self.checked_dependencies);
        let prepared = PreparedRun::new(queries, &fingerprint, inputs);
        self.in_flight = Some(InFlightRun {
            handle: prepared.handle(),
            track_total: prepared.track_count(),
            fingerprint,
        });
        Some(prepared)
    }

    /// Take in the outcome of the run in flight: its results become the current
    /// ones, or - when it was cancelled - the previous results stay untouched.
    pub fn finish_run(&mut self, outcome: RunOutcome) {
        let Some(in_flight) = self
            .in_flight
            .take_if(|run| outcome.belongs_to(&run.handle))
        else {
            return;
        };
        let Some((product, track_data)) = outcome.into_product() else {
            return;
        };
        let mut results = match product {
            RunProduct::Points(run) => RunResults::Points(PointsResults::project(run, track_data)),
            RunProduct::Channel(run) => RunResults::Channel(ChannelResults::project(*run)),
        };
        results.set_run(self.next_run);
        self.next_run = self.next_run.saturating_add(1);
        self.completed = Some(CompletedRun {
            results,
            fingerprint: in_flight.fingerprint,
        });
    }

    /// Forget the run in flight without results, for a driver that lost it.
    pub fn abandon_run(&mut self) {
        self.in_flight = None;
    }

    /// Request that the run in flight stop. It reports back as cancelled,
    /// leaving the previous results in place.
    pub fn cancel_run(&self) {
        if let Some(in_flight) = &self.in_flight {
            in_flight.handle.cancel();
        }
    }

    /// Whether a run is in flight, so a second one cannot start.
    pub fn run_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// How far the run in flight has come, absent when none is.
    pub fn progress(&self) -> Option<QueryProgress> {
        let in_flight = self.in_flight.as_ref()?;
        Some(QueryProgress {
            tracks_prepared: in_flight.handle.tracks_prepared(),
            track_total: in_flight.track_total,
        })
    }

    /// The results of the last completed run, absent until one completes.
    pub fn results(&self) -> Option<&RunResults> {
        Some(&self.completed.as_ref()?.results)
    }

    pub fn result_dependencies(&self) -> Option<RunDependencies> {
        Some(self.completed.as_ref()?.fingerprint.dependencies)
    }

    /// Matches of the last completed run.
    pub fn matches(&self) -> Option<&QueryMatches> {
        Some(self.results()?.matches())
    }

    /// The map reads track indices from runs over the current recording generation.
    pub fn map_matches(&self, files: LoadedFilesView<'_>) -> Option<&QueryMatches> {
        let completed = self.completed.as_ref()?;
        completed
            .fingerprint
            .matches_loaded_files(files)
            .then_some(completed.results.matches())
    }

    /// Mark the last run's results stale when the current input fingerprint differs.
    pub fn refresh_staleness(&mut self, inputs: RunInputs<'_>) {
        let Some(completed) = &mut self.completed else {
            return;
        };
        let stale = !completed.fingerprint.matches(inputs);
        completed.results.set_stale(stale);
    }

    /// Drop the last run's results so the map returns to normal, abandoning any
    /// run still in flight.
    pub fn clear_results(&mut self) {
        self.completed = None;
        self.in_flight = None;
    }
}

impl Default for QuerySession {
    fn default() -> Self {
        Self::new()
    }
}

impl From<CheckedQueryText> for QuerySession {
    fn from(
        CheckedQueryText {
            text,
            schema,
            chunks,
        }: CheckedQueryText,
    ) -> Self {
        Self {
            checked_text: text.clone(),
            text,
            checked_schema: schema,
            checked_dependencies: RunDependencies::of(&chunks),
            chunks,
            ..Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_query::{AggregateColumn, TableColumn};
    use gt_types::{FileIdx, TrackIdx, TrackRef};
    use gt_ui_types::{GeomagneticPoint, GeomagneticSeries, TecPoint, TecSeries};
    use rstest::rstest;

    use super::*;
    use crate::fingerprint::{JammingValues, SnapErrorValues};
    use crate::results::MatchValues;
    use crate::test_util::{self, LoadedState};

    #[test]
    fn a_session_runs_holds_clears_and_runs_again() {
        let state = LoadedState::with_channels(vec![]);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut session = QuerySession::new();
        assert!(session.matches().is_none(), "no run, no matches");

        // The first point moves at 36 km/h, the second carries no velocity.
        test_util::run_text(
            &mut session,
            &state,
            "points | where velocity > 30 km/h | draw",
        );
        let matches = session.matches().expect("a completed run has matches");
        assert_eq!(matches.draws.len(), 1);
        assert_eq!(matches.draws[0].ranges_for(track), [test_util::rng(0, 1)]);
        assert!(!matches.stale, "a fresh run is not stale");

        session.clear_results();
        assert!(session.matches().is_none(), "clearing drops the results");

        // A second run over the same session: the hide mode takes the map
        // effect the other way round.
        test_util::run_text(
            &mut session,
            &state,
            "points | where velocity > 30 km/h | hide",
        );
        let matches = session.matches().expect("the second run has matches");
        assert!(matches.draws.is_empty(), "hide draws nothing");
        assert_eq!(matches.hidden_ranges(track), [test_util::rng(0, 1)]);
    }

    #[test]
    fn each_completed_run_numbers_its_matches() {
        let state = LoadedState::with_channels(vec![]);
        let mut session = QuerySession::new();
        let text = "points | where velocity > 30 km/h | draw";
        test_util::run_text(&mut session, &state, text);
        assert_eq!(session.matches().expect("first run").run, 1);
        test_util::run_text(&mut session, &state, text);
        assert_eq!(session.matches().expect("second run").run, 2);

        // A cancelled run keeps the previous results, numbers and all.
        session.sync_checks(&state.schema());
        let prepared = session.start_run(state.inputs()).expect("checks");
        session.cancel_run();
        session.finish_run(prepared.execute());
        assert_eq!(session.matches().expect("previous results").run, 2);
    }

    #[test]
    fn a_series_change_expires_results_for_the_queries_that_read_it() {
        let state = LoadedState::with_channels(vec![]);
        let inputs = state.inputs();
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let values = Arc::new(vec![Some(2.0), None]);
        let snap_errors = SnapErrorValues::from_iter([(track, Arc::clone(&values))]);
        let jamming = JammingValues::from_iter([(track, values)]);
        let mut geomagnetic = GeomagneticSeries::default();
        geomagnetic.points_by_track.insert(
            track,
            Arc::new(vec![GeomagneticPoint {
                x_secs: 0.0,
                kp: None,
                hp30: Some(2.0),
            }]),
        );
        let mut tec = TecSeries::default();
        tec.points_by_track.insert(
            track,
            Arc::new(vec![TecPoint {
                x_secs: 0.0,
                tecu: Some(2.0),
            }]),
        );

        let mut velocity = QuerySession::new();
        test_util::run_text(&mut velocity, &state, "points | where velocity > 30 km/h");
        let mut consumer = QuerySession::new();
        for (text, current_inputs) in [
            (
                "points | table snap_error",
                RunInputs {
                    snap_errors: &snap_errors,
                    ..inputs
                },
            ),
            (
                "points | table jamming",
                RunInputs {
                    jamming: &jamming,
                    ..inputs
                },
            ),
            (
                "points | table hp30",
                RunInputs {
                    geomagnetic: &geomagnetic,
                    ..inputs
                },
            ),
            (
                "points | where tec > 1",
                RunInputs {
                    tec: &tec,
                    ..inputs
                },
            ),
        ] {
            test_util::run_text(&mut consumer, &state, text);
            consumer.refresh_staleness(current_inputs);
            velocity.refresh_staleness(current_inputs);
            assert!(consumer.results().unwrap().stale(), "{text}");
            assert!(!velocity.results().unwrap().stale(), "{text}");
        }
    }

    #[test]
    fn a_later_block_aggregate_depends_on_its_geomagnetic_index() {
        let state = LoadedState::with_channels(vec![]);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut geomagnetic = GeomagneticSeries::default();
        let original = GeomagneticPoint {
            x_secs: 0.0,
            hp30: Some(2.0),
            kp: Some(3.0),
        };
        let pair = |index| Arc::new(vec![index; 2]);
        geomagnetic.points_by_track.insert(track, pair(original));
        let mut session = QuerySession::new();
        session.set_text("points | table velocity\n\npoints | window 2 | table avg(kp)".to_owned());
        session.sync_checks(&state.schema());
        let run = session
            .start_run(RunInputs {
                geomagnetic: &geomagnetic,
                ..state.inputs()
            })
            .unwrap();
        session.finish_run(run.execute());
        drop(run);
        let Some(RunResults::Points(results)) = session.results() else {
            panic!("expected point results");
        };
        assert_eq!(
            results.queries[1].matches[0].matches[0].aggregates,
            [Some(3.0)]
        );

        session.set_text("points | table hp30".to_owned());
        session.sync_checks(&state.schema());
        geomagnetic.points_by_track.insert(
            track,
            pair(GeomagneticPoint {
                hp30: Some(4.0),
                ..original
            }),
        );
        session.refresh_staleness(RunInputs {
            geomagnetic: &geomagnetic,
            ..state.inputs()
        });
        assert!(!session.results().unwrap().stale());

        geomagnetic.points_by_track.insert(
            track,
            pair(GeomagneticPoint {
                kp: Some(5.0),
                ..original
            }),
        );
        session.refresh_staleness(RunInputs {
            geomagnetic: &geomagnetic,
            ..state.inputs()
        });
        assert!(session.results().unwrap().stale());
    }

    #[test]
    fn a_cancelled_run_keeps_the_previous_results() {
        let state = LoadedState::with_channels(vec![]);
        let mut session = QuerySession::new();
        test_util::run_text(
            &mut session,
            &state,
            "points | where velocity > 30 km/h | draw",
        );
        let before = session
            .matches()
            .expect("the first run completed")
            .draws
            .len();

        session.set_text("points | where velocity > 1 km/h | hide".to_owned());
        session.sync_checks(&state.schema());
        let prepared = session.start_run(state.inputs()).expect("checks");
        session.cancel_run();
        session.finish_run(prepared.execute());

        assert!(!session.run_in_flight(), "the cancelled run completed");
        let matches = session.matches().expect("the previous results stand");
        assert_eq!(matches.draws.len(), before, "the draw layer is untouched");
    }

    #[test]
    fn a_second_run_cannot_start_while_one_is_in_flight() {
        let state = LoadedState::with_channels(vec![]);
        let mut session = QuerySession::new();
        session.set_text("points | where velocity > 30 km/h".to_owned());
        session.sync_checks(&state.schema());
        let prepared = session.start_run(state.inputs()).expect("checks");
        assert!(
            session.start_run(state.inputs()).is_none(),
            "one run at a time"
        );
        session.finish_run(prepared.execute());
        assert!(
            session.start_run(state.inputs()).is_some(),
            "and then again"
        );
    }

    #[test]
    fn a_failing_query_never_starts_a_run() {
        let state = LoadedState::with_channels(vec![]);
        let mut session = QuerySession::new();
        session.set_text("points | where nope > 1".to_owned());
        session.sync_checks(&state.schema());
        assert!(!session.all_ok());
        assert!(session.start_run(state.inputs()).is_none());
    }

    #[test]
    fn checks_refresh_on_text_and_on_schema_changes() {
        let state = LoadedState::with_channels(vec![test_util::scalar_channel(
            "accel",
            Some("g"),
            &[(0, 1.0)],
        )]);
        let mut session = QuerySession::new();
        session.set_text("points | window 2 | where max(@accel) > 1 g".to_owned());
        // Without the channel in the schema, the query cannot check.
        assert_eq!(
            session.sync_checks(&ChannelSchema::new()),
            CheckRefresh::TextChanged
        );
        assert!(!session.all_ok(), "an unknown channel fails the check");
        assert_eq!(
            session.sync_checks(&ChannelSchema::new()),
            CheckRefresh::Unchanged
        );
        // Loading the file that carries the channel resolves the error without
        // the text changing.
        assert_eq!(
            session.sync_checks(&state.schema()),
            CheckRefresh::SchemaChanged
        );
        assert!(session.all_ok(), "the loaded channel resolves the error");
    }

    #[rstest]
    // Every query points-source: the composing pipeline.
    #[case("points | where velocity > 1 km/h", RunKind::Points)]
    // A lone channel source: standalone run.
    #[case("@accel | where norm(@accel) > 1 g", RunKind::Channel)]
    // A channel source mixed with a points query: disallowed.
    #[case(
        "points | where velocity > 1 km/h\n\n@accel | where norm(@accel) > 1 g",
        RunKind::MixedChannel
    )]
    fn run_kind_classifies_the_editor_queries(#[case] text: &str, #[case] expected: RunKind) {
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.0, 0.0, 0.0])],
        )]);
        let mut session = QuerySession::new();
        session.set_text(text.to_owned());
        session.sync_checks(&state.schema());
        assert!(session.all_ok(), "fixture queries must check");
        assert_eq!(session.run_kind(), expected);
    }

    #[test]
    fn a_mixed_channel_editor_never_starts_a_run() {
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.0, 0.0, 0.0])],
        )]);
        let mut session = QuerySession::new();
        session.set_text(
            "points | where velocity > 1 km/h\n\n@accel | where norm(@accel) > 1 g".to_owned(),
        );
        session.sync_checks(&state.schema());
        assert!(session.all_ok(), "both queries check on their own");
        assert!(
            session.start_run(state.inputs()).is_none(),
            "a channel source must stand alone"
        );
    }

    #[test]
    fn a_channel_source_run_pairs_matches_with_the_timeline() {
        // Sample 0 (1.5 g -> 14.7 m/s2) clears 1 g. Sample 1 (0.2 g -> 1.96)
        // does not. The matched sample (at t=0 s) bands the track: a draw halo
        // over the enclosing nav-point range.
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.5, 0.0, 0.0]), (1, [0.2, 0.0, 0.0])],
        )]);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut session = QuerySession::new();
        test_util::run_text(&mut session, &state, "@accel | where norm(@accel) > 1 g");

        let Some(RunResults::Channel(results)) = session.results() else {
            panic!("a channel source produces channel results");
        };
        assert_eq!(results.channel, "accel");
        assert_eq!(results.components, ["x", "y", "z"]);
        assert_eq!(results.tracks.len(), 1);
        assert_eq!(
            results.tracks[0].matches,
            [test_util::matched_rows(test_util::rng(0, 1))]
        );
        assert_eq!(results.tracks[0].timeline.times.len(), 2);
        assert_eq!(results.matches.draws.len(), 1);
        assert_eq!(
            results.matches.draws[0].ranges_for(track),
            [test_util::rng(0, 1)]
        );
    }

    /// `g` in the evaluator's base unit, which channel samples are valued in.
    fn gravity_in_base_units() -> f64 {
        geotrace_sdk_units::Unit::from_label("g")
            .expect("g is a recognized unit")
            .to_base()
    }

    /// The aggregate values of every match of a points run, over its queries in
    /// editor order.
    fn points_aggregates(session: &QuerySession) -> Vec<Vec<Option<f64>>> {
        let Some(RunResults::Points(results)) = session.results() else {
            panic!("a points source produces points results");
        };
        results
            .queries
            .iter()
            .flat_map(|query| &query.matches)
            .flat_map(|track| &track.matches)
            .map(|matched| matched.aggregates.clone())
            .collect()
    }

    /// The match covers point 0 alone: it moves at 36 km/h, point 1 does not.
    /// The aggregate reduces the one accel sample recorded in that second.
    #[test]
    fn a_points_table_values_an_aggregate_column_over_each_match() {
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.5, 0.0, 0.0]), (1, [0.2, 0.0, 0.0])],
        )]);
        let mut session = QuerySession::new();
        test_util::run_text(
            &mut session,
            &state,
            "points | window 1 | where avg(velocity) > 30 km/h | table time, max(@accel.x)",
        );

        let Some(RunResults::Points(results)) = session.results() else {
            panic!("a points source produces points results");
        };
        let query = results.queries.first().expect("the run has one query");
        assert_eq!(
            query
                .columns
                .iter()
                .map(TableColumn::label)
                .collect::<Vec<_>>(),
            ["time", "max(@accel.x)"]
        );
        assert_eq!(
            points_aggregates(&session),
            [vec![Some(1.5 * gravity_in_base_units())]]
        );
    }

    /// The accel samples start a second after the match.
    #[test]
    fn an_aggregate_over_a_match_without_channel_samples_has_no_value() {
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(1, [1.5, 0.0, 0.0])],
        )]);
        let mut session = QuerySession::new();
        test_util::run_text(
            &mut session,
            &state,
            "points | window 1 | where avg(velocity) > 30 km/h | table time, max(@accel.x)",
        );
        assert_eq!(points_aggregates(&session), [vec![None]]);
    }

    /// The match covers sample 0 alone: it clears 1 g, sample 1 does not.
    #[test]
    fn a_channel_table_values_an_aggregate_column_over_each_match() {
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.5, 0.0, 0.0]), (1, [0.2, 0.0, 0.0])],
        )]);
        let mut session = QuerySession::new();
        test_util::run_text(
            &mut session,
            &state,
            "@accel | window 1 | where max(norm(@accel)) > 1 g | table max(@accel.x)",
        );

        let Some(RunResults::Channel(results)) = session.results() else {
            panic!("a channel source produces channel results");
        };
        assert_eq!(
            results
                .aggregate_columns
                .iter()
                .map(AggregateColumn::label)
                .collect::<Vec<_>>(),
            ["max(@accel.x)"]
        );
        assert_eq!(
            results.tracks[0].matches,
            [MatchValues {
                rows: test_util::rng(0, 1),
                aggregates: vec![Some(1.5 * gravity_in_base_units())],
            }]
        );
    }

    #[test]
    fn channel_source_spans_point_at_the_source_token() {
        let state = LoadedState::with_channels(vec![test_util::vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.0, 0.0, 0.0])],
        )]);
        let mut session = QuerySession::new();
        session.set_text(
            "points | where velocity > 1 km/h\n\n@accel | where norm(@accel) > 1 g".to_owned(),
        );
        session.sync_checks(&state.schema());
        let spans = session.channel_source_spans();
        assert_eq!(spans.len(), 1, "only the channel-source chunk underlines");
        assert_eq!(
            session.text().get(spans[0].start..spans[0].end),
            Some("@accel")
        );
    }
}
