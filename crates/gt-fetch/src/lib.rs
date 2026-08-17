//! [`Transport`] sends and receives HTTP requests for GeoTrace's fetch
//! pipelines. Which implementation is in use is the application's choice, made
//! once at startup and supplied as a [`TransportSource`].
//!
//! Response classification is pipeline-specific: each fetch pipeline classifies
//! [`HttpResponse`]s into its own outcome type via [`send_classified`], which
//! retries a transient failure once, the same policy for every pipeline.

use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Retries per classified send, for transient failures only.
const RETRIES: usize = 1;

/// Timeout per request. Generous, because some hosts do server-side work
/// before responding.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Request header (name, value) a host can use to attribute traffic, sent
/// with every request.
pub const CLIENT_ID_HEADER: (&str, &str) = ("X-Client-Id", "geotrace");

/// One request as a transport sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpRequest {
    Get {
        url: String,
    },
    /// A POST with a JSON body, sent as `application/json`.
    PostJson {
        url: String,
        body: String,
    },
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self::Get { url: url.into() }
    }

    pub fn post_json(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self::PostJson {
            url: url.into(),
            body: body.into(),
        }
    }

    pub fn url(&self) -> &str {
        match self {
            Self::Get { url } | Self::PostJson { url, .. } => url,
        }
    }
}

/// One HTTP response: what classification needs from it.
///
/// The body is text by default. [`BytesResponse`] is the same response with an
/// undecoded body, which is what a host serving a compressed file returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse<B = String> {
    pub status: u16,
    pub body: B,
}

/// A response whose body was never decoded as text, for hosts serving files
/// in a binary format.
pub type BytesResponse = HttpResponse<Vec<u8>>;

impl<B> HttpResponse<B> {
    /// Whether the status is inside the range HTTP defines. Classifiers
    /// treat anything else as a deterministic failure.
    pub fn status_is_valid(&self) -> bool {
        reqwest::StatusCode::from_u16(self.status).is_ok()
    }

    /// 2xx.
    pub fn is_success(&self) -> bool {
        reqwest::StatusCode::from_u16(self.status).is_ok_and(|code| code.is_success())
    }

    /// 5xx.
    pub fn is_server_error(&self) -> bool {
        reqwest::StatusCode::from_u16(self.status).is_ok_and(|code| code.is_server_error())
    }

    /// The status with its canonical reason phrase (`"503 Service
    /// Unavailable"`), or the bare number when it has none.
    pub fn status_line(&self) -> String {
        reqwest::StatusCode::from_u16(self.status)
            .ok()
            .and_then(|code| code.canonical_reason())
            .map_or_else(
                || self.status.to_string(),
                |reason| format!("{} {reason}", self.status),
            )
    }
}

/// A failure below the HTTP layer (connection, timeout, TLS, ...).
#[derive(Debug, thiserror::Error)]
#[error("request failed: {detail}")]
pub struct TransportError {
    pub detail: String,
}

/// Implemented by [`HttpTransport`] and [`OfflineTransport`], for responses
/// whose body is `B`. See [`HttpResponse`] for what the two bodies are.
pub trait Transport<B = String> {
    /// Send one request and return the raw response.
    ///
    /// Implementations may apply pacing (rate limits) internally: a call may
    /// block until the host can be contacted again.
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse<B>, TransportError>;
}

/// Why a request failed while offline, in logs and in the UI.
pub const OFFLINE_DETAIL: &str = "GeoTrace is running offline";

/// Fails every request with [`OFFLINE_DETAIL`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OfflineTransport;

impl<B> Transport<B> for OfflineTransport {
    fn send(&self, _request: &HttpRequest) -> Result<HttpResponse<B>, TransportError> {
        Err(TransportError {
            detail: OFFLINE_DETAIL.to_owned(),
        })
    }
}

/// The transport in use, picked by [`TransportSource`].
pub enum Connection {
    Http(HttpTransport),
    Offline(OfflineTransport),
}

impl<B> Transport<B> for Connection
where
    HttpTransport: Transport<B>,
    OfflineTransport: Transport<B>,
{
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse<B>, TransportError> {
        match self {
            Self::Http(transport) => Transport::send(transport, request),
            Self::Offline(transport) => Transport::send(transport, request),
        }
    }
}

/// Which transport the application runs with, selected once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportSource {
    Network,
    Offline,
}

impl TransportSource {
    /// Open a transport. See [`HttpTransport::new`].
    ///
    /// Called again whenever the host changes: a changed host gets its own
    /// connection pool and pacing state.
    pub fn connect(self, pacing: Option<Duration>) -> Result<Connection, TransportError> {
        match self {
            Self::Network => Ok(Connection::Http(HttpTransport::new(pacing)?)),
            Self::Offline => Ok(Connection::Offline(OfflineTransport)),
        }
    }
}

/// Completion time of the most recent send. The transport sleeps the calling
/// thread until one interval has passed since this instant.
struct Pacing {
    interval: Duration,
    last_send: Mutex<Option<Instant>>,
}

/// Blocking production transport over reqwest, with transparent gzip
/// decoding. Sends [`CLIENT_ID_HEADER`] with every request.
pub struct HttpTransport {
    client: reqwest::blocking::Client,
    pacing: Option<Pacing>,
}

impl HttpTransport {
    /// A transport pacing sends to at most one per `pacing` interval.
    /// `None` leaves pacing to the caller.
    pub fn new(pacing: Option<Duration>) -> Result<Self, TransportError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| TransportError {
                detail: format!("{err:#}"),
            })?;
        Ok(Self {
            client,
            pacing: pacing.map(|interval| Pacing {
                interval,
                last_send: Mutex::new(None),
            }),
        })
    }

    fn pace(&self) {
        let Some(pacing) = &self.pacing else {
            return;
        };
        let mut last_send = pacing.last_send.lock();
        if let Some(last) = *last_send
            && let Some(remaining) = pacing.interval.checked_sub(last.elapsed())
        {
            std::thread::sleep(remaining);
        }
        *last_send = Some(Instant::now());
    }
}

impl HttpTransport {
    /// Pace, send, and hand back the response for the caller to read the body
    /// from.
    fn send_paced(
        &self,
        request: &HttpRequest,
    ) -> Result<reqwest::blocking::Response, TransportError> {
        self.pace();

        let (header_name, header_value) = CLIENT_ID_HEADER;
        let builder = match request {
            HttpRequest::Get { url } => self.client.get(url),
            HttpRequest::PostJson { url, body } => self
                .client
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone()),
        };
        builder
            .header(header_name, header_value)
            .send()
            .map_err(|err| TransportError {
                detail: format!("{err:#}"),
            })
    }
}

impl Transport<String> for HttpTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let response = self.send_paced(request)?;
        let status = response.status().as_u16();
        let body = response.text().map_err(|err| TransportError {
            detail: format!("{err:#}"),
        })?;
        Ok(HttpResponse { status, body })
    }
}

impl Transport<Vec<u8>> for HttpTransport {
    fn send(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
        let response = self.send_paced(request)?;
        let status = response.status().as_u16();
        let body = response.bytes().map_err(|err| TransportError {
            detail: format!("{err:#}"),
        })?;
        Ok(BytesResponse {
            status,
            body: body.to_vec(),
        })
    }
}

/// A classified response: a final outcome, or a transient failure.
/// [`send_classified`] retries a transient failure once.
pub enum Classified<T> {
    Outcome(T),
    Transient(String),
}

/// Send `request`, retrying transient failures once: a [`TransportError`],
/// or a response `classify` calls [`Classified::Transient`]. When the first
/// attempt fails deterministically or the retry also fails, the last failure's
/// detail goes to `failure`.
pub fn send_classified<B, T>(
    transport: &impl Transport<B>,
    request: &HttpRequest,
    classify: impl Fn(HttpResponse<B>) -> Classified<T>,
    failure: impl FnOnce(String) -> T,
) -> T {
    let mut last_failure = String::new();
    for _ in 0..=RETRIES {
        match transport.send(request) {
            Ok(response) => match classify(response) {
                Classified::Outcome(outcome) => return outcome,
                Classified::Transient(detail) => last_failure = detail,
            },
            Err(err) => last_failure = format!("{err:#}"),
        }
    }
    failure(last_failure)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use rstest::rstest;

    use super::*;

    fn response(status: u16, body: &str) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status,
            body: body.to_owned(),
        })
    }

    fn transport_error(detail: &str) -> Result<HttpResponse, TransportError> {
        Err(TransportError {
            detail: detail.to_owned(),
        })
    }

    /// Replays a scripted sequence and records the requests it was sent.
    struct CannedTransport {
        script: RefCell<Vec<Result<HttpResponse, TransportError>>>,
        requests: RefCell<Vec<HttpRequest>>,
    }

    impl CannedTransport {
        fn new(script: Vec<Result<HttpResponse, TransportError>>) -> Self {
            Self {
                script: RefCell::new(script),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn sends(&self) -> usize {
            self.requests.borrow().len()
        }
    }

    impl Transport for CannedTransport {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
            self.requests.borrow_mut().push(request.clone());
            let mut script = self.script.borrow_mut();
            if script.is_empty() {
                return transport_error("the test under-declared its requests");
            }
            script.remove(0)
        }
    }

    /// The classifier every retry test runs: 2xx is a final `Ok`-like
    /// outcome, 5xx is transient, anything else fails outright.
    fn classify(response: HttpResponse) -> Classified<Result<String, String>> {
        if response.is_success() {
            return Classified::Outcome(Ok(response.body));
        }
        if response.is_server_error() {
            return Classified::Transient(response.status_line());
        }
        Classified::Outcome(Err(response.status_line()))
    }

    fn send(script: Vec<Result<HttpResponse, TransportError>>) -> (Result<String, String>, usize) {
        let transport = CannedTransport::new(script);
        let request = HttpRequest::get("https://example.invalid/dataset");
        let outcome = send_classified(&transport, &request, classify, Err);
        (outcome, transport.sends())
    }

    #[test]
    fn a_success_returns_the_classified_outcome() {
        let (outcome, sends) = send(vec![response(200, "body")]);
        assert_eq!(outcome, Ok("body".to_owned()));
        assert_eq!(sends, 1);
    }

    #[rstest]
    #[case::bad_request(400)]
    #[case::not_found(404)]
    #[case::too_many_requests(429)]
    fn a_final_outcome_is_never_retried(#[case] status: u16) {
        let (outcome, sends) = send(vec![response(status, "")]);
        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(sends, 1);
    }

    #[rstest]
    #[case::internal(500)]
    #[case::unavailable(503)]
    fn a_transient_response_is_retried_once(#[case] status: u16) {
        let (outcome, sends) = send(vec![response(status, ""), response(status, "")]);
        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(sends, 2);
    }

    #[test]
    fn a_retried_transient_that_succeeds_returns_the_outcome() {
        let (outcome, sends) = send(vec![response(503, ""), response(200, "body")]);
        assert_eq!(outcome, Ok("body".to_owned()));
        assert_eq!(sends, 2);
    }

    #[test]
    fn a_transport_failure_is_retried_once() {
        let (outcome, sends) = send(vec![
            transport_error("connection reset"),
            response(200, "body"),
        ]);
        assert_eq!(outcome, Ok("body".to_owned()));
        assert_eq!(sends, 2);
    }

    #[test]
    fn a_failure_reports_the_last_detail() {
        let (outcome, _) = send(vec![
            transport_error("connection reset"),
            transport_error("timed out"),
        ]);
        assert_eq!(outcome, Err("request failed: timed out".to_owned()));
    }

    #[rstest]
    #[case::below_the_floor(99, false)]
    #[case::at_the_floor(100, true)]
    #[case::at_the_limit(999, true)]
    #[case::zero(0, false)]
    fn the_status_range_matches_what_http_defines(#[case] status: u16, #[case] valid: bool) {
        let response = HttpResponse {
            status,
            body: String::new(),
        };
        assert_eq!(response.status_is_valid(), valid);
    }

    #[test]
    fn a_known_status_line_contains_its_reason_phrase() {
        let response = HttpResponse {
            status: 503,
            body: String::new(),
        };
        assert_eq!(response.status_line(), "503 Service Unavailable");
    }

    #[test]
    fn an_unknown_status_line_is_the_bare_number() {
        let response = HttpResponse {
            status: 599,
            body: String::new(),
        };
        assert_eq!(response.status_line(), "599");
    }

    #[test]
    fn both_request_forms_expose_their_url() {
        assert_eq!(HttpRequest::get("https://a/b").url(), "https://a/b");
        assert_eq!(
            HttpRequest::post_json("https://a/b", "{}").url(),
            "https://a/b"
        );
    }

    #[test]
    fn the_offline_source_refuses_every_request() {
        let transport = TransportSource::Offline
            .connect(None)
            .expect("the offline source connects");
        let err = Transport::<String>::send(
            &transport,
            &HttpRequest::get("https://example.invalid/dataset"),
        )
        .expect_err("offline transport refuses");
        assert!(err.detail.contains(OFFLINE_DETAIL));
    }

    #[test]
    fn the_offline_source_refuses_a_bytes_request_too() {
        let transport = TransportSource::Offline
            .connect(None)
            .expect("the offline source connects");
        let err = Transport::<Vec<u8>>::send(
            &transport,
            &HttpRequest::get("https://example.invalid/file.gz"),
        )
        .expect_err("offline transport refuses");
        assert!(err.detail.contains(OFFLINE_DETAIL));
    }

    /// Replays one scripted bytes response.
    struct CannedBytesTransport {
        script: RefCell<Vec<Result<BytesResponse, TransportError>>>,
        sends: RefCell<usize>,
    }

    impl Transport<Vec<u8>> for CannedBytesTransport {
        fn send(&self, _request: &HttpRequest) -> Result<BytesResponse, TransportError> {
            *self.sends.borrow_mut() += 1;
            let mut script = self.script.borrow_mut();
            if script.is_empty() {
                return Err(TransportError {
                    detail: "the test under-declared its requests".to_owned(),
                });
            }
            script.remove(0)
        }
    }

    /// A body no UTF-8 decode survives reaches the classifier unchanged, which
    /// is the whole reason the bytes path exists.
    #[test]
    fn a_bytes_body_reaches_the_classifier_undecoded() {
        let gzip_magic = vec![0x1f, 0x8b, 0x08, 0x00];
        let transport = CannedBytesTransport {
            script: RefCell::new(vec![
                Ok(BytesResponse {
                    status: 503,
                    body: Vec::new(),
                }),
                Ok(BytesResponse {
                    status: 200,
                    body: gzip_magic.clone(),
                }),
            ]),
            sends: RefCell::new(0),
        };

        let outcome = send_classified(
            &transport,
            &HttpRequest::get("https://example.invalid/file.gz"),
            |response| {
                if response.is_server_error() {
                    return Classified::Transient(response.status_line());
                }
                Classified::Outcome(Ok(response.body))
            },
            Err,
        );

        assert_eq!(outcome, Ok(gzip_magic));
        assert_eq!(*transport.sends.borrow(), 2, "the 503 was retried once");
    }
}
