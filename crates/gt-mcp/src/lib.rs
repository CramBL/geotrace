use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};

use gt_session_bridge::{
    ApplyProcessing, ErrorCode, GetHistoryOpenStatus, GetQueryResults, GetQueryStatus,
    GetRangeDetails, GetRecording, GetTrack, ListHistoryRecordings, ListMetrics, ListRecordings,
    ManageRecordings, OpenHistoryRecording, OpenPanel, OpenPoint, PanelOpenState, QueryJobRequest,
    QueryText, RangeTarget, Reply, Request, RequestError, SessionPanel, SetSessionView, SetSetting,
    SetTrackVisibility, SettingSelection, ShowOnMap, ShowOnPlot,
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
        description = "Read the status of a History open. With wait_ms, wait for completion or a desktop decision. A ready result includes the loaded recording ID.",
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
        description = "Read the status or summary of a query job returned by run_query. With wait_ms, wait for completion up to the given duration.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_query_status(
        &self,
        Parameters(args): Parameters<GetQueryStatus>,
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
        description = "List stored History recordings by navigation start time, newest first. Entries without a stored time range appear last. Bounds use RFC 3339 UTC. Filter by overlapping UTC dates and use next_cursor with the same dates to continue. Waits briefly while History prepares its initial listing.",
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
        description = "Open local .gtd or .log files by absolute path, or unload recordings by reference. Opening waits briefly and reports each path's outcome. Unloading keeps History records.",
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
        description = "Open History, Log viewer, Plot, Query, Settings, or Sky trails and report its placement in GeoTrace. For Sky trails, supply a current track with satellite positions.",
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
        description = "Center the map on a visible recorded point and open its point information. Returns the point GeoTrace opened.",
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
        description = "Open Settings on the page for a setting key. GeoTrace scrolls to the control after pending input and scrolling finish.",
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
        description = "Change the map base layer, map category visibility, or global UTC time filter. Returns the updated session state.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_session_view(
        &self,
        Parameters(args): Parameters<SetSessionView>,
    ) -> CallToolResult {
        self.inspect(Request::SetSessionView(args)).await
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
        description = "Frame and highlight a track, point range or retained point-query match on the map. Returns the track and half-open point range. Requires Query highlights.",
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
        description = "Open the plot and frame a track, an original point range, or a retained query match. Returns the framed sample time range after GeoTrace updates the plot.",
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
        let response = self.bridge.request(&request).await;
        let panel_open_failed =
            matches!(&response, Ok(Reply::PanelOpen { state, .. }) if !state.succeeded());
        let result = match response {
            Ok(Reply::Error {
                code: ErrorCode::ReplyTooLarge,
            }) if checking_query => {
                return Self::tool_error(
                    "The query diagnostics exceed the reply size limit. Check fewer blocks at a time. For a single oversized diagnostic, check the query in the GeoTrace editor.",
                );
            }
            Ok(Reply::Error { code }) => return Self::tool_error(Self::reply_error_message(code)),
            Ok(reply) if !Self::accepts_reply(&request, &reply) => {
                return Self::tool_error("GeoTrace returned an unexpected reply for this tool.");
            }
            Ok(
                reply @ (Reply::FilesOpened { .. }
                | Reply::MapPointOpened(_)
                | Reply::MapRangeFramed { .. }
                | Reply::PanelOpen { .. }
                | Reply::PlotFramed { .. }
                | Reply::ProcessingApplied
                | Reply::QueryDraftOpened
                | Reply::RecordingsUnloaded { .. }
                | Reply::SettingOpened { .. }
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
            Err(error) => {
                return Self::tool_error(Self::connection_error_message(&request, &error));
            }
        };
        match result {
            Ok(value) if panel_open_failed => CallToolResult::structured_error(value),
            Ok(value) => CallToolResult::structured(value),
            Err(error) => {
                Self::tool_error(format!("Could not encode the GeoTrace result ({error})."))
            }
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Each arm checks an independent request and reply pair"
    )]
    fn accepts_reply(request: &Request, reply: &Reply) -> bool {
        match request {
            Request::ApplyProcessing(_) => matches!(reply, Reply::ProcessingApplied),
            Request::CancelQuery(args) => {
                matches!(reply, Reply::QueryJob { job } if job.job == args.job)
            }
            Request::CheckQuery(_) => matches!(reply, Reply::QueryCheck { .. }),
            Request::DescribeSetting(_) | Request::SetSetting(_) => {
                matches!(reply, Reply::Setting { .. })
            }
            Request::GetHistoryOpenStatus(args) => {
                matches!(reply, Reply::HistoryOpen { operation } if operation.operation == args.operation)
            }
            Request::GetQueryStatus(args) => {
                matches!(reply, Reply::QueryJob { job } if job.job == args.job)
            }
            Request::OpenHistoryRecording(_) => {
                matches!(reply, Reply::HistoryOpen { .. })
            }
            Request::GetQueryResults(_) => matches!(reply, Reply::QueryResults { .. }),
            Request::GetRangeDetails(_) => matches!(reply, Reply::Range { .. }),
            Request::GetRecording(_) => matches!(reply, Reply::Recording { .. }),
            Request::GetSessionState | Request::SetSessionView(_) => {
                matches!(reply, Reply::SessionState { .. })
            }
            Request::GetSettings => matches!(reply, Reply::Settings { .. }),
            Request::GetTrack(_) => matches!(reply, Reply::Track { .. }),
            Request::ListHistoryRecordings(_) => matches!(reply, Reply::HistoryRecordings { .. }),
            Request::ListMetrics(_) => matches!(reply, Reply::Metrics { .. }),
            Request::ListRecordings(_) => matches!(reply, Reply::Recordings { .. }),
            Request::ManageRecordings(ManageRecordings::Open { .. }) => {
                matches!(reply, Reply::FilesOpened { .. })
            }
            Request::ManageRecordings(ManageRecordings::Unload { .. }) => {
                matches!(reply, Reply::RecordingsUnloaded { .. })
            }
            Request::OpenPanel(args) => {
                matches!(reply, Reply::PanelOpen { panel, state }
                    if *panel == args.panel() && (*panel == SessionPanel::Plot) == (*state == PanelOpenState::Docked))
            }
            Request::OpenPoint(args) => {
                matches!(reply, Reply::MapPointOpened(point) if point == args)
            }
            Request::ShowOnMap(args) => match (&args.target, reply) {
                (
                    RangeTarget::Track { track, start, end },
                    Reply::MapRangeFramed {
                        track: framed_track,
                        start: framed_start,
                        end: framed_end,
                    },
                ) => {
                    track == framed_track
                        && *framed_start == start.unwrap_or_default()
                        && end.is_none_or(|end| end == *framed_end)
                        && framed_start < framed_end
                }
                (
                    RangeTarget::QueryMatch { job, .. },
                    Reply::MapRangeFramed { track, start, end },
                ) => track.recording.session == job.session && start < end,
                _ => false,
            },
            Request::OpenQuery(_) => matches!(reply, Reply::QueryDraftOpened),
            Request::OpenSetting(args) => {
                matches!(reply, Reply::SettingOpened { key } if *key == args.key)
            }
            Request::RunQuery(_) => {
                matches!(reply, Reply::QueryCheck { .. } | Reply::QueryJob { .. })
            }
            Request::SetTrackVisibility(_) => matches!(reply, Reply::TrackVisibility(_)),
            Request::ShowOnPlot(args) => match (&args.target, reply) {
                (RangeTarget::Track { track, .. }, Reply::PlotFramed { track: framed, .. }) => {
                    track == framed
                }
                (RangeTarget::QueryMatch { job, .. }, Reply::PlotFramed { track, .. }) => {
                    track.recording.session == job.session
                }
                _ => false,
            },
        }
    }

    fn reply_error_message(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::Busy => "GeoTrace is busy. Try again.",
            ErrorCode::DuplicateDisplayCategory => "Specify each map display category once.",
            ErrorCode::EmptyLogFile => "The log file is empty.",
            ErrorCode::FileNotFound => "The file does not exist.",
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
            ErrorCode::InvalidTimeFilter => {
                "Set at least one UTC bound. The start must be at or before the end."
            }
            ErrorCode::MapboxTokenRequired => {
                "Enter a Mapbox token in Settings to use the satellite layer."
            }
            ErrorCode::MapPresentationStale => {
                "The recordings, time filter or query changed before the map could apply the request."
            }
            ErrorCode::MapPresentationSuperseded => "A later map action superseded the request.",
            ErrorCode::MapPresentationTimedOut => {
                "The map could not apply the request in time. Show the map and try again."
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
            ErrorCode::PlotPresentationStale => {
                "The plot did not frame the range because the recordings, filter, visibility or query changed. The request may have opened the plot."
            }
            ErrorCode::PlotPresentationSuperseded => {
                "A later plot action superseded the requested framing. The request may have opened the plot."
            }
            ErrorCode::PlotPresentationTimedOut => {
                "GeoTrace could not frame the range in time. The request may have opened the plot. Show the plot and try again."
            }
            ErrorCode::PlotTimeUnrepresentable => {
                "Choose a plot range with both timestamps in RFC 3339 years 0000 to 9999."
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
            ErrorCode::UnreadableFile => "GeoTrace could not read the file.",
            ErrorCode::UnrecognizedLogFormat => "GeoTrace did not recognize the log format.",
            ErrorCode::UnsupportedFileExtension => "Choose a .gtd recording or a .log file.",
            ErrorCode::UnsupportedRecording => "GeoTrace could not read the recording format.",
        }
    }

    fn connection_error_message(request: &Request, error: &RequestError) -> String {
        match error {
            RequestError::BeforeSend(error) => match error.kind() {
                ErrorKind::ConnectionRefused | ErrorKind::NotFound => START_DESKTOP.to_owned(),
                ErrorKind::FileTooLarge => {
                    Self::reply_error_message(ErrorCode::RequestTooLarge).to_owned()
                }
                ErrorKind::PermissionDenied => {
                    "The GeoTrace session denied access. Run GeoTrace and its companion as the same user."
                        .to_owned()
                }
                ErrorKind::TimedOut => {
                    "Could not connect to GeoTrace in time. Try again.".to_owned()
                }
                kind => format!("Could not contact GeoTrace ({kind})."),
            },
            RequestError::DuringExchange(error) => {
                let Some(guidance) = Self::uncertain_exchange_guidance(request) else {
                    return match error.kind() {
                        ErrorKind::TimedOut => {
                            "GeoTrace did not respond in time. Try again.".to_owned()
                        }
                        kind => format!("Lost the connection to GeoTrace ({kind}). Try again."),
                    };
                };
                match error.kind() {
                    ErrorKind::TimedOut => format!("GeoTrace did not respond in time. {guidance}"),
                    kind => format!(
                        "Lost the connection to GeoTrace ({kind}) after sending the request. {guidance}"
                    ),
                }
            }
        }
    }

    fn uncertain_exchange_guidance(request: &Request) -> Option<&'static str> {
        match request {
            Request::CheckQuery(_)
            | Request::DescribeSetting(_)
            | Request::GetHistoryOpenStatus(_)
            | Request::GetQueryResults(_)
            | Request::GetQueryStatus(_)
            | Request::GetRangeDetails(_)
            | Request::GetRecording(_)
            | Request::GetSessionState
            | Request::GetSettings
            | Request::GetTrack(_)
            | Request::ListHistoryRecordings(_)
            | Request::ListMetrics(_)
            | Request::ListRecordings(_) => None,
            Request::CancelQuery(_)
            | Request::OpenPanel(_)
            | Request::OpenPoint(_)
            | Request::OpenQuery(_)
            | Request::OpenSetting(_)
            | Request::SetSessionView(_)
            | Request::SetSetting(_)
            | Request::SetTrackVisibility(_)
            | Request::ShowOnMap(_)
            | Request::ShowOnPlot(_) => Some(
                "GeoTrace may have applied the change. Inspecting state or retrying the same call is safe.",
            ),
            Request::ApplyProcessing(_) | Request::ManageRecordings(_) => Some(
                "GeoTrace may have already started or applied the request. Check list_recordings or get_session_state before retrying.",
            ),
            Request::OpenHistoryRecording(_) => Some(
                "GeoTrace may have already started opening the recording. Check list_recordings, or retry open_history_recording if it is still pending.",
            ),
            Request::RunQuery(_) => Some(
                "GeoTrace may have already started the query. Retry run_query to reclaim an active job or start a new one if it already finished.",
            ),
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
    Fixture(fn(&Request) -> Result<Reply, RequestError>),
    #[cfg(test)]
    Pending(tests::PendingBridge),
}

impl Bridge {
    async fn request(&self, request: &Request) -> Result<Reply, RequestError> {
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
const QUERY_LANGUAGE_MIME_TYPE: &str = "text/markdown";
const START_DESKTOP: &str = "Start MCP in GeoTrace Settings and try again.";

#[cfg(test)]
mod tests;
