use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gt_query::{
    AggregateColumn, CheckedQuery, DrawContribution, Params, PipelineOutput, QueryOutput,
    RunSummary, TableColumn, TrackInput, TrackMatches,
};
use gt_types::{Channel, DisplayMode, LoadedFile, NavPoint, TrackRef};
use gt_ui_types::QueryMatches;
use rustc_hash::FxHashMap;

use crate::fingerprint::RunInputs;
use crate::provider::{CapturedTrackValues, SliceProvider, TrackProvider, TrackQueryData};
use crate::results::{
    ChannelTrackResult, MatchValues, TrackMatchValues, channel_query_matches, matched_point_ranges,
};

/// Per-track derived series of one run, keyed by the track they came from.
pub(crate) type RunTrackData = FxHashMap<TrackRef, TrackQueryData>;

/// How a run dispatches, determined from the checked queries' sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Every query is points-source: run the composing pipeline.
    Points,
    /// A single channel-source query: run it standalone over its samples.
    Channel,
    /// A channel source mixed with other queries - not allowed, since a channel
    /// has its own timeline and cannot compose in one pipeline.
    MixedChannel,
}

/// The handle shared between a run and its driver: the driver's cancel flag and
/// the run's progress counter.
///
/// [`QuerySession`](crate::QuerySession) keeps one so the UI can cancel and
/// report progress while the run evaluates elsewhere.
#[derive(Clone, Default)]
pub struct RunHandle {
    cancel: Arc<AtomicBool>,
    tracks_prepared: Arc<AtomicUsize>,
}

impl RunHandle {
    /// Request that the run stop. It finishes with a cancelled [`RunOutcome`],
    /// which leaves the previous results in place.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation was requested.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Tracks whose derived series are ready, for the progress line.
    pub fn tracks_prepared(&self) -> usize {
        self.tracks_prepared.load(Ordering::Relaxed)
    }

    fn note_track_prepared(&self) {
        self.tracks_prepared.fetch_add(1, Ordering::Relaxed);
    }
}

/// One track's owned data for a run: the full point vector and channels, plus
/// the sub-range of points passing the time filter.
struct TrackSnapshot {
    track_ref: TrackRef,
    points: Vec<NavPoint>,
    channels: Vec<Channel>,
    slice: Range<usize>,
    /// The series captured when the run was prepared - queries stay
    /// synchronous over already-computed data and never trigger an upload.
    captured: CapturedTrackValues,
}

impl TrackSnapshot {
    /// Snapshot each of `tracks` from the loaded files.
    ///
    /// Cloning the points is the simple-and-correct baseline. An `Arc`-based
    /// snapshot is the known follow-up if this shows up in profiling.
    fn collect(tracks: &[TrackRef], inputs: RunInputs<'_>) -> Vec<Self> {
        let RunInputs {
            loaded_files,
            filter,
            snap_errors,
            jamming,
            geomagnetic,
            tec,
            ..
        } = inputs;
        let files: &[LoadedFile] = loaded_files.files();
        tracks
            .iter()
            .filter_map(|&track_ref| {
                let track = track_ref.resolve(files)?;
                let slice = gt_filter::time_filtered_range(&track.points, filter);
                Some(Self {
                    track_ref,
                    points: track.points.clone(),
                    channels: track.channels.clone(),
                    slice,
                    captured: CapturedTrackValues {
                        snap_error: snap_errors.get(&track_ref).cloned(),
                        jamming: jamming.get(&track_ref).cloned(),
                        geomagnetic: geomagnetic.points_by_track.get(&track_ref).cloned(),
                        tec: tec.points_by_track.get(&track_ref).cloned(),
                    },
                })
            })
            .collect()
    }

    fn slice_provider<'a>(&'a self, data: Option<&'a TrackQueryData>) -> SliceProvider<'a> {
        SliceProvider::new(
            TrackProvider::new(&self.points, &self.channels, data),
            self.slice.start,
            self.slice.len(),
        )
    }
}

/// One run of the editor's checked queries over an owned snapshot of the
/// tracks it evaluates.
///
/// Prepared by [`QuerySession::start_run`](crate::QuerySession::start_run) and
/// evaluated by [`execute`](Self::execute), which is synchronous and free of
/// threads: the app runs it on a worker thread, a test runs it inline.
pub struct PreparedRun {
    queries: Vec<CheckedQuery>,
    tracks: Vec<TrackSnapshot>,
    handle: RunHandle,
}

impl PreparedRun {
    pub(crate) fn new(
        queries: Vec<CheckedQuery>,
        tracks: &[TrackRef],
        inputs: RunInputs<'_>,
    ) -> Self {
        Self {
            queries,
            tracks: TrackSnapshot::collect(tracks, inputs),
            handle: RunHandle::default(),
        }
    }

    /// The handle for cancelling this run and watching its progress.
    pub fn handle(&self) -> RunHandle {
        self.handle.clone()
    }

    /// How many tracks the run evaluates, the denominator of its progress.
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Evaluate the run: derived series per track, then the sequential pipeline
    /// evaluation, with cancellation checks between tracks (gt-query checks
    /// within them).
    pub fn execute(&self) -> RunOutcome {
        let cancelled = || self.handle.cancelled();
        // A channel-source query runs standalone (the session gates a mix), on
        // its own sample timeline.
        if let Some(query) = self.queries.first().filter(|q| q.is_channel_source()) {
            return self.execute_channel(query, &cancelled);
        }
        let uses_util = self
            .queries
            .iter()
            .any(|q| q.referenced_metrics().iter().any(|m| m.is_util()));
        let uses_slip = self
            .queries
            .iter()
            .any(|q| q.referenced_metrics().iter().any(|m| m.is_slip()));
        let params = merge_params(&self.queries);

        let mut track_data = RunTrackData::default();
        for snapshot in &self.tracks {
            if cancelled() {
                return RunOutcome::cancelled();
            }
            track_data.insert(
                snapshot.track_ref,
                TrackQueryData::derive(
                    &snapshot.points,
                    params,
                    uses_util,
                    uses_slip,
                    snapshot.slice.start,
                    snapshot.captured.clone(),
                ),
            );
            self.handle.note_track_prepared();
        }

        let providers: Vec<(TrackRef, SliceProvider<'_>)> = self
            .tracks
            .iter()
            .map(|snapshot| {
                (
                    snapshot.track_ref,
                    snapshot.slice_provider(track_data.get(&snapshot.track_ref)),
                )
            })
            .collect();
        let inputs = track_inputs(&providers);

        match gt_query::run_pipeline(&self.queries, &inputs, &cancelled) {
            Some(pipeline) => RunOutcome::completed(
                RunProduct::Points(PointsRun::with_aggregates_valued(pipeline, &providers)),
                track_data,
            ),
            None => RunOutcome::cancelled(),
        }
    }

    /// Evaluate one channel-source `query` over each track's sample timeline.
    /// Nav metrics are rejected on a channel source, so no derived series are
    /// needed.
    fn execute_channel(&self, query: &CheckedQuery, cancelled: &impl Fn() -> bool) -> RunOutcome {
        let providers: Vec<(TrackRef, SliceProvider<'_>)> = self
            .tracks
            .iter()
            .map(|snapshot| {
                self.handle.note_track_prepared();
                (snapshot.track_ref, snapshot.slice_provider(None))
            })
            .collect();
        let inputs = track_inputs(&providers);

        let Some(output) = gt_query::run_cancellable(query, &inputs, cancelled) else {
            return RunOutcome::cancelled();
        };
        // The source channel is present (the query checked as a channel source).
        let name = query.source_channel().unwrap_or_default();
        // Component labels for the table headers, from any track carrying the
        // channel. Components are structural (the channel's shape), not per-track
        // content, so the first compatible definition is sufficient. Incompatible
        // definitions are rejected while checking the query.
        let components = self
            .tracks
            .iter()
            .flat_map(|s| &s.channels)
            .find(|c| c.name == name)
            .map(|c| c.components.clone())
            .unwrap_or_default();
        // Pair each track's matched sample ranges with its timeline, for the table.
        let track_results: Vec<ChannelTrackResult> = output
            .matches
            .iter()
            .filter_map(|tm| {
                let provider = providers.iter().find(|(tr, _)| *tr == tm.track)?;
                Some(ChannelTrackResult {
                    track: tm.track,
                    matches: matches_with_aggregates_valued(
                        &tm.ranges,
                        &output.columns,
                        &provider.1,
                    ),
                    timeline: gt_query::MetricProvider::channel_timeline(&provider.1, name),
                    unit: self
                        .tracks
                        .iter()
                        .find(|snapshot| snapshot.track_ref == tm.track)
                        .and_then(|snapshot| snapshot.channels.iter().find(|c| c.name == name))
                        .and_then(|channel| channel.unit.clone()),
                })
            })
            .collect();

        // Project each track's matched sample spans onto its nav points, for the
        // map halos: a matched span bands the track segments it covers.
        let per_track: FxHashMap<TrackRef, (Vec<Range<usize>>, usize)> = track_results
            .iter()
            .filter_map(|result| {
                let snapshot = self.tracks.iter().find(|s| s.track_ref == result.track)?;
                let point_ranges =
                    matched_point_ranges(&snapshot.points, &result.timeline, &result.matches);
                let len = snapshot.points.len();
                (!point_ranges.is_empty()).then_some((result.track, (point_ranges, len)))
            })
            .collect();
        let matches = channel_query_matches(query.mode(), &per_track);

        RunOutcome::completed(
            RunProduct::Channel(Box::new(ChannelRun {
                channel: name.to_owned(),
                components,
                aggregate_columns: gt_query::aggregate_columns(&output.columns)
                    .cloned()
                    .collect(),
                summary: output.summary,
                tracks: track_results,
                matches,
            })),
            RunTrackData::default(),
        )
    }
}

fn track_inputs<'a>(
    providers: &'a [(TrackRef, SliceProvider<'a>)],
) -> Vec<TrackInput<'a, SliceProvider<'a>>> {
    providers
        .iter()
        .map(|(track_ref, provider)| TrackInput {
            track: *track_ref,
            provider,
        })
        .collect()
}

/// Merge the `with` parameters of every query, taking the first value set for
/// each. The derived util/slip series are computed once per track, so a later
/// query that declares a different mask reuses the first (a rare conflict).
fn merge_params(queries: &[CheckedQuery]) -> Params {
    let mut merged = Params::default();
    for query in queries {
        let params = query.params();
        merged.mask_deg = merged.mask_deg.or(params.mask_deg);
        merged.snr_drop_db_hz = merged.snr_drop_db_hz.or(params.snr_drop_db_hz);
        merged.slip_window_s = merged.slip_window_s.or(params.slip_window_s);
    }
    merged
}

/// What a [`PreparedRun`] produced, on its way back to
/// [`QuerySession::finish_run`](crate::QuerySession::finish_run).
///
/// Opaque: the session turns it into [`RunResults`](crate::RunResults). A
/// cancelled run carries no partial output - the previous results stand.
pub struct RunOutcome(Evaluation);

enum Evaluation {
    Completed {
        product: RunProduct,
        track_data: RunTrackData,
    },
    Cancelled,
}

impl RunOutcome {
    fn completed(product: RunProduct, track_data: RunTrackData) -> Self {
        Self(Evaluation::Completed {
            product,
            track_data,
        })
    }

    fn cancelled() -> Self {
        Self(Evaluation::Cancelled)
    }

    /// The evaluated product, or `None` when the run was cancelled.
    pub(crate) fn into_product(self) -> Option<(RunProduct, RunTrackData)> {
        match self.0 {
            Evaluation::Completed {
                product,
                track_data,
            } => Some((product, track_data)),
            Evaluation::Cancelled => None,
        }
    }
}

/// A run's product, dispatched on the source of its queries.
pub(crate) enum RunProduct {
    /// A composed points pipeline.
    Points(PointsRun),
    /// A standalone channel-source run. Boxed: several times the size of the
    /// pipeline variant.
    Channel(Box<ChannelRun>),
}

/// A points pipeline's output with every query's aggregate table columns
/// valued over each of its matches, still at the evaluator's own point indices.
pub(crate) struct PointsRun {
    pub(crate) queries: Vec<PointsQueryRun>,
    pub(crate) hidden: Vec<TrackMatches>,
    pub(crate) draws: Vec<DrawContribution>,
}

impl PointsRun {
    fn with_aggregates_valued(
        PipelineOutput {
            queries,
            hidden,
            draws,
        }: PipelineOutput,
        providers: &[(TrackRef, SliceProvider<'_>)],
    ) -> Self {
        Self {
            queries: queries
                .into_iter()
                .map(|query| PointsQueryRun::with_aggregates_valued(query, providers))
                .collect(),
            hidden,
            draws,
        }
    }
}

/// One query of a points pipeline: what it counted, how it changes the map, the
/// columns its match table lists, and its matches with their aggregate values.
pub(crate) struct PointsQueryRun {
    pub(crate) mode: DisplayMode,
    pub(crate) columns: Vec<TableColumn>,
    pub(crate) summary: RunSummary,
    pub(crate) matches: Vec<TrackMatchValues>,
}

impl PointsQueryRun {
    fn with_aggregates_valued(
        QueryOutput {
            mode,
            matches,
            columns,
            summary,
        }: QueryOutput,
        providers: &[(TrackRef, SliceProvider<'_>)],
    ) -> Self {
        Self {
            matches: matches
                .iter()
                .filter_map(|track_matches| {
                    let provider = providers
                        .iter()
                        .find(|(track, _)| *track == track_matches.track)?;
                    Some(TrackMatchValues {
                        track: track_matches.track,
                        matches: matches_with_aggregates_valued(
                            &track_matches.ranges,
                            &columns,
                            &provider.1,
                        ),
                    })
                })
                .collect(),
            mode,
            columns,
            summary,
        }
    }
}

/// Each of `ranges` as a match, with every aggregate column of `columns`
/// reduced over it.
fn matches_with_aggregates_valued(
    ranges: &[Range<usize>],
    columns: &[TableColumn],
    provider: &SliceProvider<'_>,
) -> Vec<MatchValues> {
    ranges
        .iter()
        .map(|rows| MatchValues {
            rows: rows.clone(),
            aggregates: gt_query::aggregate_columns(columns)
                .map(|column| column.value_over_match(provider, rows.clone()))
                .collect(),
        })
        .collect()
}

/// A channel-source run's raw output, before the panel projection.
pub(crate) struct ChannelRun {
    pub(crate) channel: String,
    /// Component labels for a vector channel (`["x","y","z"]`), empty for a
    /// scalar. Column headers for the sample table.
    pub(crate) components: Vec<String>,
    /// The query's aggregate `table` columns, in table order.
    pub(crate) aggregate_columns: Vec<AggregateColumn>,
    pub(crate) summary: RunSummary,
    pub(crate) tracks: Vec<ChannelTrackResult>,
    /// The map effect: matched sample spans projected onto the track as
    /// enclosing nav-point ranges, honoring the query's draw/keep/hide mode.
    pub(crate) matches: QueryMatches,
}
