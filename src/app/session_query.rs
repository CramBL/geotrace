use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_session_bridge::{
    ErrorCode, GetQueryResults, QueryJobId, QueryJobState, QueryJobStatus, QueryPreparation,
    QueryResults, Reply, Request, SessionRequest,
};
use gt_types::Generation;
use uuid::Uuid;

use super::App;
use super::query_run::{BackgroundQuery, QuerySeries};

#[derive(Default)]
pub(super) struct SessionQuery {
    job: Option<Job>,
}

impl SessionQuery {
    #[cfg(test)]
    pub(super) fn is_active(&self) -> bool {
        self.active_job().is_some()
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
        &self,
        files: LoadedFilesView<'_>,
        session: Uuid,
        text: String,
        staged: &mut Option<Job>,
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
        let query = match QueryPreparation::check(text, schema)? {
            QueryPreparation::Rejected(validation) => return Ok(Reply::QueryCheck { validation }),
            QueryPreparation::Ready(checked) => BackgroundQuery::with_session(checked.into()),
        };

        let job = Job {
            id: QueryJobId {
                session,
                id: Uuid::new_v4(),
            },
            query,
            phase: JobPhase::Preparing,
            data_generation: files.data_generation(),
        };
        let reply = job.reply();
        *staged = Some(job);
        Ok(reply)
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
        let series = QuerySeries::capture(app);
        let shared = app.shared.borrow();
        let inputs = series.inputs(
            shared.loaded_files.view(),
            shared.tree.visibility(),
            &shared.filter,
        );
        self.phase = if inputs.is_some_and(|inputs| self.query.start(ctx, inputs)) {
            JobPhase::Started
        } else {
            JobPhase::Failed
        };
    }

    fn cancel(&mut self) {
        match self.phase {
            JobPhase::Preparing => self.phase = JobPhase::Cancelled,
            JobPhase::Started if self.query.session().run_in_flight() => {
                self.query.session().cancel_run();
                self.phase = JobPhase::Cancelling;
            }
            _ => {}
        }
    }

    fn refresh(&mut self, app: &mut App) {
        if self.phase != JobPhase::Started || self.query.session().results().is_none() {
            return;
        }
        if self.data_generation != app.shared.borrow().loaded_files.data_generation() {
            self.phase = JobPhase::Stale;
            return;
        }
        let series = QuerySeries::capture(app);
        let shared = app.shared.borrow();
        if let Some(inputs) = series.inputs(
            shared.loaded_files.view(),
            shared.tree.visibility(),
            &shared.filter,
        ) {
            self.query.update(inputs);
        }
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

    fn results_page(
        &self,
        files: LoadedFilesView<'_>,
        args: GetQueryResults,
    ) -> Result<Reply, ErrorCode> {
        let session = self.query.session();
        let results = match self.phase {
            JobPhase::Cancelled | JobPhase::Failed => {
                return Err(ErrorCode::QueryResultsUnavailable);
            }
            JobPhase::Cancelling | JobPhase::Preparing => return Err(ErrorCode::QueryNotComplete),
            JobPhase::Stale => return Err(ErrorCode::StaleQueryResults),
            JobPhase::Started if session.run_in_flight() => {
                return Err(ErrorCode::QueryNotComplete);
            }
            JobPhase::Started => session
                .results()
                .ok_or(ErrorCode::QueryResultsUnavailable)?,
        };
        QueryResults::project(results, files, args).map(|results| Reply::QueryResults { results })
    }
}

impl App {
    pub(super) fn dispatch_session_request(&mut self, ctx: &egui::Context) {
        self.session_query.poll();
        if let Some(mut bridge) = self.session_bridge.take() {
            let mut staged = None;
            let handed_off =
                bridge.dispatch(|request| self.execute_session_request(request, &mut staged));
            if handed_off && let Some(job) = staged {
                self.session_query.job = Some(job);
            }
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

    #[cfg(test)]
    pub(super) fn execute_session_request_in_test(&mut self, request: SessionRequest) -> Reply {
        let mut staged = None;
        let reply = self.execute_session_request(request, &mut staged);
        if let Some(job) = staged {
            self.session_query.job = Some(job);
        }
        reply
    }

    fn execute_session_request(
        &mut self,
        request: SessionRequest,
        staged: &mut Option<Job>,
    ) -> Reply {
        let mut queries = std::mem::take(&mut self.session_query);
        let result = match request.request {
            Request::CancelQuery(args) => queries.job_mut(request.session, args.job).map(|job| {
                job.cancel();
                job.refresh(self);
                job.reply()
            }),
            Request::GetQueryResults(args) => {
                queries.job_mut(request.session, args.job).and_then(|job| {
                    job.refresh(self);
                    job.results_page(self.shared.borrow().loaded_files.view(), args)
                })
            }
            Request::GetQueryStatus(args) => {
                queries.job_mut(request.session, args.job).map(|job| {
                    job.refresh(self);
                    job.reply()
                })
            }
            Request::RunQuery(args) => queries.submit(
                self.shared.borrow().loaded_files.view(),
                request.session,
                args.text,
                staged,
            ),
            _ => {
                let shared = self.shared.borrow();
                Ok(request.inspect(shared.loaded_files.view(), &shared.recording_name_template))
            }
        };
        self.session_query = queries;
        result.unwrap_or_else(Reply::from)
    }
}
