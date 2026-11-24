use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};

use gt_session_bridge::{
    ErrorCode, GetRangeDetails, GetRecording, GetTrack, ListRecordings, Reply, Request,
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
        description = "Read a loaded track's time range, counts, distance and map bounds.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_track(&self, Parameters(args): Parameters<GetTrack>) -> CallToolResult {
        self.inspect(Request::GetTrack(args)).await
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

    async fn inspect(&self, request: Request) -> CallToolResult {
        let result = match self.bridge.request(request).await {
            Ok(Reply::Range { details }) => serde_json::to_value(details),
            Ok(Reply::Recording { details }) => serde_json::to_value(details),
            Ok(Reply::Recordings { page }) => serde_json::to_value(page),
            Ok(Reply::Track { details }) => serde_json::to_value(details),
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
            ErrorCode::InvalidRange => "Choose an ordered point range within the track.",
            ErrorCode::InvalidRequest | ErrorCode::RequestTooLarge => {
                "GeoTrace could not read the request. Use the companion from the same release."
            }
            ErrorCode::RecordingNotFound => {
                "The recording is no longer loaded. List the loaded recordings again."
            }
            ErrorCode::ReplyTooLarge => "The reply is too large. Use a smaller page limit.",
            ErrorCode::StaleSession => {
                "The desktop session has changed. List the loaded recordings from the beginning."
            }
            ErrorCode::StaleTrack => {
                "The track reference is stale. List the recording's tracks from the beginning."
            }
        }
    }

    fn connection_error_message(kind: ErrorKind) -> String {
        match kind {
            ErrorKind::ConnectionRefused | ErrorKind::NotFound => START_DESKTOP.to_owned(),
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
