use std::iter;
use std::ops::Range;
use std::time::{Duration, Instant};

use egui_notify::Toasts;
use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingData;
use gt_plot::PlotViewRequest;
use gt_query::QueryMetric;
use gt_query_run::MatchExtent;
use gt_session_bridge::{
    ErrorCode, QueryJobId, RangeTarget, Reply, SessionReply, ShowOnPlot, TimeRange, TrackReference,
};
use gt_types::{Generation, GpsTime, MetricKind, TimeRange as LoadedTimeRange, TrackRef};
use uuid::Uuid;

use super::mcp_action_notice::McpActionNotice;
use super::pane_presentation::{PanePresentation, PresentationDetail, PresentationQueryMatch};
use super::query_run::BackgroundQuery;
use super::{App, SharedAppState};

pub(super) struct PlotPresentation {
    pub(super) id: Uuid,
    pub(super) view: PlotViewRequest,
    pub(super) target: PanePresentation,
    track: TrackReference,
    time_range: TimeRange,
    generation: Generation<RecordingData>,
    filter: GlobalFilter,
    job: Option<QueryJobId>,
}

pub(super) struct QueuedPlotPresentation {
    presentation: PlotPresentation,
    reply: SessionReply,
    expires: Instant,
}

impl QueuedPlotPresentation {
    pub(super) fn waiting_view(&self) -> Option<&PlotViewRequest> {
        (self.reply.client_is_waiting() && Instant::now() < self.expires)
            .then_some(&self.presentation.view)
    }

    #[cfg(test)]
    pub(super) fn expire(&mut self) {
        self.expires = Instant::now();
    }
}

impl PlotPresentation {
    fn new(
        shared: &SharedAppState,
        session: Uuid,
        target: PanePresentation,
        job: Option<QueryJobId>,
        prepare_view: impl FnOnce() -> Result<(PlotViewRequest, TimeRange), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        let track = shared
            .loaded_files
            .view()
            .session_track_ref(target.track)
            .ok_or(ErrorCode::StaleTrack)?;
        if !shared.plot_state.is_track_visible(
            shared.loaded_files.files(),
            shared.tree.visibility(),
            &shared.filter,
            target.track,
        ) {
            return Err(ErrorCode::NoVisiblePlotTime);
        }
        let (view, time_range) = prepare_view()?;
        Ok(Self {
            id: Uuid::new_v4(),
            view,
            target,
            track: TrackReference::new(session, track),
            time_range,
            generation: shared.loaded_files.data_generation(),
            filter: shared.filter,
            job,
        })
    }

    fn points(
        shared: &SharedAppState,
        session: Uuid,
        track: TrackRef,
        points: Range<usize>,
        metrics: Vec<MetricKind>,
        query_match: Option<PresentationQueryMatch>,
    ) -> Result<Self, ErrorCode> {
        let loaded = track
            .resolve(shared.loaded_files.files())
            .ok_or(ErrorCode::StaleTrack)?;
        let detail = query_match.map_or(
            PresentationDetail::Points {
                start: points.start,
                end: points.end,
            },
            PresentationQueryMatch::detail,
        );
        let mut times = loaded
            .points
            .get(points)
            .ok_or(ErrorCode::InvalidRange)?
            .iter()
            .map(|point| point.tpv.time().utc())
            .filter(|&time| shared.filter.contains_time(time));
        Self::new(
            shared,
            session,
            PanePresentation { track, detail },
            query_match.map(|source| source.job),
            || {
                let first = times.next().ok_or(ErrorCode::NoVisiblePlotTime)?;
                let mut sample_range = LoadedTimeRange::new(first, first);
                let times = iter::once(first).chain(times.inspect(|&at| {
                    sample_range = sample_range.union(LoadedTimeRange::new(at, at));
                }));
                let seconds = times.map(|utc| GpsTime::from_utc(utc).as_secs_f64_with_subseconds());
                let view = PlotViewRequest::new(seconds, metrics, None)
                    .ok_or(ErrorCode::NoVisiblePlotTime)?;
                let time_range = TimeRange::from(sample_range)
                    .rfc3339()
                    .ok_or(ErrorCode::PlotTimeUnrepresentable)?;
                Ok((view, time_range))
            },
        )
    }

    fn is_current(&self, shared: &SharedAppState, queries: &BackgroundQuery) -> bool {
        self.generation == shared.loaded_files.data_generation()
            && self.filter == shared.filter
            && shared.tree.visibility().track_enabled(self.target.track)
            && self
                .job
                .is_none_or(|id| queries.has_current_results(id, shared.loaded_files.view()))
    }
}

impl App {
    pub(super) fn session_show_on_plot(
        &mut self,
        session: Uuid,
        ShowOnPlot { target, metrics }: ShowOnPlot,
    ) -> Result<PlotPresentation, ErrorCode> {
        let metrics = metrics
            .iter()
            .map(|name| {
                name.parse::<QueryMetric>()
                    .ok()
                    .and_then(QueryMetric::metric_kind)
                    .ok_or(ErrorCode::InvalidPlotMetric)
            })
            .collect::<Result<Vec<_>, _>>()?;
        match target {
            RangeTarget::Track { track, start, end } => {
                let shared = self.shared.borrow();
                let (resolved, points) =
                    track.resolve_range(shared.loaded_files.view(), session, start..end)?;
                PlotPresentation::points(&shared, session, resolved, points, metrics, None)
            }
            RangeTarget::QueryMatch {
                job,
                query,
                matched: position,
            } => {
                self.require_completed_query(session, job)?;
                let shared = self.shared.borrow();
                let results = self
                    .editor_query
                    .session()
                    .retained_results(shared.loaded_files.view())?;
                let matched = results.query(query)?.match_at(position.into())?;
                let source = PresentationQueryMatch {
                    job,
                    query,
                    match_ordinal: position.match_ordinal,
                };
                match matched.extent() {
                    MatchExtent::Points(points) => PlotPresentation::points(
                        &shared,
                        session,
                        matched.track(),
                        points.clone(),
                        metrics,
                        Some(source),
                    ),
                    MatchExtent::Channel { name, times } => {
                        let target = PanePresentation {
                            track: matched.track(),
                            detail: source.detail(),
                        };
                        PlotPresentation::new(&shared, session, target, Some(job), || {
                            let view = PlotViewRequest::new(
                                times.iter().copied(),
                                metrics,
                                Some((*name).to_owned()),
                            )
                            .ok_or(ErrorCode::NoVisiblePlotTime)?;
                            let time_range = TimeRange::from_unix_seconds(view.extent())
                                .ok_or(ErrorCode::PlotTimeUnrepresentable)?;
                            Ok((view, time_range))
                        })
                    }
                }
            }
        }
    }

    pub(super) fn queue_session_plot(
        &mut self,
        presentation: PlotPresentation,
        reply: SessionReply,
    ) {
        if !reply.client_is_waiting() {
            return;
        }
        let mut shared = self.shared.borrow_mut();
        shared.poll_session_plot(&self.ctx, &self.editor_query);
        shared.reject_pending_session_plot(ErrorCode::PlotPresentationSuperseded);
        shared.pending_session_plot = Some(QueuedPlotPresentation {
            presentation,
            reply,
            expires: Instant::now() + PLOT_PRESENTATION_WAIT,
        });
        self.tiles_tree.tiles.set_visible(self.plot_tile_id, true);
        self.ctx.request_repaint();
        self.ctx.request_repaint_after(PLOT_PRESENTATION_WAIT);
    }
}

impl SharedAppState {
    pub(super) fn poll_session_plot(&mut self, ctx: &egui::Context, queries: &BackgroundQuery) {
        if let Some(pending) = self.pending_session_plot.take()
            && pending.reply.client_is_waiting()
        {
            if Instant::now() >= pending.expires {
                pending.reply.send(ErrorCode::PlotPresentationTimedOut);
            } else if !pending.presentation.is_current(self, queries) {
                pending.reply.send(ErrorCode::PlotPresentationStale);
            } else {
                ctx.request_repaint_after(
                    pending.expires.saturating_duration_since(Instant::now()),
                );
                self.pending_session_plot = Some(pending);
            }
        }
        if self
            .presented_session_plot
            .as_ref()
            .is_some_and(|active| !active.is_current(self, queries))
        {
            self.presented_session_plot = None;
        }
    }

    pub(super) fn reject_pending_session_plot(&mut self, error: ErrorCode) {
        if let Some(pending) = self.pending_session_plot.take() {
            pending.reply.send(error);
        }
    }

    pub(super) fn confirm_session_plot(&mut self, toasts: &mut Toasts) {
        if let Some(pending) = self.pending_session_plot.take() {
            let presentation = pending.presentation;
            let reply = Reply::PlotFramed {
                track: presentation.track,
                time_range: presentation.time_range,
            };
            self.presented_session_plot = Some(presentation);
            McpActionNotice::FramePlot.enqueue(toasts);
            pending.reply.send(reply);
        }
    }
}

const PLOT_PRESENTATION_WAIT: Duration = Duration::from_millis(500);
