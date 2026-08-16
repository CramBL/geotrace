use gt_query::lexer;
use gt_query::{ChannelSchema, CheckedQuery, Span};
use gt_ui_types::QueryMatches;

use crate::check::{QueryChunk, check_all};
use crate::fingerprint::{RunFingerprint, RunInputs};
use crate::results::{ChannelResults, PointsResults, RunResults};
use crate::run::{PreparedRun, RunHandle, RunKind, RunOutcome, RunProduct};

/// What [`QuerySession::sync_checks`] found changed since the last check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckRefresh {
    /// Text and schema both unchanged, so the chunks were kept.
    Unchanged,
    /// The channel schema changed - a file load or unload - but the text did
    /// not, so a `@name` error may have resolved on its own.
    SchemaChanged,
    /// The text changed, possibly along with the schema.
    TextChanged,
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
    in_flight: Option<InFlightRun>,
    completed: Option<CompletedRun>,
}

impl Default for QuerySession {
    fn default() -> Self {
        Self::new()
    }
}

impl QuerySession {
    /// An empty session: no text, no results.
    pub fn new() -> Self {
        let text = String::new();
        Self {
            chunks: check_all(&text, &ChannelSchema::new()),
            checked_text: text.clone(),
            checked_schema: ChannelSchema::new(),
            text,
            in_flight: None,
            completed: None,
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
        self.chunks = check_all(&self.text, schema);
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

    /// Whether every query in the editor checks and there is at least one, so
    /// a run is possible.
    pub fn all_ok(&self) -> bool {
        !self.chunks.is_empty() && self.chunks.iter().all(|c| c.result.is_ok())
    }

    /// How a run of the current (checked) queries would dispatch, or why it
    /// cannot run. Only meaningful when [`all_ok`](Self::all_ok).
    pub fn run_kind(&self) -> RunKind {
        let sources = self
            .chunks
            .iter()
            .filter_map(|c| c.result.as_ref().ok())
            .map(CheckedQuery::is_channel_source);
        let (channel_count, total) = sources.fold((0, 0), |(ch, n), is_channel| {
            (ch + usize::from(is_channel), n + 1)
        });
        match channel_count {
            0 => RunKind::Points,
            // A channel source has its own timeline, so it cannot compose with
            // other queries in one pipeline: it must stand alone.
            _ if channel_count == total && total == 1 => RunKind::Channel,
            _ => RunKind::MixedChannel,
        }
    }

    /// The editor-coordinate span of each channel-source chunk's `@name` source
    /// token, for the mixed-channel underline.
    pub fn channel_source_spans(&self) -> Vec<Span> {
        self.chunks
            .iter()
            .filter(|c| c.result.as_ref().is_ok_and(CheckedQuery::is_channel_source))
            .filter_map(|c| {
                let src = self.text.get(c.range.clone())?;
                let first = lexer::tokenize(src).into_iter().next()?;
                Some(Span::new(
                    first.span.start + c.range.start,
                    first.span.end + c.range.start,
                ))
            })
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
        let fingerprint = RunFingerprint::of(inputs);
        let prepared = PreparedRun::new(queries, fingerprint.tracks(), inputs);
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
        let Some(in_flight) = self.in_flight.take() else {
            return;
        };
        let Some((product, track_data)) = outcome.into_product() else {
            return;
        };
        let results = match product {
            RunProduct::Points(pipeline) => {
                RunResults::Points(PointsResults::project(&pipeline, track_data))
            }
            RunProduct::Channel(run) => RunResults::Channel(ChannelResults::project(*run)),
        };
        self.completed = Some(CompletedRun {
            results,
            fingerprint: in_flight.fingerprint,
        });
    }

    /// Forget the run in flight without results, for a driver that lost it.
    pub fn abandon_run(&mut self) {
        self.in_flight = None;
    }

    /// Ask the run in flight to stop. It reports back as cancelled, leaving the
    /// previous results in place.
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

    /// Matches of the last run, for the map. `None` when there was no run.
    pub fn matches(&self) -> Option<&QueryMatches> {
        Some(self.results()?.matches())
    }

    /// Compare the current inputs against the ones the last run saw and mark
    /// its results stale when they differ, so the map and the panel gray out
    /// instead of showing point indices that may address other data.
    pub fn refresh_staleness(&mut self, inputs: RunInputs<'_>) {
        let Some(completed) = &mut self.completed else {
            return;
        };
        let stale = RunFingerprint::of(inputs) != completed.fingerprint;
        completed.results.set_stale(stale);
    }

    /// Drop the last run's results so the map returns to normal, abandoning any
    /// run still in flight.
    pub fn clear_results(&mut self) {
        self.completed = None;
        self.in_flight = None;
    }
}

#[cfg(test)]
mod tests {
    use gt_filter::GlobalFilter;
    use gt_loaded_files::{FileHistory, LoadedFiles};
    use gt_types::{FileIdx, TrackIdx, TrackRef};
    use gt_ui_types::TrackDataVisibility;
    use rstest::rstest;

    use super::*;
    use crate::fingerprint::{JammingValues, SnapErrorValues};
    use crate::schema::schema_from_files;
    use crate::test_fixtures::{file_with_channels, rng, scalar_channel, vector_channel};

    /// The loaded state a session runs against, owned so the borrowed
    /// [`RunInputs`] can be rebuilt per call.
    struct LoadedState {
        files: LoadedFiles,
        visibility: TrackDataVisibility,
        filter: GlobalFilter,
        snap_errors: SnapErrorValues,
        jamming: JammingValues,
    }

    impl LoadedState {
        fn with_channels(channels: Vec<gt_types::Channel>) -> Self {
            let mut files = LoadedFiles::new();
            files.push(file_with_channels(channels), FileHistory::None);
            let visibility = TrackDataVisibility::from_loaded(files.files());
            Self {
                files,
                visibility,
                filter: GlobalFilter::default(),
                snap_errors: SnapErrorValues::default(),
                jamming: JammingValues::default(),
            }
        }

        fn inputs(&self) -> RunInputs<'_> {
            RunInputs {
                loaded_files: self.files.view(),
                visibility: &self.visibility,
                filter: &self.filter,
                snap_errors: &self.snap_errors,
                jamming: &self.jamming,
            }
        }

        fn schema(&self) -> ChannelSchema {
            schema_from_files(self.files.files())
        }
    }

    /// Drive one run of `text` to completion, the way a headless caller does.
    fn run_text(session: &mut QuerySession, state: &LoadedState, text: &str) {
        session.set_text(text.to_owned());
        session.sync_checks(&state.schema());
        let prepared = session
            .start_run(state.inputs())
            .expect("the query checks and nothing is in flight");
        assert!(
            session.run_in_flight(),
            "the run is in flight until it completes"
        );
        session.finish_run(prepared.execute());
        assert!(!session.run_in_flight(), "the run completed");
    }

    #[test]
    fn a_session_runs_holds_clears_and_runs_again() {
        let state = LoadedState::with_channels(vec![]);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut session = QuerySession::new();
        assert!(session.matches().is_none(), "no run, no matches");

        // The first point moves at 36 km/h, the second carries no velocity.
        run_text(
            &mut session,
            &state,
            "points | where velocity > 30 km/h | draw",
        );
        let matches = session.matches().expect("a completed run has matches");
        assert_eq!(matches.draws.len(), 1);
        assert_eq!(matches.draws[0].ranges_for(track), [rng(0, 1)]);
        assert!(!matches.stale, "a fresh run is not stale");

        session.clear_results();
        assert!(session.matches().is_none(), "clearing drops the results");

        // A second run over the same session: the hide mode takes the map
        // effect the other way round.
        run_text(
            &mut session,
            &state,
            "points | where velocity > 30 km/h | hide",
        );
        let matches = session.matches().expect("the second run has matches");
        assert!(matches.draws.is_empty(), "hide draws nothing");
        assert_eq!(matches.hidden_ranges(track), [rng(0, 1)]);
    }

    #[test]
    fn results_go_stale_when_the_inputs_change() {
        let mut state = LoadedState::with_channels(vec![]);
        let mut session = QuerySession::new();
        run_text(
            &mut session,
            &state,
            "points | where velocity > 30 km/h | draw",
        );

        session.refresh_staleness(state.inputs());
        assert!(
            !session.results().expect("results").stale(),
            "unchanged inputs keep the results fresh"
        );

        state.snap_errors.insert(
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            std::sync::Arc::new(vec![Some(1.0), None]),
        );
        session.refresh_staleness(state.inputs());
        assert!(
            session.results().expect("results").stale(),
            "a snap run the results never saw grays them out"
        );
    }

    #[test]
    fn a_cancelled_run_keeps_the_previous_results() {
        let state = LoadedState::with_channels(vec![]);
        let mut session = QuerySession::new();
        run_text(
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
        let state =
            LoadedState::with_channels(vec![scalar_channel("accel", Some("g"), &[(0, 1.0)])]);
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
        let state = LoadedState::with_channels(vec![vector_channel(
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
        let state = LoadedState::with_channels(vec![vector_channel(
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
        // Sample 0 (1.5 g -> 14.7 m/s2) clears 1 g; sample 1 (0.2 g -> 1.96)
        // does not. The matched sample (at t=0 s) bands the track: a draw halo
        // over the enclosing nav-point range.
        let state = LoadedState::with_channels(vec![vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.5, 0.0, 0.0]), (1, [0.2, 0.0, 0.0])],
        )]);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut session = QuerySession::new();
        run_text(&mut session, &state, "@accel | where norm(@accel) > 1 g");

        let Some(RunResults::Channel(results)) = session.results() else {
            panic!("a channel source produces channel results");
        };
        assert_eq!(results.channel, "accel");
        assert_eq!(results.components, ["x", "y", "z"]);
        assert_eq!(results.tracks.len(), 1);
        assert_eq!(results.tracks[0].ranges, [rng(0, 1)]);
        assert_eq!(results.tracks[0].timeline.times.len(), 2);
        assert_eq!(results.matches.draws.len(), 1);
        assert_eq!(results.matches.draws[0].ranges_for(track), [rng(0, 1)]);
    }

    #[test]
    fn channel_source_spans_point_at_the_source_token() {
        let state = LoadedState::with_channels(vec![vector_channel(
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
