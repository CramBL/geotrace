use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_query_run::{QuerySession, RunDependencies};
use gt_session_bridge::{
    ApplyProcessing, ErrorCode, Inspection, InspectionRequest, QueryJobId, QueryJobState,
    QueryJobStatus, QueryPreparation, QueryResults, Reply, Request, SessionRequest,
};
use gt_types::Generation;
use uuid::Uuid;

use super::App;
use super::query_run::{BackgroundQuery, CapturedQueryInputs, QuerySeries};

#[derive(Default)]
pub(super) struct SessionQuery {
    job: Option<Job>,
}

impl SessionQuery {
    #[cfg(test)]
    pub(super) fn is_active(&self) -> bool {
        self.active_job().is_some()
    }

    #[cfg(test)]
    pub(super) fn retained_reply(&self) -> Option<Reply> {
        self.job.as_ref().map(Job::reply)
    }

    pub(super) fn refresh(&mut self, inputs: CapturedQueryInputs<'_>) {
        if let Some(job) = &mut self.job
            && job
                .refresh_dependencies(inputs.run_inputs().loaded_files.data_generation())
                .is_some()
        {
            job.query.update(inputs);
        }
    }

    pub(super) fn result_session(
        &mut self,
        app: &mut App,
        session: Uuid,
        id: QueryJobId,
    ) -> Result<&QuerySession, ErrorCode> {
        let job = self.job_mut(session, id)?;
        job.refresh(app);
        job.result_session()
    }

    pub(super) fn has_current_results(&self, id: QueryJobId, files: LoadedFilesView<'_>) -> bool {
        self.job
            .as_ref()
            .filter(|job| job.id == id)
            .and_then(|job| job.result_session().ok())
            .is_some_and(|session| session.retained_results(files).is_ok())
    }

    fn poll(&mut self) {
        let Some(job) = &mut self.job else {
            return;
        };
        let received = job.query.receive_outcome();
        if job.phase == JobPhase::Cancelling && !job.query.session().run_in_flight() {
            job.phase = if received {
                job.query.clear_results();
                JobPhase::Cancelled
            } else {
                JobPhase::Failed
            };
        }
    }

    fn job_mut(&mut self, session: Uuid, id: QueryJobId) -> Result<&mut Job, ErrorCode> {
        if id.session != session {
            return Err(ErrorCode::StaleSession);
        }
        self.job
            .as_mut()
            .filter(|job| job.id == id)
            .ok_or(ErrorCode::QueryJobNotFound)
    }

    fn submit(
        &mut self,
        files: LoadedFilesView<'_>,
        session: Uuid,
        text: String,
    ) -> Result<Reply, ErrorCode> {
        if let Some(active) = self.active_job() {
            return Ok(Reply::QueryJob {
                job: QueryJobStatus {
                    job: active.id,
                    state: QueryJobState::Busy,
                },
            });
        }
        let schema = gt_query_run::schema_from_files(files.files());
        let checked = match QueryPreparation::check(text, schema)? {
            QueryPreparation::Rejected(validation) => {
                return Ok(Reply::QueryCheck { validation });
            }
            QueryPreparation::Ready(checked) => checked,
        };

        let job = self.job.insert(Job {
            id: QueryJobId {
                session,
                id: Uuid::new_v4(),
            },
            query: BackgroundQuery::with_session(checked.into()),
            phase: JobPhase::Preparing,
            data_generation: files.data_generation(),
        });
        Ok(job.reply())
    }

    fn active_job(&self) -> Option<&Job> {
        self.job
            .as_ref()
            .filter(|job| job.phase == JobPhase::Preparing || job.query.session().run_in_flight())
    }
}

struct Job {
    id: QueryJobId,
    query: BackgroundQuery,
    phase: JobPhase,
    data_generation: Generation<RecordingData>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobPhase {
    Cancelled,
    Cancelling,
    Failed,
    Preparing,
    Started,
    Stale,
}

impl Job {
    fn prepare(&mut self, ctx: &egui::Context, app: &mut App) {
        if self.data_generation != app.shared.borrow().loaded_files.data_generation() {
            self.phase = JobPhase::Stale;
            return;
        }
        let series =
            QuerySeries::capture_for_query(app, self.query.session().checked_dependencies());
        let shared = app.shared.borrow();
        let inputs = series.inputs(shared.loaded_files.view(), &shared.tree, &shared.filter);
        self.phase = if inputs.is_some_and(|inputs| self.query.start(ctx, inputs.run_inputs())) {
            JobPhase::Started
        } else {
            JobPhase::Failed
        };
    }

    fn refresh_dependencies(
        &mut self,
        data_generation: Generation<RecordingData>,
    ) -> Option<RunDependencies> {
        if self.phase != JobPhase::Started {
            return None;
        }
        let dependencies = self.query.session().result_dependencies()?;
        if self.data_generation != data_generation {
            self.phase = JobPhase::Stale;
            return None;
        }
        Some(dependencies)
    }

    fn refresh(&mut self, app: &mut App) {
        let data_generation = app.shared.borrow().loaded_files.data_generation();
        let Some(dependencies) = self.refresh_dependencies(data_generation) else {
            return;
        };
        let series = QuerySeries::capture_for_query(app, dependencies);
        let shared = app.shared.borrow();
        if let Some(inputs) =
            series.inputs(shared.loaded_files.view(), &shared.tree, &shared.filter)
        {
            self.query.update(inputs);
        }
    }

    fn cancel(&mut self) {
        self.phase = match self.phase {
            JobPhase::Preparing => JobPhase::Cancelled,
            JobPhase::Started if self.query.session().run_in_flight() => {
                self.query.session().cancel_run();
                JobPhase::Cancelling
            }
            phase => phase,
        };
    }

    fn reply(&self) -> Reply {
        let session = self.query.session();
        let state = match self.phase {
            JobPhase::Cancelled => QueryJobState::Cancelled,
            JobPhase::Cancelling => QueryJobState::Cancelling,
            JobPhase::Failed => QueryJobState::Failed,
            JobPhase::Preparing => QueryJobState::Preparing,
            JobPhase::Stale => QueryJobState::Stale,
            JobPhase::Started => match (session.progress(), session.results()) {
                (Some(progress), _) => QueryJobState::Running {
                    tracks_prepared: progress.tracks_prepared,
                    track_total: progress.track_total,
                },
                (None, Some(results)) => QueryJobState::of_results(results),
                (None, None) => QueryJobState::Failed,
            },
        };
        Reply::QueryJob {
            job: QueryJobStatus {
                job: self.id,
                state,
            },
        }
    }

    fn result_session(&self) -> Result<&QuerySession, ErrorCode> {
        let session = self.query.session();
        match self.phase {
            JobPhase::Cancelled | JobPhase::Failed => Err(ErrorCode::QueryResultsUnavailable),
            JobPhase::Cancelling | JobPhase::Preparing => Err(ErrorCode::QueryNotComplete),
            JobPhase::Stale => Err(ErrorCode::StaleQueryResults),
            JobPhase::Started if session.run_in_flight() => Err(ErrorCode::QueryNotComplete),
            JobPhase::Started => Ok(session),
        }
    }
}

impl App {
    pub(super) fn dispatch_session_request(&mut self, ctx: &egui::Context) {
        self.session_query.poll();
        if let Some(mut bridge) = self.session_bridge.take() {
            bridge.dispatch(|request| {
                self.execute_session_request(request)
                    .unwrap_or_else(Reply::from)
            });
            self.session_bridge = Some(bridge);
        }
        // Send the job ID before copying the selected tracks.
        self.prepare_session_query(ctx);
    }

    pub(super) fn prepare_session_query(&mut self, ctx: &egui::Context) {
        let Some(mut job) = self
            .session_query
            .job
            .take_if(|job| job.phase == JobPhase::Preparing)
        else {
            return;
        };
        job.prepare(ctx, self);
        self.session_query.job = Some(job);
    }

    pub(super) fn execute_session_request(
        &mut self,
        SessionRequest { request, session }: SessionRequest,
    ) -> Result<Reply, ErrorCode> {
        match request {
            Request::ApplyProcessing(ApplyProcessing {}) => {
                if self.shutdown_requested() {
                    return Err(ErrorCode::ShuttingDown);
                }
                if !self.has_loaded_nav_points() {
                    return Err(ErrorCode::NoLoadedNavPoints);
                }
                self.apply_resegmentation();
                Ok(Reply::ProcessingApplied)
            }
            Request::CancelQuery(args) => self.execute_with_session_query(|app, queries| {
                let job = queries.job_mut(session, args.job)?;
                job.cancel();
                job.refresh(app);
                Ok(job.reply())
            }),
            Request::CheckQuery(args) => {
                self.inspect_session_request(session, InspectionRequest::CheckQuery(args))
            }
            Request::DescribeSetting(args) => Ok(Reply::Setting {
                setting: self.describe_setting(args.key),
            }),
            Request::GetHistoryOpenStatus(args) => self
                .history_open_status(session, args)
                .map(|operation| Reply::HistoryOpen { operation }),
            Request::GetQueryResults(args) => self.execute_with_session_query(|app, queries| {
                let run = queries.result_session(app, session, args.job)?;
                let shared = app.shared.borrow();
                let results = run.retained_results(shared.loaded_files.view())?;
                QueryResults::project(&results, args).map(|results| Reply::QueryResults { results })
            }),
            Request::GetQueryStatus(args) => self.execute_with_session_query(|app, queries| {
                let job = queries.job_mut(session, args.job)?;
                job.refresh(app);
                Ok(job.reply())
            }),
            Request::GetRangeDetails(args) => {
                self.inspect_session_request(session, InspectionRequest::GetRangeDetails(args))
            }
            Request::GetRecording(args) => {
                self.inspect_session_request(session, InspectionRequest::GetRecording(args))
            }
            Request::GetSettings => Ok(Reply::Settings {
                settings: self.get_settings(),
            }),
            Request::GetTrack(args) => {
                self.inspect_session_request(session, InspectionRequest::GetTrack(args))
            }
            Request::ListHistoryRecordings(args) => self.list_history_recordings(session, args),
            Request::ListMetrics(args) => {
                self.inspect_session_request(session, InspectionRequest::ListMetrics(args))
            }
            Request::ListRecordings(args) => {
                self.inspect_session_request(session, InspectionRequest::ListRecordings(args))
            }
            Request::OpenHistoryRecording(args) => self
                .open_session_history_recording(session, args)
                .map(|operation| Reply::HistoryOpen { operation }),
            Request::OpenPoint(args) => {
                let presentation = self.session_open_point(session, args)?;
                self.shared.borrow_mut().queue_session_map(presentation);
                Ok(Reply::MapPresentationQueued)
            }
            Request::OpenQuery(args) => {
                self.pending_session_query_draft = Some(args.text);
                Ok(Reply::QueryDraftQueued)
            }
            Request::OpenSetting(args) => {
                self.pending_session_setting = Some(args.key);
                Ok(Reply::SettingOpenQueued)
            }
            Request::RunQuery(args) => self.session_query.submit(
                self.shared.borrow().loaded_files.view(),
                session,
                args.text,
            ),
            Request::SetSetting(args) => self
                .set_session_setting(args)
                .map(|setting| Reply::Setting { setting }),
            Request::ShowOnMap(args) => self.execute_with_session_query(|app, queries| {
                let presentation = app.session_show_on_map(queries, session, args)?;
                app.shared.borrow_mut().queue_session_map(presentation);
                Ok(Reply::MapPresentationQueued)
            }),
            Request::ShowOnPlot(args) => self.execute_with_session_query(|app, queries| {
                let presentation = app.session_show_on_plot(queries, session, args)?;
                app.shared.borrow_mut().pending_session_plot = Some(presentation);
                app.tiles_tree.tiles.set_visible(app.plot_tile_id, true);
                Ok(Reply::PlotPresentationQueued)
            }),
        }
    }

    fn execute_with_session_query(
        &mut self,
        execute: impl FnOnce(&mut Self, &mut SessionQuery) -> Result<Reply, ErrorCode>,
    ) -> Result<Reply, ErrorCode> {
        let mut queries = std::mem::take(&mut self.session_query);
        let result = execute(self, &mut queries);
        self.session_query = queries;
        result
    }

    fn inspect_session_request(
        &self,
        session: Uuid,
        request: InspectionRequest,
    ) -> Result<Reply, ErrorCode> {
        let shared = self.shared.borrow();
        Inspection::new(
            shared.loaded_files.view(),
            &shared.recording_name_template,
            session,
        )
        .execute(request)
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use gt_query_run::QuerySession;
    use gt_session_bridge::{
        QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus, Reply, Request, SessionRequest,
    };
    use uuid::Uuid;

    use super::{BackgroundQuery, Job, JobPhase, QuerySeries};
    use crate::app::test_util::harness::{self, TestDroppedFile};
    use crate::app::test_util::recordings;
    use crate::app::test_util::session::SESSION_ID;

    #[test]
    fn cancellation_signals_the_run_before_replying() {
        let mut harness = Harness::builder().build_eframe(harness::transient_app);
        harness::drop_file_and_wait_for_load(
            &mut harness,
            TestDroppedFile::bytes(recordings::sample_bytes(), "test.gtd"),
        );
        let app = harness.state_mut();
        let series = QuerySeries::capture(app);
        let mut query = QuerySession::new();
        query.set_text("points".to_owned());
        let (run, data_generation) = {
            let shared = app.shared.borrow();
            let files = shared.loaded_files.view();
            query.sync_checks(&gt_query_run::schema_from_files(files.files()));
            let inputs = series.inputs(files, &shared.tree, &shared.filter).unwrap();
            (
                query.start_run(inputs.run_inputs()).unwrap(),
                files.data_generation(),
            )
        };
        let handle = run.handle();
        let job = QueryJobId {
            session: SESSION_ID,
            id: Uuid::nil(),
        };
        app.session_query.job = Some(Job {
            id: job,
            query: BackgroundQuery::with_session(query),
            phase: JobPhase::Started,
            data_generation,
        });
        let expected = Reply::QueryJob {
            job: QueryJobStatus {
                job,
                state: QueryJobState::Cancelling,
            },
        };

        let reply = app.execute_session_request(SessionRequest {
            session: SESSION_ID,
            request: Request::CancelQuery(QueryJobRequest { job }),
        });
        assert_eq!(reply, Ok(expected.clone()));
        assert!(handle.cancelled());
        assert_eq!(app.session_query.retained_reply(), Some(expected));
    }
}
