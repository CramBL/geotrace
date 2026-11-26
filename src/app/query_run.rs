use std::sync::mpsc::{self, Receiver, TryRecvError};

use gt_filter::GlobalFilter;
use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_query_run::{
    CheckedQueryText, JammingValues, QuerySession, RunCompletion, RunDependencies, RunInputs,
    RunOutcome, SnapErrorValues,
};
use gt_session_bridge::{ErrorCode, QueryJobId};
use gt_side_panel::TreeState;
use gt_types::Generation;
use gt_ui_types::{GeomagneticSeries, JammingSeries, TecSeries, TrackDataVisibility};
use uuid::Uuid;

use super::snap_state::SnapErrorCache;
use super::track_day_values::ArchivedTrackValues;
use super::{App, background_thread};

#[derive(Default)]
pub(super) struct BackgroundQuery {
    session: QuerySession,
    operation: Option<QueryOperation>,
    worker: Option<Receiver<RunOutcome>>,
    last_freshness_check: Option<QueryInputsRevision>,
}

#[derive(Clone, Copy)]
struct QueryOperation {
    id: Uuid,
    scope: Option<Uuid>,
    phase: QueryPhase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum QueryPhase {
    Cancelled,
    Cancelling,
    Completed,
    Failed,
    Preparing(Generation<RecordingData>),
    Running,
    Stale,
}

impl QueryPhase {
    fn is_active(self) -> bool {
        matches!(self, Self::Preparing(_) | Self::Running | Self::Cancelling)
    }
}

impl BackgroundQuery {
    pub(super) fn active_job(&mut self, scope: Uuid) -> Result<Option<QueryJobId>, ErrorCode> {
        let Some(operation) = self
            .operation
            .as_mut()
            .filter(|operation| operation.phase.is_active())
        else {
            return Ok(None);
        };
        if *operation.scope.get_or_insert(scope) != scope {
            return Err(ErrorCode::StaleSession);
        }
        Ok(Some(QueryJobId {
            session: scope,
            id: operation.id,
        }))
    }

    pub(super) fn phase_for_job(
        &self,
        scope: Uuid,
        job: QueryJobId,
    ) -> Result<QueryPhase, ErrorCode> {
        if job.session != scope {
            return Err(ErrorCode::StaleSession);
        }
        self.operation
            .filter(|operation| operation.scope == Some(scope) && operation.id == job.id)
            .map(|operation| operation.phase)
            .ok_or(ErrorCode::QueryJobNotFound)
    }

    pub(super) fn has_current_results(&self, job: QueryJobId, files: LoadedFilesView<'_>) -> bool {
        self.phase_for_job(job.session, job) == Ok(QueryPhase::Completed)
            && self.session.retained_results(files).is_ok()
    }

    pub(super) fn is_active(&self) -> bool {
        self.operation
            .is_some_and(|operation| operation.phase.is_active())
    }

    pub(super) fn queue(
        &mut self,
        scope: Uuid,
        checked: CheckedQueryText,
        generation: Generation<RecordingData>,
    ) -> QueryJobId {
        self.session.install_checked_text(checked);
        let id = Uuid::new_v4();
        self.operation = Some(QueryOperation {
            id,
            scope: Some(scope),
            phase: QueryPhase::Preparing(generation),
        });
        QueryJobId { session: scope, id }
    }

    pub(super) fn start_editor(&mut self, ctx: &egui::Context, inputs: RunInputs<'_>) -> bool {
        if self.is_active() || !self.start(ctx, inputs) {
            return false;
        }
        self.operation = Some(QueryOperation {
            id: Uuid::new_v4(),
            scope: None,
            phase: QueryPhase::Running,
        });
        true
    }

    fn start(&mut self, ctx: &egui::Context, inputs: RunInputs<'_>) -> bool {
        let Some(prepared) = self.session.start_run(inputs) else {
            return false;
        };
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        background_thread::spawn_or_panic("query-run", move || {
            sender.send(prepared.execute()).ok();
            ctx.request_repaint();
        });
        self.worker = Some(receiver);
        true
    }

    pub(super) fn refresh(&mut self, inputs: CapturedQueryInputs<'_>) {
        if let Some(dependencies) = self.session.result_dependencies()
            && self
                .last_freshness_check
                .is_none_or(|previous| !previous.matches(inputs.revision, dependencies))
        {
            self.session.refresh_staleness(inputs.run);
            self.last_freshness_check = Some(inputs.revision);
        }
    }

    pub(super) fn cancel(&mut self) {
        if let Some(operation) = &mut self.operation {
            operation.phase = match operation.phase {
                QueryPhase::Preparing(_) => QueryPhase::Cancelled,
                QueryPhase::Running => {
                    self.session.cancel_run();
                    QueryPhase::Cancelling
                }
                phase => phase,
            };
        }
    }

    pub(super) fn clear_results(&mut self) {
        self.cancel();
        if self
            .operation
            .is_some_and(|operation| !operation.phase.is_active())
        {
            self.operation = None;
        }
        self.session.clear_completed_results();
        self.last_freshness_check = None;
    }

    fn poll(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let phase = match worker.try_recv() {
            Ok(outcome) => match self.session.finish_run(outcome) {
                RunCompletion::Completed => QueryPhase::Completed,
                RunCompletion::Cancelled => QueryPhase::Cancelled,
                RunCompletion::Unmatched => {
                    log::error!("Query worker returned an outcome for a different run");
                    self.session.abandon_run();
                    QueryPhase::Failed
                }
            },
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                log::error!("Query worker disconnected before sending an outcome");
                self.session.abandon_run();
                QueryPhase::Failed
            }
        };
        self.worker = None;
        if let Some(operation) = &mut self.operation {
            operation.phase = phase;
        }
        if phase == QueryPhase::Completed {
            self.last_freshness_check = None;
        }
        matches!(phase, QueryPhase::Completed | QueryPhase::Cancelled)
    }

    pub(super) fn session(&self) -> &QuerySession {
        &self.session
    }

    pub(super) fn session_mut(&mut self) -> &mut QuerySession {
        &mut self.session
    }
}

impl Drop for BackgroundQuery {
    fn drop(&mut self) {
        self.session.cancel_run();
    }
}

impl App {
    pub(super) fn receive_query_completion(&mut self) {
        if self.editor_query.poll() {
            self.query_window.select_results_tab();
        }
    }

    pub(super) fn prepare_session_query(&mut self, ctx: &egui::Context) {
        let Some(QueryOperation {
            phase: QueryPhase::Preparing(generation),
            ..
        }) = self.editor_query.operation
        else {
            return;
        };
        let phase = if self.shared.borrow().loaded_files.data_generation() == generation {
            let series = QuerySeries::capture_for_query(
                self,
                self.editor_query.session.checked_dependencies(),
            );
            let shared = self.shared.borrow();
            let inputs = series.inputs(shared.loaded_files.view(), &shared.tree, &shared.filter);
            if inputs.is_some_and(|inputs| self.editor_query.start(ctx, inputs.run_inputs())) {
                QueryPhase::Running
            } else {
                QueryPhase::Failed
            }
        } else {
            QueryPhase::Stale
        };
        if let Some(operation) = &mut self.editor_query.operation {
            operation.phase = phase;
        }
    }

    pub(super) fn refresh_query_results(&mut self) -> Result<(), ErrorCode> {
        let dependencies = {
            let shared = self.shared.borrow();
            let session = self.editor_query.session();
            session.displayed_results(shared.loaded_files.view())?;
            session
                .result_dependencies()
                .ok_or(ErrorCode::QueryResultsUnavailable)?
        };
        let series = QuerySeries::capture_for_query(self, dependencies);
        let shared = self.shared.borrow();
        let inputs = series
            .inputs(shared.loaded_files.view(), &shared.tree, &shared.filter)
            .ok_or(ErrorCode::StaleQueryResults)?;
        self.editor_query.refresh(inputs);
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct CapturedQueryInputs<'a> {
    run: RunInputs<'a>,
    revision: QueryInputsRevision,
}

impl<'a> CapturedQueryInputs<'a> {
    pub(super) fn run_inputs(self) -> RunInputs<'a> {
        self.run
    }
}

#[derive(Clone, Copy)]
struct QueryInputsRevision {
    series: QuerySeriesRevision,
    visibility: Generation<TrackDataVisibility>,
    filter: GlobalFilter,
}

impl QueryInputsRevision {
    fn matches(self, current: Self, dependencies: RunDependencies) -> bool {
        self.filter == current.filter
            && self.visibility == current.visibility
            && self.series.data == current.series.data
            && (!dependencies.uses_snap_error()
                || self.series.snap_errors == current.series.snap_errors)
            && (!dependencies.uses_jamming() || self.series.jamming == current.series.jamming)
            && (!dependencies.uses_geomagnetic()
                || self.series.geomagnetic == current.series.geomagnetic)
            && (!dependencies.uses_tec() || self.series.tec == current.series.tec)
    }
}

#[derive(Clone, Copy)]
struct QuerySeriesRevision {
    data: Generation<RecordingData>,
    snap_errors: Generation<SnapErrorCache>,
    jamming: Generation<ArchivedTrackValues>,
    geomagnetic: Generation<ArchivedTrackValues>,
    tec: Generation<ArchivedTrackValues>,
}

pub(super) struct QuerySeries {
    jamming: JammingSeries,
    geomagnetic: GeomagneticSeries,
    tec: TecSeries,
    snap_errors: SnapErrorValues,
    jamming_values: JammingValues,
    revision: QuerySeriesRevision,
}

impl QuerySeries {
    pub(super) fn capture(app: &mut App) -> Self {
        Self::capture_series(app, None)
    }

    pub(super) fn capture_for_query(app: &mut App, dependencies: RunDependencies) -> Self {
        Self::capture_series(app, Some(dependencies))
    }

    fn capture_series(app: &mut App, dependencies: Option<RunDependencies>) -> Self {
        let snap_errors = if dependencies.is_none_or(RunDependencies::uses_snap_error) {
            app.snap_error_values()
        } else {
            SnapErrorValues::default()
        };
        let shared = app.shared.borrow();
        let files = shared.loaded_files.view();
        let (jamming, jamming_values) = if dependencies.is_none_or(RunDependencies::uses_jamming) {
            (app.jamming.plot_series(files), app.jamming.query_values())
        } else {
            (JammingSeries::default(), JammingValues::default())
        };
        let geomagnetic = if dependencies.is_none_or(RunDependencies::uses_geomagnetic) {
            app.geomagnetic_indices.plot_series(files)
        } else {
            GeomagneticSeries::default()
        };
        let tec = if dependencies.is_none_or(RunDependencies::uses_tec) {
            app.tec_maps.plot_series(files)
        } else {
            TecSeries::default()
        };
        Self {
            jamming,
            geomagnetic,
            tec,
            snap_errors,
            jamming_values,
            revision: QuerySeriesRevision {
                data: files.data_generation(),
                snap_errors: app.snap_error_cache.generation(),
                jamming: app.jamming.track_values_generation(),
                geomagnetic: app.geomagnetic_indices.track_values_generation(),
                tec: app.tec_maps.track_values_generation(),
            },
        }
    }

    pub(super) fn jamming(&self) -> &JammingSeries {
        &self.jamming
    }

    pub(super) fn geomagnetic(&self) -> &GeomagneticSeries {
        &self.geomagnetic
    }

    pub(super) fn tec(&self) -> &TecSeries {
        &self.tec
    }

    pub(super) fn inputs<'a>(
        &'a self,
        loaded_files: LoadedFilesView<'a>,
        tree: &'a TreeState,
        filter: &'a GlobalFilter,
    ) -> Option<CapturedQueryInputs<'a>> {
        (self.revision.data == loaded_files.data_generation()).then_some(CapturedQueryInputs {
            run: RunInputs {
                loaded_files,
                visibility: tree.visibility(),
                filter,
                snap_errors: &self.snap_errors,
                jamming: &self.jamming_values,
                geomagnetic: &self.geomagnetic,
                tec: &self.tec,
            },
            revision: QueryInputsRevision {
                series: self.revision,
                visibility: tree.visibility_generation(),
                filter: *filter,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable as _};
    use gt_session_bridge::{ErrorCode, QueryJobState, Request};
    use gt_test_utils::recording_fixtures;
    use rstest::rstest;

    use super::{QueryPhase, QuerySeries};
    use crate::app::test_util::harness::{self, TestDroppedFile};
    use crate::app::test_util::session;

    #[derive(Clone, Copy)]
    enum RunEnd {
        Cancel,
        Clear,
        Disconnect,
    }

    #[rstest]
    #[case::cancel(RunEnd::Cancel, QueryJobState::Cancelled)]
    #[case::clear(RunEnd::Clear, QueryJobState::Cancelled)]
    #[case::disconnect(RunEnd::Disconnect, QueryJobState::Failed)]
    fn a_query_job_without_new_results_cannot_page_the_previous_run(
        #[case] end: RunEnd,
        #[case] status: QueryJobState,
    ) {
        let mut harness = Harness::builder().build_eframe(harness::transient_app);
        harness::drop_file_and_wait_for_load(
            &mut harness,
            TestDroppedFile::bytes(recording_fixtures::minimal_gtd_bytes(), "test.gtd"),
        );
        let previous = session::start_query(&mut harness, "points | draw");
        assert!(matches!(
            session::wait_for_query_to_stop(&mut harness, previous),
            QueryJobState::Completed { .. }
        ));
        let job = session::start_query(&mut harness, "points | where velocity > 999 km/h");
        let app = harness.state_mut();
        let past_matches = app.editor_query.session.matches().unwrap().clone();
        let series = QuerySeries::capture(app);
        let prepared = {
            let shared = app.shared.borrow();
            let inputs = series
                .inputs(shared.loaded_files.view(), &shared.tree, &shared.filter)
                .unwrap();
            app.editor_query
                .session
                .start_run(inputs.run_inputs())
                .unwrap()
        };
        let handle = prepared.handle();
        let outcome = prepared.execute();
        let (send, receive) = mpsc::channel();
        app.editor_query.operation.as_mut().unwrap().phase = QueryPhase::Running;
        app.editor_query.worker = Some(receive);

        match end {
            RunEnd::Cancel => {
                harness.run_steps(2);
                harness
                    .get_by_role_and_label(Role::Button, "Cancel")
                    .click();
                harness.step();
                assert_eq!(
                    session::query_status(&mut harness, job),
                    QueryJobState::Cancelling
                );
                assert!(handle.cancelled());
                send.send(outcome).unwrap();
            }
            RunEnd::Clear => {
                harness.state_mut().editor_query.clear_results();
                assert_eq!(
                    session::query_status(&mut harness, job),
                    QueryJobState::Cancelling
                );
                assert!(handle.cancelled());
                send.send(outcome).unwrap();
            }
            RunEnd::Disconnect => drop(send),
        }
        harness::step_minimized_logic(&mut harness);
        assert_eq!(session::query_status(&mut harness, job), status);
        let shown = harness.state().editor_query.session.matches();
        assert_eq!(
            shown,
            (!matches!(end, RunEnd::Clear)).then_some(&past_matches)
        );
        assert_eq!(
            session::execute(
                &mut harness,
                Request::GetQueryResults(session::first_query_match_request(job))
            ),
            ErrorCode::QueryResultsUnavailable.into()
        );
        assert_eq!(
            session::execute(
                &mut harness,
                Request::GetQueryResults(session::first_query_match_request(previous))
            ),
            ErrorCode::QueryJobNotFound.into()
        );
    }
}
