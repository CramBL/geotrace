use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_query_run::RunResults;
use gt_session_bridge::{
    ApplyProcessing, ErrorCode, QueryJobId, QueryJobState, QueryJobStatus, QueryPreparation,
    QueryResults, Reply, Request, SessionRequest, SettingKey,
};
use gt_types::Generation;
use uuid::Uuid;

use super::App;
use super::query_run::{BackgroundQuery, CapturedQueryInputs, QuerySeries};
use super::session_history::StagedHistoryOpen;
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
            && job.can_refresh(inputs.run_inputs().loaded_files.data_generation())
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

#[derive(Default)]
struct StagedSessionChanges {
    apply_processing: bool,
    history_open: Option<StagedHistoryOpen>,
    job: Option<Job>,
    map: Option<MapPresentation>,
    plot: Option<PlotPresentation>,
    query_draft: Option<String>,
    open_setting: Option<SettingKey>,
    setting_change: Option<SettingChange>,
}

impl StagedSessionChanges {
    fn queue_map(&mut self, presentation: MapPresentation) -> Reply {
        self.map = Some(presentation);
        Reply::MapPresentationQueued
    }

    fn queue_plot(&mut self, presentation: PlotPresentation) -> Reply {
        self.plot = Some(presentation);
        Reply::PlotPresentationQueued
    }

    fn queue_query_draft(&mut self, text: String) -> Reply {
        self.query_draft = Some(text);
        Reply::QueryDraftQueued
    }

    fn queue_setting(&mut self, key: SettingKey) -> Reply {
        self.open_setting = Some(key);
        Reply::SettingOpenQueued
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
        let series = QuerySeries::capture(app);
        let shared = app.shared.borrow();
        let inputs = series.inputs(shared.loaded_files.view(), &shared.tree, &shared.filter);
        self.phase = if inputs.is_some_and(|inputs| self.query.start(ctx, inputs.run_inputs())) {
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

    fn can_refresh(&mut self, data_generation: Generation<RecordingData>) -> bool {
        if self.phase != JobPhase::Started || self.query.session().results().is_none() {
            return false;
        }
        if self.data_generation != data_generation {
            self.phase = JobPhase::Stale;
            return false;
        }
        true
    }

    fn refresh(&mut self, app: &mut App) {
        if !self.can_refresh(app.shared.borrow().loaded_files.data_generation()) {
            return;
        }
        let series = QuerySeries::capture(app);
        let shared = app.shared.borrow();
        if let Some(inputs) =
            series.inputs(shared.loaded_files.view(), &shared.tree, &shared.filter)
        {
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
            let mut staged = StagedSessionChanges::default();
            let handed_off =
                bridge.dispatch(|request| self.execute_session_request(request, &mut staged));
            self.adopt_session_changes(staged, handed_off);
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
        let mut staged = StagedSessionChanges::default();
        let reply = self.execute_session_request(request, &mut staged);
        self.adopt_session_changes(staged, handed_off);
        reply
    }

    fn adopt_session_changes(&mut self, staged: StagedSessionChanges, handed_off: bool) {
        if !handed_off {
            return;
        }
        if staged.apply_processing {
            self.apply_resegmentation();
        }
        if let Some(history_open) = staged.history_open {
            self.accept_history_open(history_open);
        }
        if let Some(job) = staged.job {
            self.session_query.job = Some(job);
        }
        if let Some(presentation) = staged.map {
            self.shared.borrow_mut().queue_session_map(presentation);
        }
        if let Some(presentation) = staged.plot {
            self.shared.borrow_mut().pending_session_plot = Some(presentation);
            self.tiles_tree.tiles.set_visible(self.plot_tile_id, true);
        }
        if let Some(text) = staged.query_draft {
            self.pending_session_query_draft = Some(text);
        }
        if let Some(key) = staged.open_setting {
            self.pending_session_setting = Some(key);
        }
        if let Some(change) = staged.setting_change {
            self.apply_setting_change(change);
        }
    }

    fn execute_session_request(
        &mut self,
        request: SessionRequest,
        staged: &mut StagedSessionChanges,
    ) -> Reply {
        if let Request::ListHistoryRecordings(args) = request.request {
            return self
                .list_history_recordings(request.session, args)
                .unwrap_or_else(Reply::from);
        }
        let mut queries = std::mem::take(&mut self.session_query);
        let result = match request.request {
            Request::ApplyProcessing(ApplyProcessing {}) => {
                if self.shutdown_requested() {
                    Err(ErrorCode::ShuttingDown)
                } else if !self.has_loaded_nav_points() {
                    Err(ErrorCode::NoLoadedNavPoints)
                } else {
                    staged.apply_processing = true;
                    Ok(Reply::ProcessingQueued)
                }
            }
            Request::CancelQuery(args) => queries.job_mut(request.session, args.job).map(|job| {
                job.cancel();
                job.refresh(self);
                job.reply()
            }),
            Request::DescribeSetting(args) => Ok(Reply::Setting {
                setting: self.describe_setting(args.key),
            }),
            Request::GetHistoryOpenStatus(args) => self.history_open_status(request.session, args),
            Request::GetQueryResults(args) => queries
                .results(self, request.session, args.job)
                .and_then(|results| {
                    QueryResults::project(results, self.shared.borrow().loaded_files.view(), args)
                        .map(|results| Reply::QueryResults { results })
                }),
            Request::GetQueryStatus(args) => {
                queries.job_mut(request.session, args.job).map(|job| {
                    job.refresh(self);
                    job.reply()
                })
            }
            Request::GetSettings => Ok(Reply::Settings {
                settings: self.get_settings(),
            }),
            Request::OpenHistoryRecording(args) => {
                self.prepare_history_open(request.session, args, &mut staged.history_open)
            }
            Request::OpenPoint(args) => self
                .session_open_point(request.session, args)
                .map(|presentation| staged.queue_map(presentation)),
            Request::OpenQuery(args) => Ok(staged.queue_query_draft(args.text)),
            Request::OpenSetting(args) => Ok(staged.queue_setting(args.key)),
            Request::RunQuery(args) => queries.submit(
                self.shared.borrow().loaded_files.view(),
                request.session,
                args.text,
                &mut staged.job,
            ),
            Request::SetSetting(args) => {
                self.prepare_setting_change(args).map(|(setting, change)| {
                    staged.setting_change = change;
                    Reply::Setting { setting }
                })
            }
            Request::ShowOnMap(args) => self
                .session_show_on_map(&mut queries, request.session, args)
                .map(|presentation| staged.queue_map(presentation)),
            Request::ShowOnPlot(args) => self
                .session_show_on_plot(&mut queries, request.session, args)
                .map(|presentation| staged.queue_plot(presentation)),
            _ => {
                let shared = self.shared.borrow();
                Ok(request.inspect(shared.loaded_files.view(), &shared.recording_name_template))
            }
        };
        self.session_query = queries;
        result.unwrap_or_else(Reply::from)
    }
}
