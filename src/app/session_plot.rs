use std::ops::Range;

use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingData;
use gt_plot::PlotViewRequest;
use gt_query::QueryMetric;
use gt_query_run::MatchExtent;
use gt_session_bridge::{ErrorCode, QueryJobId, RangeTarget, ShowOnPlot};
use gt_types::{Generation, MetricKind, TrackRef};
use uuid::Uuid;

use super::pane_presentation::{PanePresentation, PresentationDetail, PresentationQueryMatch};
use super::query_run::BackgroundQuery;
use super::{App, SharedAppState};

pub(super) struct PlotPresentation {
    pub(super) id: Uuid,
    pub(super) view: PlotViewRequest,
    pub(super) target: PanePresentation,
    generation: Generation<RecordingData>,
    filter: GlobalFilter,
    job: Option<QueryJobId>,
}

impl PlotPresentation {
    fn new(
        shared: &SharedAppState,
        target: PanePresentation,
        job: Option<QueryJobId>,
        prepare_view: impl FnOnce() -> Option<PlotViewRequest>,
    ) -> Result<Self, ErrorCode> {
        if !shared.plot_state.is_track_visible(
            shared.loaded_files.files(),
            shared.tree.visibility(),
            &shared.filter,
            target.track,
        ) {
            return Err(ErrorCode::NoVisiblePlotTime);
        }
        let view = prepare_view().ok_or(ErrorCode::NoVisiblePlotTime)?;
        Ok(Self {
            id: Uuid::new_v4(),
            view,
            target,
            generation: shared.loaded_files.data_generation(),
            filter: shared.filter,
            job,
        })
    }

    fn points(
        shared: &SharedAppState,
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
        let times = loaded
            .points
            .get(points)
            .ok_or(ErrorCode::InvalidRange)?
            .iter()
            .map(|point| point.tpv.time())
            .filter(|time| shared.filter.contains_time(time.utc()))
            .map(|time| time.as_secs_f64_with_subseconds());
        Self::new(
            shared,
            PanePresentation { track, detail },
            query_match.map(|source| source.job),
            || PlotViewRequest::new(times, metrics, None),
        )
    }

    pub(super) fn is_current(&self, shared: &SharedAppState, queries: &BackgroundQuery) -> bool {
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
                let (track, points) =
                    track.resolve_range(shared.loaded_files.view(), session, start..end)?;
                PlotPresentation::points(&shared, track, points, metrics, None)
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
                        PlotPresentation::new(&shared, target, Some(job), || {
                            PlotViewRequest::new(
                                times.iter().copied(),
                                metrics,
                                Some((*name).to_owned()),
                            )
                        })
                    }
                }
            }
        }
    }
}

impl SharedAppState {
    pub(super) fn discard_stale_session_plot(&mut self, queries: &BackgroundQuery) {
        let stale = |presentation: &PlotPresentation| !presentation.is_current(self, queries);
        let pending_stale = self.pending_session_plot.as_ref().is_some_and(stale);
        let presented_stale = self.presented_session_plot.as_ref().is_some_and(stale);
        if pending_stale {
            self.pending_session_plot = None;
        }
        if presented_stale {
            self.presented_session_plot = None;
        }
    }
}
