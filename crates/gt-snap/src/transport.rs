//! Send a request plan to a Valhalla server and classify the outcomes.
//!
//! The [`Transport`] trait is the seam between the pure pipeline and the
//! network. Which one is in use is the application's choice, made once at
//! startup and supplied as a [`TransportSource`]. Nothing here reads the
//! process environment.
//!
//! [`send_plan`] drives one chunk at a time, in order:
//!
//! - Transport-level failures and server errors (HTTP 5xx) get one retry:
//!   they may be transient. Any 4xx is deterministic and is never retried.
//!   Retrying would spend fair-use budget for the same answer.
//! - The server's error 444 (off-network) is a valid result, not a failure:
//!   it becomes [`ChunkOutcome::OffNetwork`].
//! - Error bodies are not always JSON (the reverse proxy answers oversized
//!   requests with an HTML 413 page). Anything unparsable is a failure
//!   holding the raw status.

use std::time::Instant;

use parking_lot::Mutex;

use crate::request_plan::{RequestPlan, SnapParams};
use crate::stitch::ChunkOutcome;
use crate::wire::{ErrorCode, ErrorResponse, TraceAttributesRequest, TraceAttributesResponse};
use crate::{CLIENT_ID_HEADER, REQUEST_INTERVAL, TRACE_ATTRIBUTES_PATH};

/// Retries per failed chunk send (transient failures only).
const RETRIES: usize = 1;

/// Timeout per request. Matching a 1000-point chunk is server-side work, so
/// this is generous.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// An HTTP response as the classifier needs it: status and raw body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// A failure below the HTTP layer (connection, timeout, TLS, ...).
#[derive(Debug, thiserror::Error)]
#[error("request failed: {detail}")]
pub struct TransportError {
    pub detail: String,
}

/// Fails every request with [`gt_types::env::OFFLINE_DETAIL`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OfflineTransport;

impl Transport for OfflineTransport {
    fn send(&self, _request: &TraceAttributesRequest) -> Result<HttpResponse, TransportError> {
        Err(TransportError {
            detail: gt_types::env::OFFLINE_DETAIL.to_owned(),
        })
    }
}

/// The transport in use, picked by [`TransportSource`].
pub enum Connection {
    Http(HttpTransport),
    Offline(OfflineTransport),
}

impl Transport for Connection {
    fn send(&self, request: &TraceAttributesRequest) -> Result<HttpResponse, TransportError> {
        match self {
            Self::Http(transport) => transport.send(request),
            Self::Offline(transport) => transport.send(request),
        }
    }
}

/// Which transport the application runs with, decided once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportSource {
    Network,
    Offline,
}

impl TransportSource {
    /// Called again whenever the server changes: each server gets its own
    /// connection pool and rate limiter.
    pub fn connect(self, server_url: &str) -> Result<Connection, TransportError> {
        match self {
            Self::Network => Ok(Connection::Http(HttpTransport::new(server_url)?)),
            Self::Offline => Ok(Connection::Offline(OfflineTransport)),
        }
    }
}

/// The seam between the snap pipeline and the network.
pub trait Transport {
    /// Send one `trace_attributes` request and return the raw response.
    ///
    /// Implementations own pacing (rate limits): a call may block until the
    /// server may be contacted again.
    fn send(&self, request: &TraceAttributesRequest) -> Result<HttpResponse, TransportError>;
}

/// Blocking production transport: reqwest against a Valhalla server, paced
/// to at most one request per [`REQUEST_INTERVAL`] (the FOSSGIS fair-use
/// limit), identifying itself via [`CLIENT_ID_HEADER`].
pub struct HttpTransport {
    client: reqwest::blocking::Client,
    url: String,
    /// Completion time of the most recent send. Pacing sleeps until one
    /// [`REQUEST_INTERVAL`] past it.
    last_send: Mutex<Option<Instant>>,
}

impl HttpTransport {
    /// A transport against `server_url` (e.g. [`crate::DEFAULT_SERVER_URL`]).
    pub fn new(server_url: &str) -> Result<Self, TransportError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| TransportError {
                detail: format!("{err:#}"),
            })?;
        Ok(Self {
            client,
            url: format!("{server_url}{TRACE_ATTRIBUTES_PATH}"),
            last_send: Mutex::new(None),
        })
    }
}

impl Transport for HttpTransport {
    fn send(&self, request: &TraceAttributesRequest) -> Result<HttpResponse, TransportError> {
        {
            let mut last_send = self.last_send.lock();
            if let Some(last) = *last_send
                && let Some(remaining) = REQUEST_INTERVAL.checked_sub(last.elapsed())
            {
                std::thread::sleep(remaining);
            }
            *last_send = Some(Instant::now());
        }

        let (header_name, header_value) = CLIENT_ID_HEADER;
        let response = self
            .client
            .post(&self.url)
            .header(header_name, header_value)
            .json(request)
            .send()
            .map_err(|err| TransportError {
                detail: format!("{err:#}"),
            })?;
        let status = response.status().as_u16();
        let body = response.text().map_err(|err| TransportError {
            detail: format!("{err:#}"),
        })?;
        Ok(HttpResponse { status, body })
    }
}

/// Send every chunk of a plan, in order, classifying each into a
/// [`ChunkOutcome`]. `progress` is called after each chunk completes with
/// (completed, total).
///
/// Never fails as a whole: per-chunk failures become
/// [`ChunkOutcome::Failed`] and stitching handles the gaps.
pub fn send_plan(
    transport: &impl Transport,
    plan: &RequestPlan,
    params: &SnapParams,
    mut progress: impl FnMut(usize, usize),
) -> Vec<ChunkOutcome> {
    let total = plan.chunks.len();
    let mut outcomes = Vec::with_capacity(total);
    for (index, chunk) in plan.chunks.iter().enumerate() {
        let request = chunk.request(params, plan.gps_accuracy_m);
        outcomes.push(send_chunk(transport, &request));
        progress(index + 1, total);
    }
    outcomes
}

/// Send one chunk with retry-on-transient-failure semantics.
fn send_chunk(transport: &impl Transport, request: &TraceAttributesRequest) -> ChunkOutcome {
    let mut last_failure = String::new();
    for _ in 0..=RETRIES {
        match transport.send(request) {
            Ok(response) => match classify(&response) {
                Classified::Outcome(outcome) => return outcome,
                Classified::Transient(detail) => last_failure = detail,
            },
            Err(err) => last_failure = format!("{err:#}"),
        }
    }
    ChunkOutcome::Failed(last_failure)
}

/// A classified response: either a final outcome, or a transient failure
/// worth retrying.
enum Classified {
    Outcome(ChunkOutcome),
    Transient(String),
}

fn classify(response: &HttpResponse) -> Classified {
    let Ok(status) = reqwest::StatusCode::from_u16(response.status) else {
        return Classified::Outcome(ChunkOutcome::Failed(format!(
            "invalid HTTP status {}",
            response.status
        )));
    };
    if status.is_success() {
        return match serde_json::from_str::<TraceAttributesResponse>(&response.body) {
            Ok(parsed) => Classified::Outcome(ChunkOutcome::Success(parsed)),
            Err(err) => Classified::Outcome(ChunkOutcome::Failed(format!(
                "HTTP {status} with unparsable success body: {err}"
            ))),
        };
    }
    if status.is_server_error() {
        return Classified::Transient(format!("HTTP {status}"));
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
