use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_query_run::{CheckedQueryText, RunDependencies, RunResults};
use gt_session_bridge::{
    ApplyProcessing, ErrorCode, Inspection, InspectionRequest, QueryJobId, QueryJobState,
    QueryJobStatus, QueryPreparation, QueryResults, Reply, Request, SessionRequest, SettingInfo,
    SettingKey,
};
use gt_types::Generation;
use uuid::Uuid;

use self::PreparedSessionResponse::{Effect, ReplyOnly};
use super::App;
use super::query_run::{BackgroundQuery, CapturedQueryInputs, QuerySeries};
use super::session_history::{HistoryOpenPreparation, PreparedHistoryOpen};
use super::session_map::MapPresentation;
use super::session_plot::PlotPresentation;
use super::settings_ui::SettingChange;

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

    pub(super) fn results(
        &mut self,
        app: &mut App,
        session: Uuid,
        id: QueryJobId,
    ) -> Result<&RunResults, ErrorCode> {
        let job = self.job_mut(session, id)?;
        job.refresh(app);
        job.results()
    }

    pub(super) fn has_current_results(&self, id: QueryJobId) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| job.id == id && job.results().is_ok())
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

    fn apply_cancellation(&mut self, cancellation: &QueryCancellation) {
        let job = self.job.as_mut().filter(|job| job.id == cancellation.job);
        debug_assert!(
            job.is_some(),
            "prepared cancellation addresses the retained job"
        );
        if let Some(job) = job {
            job.phase = match cancellation.transition {
                CancelTransition::BeforeStart => JobPhase::Cancelled,
                CancelTransition::InFlight => {
                    job.query.session().cancel_run();
                    JobPhase::Cancelling
                }
            };
        }
    }

    fn prepare_submission(
        &self,
        files: LoadedFilesView<'_>,
        session: Uuid,
        text: String,
    ) -> Result<PreparedSessionResponse, ErrorCode> {
        if let Some(active) = self.active_job() {
            return Ok(ReplyOnly(Reply::QueryJob {
                job: QueryJobStatus {
                    job: active.id,
                    state: QueryJobState::Busy,
                },
            }));
        }
        let schema = gt_query_run::schema_from_files(files.files());
        let checked = match QueryPreparation::check(text, schema)? {
            QueryPreparation::Rejected(validation) => {
                return Ok(ReplyOnly(Reply::QueryCheck { validation }));
            }
            QueryPreparation::Ready(checked) => checked,
        };

        Ok(Effect(SessionEffect::StartQuery(PreparedQueryJob {
            id: QueryJobId {
                session,
                id: Uuid::new_v4(),
            },
            checked,
            data_generation: files.data_generation(),
        })))
    }

    fn active_job(&self) -> Option<&Job> {
        self.job
            .as_ref()
            .filter(|job| job.phase == JobPhase::Preparing || job.query.session().run_in_flight())
    }
}

struct PreparedQueryJob {
    id: QueryJobId,
    checked: CheckedQueryText,
    data_generation: Generation<RecordingData>,
}

struct QueryCancellation {
    job: QueryJobId,
    transition: CancelTransition,
}

enum CancelTransition {
    BeforeStart,
    InFlight,
}

struct Job {
    id: QueryJobId,
    query: BackgroundQuery,
    phase: JobPhase,
    data_generation: Generation<RecordingData>,
}

enum PreparedSessionResponse {
    Effect(SessionEffect),
    ReplyOnly(Reply),
}

impl PreparedSessionResponse {
    fn into_handoff(self) -> (Reply, Option<SessionEffect>) {
        match self {
            Self::Effect(effect) => (effect.acknowledgement(), Some(effect)),
            Self::ReplyOnly(reply) => (reply, None),
        }
    }
}

enum SessionEffect {
    ApplyProcessing,
    CancelQuery(QueryCancellation),
    OpenHistory(Box<PreparedHistoryOpen>),
    OpenQuery(String),
    OpenSetting(SettingKey),
    PresentMap(MapPresentation),
    PresentPlot(PlotPresentation),
    SetSetting {
        setting: SettingInfo,
        change: SettingChange,
    },
    StartQuery(PreparedQueryJob),
}

impl SessionEffect {
    fn apply(self, app: &mut App) {
        match self {
            Self::ApplyProcessing => app.apply_resegmentation(),
            Self::CancelQuery(cancel) => app.session_query.apply_cancellation(&cancel),
            Self::OpenHistory(open) => app.accept_history_open(*open),
            Self::OpenQuery(text) => app.pending_session_query_draft = Some(text),
            Self::OpenSetting(key) => app.pending_session_setting = Some(key),
            Self::PresentMap(presentation) => {
                app.shared.borrow_mut().queue_session_map(presentation)
            }
            Self::PresentPlot(presentation) => {
                app.shared.borrow_mut().pending_session_plot = Some(presentation);
                app.tiles_tree.tiles.set_visible(app.plot_tile_id, true);
            }
            Self::SetSetting { change, .. } => app.apply_setting_change(change),
            Self::StartQuery(job) => {
                app.session_query.job = Some(Job {
                    id: job.id,
                    query: BackgroundQuery::with_session(job.checked.into()),
                    phase: JobPhase::Preparing,
                    data_generation: job.data_generation,
                });
            }
        }
    }

    fn acknowledgement(&self) -> Reply {
        match self {
            Self::ApplyProcessing => Reply::ProcessingQueued,
            Self::CancelQuery(cancel) => Reply::QueryJob {
                job: QueryJobStatus {
                    job: cancel.job,
                    state: match cancel.transition {
                        CancelTransition::BeforeStart => QueryJobState::Cancelled,
                        CancelTransition::InFlight => QueryJobState::Cancelling,
                    },
                },
            },
            Self::OpenHistory(open) => Reply::HistoryOpen {
                operation: open.status(),
            },
            Self::OpenQuery(_) => Reply::QueryDraftQueued,
            Self::OpenSetting(_) => Reply::SettingOpenQueued,
            Self::PresentMap(_) => Reply::MapPresentationQueued,
            Self::PresentPlot(_) => Reply::PlotPresentationQueued,
            Self::SetSetting { setting, .. } => Reply::Setting {
                setting: setting.clone(),
            },
            Self::StartQuery(job) => Reply::QueryJob {
                job: QueryJobStatus {
                    job: job.id,
                    state: QueryJobState::Preparing,
                },
            },
        }
    }
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

    fn prepare_cancellation(&self) -> Option<QueryCancellation> {
        let transition = match self.phase {
            JobPhase::Preparing => CancelTransition::BeforeStart,
            JobPhase::Started if self.query.session().run_in_flight() => CancelTransition::InFlight,
            _ => return None,
        };
        Some(QueryCancellation {
            job: self.id,
            transition,
        })
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

    fn results(&self) -> Result<&RunResults, ErrorCode> {
        let session = self.query.session();
        let results = match self.phase {
            JobPhase::Cancelled | JobPhase::Failed => Err(ErrorCode::QueryResultsUnavailable),
            JobPhase::Cancelling | JobPhase::Preparing => Err(ErrorCode::QueryNotComplete),
            JobPhase::Stale => Err(ErrorCode::StaleQueryResults),
            JobPhase::Started if session.run_in_flight() => Err(ErrorCode::QueryNotComplete),
            JobPhase::Started => session.results().ok_or(ErrorCode::QueryResultsUnavailable),
        }?;
        if results.stale() {
            return Err(ErrorCode::StaleQueryResults);
        }
        Ok(results)
    }
}

impl App {
    pub(super) fn dispatch_session_request(&mut self, ctx: &egui::Context) {
        self.session_query.poll();
        if let Some(mut bridge) = self.session_bridge.take() {
            if let Some(effect) = bridge
                .dispatch(|request| self.prepare_session_request(request).into_handoff())
                .flatten()
            {
                effect.apply(self);
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
    pub(super) fn execute_session_request_in_test(
        &mut self,
        request: SessionRequest,
        handed_off: bool,
    ) -> Reply {
        let (reply, effect) = self.prepare_session_request(request).into_handoff();
        if handed_off && let Some(effect) = effect {
            effect.apply(self);
        }
        reply
    }

    fn prepare_session_request(
        &mut self,
        SessionRequest { request, session }: SessionRequest,
    ) -> PreparedSessionResponse {
        match request {
            Request::ApplyProcessing(ApplyProcessing {}) => {
                if self.shutdown_requested() {
                    Err(ErrorCode::ShuttingDown)
                } else if !self.has_loaded_nav_points() {
                    Err(ErrorCode::NoLoadedNavPoints)
                } else {
                    Ok(Effect(SessionEffect::ApplyProcessing))
                }
            }
            Request::CancelQuery(args) => self.execute_with_session_query(|app, queries| {
                let job = queries.job_mut(session, args.job)?;
                Ok(if let Some(cancellation) = job.prepare_cancellation() {
                    Effect(SessionEffect::CancelQuery(cancellation))
                } else {
                    job.refresh(app);
                    ReplyOnly(job.reply())
                })
            }),
            Request::CheckQuery(args) => {
                self.inspect_session_request(session, InspectionRequest::CheckQuery(args))
            }
            Request::DescribeSetting(args) => Ok(ReplyOnly(Reply::Setting {
                setting: self.describe_setting(args.key),
            })),
            Request::GetHistoryOpenStatus(args) => self
                .history_open_status(session, args)
                .map(|operation| ReplyOnly(Reply::HistoryOpen { operation })),
            Request::GetQueryResults(args) => self.execute_with_session_query(|app, queries| {
                let results = queries.results(app, session, args.job)?;
                QueryResults::project(results, app.shared.borrow().loaded_files.view(), args)
                    .map(|results| ReplyOnly(Reply::QueryResults { results }))
            }),
            Request::GetQueryStatus(args) => self.execute_with_session_query(|app, queries| {
                let job = queries.job_mut(session, args.job)?;
                job.refresh(app);
                Ok(ReplyOnly(job.reply()))
            }),
            Request::GetRangeDetails(args) => {
                self.inspect_session_request(session, InspectionRequest::GetRangeDetails(args))
            }
            Request::GetRecording(args) => {
                self.inspect_session_request(session, InspectionRequest::GetRecording(args))
            }
            Request::GetSettings => Ok(ReplyOnly(Reply::Settings {
                settings: self.get_settings(),
            })),
            Request::GetTrack(args) => {
                self.inspect_session_request(session, InspectionRequest::GetTrack(args))
            }
            Request::ListHistoryRecordings(args) => {
                self.list_history_recordings(session, args).map(ReplyOnly)
            }
            Request::ListMetrics(args) => {
                self.inspect_session_request(session, InspectionRequest::ListMetrics(args))
            }
            Request::ListRecordings(args) => {
                self.inspect_session_request(session, InspectionRequest::ListRecordings(args))
            }
            Request::OpenHistoryRecording(args) => {
                self.prepare_history_open(session, args)
                    .map(|preparation| match preparation {
                        HistoryOpenPreparation::Pending(operation) => {
                            ReplyOnly(Reply::HistoryOpen { operation })
                        }
                        HistoryOpenPreparation::Start(open) => {
                            Effect(SessionEffect::OpenHistory(open))
                        }
                    })
            }
            Request::OpenPoint(args) => self
                .session_open_point(session, args)
                .map(|presentation| Effect(SessionEffect::PresentMap(presentation))),
            Request::OpenQuery(args) => Ok(Effect(SessionEffect::OpenQuery(args.text))),
            Request::OpenSetting(args) => Ok(Effect(SessionEffect::OpenSetting(args.key))),
            Request::RunQuery(args) => self.session_query.prepare_submission(
                self.shared.borrow().loaded_files.view(),
                session,
                args.text,
            ),
            Request::SetSetting(args) => {
                self.prepare_setting_change(args)
                    .map(|(setting, change)| match change {
                        Some(change) => Effect(SessionEffect::SetSetting { setting, change }),
                        None => ReplyOnly(Reply::Setting { setting }),
                    })
            }
            Request::ShowOnMap(args) => self.execute_with_session_query(|app, queries| {
                app.session_show_on_map(queries, session, args)
                    .map(|presentation| Effect(SessionEffect::PresentMap(presentation)))
            }),
            Request::ShowOnPlot(args) => self.execute_with_session_query(|app, queries| {
                app.session_show_on_plot(queries, session, args)
                    .map(|presentation| Effect(SessionEffect::PresentPlot(presentation)))
            }),
        }
        .unwrap_or_else(|error| ReplyOnly(error.into()))
    }

    fn execute_with_session_query(
        &mut self,
        execute: impl FnOnce(&mut Self, &mut SessionQuery) -> Result<PreparedSessionResponse, ErrorCode>,
    ) -> Result<PreparedSessionResponse, ErrorCode> {
        let mut queries = std::mem::take(&mut self.session_query);
        let result = execute(self, &mut queries);
        self.session_query = queries;
        result
    }

    fn inspect_session_request(
        &self,
        session: Uuid,
        request: InspectionRequest,
    ) -> Result<PreparedSessionResponse, ErrorCode> {
        let shared = self.shared.borrow();
        Inspection::new(
            shared.loaded_files.view(),
            &shared.recording_name_template,
            session,
        )
        .execute(request)
        .map(ReplyOnly)
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
    fn cancellation_signals_the_run_after_reply_handoff() {
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
        let request = || SessionRequest {
            session: SESSION_ID,
            request: Request::CancelQuery(QueryJobRequest { job }),
        };
        let expected = Reply::QueryJob {
            job: QueryJobStatus {
                job,
                state: QueryJobState::Cancelling,
            },
        };
        let started = app.session_query.retained_reply();

        assert_eq!(
            app.execute_session_request_in_test(request(), false),
            expected
        );
        assert!(!handle.cancelled());
        assert_eq!(app.session_query.retained_reply(), started);

        assert_eq!(
            app.execute_session_request_in_test(request(), true),
            expected
        );
        assert!(handle.cancelled());
        assert_eq!(app.session_query.retained_reply(), Some(expected));
    }
}
