use gt_session_bridge::{
    ApplyProcessing, ErrorCode, Inspection, InspectionRequest, OpenPanel, QueryJobId,
    QueryJobState, QueryJobStatus, QueryPreparation, QueryResults, Reply, Request, SessionRequest,
    TrackTreeState,
};
use gt_side_panel::{CheckState, NodeKey};
use gt_types::TrackRef;
use uuid::Uuid;

use super::App;
use super::mcp_action_notice::McpActionNotice;
use super::query_run::QueryPhase;
use super::session_map::MapPresentation;
use super::session_operations::SessionOperationStatus;
use super::session_plot::PlotPresentation;

pub(super) enum SessionCommand {
    Completed(Reply),
    Map(MapPresentation),
    Operation(SessionOperationStatus),
    Panel { session: Uuid, args: OpenPanel },
    Plot(PlotPresentation),
}

impl App {
    fn open_session_query_draft(&mut self, text: String) -> Result<Reply, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let opened = self.query_window.reveal_editor(&self.ctx);
        let edited = self.editor_query.session().text() != text;
        if edited {
            self.editor_query.session_mut().set_text(text);
            self.query_window.invalidate_editor_assistance();
        }
        if opened {
            McpActionNotice::OpenQueryEditor.enqueue(&mut self.toasts);
        } else if edited {
            McpActionNotice::UpdateQueryDraft.enqueue(&mut self.toasts);
        }
        self.ctx.request_repaint();
        Ok(Reply::QueryDraftOpened)
    }

    fn submit_session_query(
        &mut self,
        scope: Uuid,
        text: String,
    ) -> Result<SessionCommand, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        if let Some(job) = self.editor_query.active_job(scope)? {
            return Ok(SessionCommand::Completed(Reply::QueryJob {
                job: QueryJobStatus {
                    job,
                    state: QueryJobState::Busy,
                },
            }));
        }
        let (schema, generation) = {
            let shared = self.shared.borrow();
            let files = shared.loaded_files.view();
            (
                gt_query_run::schema_from_files(files.files()),
                files.data_generation(),
            )
        };
        let checked = match QueryPreparation::check(text, schema)? {
            QueryPreparation::Rejected(validation) => {
                return Ok(SessionCommand::Completed(Reply::QueryCheck { validation }));
            }
            QueryPreparation::Ready(checked) => checked,
        };
        let job = self.editor_query.queue(scope, checked, generation);
        self.query_window.reveal_editor(&self.ctx);
        self.query_window.invalidate_editor_assistance();
        self.query_window
            .record_run(self.editor_query.session().text());
        self.sync_settings_autosave();
        McpActionNotice::QueueQuery.enqueue(&mut self.toasts);
        Ok(SessionCommand::Operation(SessionOperationStatus::Query(
            QueryJobStatus {
                job,
                state: QueryJobState::Preparing,
            },
        )))
    }

    pub(super) fn require_completed_query(
        &mut self,
        session: Uuid,
        job: QueryJobId,
    ) -> Result<(), ErrorCode> {
        match self.editor_query.phase_for_job(session, job)? {
            QueryPhase::Completed => self.refresh_query_results(),
            QueryPhase::Cancelled | QueryPhase::Failed => Err(ErrorCode::QueryResultsUnavailable),
            QueryPhase::Cancelling | QueryPhase::Preparing(_) | QueryPhase::Running => {
                Err(ErrorCode::QueryNotComplete)
            }
            QueryPhase::Stale => Err(ErrorCode::StaleQueryResults),
        }
    }

    pub(super) fn session_query_status(
        &mut self,
        session: Uuid,
        job: QueryJobId,
    ) -> Result<QueryJobStatus, ErrorCode> {
        let state =
            match self.editor_query.phase_for_job(session, job)? {
                QueryPhase::Cancelled => QueryJobState::Cancelled,
                QueryPhase::Cancelling => QueryJobState::Cancelling,
                QueryPhase::Completed => match self.refresh_query_results() {
                    Ok(()) => self
                        .editor_query
                        .session()
                        .results()
                        .map_or(QueryJobState::Failed, QueryJobState::of_results),
                    Err(ErrorCode::StaleQueryResults) => QueryJobState::Stale,
                    Err(error) => return Err(error),
                },
                QueryPhase::Failed => QueryJobState::Failed,
                QueryPhase::Preparing(_) => QueryJobState::Preparing,
                QueryPhase::Running => self.editor_query.session().progress().map_or(
                    QueryJobState::Failed,
                    |progress| QueryJobState::Running {
                        tracks_prepared: progress.tracks_prepared,
                        track_total: progress.track_total,
                    },
                ),
                QueryPhase::Stale => QueryJobState::Stale,
            };
        Ok(QueryJobStatus { job, state })
    }

    pub(super) fn execute_session_request(
        &mut self,
        SessionRequest { request, session }: SessionRequest,
    ) -> Result<SessionCommand, ErrorCode> {
        let reply = match request {
            Request::ApplyProcessing(ApplyProcessing {}) => {
                if self.shutdown_requested() {
                    return Err(ErrorCode::ShuttingDown);
                }
                if !self.has_loaded_nav_points() {
                    return Err(ErrorCode::NoLoadedNavPoints);
                }
                self.apply_resegmentation();
                McpActionNotice::ReprocessRecordings.enqueue(&mut self.toasts);
                Ok(Reply::ProcessingApplied)
            }
            Request::CancelQuery(args) => {
                let phase = self.editor_query.phase_for_job(session, args.job)?;
                self.editor_query.cancel();
                match phase {
                    QueryPhase::Preparing(_) => {
                        McpActionNotice::CancelQueuedQuery.enqueue(&mut self.toasts)
                    }
                    QueryPhase::Running => {
                        McpActionNotice::RequestQueryCancellation.enqueue(&mut self.toasts)
                    }
                    _ => {}
                }
                self.session_query_status(session, args.job)
                    .map(|job| Reply::QueryJob { job })
            }
            Request::CheckQuery(args) => {
                self.inspect_session_request(session, InspectionRequest::CheckQuery(args))
            }
            Request::DescribeSetting(args) => Ok(Reply::Setting {
                setting: self.describe_setting(args.key),
            }),
            Request::GetHistoryOpenStatus(args) => self
                .history_open_status(session, args)
                .map(|operation| Reply::HistoryOpen { operation }),
            Request::GetQueryResults(args) => {
                self.require_completed_query(session, args.job)?;
                let shared = self.shared.borrow();
                let results = self
                    .editor_query
                    .session()
                    .retained_results(shared.loaded_files.view())?;
                QueryResults::project(&results, args).map(|results| Reply::QueryResults { results })
            }
            Request::GetQueryStatus(args) => self
                .session_query_status(session, args.job)
                .map(|job| Reply::QueryJob { job }),
            Request::GetRangeDetails(args) => {
                self.inspect_session_request(session, InspectionRequest::GetRangeDetails(args))
            }
            Request::GetRecording(args) => {
                self.inspect_session_request(session, InspectionRequest::GetRecording(args))
            }
            Request::GetSessionState => Ok(Reply::SessionState {
                state: self.session_state(session),
            }),
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
            Request::ManageRecordings(args) => self.manage_recordings(session, args),
            Request::OpenHistoryRecording(args) => {
                let status = self.open_session_history_recording(session, args)?;
                return Ok(SessionCommand::Operation(SessionOperationStatus::History(
                    status,
                )));
            }
            Request::OpenPanel(args) => return Ok(SessionCommand::Panel { session, args }),
            Request::OpenPoint(args) => {
                let presentation = self.session_open_point(session, args)?;
                return Ok(SessionCommand::Map(presentation));
            }
            Request::OpenQuery(args) => self.open_session_query_draft(args.text),
            Request::OpenSetting(args) => self.open_session_setting(args.key),
            Request::RunQuery(args) => return self.submit_session_query(session, args.text),
            Request::SetSessionView(args) => self.set_session_view(session, args),
            Request::SetSetting(args) => self
                .set_session_setting(args)
                .map(|setting| Reply::Setting { setting }),
            Request::SetTrackVisibility(args) => self.set_session_track_visibility(session, &args),
            Request::ShowOnMap(args) => {
                let presentation = self.session_show_on_map(session, args)?;
                return Ok(SessionCommand::Map(presentation));
            }
            Request::ShowOnPlot(args) => {
                let presentation = self.session_show_on_plot(session, args)?;
                return Ok(SessionCommand::Plot(presentation));
            }
        }?;
        Ok(SessionCommand::Completed(reply))
    }

    fn inspect_session_request(
        &self,
        session: Uuid,
        request: InspectionRequest,
    ) -> Result<Reply, ErrorCode> {
        let shared = self.shared.borrow();
        let track_tree = |track: TrackRef| {
            Some(TrackTreeState {
                enabled: shared.tree.track_node(track)?.check == CheckState::On,
                selected: shared.tree.selection.contains(&NodeKey::Track(track)),
            })
        };
        Inspection::new(
            shared.loaded_files.view(),
            &shared.recording_name_template,
            session,
        )
        .with_track_tree(&track_tree)
        .execute(request)
    }
}
