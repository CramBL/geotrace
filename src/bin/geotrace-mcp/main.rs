use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};

use gt_session_bridge::{
    ErrorCode, GetQueryResults, GetRangeDetails, GetRecording, GetTrack, ListMetrics,
    ListRecordings, OpenPoint, QueryJobRequest, QueryText, Reply, Request, SetSetting,
    SettingSelection, ShowOnMap, ShowOnPlot,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::QuitReason;
use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router, transport};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
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

#[derive(Clone, Copy)]
struct Companion {
    bridge: Bridge,
}

#[tool_router]
impl Companion {
    #[tool(
        description = "Apply the current Processing settings to loaded recordings with navigation points. GeoTrace acknowledges the request with processing_queued.",
        annotations(
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn apply_processing(&self) -> CallToolResult {
        self.inspect(Request::ApplyProcessing).await
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
        description = "Inspect a loaded recording and its track references. Use next_after as after for the next page.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_recording(&self, Parameters(args): Parameters<GetRecording>) -> CallToolResult {
        self.inspect(Request::GetRecording(args)).await
    }

    #[tool(
        description = "List Analysis settings and recording Processing settings with their limits and help.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_settings(&self) -> CallToolResult {
        self.inspect(Request::GetSettings).await
    }

    #[tool(
        description = "Read a loaded track's time range, counts, distance and map bounds.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_track(&self, Parameters(args): Parameters<GetTrack>) -> CallToolResult {
        self.inspect(Request::GetTrack(args)).await
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
        description = "Start a background query using the current filter and enabled tracks. A busy response includes the active job ID. Poll the job with get_query_status.",
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
        description = "Frame and highlight a track, a half-open range of original point indices, or a retained point-query match on the map.",
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
        let result = match self.bridge.request(request).await {
            Ok(
                reply @ (Reply::MapPresentationQueued
                | Reply::PlotPresentationQueued
                | Reply::ProcessingQueued
                | Reply::QueryDraftQueued
                | Reply::SettingOpenQueued
                | Reply::Settings { .. }),
            ) => serde_json::to_value(reply),
            Ok(Reply::Metrics { page }) => serde_json::to_value(page),
            Ok(Reply::QueryCheck { validation }) => serde_json::to_value(validation),
            Ok(Reply::QueryJob { job }) => serde_json::to_value(job),
            Ok(Reply::QueryResults { results }) => serde_json::to_value(results),
            Ok(Reply::Range { details }) => serde_json::to_value(details),
            Ok(Reply::Recording { details }) => serde_json::to_value(details),
            Ok(Reply::Recordings { page }) => serde_json::to_value(page),
            Ok(Reply::Setting { setting }) => serde_json::to_value(setting),
            Ok(Reply::Track { details }) => serde_json::to_value(details),
            Ok(Reply::Error {
                code: ErrorCode::ReplyTooLarge,
            }) if checking_query => {
                return Self::tool_error(
                    "The query diagnostics exceed the reply size limit. Check fewer blocks at a time. For a single oversized diagnostic, check the query in the GeoTrace editor.",
                );
            }
            Ok(Reply::Error { code }) => return Self::tool_error(Self::reply_error_message(code)),
            Err(error) => return Self::tool_error(Self::connection_error_message(error.kind())),
        };
        match result {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => {
                Self::tool_error(format!("Could not encode the GeoTrace result ({error})."))
            }
        }
    }

    fn reply_error_message(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::Busy => "GeoTrace is busy. Try again.",
            ErrorCode::InactiveSetting => {
                "This setting is disabled. Use describe_setting for the prerequisite."
            }
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
            ErrorCode::NoVisibleMapGeometry => {
                "The selected points have no visible geometry under the current map settings."
            }
            ErrorCode::NoVisiblePlotTime => {
                "The selected range has no visible plot timestamps under the current filter and track visibility."
            }
            ErrorCode::QueryColumnsTooLarge => "Query result columns exceed the page size limit.",
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
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(COMPANION_NAME, VERSION))
    }
}

#[derive(Clone, Copy)]
enum Bridge {
    Desktop,
    #[cfg(test)]
    Fixture(fn(Request) -> io::Result<Reply>),
}

impl Bridge {
    async fn request(self, request: Request) -> io::Result<Reply> {
        match self {
            Self::Desktop => gt_session_bridge::request(request).await,
            #[cfg(test)]
            Self::Fixture(query) => query(request),
        }
    }
}

const COMPANION_NAME: &str = "geotrace-mcp";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const START_DESKTOP: &str = "Start GeoTrace with --session-bridge and try again.";

#[cfg(test)]
mod tests;
