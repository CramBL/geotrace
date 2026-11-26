use std::ops::Range;

use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingData;
use gt_plot::PlotViewRequest;
use gt_query::QueryMetric;
use gt_query_run::MatchExtent;
use gt_session_bridge::{ErrorCode, QueryJobId, RangeTarget, ShowOnPlot};
use gt_types::{Generation, MetricKind, TrackRef};
use uuid::Uuid;

use super::query_run::BackgroundQuery;
use super::{App, SharedAppState};

pub(super) struct PlotPresentation {
    pub(super) view: PlotViewRequest,
    generation: Generation<RecordingData>,
    filter: GlobalFilter,
    track: TrackRef,
    job: Option<QueryJobId>,
}

impl PlotPresentation {
    fn new(
        shared: &SharedAppState,
        track: TrackRef,
        job: Option<QueryJobId>,
        prepare_view: impl FnOnce() -> Option<PlotViewRequest>,
    ) -> Result<Self, ErrorCode> {
        if !shared.plot_state.is_track_visible(
            shared.loaded_files.files(),
            shared.tree.visibility(),
            &shared.filter,
            track,
        ) {
            return Err(ErrorCode::NoVisiblePlotTime);
        }
        let view = prepare_view().ok_or(ErrorCode::NoVisiblePlotTime)?;
        Ok(Self {
            view,
            generation: shared.loaded_files.data_generation(),
            filter: shared.filter,
            track,
            job,
        })
    }

    fn points(
        shared: &SharedAppState,
        track: TrackRef,
        points: Range<usize>,
        metrics: Vec<MetricKind>,
        job: Option<QueryJobId>,
    ) -> Result<Self, ErrorCode> {
        let loaded = track
            .resolve(shared.loaded_files.files())
            .ok_or(ErrorCode::StaleTrack)?;
        let points = loaded.points.get(points).ok_or(ErrorCode::InvalidRange)?;
        let times = points
            .iter()
            .map(|point| point.tpv.time())
            .filter(|time| shared.filter.contains_time(time.utc()))
            .map(|time| time.as_secs_f64_with_subseconds());
        Self::new(shared, track, job, || {
            PlotViewRequest::new(times, metrics, None)
        })
    }

    pub(super) fn is_current(&self, shared: &SharedAppState, queries: &BackgroundQuery) -> bool {
        self.generation == shared.loaded_files.data_generation()
            && self.filter == shared.filter
            && shared.tree.visibility().track_enabled(self.track)
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
                matched,
            } => {
                self.require_completed_query(session, job)?;
                let shared = self.shared.borrow();
                let results = self
                    .editor_query
                    .session()
                    .retained_results(shared.loaded_files.view())?;
                let matched = results.query(query)?.match_at(matched.into())?;
                match matched.extent() {
                    MatchExtent::Points(points) => PlotPresentation::points(
                        &shared,
                        matched.track(),
                        points.clone(),
                        metrics,
                        Some(job),
                    ),
                    MatchExtent::Channel { name, times } => {
                        PlotPresentation::new(&shared, matched.track(), Some(job), || {
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
