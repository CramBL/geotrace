//! Send a request plan to a Valhalla server and classify the outcomes.
//!
//! [`gt_fetch::Transport`] sends requests over the network. `classify` below
//! maps an [`gt_fetch::HttpResponse`] to a [`ChunkOutcome`] for one chunk of a
//! snap plan.
//!
//! [`send_plan`] drives one chunk at a time, in order:
//!
//! - Transport-level failures and server errors (HTTP 5xx) get one retry:
//!   they may be transient. Any 4xx is deterministic and is never retried.
//!   Retrying would spend fair-use budget for the same result.
//! - The server's error 444 (off-network) is a valid result, not a failure:
//!   it becomes [`ChunkOutcome::OffNetwork`].
//! - Error bodies are not always JSON (the reverse proxy returns an HTML 413
//!   page for oversized requests). Anything unparsable is a failure
//!   holding the raw status.

use gt_fetch::{Classified, HttpRequest, HttpResponse, Transport};

use crate::TRACE_ATTRIBUTES_PATH;
use crate::request_plan::{RequestPlan, SnapParams};
use crate::stitch::ChunkOutcome;
use crate::wire::{ErrorCode, ErrorResponse, TraceAttributesRequest, TraceAttributesResponse};

/// Send every chunk of a plan, in order, to `server_url` (e.g.
/// [`crate::DEFAULT_SERVER_URL`]), classifying each into a [`ChunkOutcome`].
/// `progress` is called after each chunk completes with (completed, total).
///
/// Never fails as a whole: per-chunk failures become
/// [`ChunkOutcome::Failed`] and stitching handles the gaps.
pub fn send_plan(
    transport: &impl Transport,
    server_url: &str,
    plan: &RequestPlan,
    params: &SnapParams,
    mut progress: impl FnMut(usize, usize),
) -> Vec<ChunkOutcome> {
    let url = format!("{server_url}{TRACE_ATTRIBUTES_PATH}");
    let total = plan.chunks.len();
    let mut outcomes = Vec::with_capacity(total);
    for (index, chunk) in plan.chunks.iter().enumerate() {
        let request = chunk.request(params, plan.gps_accuracy_m);
        outcomes.push(send_chunk(transport, &url, &request));
        progress(index + 1, total);
    }
    outcomes
}

/// Send one chunk with retry-on-transient-failure semantics.
fn send_chunk(
    transport: &impl Transport,
    url: &str,
    request: &TraceAttributesRequest,
) -> ChunkOutcome {
    let body = match serde_json::to_string(request) {
        Ok(body) => body,
        Err(err) => return ChunkOutcome::Failed(format!("serializing the request: {err}")),
    };
    let request = HttpRequest::post_json(url, body);
    gt_fetch::send_classified(
        transport,
        &request,
        |response| classify(&response),
        ChunkOutcome::Failed,
    )
}

fn classify(response: &HttpResponse) -> Classified<ChunkOutcome> {
    if !response.status_is_valid() {
        return Classified::Outcome(ChunkOutcome::Failed(format!(
            "invalid HTTP status {}",
            response.status
        )));
    }
    if response.is_success() {
        return match serde_json::from_str::<TraceAttributesResponse>(&response.body) {
            Ok(parsed) => Classified::Outcome(ChunkOutcome::Success(parsed)),
            Err(err) => Classified::Outcome(ChunkOutcome::Failed(format!(
                "HTTP {} with unparsable success body: {err}",
                response.status_line()
            ))),
        };
    }
    if response.is_server_error() {
        return Classified::Transient(format!("HTTP {}", response.status_line()));
    }
    match serde_json::from_str::<ErrorResponse>(&response.body) {
        Ok(error) if error.error_code == ErrorCode::OffNetwork => {
            Classified::Outcome(ChunkOutcome::OffNetwork)
        }
        Ok(error) => Classified::Outcome(ChunkOutcome::Failed(format!(
            "server error {}: {}",
            u32::from(error.error_code),
            error.error
        ))),
        // Not JSON: e.g. the reverse proxy's HTML 413 page.
        Err(_) => Classified::Outcome(ChunkOutcome::Failed(format!(
            "HTTP {} with non-JSON error body",
            response.status
        ))),
    }
}
