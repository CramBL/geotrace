use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::num::NonZeroUsize;

use gt_loaded_files::MAX_RECORDINGS_PER_PAGE;
use gt_session_bridge::{ErrorCode, Reply, Request};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::JsonSchema;
use rmcp::service::QuitReason;
use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router, transport};
use serde::Deserialize;

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
        description = "List loaded GeoTrace recordings. Use next_after as after for the next page.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_recordings(
        &self,
        Parameters(args): Parameters<ListRecordingsArgs>,
    ) -> CallToolResult {
        let request = Request::ListRecordings {
            after: args.after,
            limit: args.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        };
        let problem = match self.bridge.request(request).await {
            Ok(Reply::Recordings { page }) => match serde_json::to_value(page) {
                Ok(value) => return CallToolResult::structured(value),
                Err(error) => format!("Could not encode the recording list ({error})."),
            },
            Ok(Reply::Error { code }) => match code {
                ErrorCode::Busy => "GeoTrace is busy. Try again.".to_owned(),
                ErrorCode::InvalidRequest | ErrorCode::RequestTooLarge => {
                    "GeoTrace could not read the request. Use the companion from the same release."
                        .to_owned()
                }
                ErrorCode::ReplyTooLarge => "The recording page is too large. Use a smaller limit.".to_owned(),
            },
            Err(error) => match error.kind() {
                ErrorKind::ConnectionRefused | ErrorKind::NotFound => START_DESKTOP.to_owned(),
                ErrorKind::PermissionDenied => {
                    "The GeoTrace session denied access. Run GeoTrace and its companion as the same user."
                        .to_owned()
                }
                ErrorKind::TimedOut => "GeoTrace did not respond in time. Try again.".to_owned(),
                kind => format!("Could not contact GeoTrace ({kind})."),
            },
        };
        CallToolResult::error(vec![ContentBlock::text(problem)])
    }
}

#[tool_handler]
impl ServerHandler for Companion {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(COMPANION_NAME, VERSION))
    }
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct ListRecordingsArgs {
    /// Cursor from the previous page in this desktop session.
    after: Option<u64>,
    /// Requested page size. The desktop app caps the number of recordings per page.
    limit: Option<NonZeroUsize>,
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
const DEFAULT_PAGE_LIMIT: NonZeroUsize = match NonZeroUsize::new(MAX_RECORDINGS_PER_PAGE) {
    Some(limit) => limit,
    None => NonZeroUsize::MIN,
};
const START_DESKTOP: &str = "Start GeoTrace with --session-bridge and try again.";

#[cfg(test)]
mod tests;
