use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::time::Duration;

use gt_session_bridge::{
    ApplyProcessing, ErrorCode, GetHistoryOpenStatus, GetQueryResults, GetRangeDetails,
    GetRecording, GetTrack, HistoryOpenState, ListHistoryRecordings, ListMetrics, ListRecordings,
    ManageRecordings, OpenHistoryRecording, OpenPanel, OpenPoint, QueryJobRequest, QueryJobState,
    QueryText, Reply, Request, SetSetting, SetTrackVisibility, SettingSelection, ShowOnMap,
    ShowOnPlot,
};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::{QuitReason, RequestContext};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt as _, tool, tool_handler, tool_router,
    transport,
};
use tokio::time::{self, Instant};

#[tokio::main(flavor = "current_thread")]
pub async fn run() -> Result<(), Box<dyn Error>> {
    if env::args_os().nth(1).as_deref() == Some(OsStr::new("--version")) {
        println!("{COMPANION_NAME} {VERSION}");
        return Ok(());
    }
    let reason = Companion {
        bridge: Bridge::Desktop,
    }
    .serve(transport::stdio_pipes())
    .await?
    .waiting()
    .await?;
    match reason {
        QuitReason::Closed | QuitReason::Cancelled => Ok(()),
        QuitReason::JoinError(error) => Err(error.into()),
        reason => Err(io::Error::other(format!("Companion stopped ({reason:?})")).into()),
    }
}

struct Companion {
    bridge: Bridge,
}

#[tool_router]
impl Companion {
    #[tool(
        description = "Rebuild loaded tracks and generated markers with current Processing settings. Returns processing_applied after the rebuild.",
        annotations(
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn apply_processing(
        &self,
        Parameters(args): Parameters<ApplyProcessing>,
    ) -> CallToolResult {
        self.inspect(Request::ApplyProcessing(args)).await
    }

    #[tool(
        description = "Cancel the running analysis for a query job. Poll with get_query_status until the job is terminal.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn cancel_query(&self, Parameters(args): Parameters<QueryJobRequest>) -> CallToolResult {
        self.inspect(Request::CancelQuery(args)).await
    }

    #[tool(
        description = "Check query editor text against the loaded recording channels. Returns diagnostics with byte spans into the full text.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn check_query(&self, Parameters(args): Parameters<QueryText>) -> CallToolResult {
        self.inspect(Request::CheckQuery(args)).await
    }

    #[tool(
        description = "Read one setting's current value, limits, and help.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn describe_setting(
        &self,
        Parameters(args): Parameters<SettingSelection>,
    ) -> CallToolResult {
        self.inspect(Request::DescribeSetting(args)).await
    }

    #[tool(
        description = "Read the status of a History open. A ready result includes the loaded recording ID.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_history_open_status(
        &self,
        Parameters(args): Parameters<GetHistoryOpenStatus>,
    ) -> CallToolResult {
        self.inspect(Request::GetHistoryOpenStatus(args)).await
    }

    #[tool(
        description = "Read a page of matches or the rows of one match from a completed query job. Use next_start for the next page.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_query_results(
        &self,
        Parameters(args): Parameters<GetQueryResults>,
    ) -> CallToolResult {
        self.inspect(Request::GetQueryResults(args)).await
    }

    #[tool(
        description = "Read the current status or summary of a query job returned by run_query.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_query_status(
        &self,
        Parameters(args): Parameters<QueryJobRequest>,
    ) -> CallToolResult {
        self.inspect(Request::GetQueryStatus(args)).await
    }

    #[tool(
        description = "Read a page of points in recorded order. Use next_start as start with the same end.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_range_details(
        &self,
        Parameters(args): Parameters<GetRangeDetails>,
    ) -> CallToolResult {
        self.inspect(Request::GetRangeDetails(args)).await
    }

    #[tool(
        description = "Inspect a loaded recording, its track tree and History rows. Pass next_after as after and stored_tracks.next_start as stored_track_start.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_recording(&self, Parameters(args): Parameters<GetRecording>) -> CallToolResult {
        self.inspect(Request::GetRecording(args)).await
    }

    #[tool(
        description = "Summarize the current GeoTrace session: loaded recordings, open panels, global time filter, map, pinned point, and query activity.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_session_state(&self) -> CallToolResult {
        self.inspect(Request::GetSessionState).await
    }

    #[tool(
        description = "List Analysis settings and recording Processing settings with their limits and help.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_settings(&self) -> CallToolResult {
        self.inspect(Request::GetSettings).await
    }

    #[tool(
        description = "Read a loaded track's time range, counts, geometry, track tree state and History membership.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_track(&self, Parameters(args): Parameters<GetTrack>) -> CallToolResult {
        self.inspect(Request::GetTrack(args)).await
    }

    #[tool(
        description = "List stored History recordings by navigation start time, newest first. Entries without a stored time range appear last. Bounds use RFC 3339 UTC. Filter by overlapping UTC dates and use next_cursor with the same dates to continue.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_history_recordings(
        &self,
        Parameters(args): Parameters<ListHistoryRecordings>,
    ) -> CallToolResult {
        self.inspect(Request::ListHistoryRecordings(args)).await
    }

    #[tool(
        description = "List built-in query metric definitions and a page of loaded recording channels.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_metrics(&self, Parameters(args): Parameters<ListMetrics>) -> CallToolResult {
        self.inspect(Request::ListMetrics(args)).await
    }

    #[tool(
        description = "List loaded GeoTrace recordings. Use next_after as after for the next page.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_recordings(
        &self,
        Parameters(args): Parameters<ListRecordings>,
    ) -> CallToolResult {
        self.inspect(Request::ListRecordings(args)).await
    }

    #[tool(
        description = "Open local .gtd or .log files by absolute path, or unload recordings by reference. Opening queues the desktop loader. Unloading keeps History records.",
        annotations(
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn manage_recordings(
        &self,
        Parameters(args): Parameters<ManageRecordings>,
    ) -> CallToolResult {
        self.inspect(Request::ManageRecordings(args)).await
    }

    #[tool(
        description = "Open a recording from list_history_recordings. Waits briefly for the result. Poll get_history_open_status if it is pending. Make any track settings choice in GeoTrace.",
        annotations(
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn open_history_recording(
        &self,
        Parameters(args): Parameters<OpenHistoryRecording>,
    ) -> CallToolResult {
        self.inspect(Request::OpenHistoryRecording(args)).await
    }

    #[tool(
        description = "Open or raise History, Log viewer, Plot, Query, Settings, or Sky trails. For Sky trails, supply a current track with satellite positions.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_panel(&self, Parameters(args): Parameters<OpenPanel>) -> CallToolResult {
        self.inspect(Request::OpenPanel(args)).await
    }

    #[tool(
        description = "Center the map on a visible recorded point and open its point information.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_point(&self, Parameters(args): Parameters<OpenPoint>) -> CallToolResult {
        self.inspect(Request::OpenPoint(args)).await
    }

    #[tool(
        description = "Open the Query window and replace the editor text with an editable draft.",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_query(&self, Parameters(args): Parameters<QueryText>) -> CallToolResult {
        self.inspect(Request::OpenQuery(args)).await
    }

    #[tool(
        description = "Open the Settings window at the selected control.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_setting(&self, Parameters(args): Parameters<SettingSelection>) -> CallToolResult {
        self.inspect(Request::OpenSetting(args)).await
    }

    #[tool(
        description = "Run a query in the desktop Query window using the current filter and enabled tracks. Waits briefly for completion. Poll get_query_status if pending. A busy response identifies the active job.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn run_query(&self, Parameters(args): Parameters<QueryText>) -> CallToolResult {
        self.inspect(Request::RunQuery(args)).await
    }

    #[tool(
        description = "Set an Analysis or recording Processing control using its boolean or numeric value in the units from describe_setting. Returns the current setting.",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_setting(&self, Parameters(args): Parameters<SetSetting>) -> CallToolResult {
        self.inspect(Request::SetSetting(args)).await
    }

    #[tool(
        description = "Show or hide loaded track rows, or solo them by enabling only the requested rows. Returns target and changed track counts.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_track_visibility(
        &self,
        Parameters(args): Parameters<SetTrackVisibility>,
    ) -> CallToolResult {
        self.inspect(Request::SetTrackVisibility(args)).await
    }

    #[tool(
        description = "Frame and highlight a track, a half-open range of original point indices, or a retained point-query match on the map. Requires Query highlights.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn show_on_map(&self, Parameters(args): Parameters<ShowOnMap>) -> CallToolResult {
        self.inspect(Request::ShowOnMap(args)).await
    }

    #[tool(
        description = "Open the plot and frame the time range of a track, an original point range, or a retained query match.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn show_on_plot(&self, Parameters(args): Parameters<ShowOnPlot>) -> CallToolResult {
        self.inspect(Request::ShowOnPlot(args)).await
    }

    async fn inspect(&self, request: Request) -> CallToolResult {
        let checking_query = matches!(&request, Request::CheckQuery(_) | Request::RunQuery(_));
        let wait_deadline = matches!(
            request,
            Request::RunQuery(_) | Request::OpenHistoryRecording(_)
        )
        .then(|| Instant::now() + ASYNC_RESULT_WAIT);
        let mut response = self.bridge.request(&request).await;
        if let (Some(deadline), Ok(reply)) = (wait_deadline, &mut response) {
            self.wait_for_result(&request, reply, deadline).await;
        }
        let result = match response {
            Ok(Reply::Error {
                code: ErrorCode::ReplyTooLarge,
            }) if checking_query => {
                return Self::tool_error(
                    "The query diagnostics exceed the reply size limit. Check fewer blocks at a time. For a single oversized diagnostic, check the query in the GeoTrace editor.",
                );
            }
            Ok(Reply::Error { code }) => return Self::tool_error(Self::reply_error_message(code)),
            Ok(reply) if !Self::accepts_success_reply(&request, &reply) => {
                return Self::tool_error("GeoTrace returned an unexpected reply for this tool.");
            }
            Ok(
                reply @ (Reply::FilesQueued { .. }
                | Reply::MapPresentationQueued
                | Reply::PanelOpenQueued { .. }
                | Reply::PlotPresentationQueued
                | Reply::ProcessingApplied
                | Reply::QueryDraftQueued
                | Reply::RecordingsUnloaded { .. }
                | Reply::SettingOpenQueued
                | Reply::Settings { .. }
                | Reply::TrackVisibility(_)),
            ) => serde_json::to_value(reply),
            Ok(Reply::HistoryOpen { operation }) => serde_json::to_value(operation),
            Ok(Reply::HistoryRecordings { page }) => serde_json::to_value(page),
            Ok(Reply::Metrics { page }) => serde_json::to_value(page),
            Ok(Reply::QueryCheck { validation }) => serde_json::to_value(validation),
            Ok(Reply::QueryJob { job }) => serde_json::to_value(job),
            Ok(Reply::QueryResults { results }) => serde_json::to_value(results),
            Ok(Reply::Range { details }) => serde_json::to_value(details),
            Ok(Reply::Recording { details }) => serde_json::to_value(details),
            Ok(Reply::Recordings { page }) => serde_json::to_value(page),
            Ok(Reply::SessionState { state }) => serde_json::to_value(state),
            Ok(Reply::Setting { setting }) => serde_json::to_value(setting),
            Ok(Reply::Track { details }) => serde_json::to_value(details),
            Err(error) => return Self::tool_error(Self::connection_error_message(error.kind())),
        };
        match result {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => {
                Self::tool_error(format!("Could not encode the GeoTrace result ({error})."))
            }
        }
    }

    async fn wait_for_result(&self, start: &Request, latest: &mut Reply, deadline: Instant) {
        while let Some(probe) = Self::pending_status_request(start, latest) {
            let result = tokio::select! {
                biased;
                () = time::sleep_until(deadline) => break,
                result = async {
                    time::sleep(STATUS_POLL_INTERVAL).await;
                    self.bridge.request(&probe).await
                } => result,
            };
            let Ok(reply) = result else {
                break;
            };
            let same_operation = match (probe, &reply) {
                (Request::GetQueryStatus(request), Reply::QueryJob { job }) => {
                    request.job == job.job
                }
                (Request::GetHistoryOpenStatus(request), Reply::HistoryOpen { operation }) => {
                    request.operation == operation.operation
                }
                _ => false,
            };
            if !same_operation {
                break;
            }
            *latest = reply;
        }
    }

    fn pending_status_request(start: &Request, reply: &Reply) -> Option<Request> {
        match (start, reply) {
            (Request::RunQuery(_), Reply::QueryJob { job })
                if matches!(
                    job.state,
                    QueryJobState::Preparing
                        | QueryJobState::Running { .. }
                        | QueryJobState::Cancelling
                ) =>
            {
                Some(Request::GetQueryStatus(QueryJobRequest { job: job.job }))
            }
            (Request::OpenHistoryRecording(_), Reply::HistoryOpen { operation })
                if matches!(
                    operation.state,
                    HistoryOpenState::Reading | HistoryOpenState::Loading
                ) =>
            {
                Some(Request::GetHistoryOpenStatus(GetHistoryOpenStatus {
                    operation: operation.operation,
                }))
            }
            _ => None,
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Each arm checks an independent request and reply pair"
    )]
    fn accepts_success_reply(request: &Request, reply: &Reply) -> bool {
        match request {
            Request::ApplyProcessing(_) => matches!(reply, Reply::ProcessingApplied),
            Request::CancelQuery(_) | Request::GetQueryStatus(_) => {
                matches!(reply, Reply::QueryJob { .. })
            }
            Request::CheckQuery(_) => matches!(reply, Reply::QueryCheck { .. }),
            Request::DescribeSetting(_) | Request::SetSetting(_) => {
                matches!(reply, Reply::Setting { .. })
            }
            Request::GetHistoryOpenStatus(_) | Request::OpenHistoryRecording(_) => {
                matches!(reply, Reply::HistoryOpen { .. })
            }
            Request::GetQueryResults(_) => matches!(reply, Reply::QueryResults { .. }),
            Request::GetRangeDetails(_) => matches!(reply, Reply::Range { .. }),
            Request::GetRecording(_) => matches!(reply, Reply::Recording { .. }),
            Request::GetSessionState => matches!(reply, Reply::SessionState { .. }),
            Request::GetSettings => matches!(reply, Reply::Settings { .. }),
            Request::GetTrack(_) => matches!(reply, Reply::Track { .. }),
            Request::ListHistoryRecordings(_) => matches!(reply, Reply::HistoryRecordings { .. }),
            Request::ListMetrics(_) => matches!(reply, Reply::Metrics { .. }),
            Request::ListRecordings(_) => matches!(reply, Reply::Recordings { .. }),
            Request::ManageRecordings(ManageRecordings::Open { .. }) => {
                matches!(reply, Reply::FilesQueued { .. })
            }
            Request::ManageRecordings(ManageRecordings::Unload { .. }) => {
                matches!(reply, Reply::RecordingsUnloaded { .. })
            }
            Request::OpenPanel(args) => {
                matches!(reply, Reply::PanelOpenQueued { panel } if *panel == args.panel())
            }
            Request::OpenPoint(_) | Request::ShowOnMap(_) => {
                matches!(reply, Reply::MapPresentationQueued)
            }
            Request::OpenQuery(_) => matches!(reply, Reply::QueryDraftQueued),
            Request::OpenSetting(_) => matches!(reply, Reply::SettingOpenQueued),
            Request::RunQuery(_) => {
                matches!(reply, Reply::QueryCheck { .. } | Reply::QueryJob { .. })
            }
            Request::SetTrackVisibility(_) => matches!(reply, Reply::TrackVisibility(_)),
            Request::ShowOnPlot(_) => matches!(reply, Reply::PlotPresentationQueued),
        }
    }

    fn reply_error_message(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::Busy => "GeoTrace is busy. Try again.",
            ErrorCode::HistoryListingFailed => {
                "GeoTrace could not list recording History. Check the History window for details."
            }
            ErrorCode::HistoryNotReady => "GeoTrace is preparing recording History. Try again.",
            ErrorCode::HistoryOpenFailed => "GeoTrace could not open the History recording.",
            ErrorCode::HistoryOpenNotFound => "This History open operation is no longer available.",
            ErrorCode::HistorySelectionExpired => {
                "List History again and choose a current recording."
            }
            ErrorCode::HistorySelectionRequiresDesktop => {
                "Open this older History recording in the GeoTrace History window."
            }
            ErrorCode::HistoryTrackLayoutUnavailable => {
                "GeoTrace could not reproduce the stored track layout."
            }
            ErrorCode::HistoryUnavailable => "GeoTrace has no open recording History database.",
            ErrorCode::InactiveSetting => {
                "This setting is disabled. Use describe_setting for the prerequisite."
            }
            ErrorCode::InvalidHistoryCursor => {
                "Start from the first page of History after changing dates or when the list refreshes."
            }
            ErrorCode::InvalidHistoryDateRange => {
                "Choose a History date range in order from earliest to latest."
            }
            ErrorCode::InvalidLocalFilePath => "Provide an absolute local file path.",
            ErrorCode::InvalidPlotMetric => {
                "Use metric identifiers from list_metrics that have plot series, such as velocity or eph."
            }
            ErrorCode::InvalidPoint => "The point index is outside the track.",
            ErrorCode::InvalidQueryCursor => "The query, match or row index is outside the result.",
            ErrorCode::InvalidRange => "Choose an ordered point range within the track.",
            ErrorCode::InvalidRequest => {
                "GeoTrace could not read the request. Use the companion from the same release."
            }
            ErrorCode::InvalidSettingType => {
                "Use the boolean or numeric value type from describe_setting."
            }
            ErrorCode::InvalidSettingValue => {
                "Use a finite value that meets the limits and precision from describe_setting."
            }
            ErrorCode::NoLoadedNavPoints => {
                "Load a recording with navigation points before applying processing."
            }
            ErrorCode::NoSkyTrailReports => {
                "The track has no satellite azimuth and elevation reports for Sky trails."
            }
            ErrorCode::NoVisibleMapGeometry => {
                "The selected points have no visible geometry under the current map settings."
            }
            ErrorCode::NoVisiblePlotTime => {
                "The selected range has no visible plot timestamps under the current filter and track visibility."
            }
            ErrorCode::QueryColumnsTooLarge => "Query result columns exceed the page size limit.",
            ErrorCode::QueryHighlightsDisabled => {
                "Enable Query highlights in the map's visibility controls before showing a range."
            }
            ErrorCode::QueryJobNotFound => {
                "GeoTrace no longer has that query job. Start the query again."
            }
            ErrorCode::QueryMatchIsChannel => {
                "Choose a match from a point query to frame it on the map."
            }
            ErrorCode::QueryNotComplete => "The query is still in progress. Poll its status.",
            ErrorCode::QueryResultsUnavailable => {
                "This query job has no readable results. Check its status or run the query again."
            }
            ErrorCode::RecordingNotFound => {
                "The recording is no longer loaded. List the loaded recordings again."
            }
            ErrorCode::ReplyTooLarge => "The reply is too large. Use a smaller page limit.",
            ErrorCode::RequestTooLarge => {
                "The request is too large. Shorten the request and try again."
            }
            ErrorCode::ShuttingDown => "GeoTrace is closing.",
            ErrorCode::StaleQueryResults => "The query results are stale. Run the query again.",
            ErrorCode::StaleSession => {
                "The desktop session has changed. Use references from the current session."
            }
            ErrorCode::StaleTrack => {
                "The track reference is stale. List the recording's tracks from the beginning."
            }
            ErrorCode::UnsupportedFileExtension => "Choose a .gtd recording or a .log file.",
        }
    }

    fn connection_error_message(kind: ErrorKind) -> String {
        match kind {
            ErrorKind::ConnectionRefused | ErrorKind::NotFound => START_DESKTOP.to_owned(),
            ErrorKind::FileTooLarge => Self::reply_error_message(ErrorCode::RequestTooLarge).to_owned(),
            ErrorKind::PermissionDenied => {
                "The GeoTrace session denied access. Run GeoTrace and its companion as the same user."
                    .to_owned()
            }
            ErrorKind::TimedOut => "GeoTrace did not respond in time. Try again.".to_owned(),
            kind => format!("Could not contact GeoTrace ({kind})."),
        }
    }

    fn tool_error(message: impl Into<String>) -> CallToolResult {
        CallToolResult::error(vec![ContentBlock::text(message)])
    }
}

#[tool_handler]
impl ServerHandler for Companion {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let cancellation = context.ct.clone();
        let router = Self::tool_router();
        let call = ToolCallContext::new(self, request, context);
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                // `rmcp` suppresses responses for cancelled requests.
                Ok(Self::tool_error("The MCP request was cancelled.").into())
            }
            result = router.call(call) => result,
        }
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(COMPANION_NAME, VERSION))
        .with_instructions(format!(
            "Read {QUERY_LANGUAGE_URI} for query syntax and examples."
        ))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let reference = Resource::new(QUERY_LANGUAGE_URI, "query-language")
            .with_title("GeoTrace query language")
            .with_description("Query syntax, operators, units and examples")
            .with_mime_type(QUERY_LANGUAGE_MIME_TYPE);
        Ok(ListResourcesResult::with_all_items(vec![reference]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != QUERY_LANGUAGE_URI {
            return Err(ErrorData::resource_not_found("Resource not found", None));
        }
        let reference = ResourceContents::text(gt_query::query_language_reference(), request.uri)
            .with_mime_type(QUERY_LANGUAGE_MIME_TYPE);
        Ok(ReadResourceResult::new(vec![reference]).into())
    }
}

enum Bridge {
    Desktop,
    #[cfg(test)]
    Fixture(fn(&Request) -> io::Result<Reply>),
    #[cfg(test)]
    Pending(tests::PendingBridge),
}

impl Bridge {
    async fn request(&self, request: &Request) -> io::Result<Reply> {
        match self {
            Self::Desktop => gt_session_bridge::request(request).await,
            #[cfg(test)]
            Self::Fixture(query) => query(request),
            #[cfg(test)]
            Self::Pending(bridge) => bridge.request(request).await,
        }
    }
}

const COMPANION_NAME: &str = "geotrace-mcp";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const QUERY_LANGUAGE_URI: &str = "geotrace://query-language";
const ASYNC_RESULT_WAIT: Duration = Duration::from_secs(1);
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const QUERY_LANGUAGE_MIME_TYPE: &str = "text/markdown";
const START_DESKTOP: &str = "Start GeoTrace with --session-bridge and try again.";

#[cfg(test)]
mod tests;
