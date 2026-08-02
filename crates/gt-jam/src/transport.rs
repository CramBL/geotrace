//! Fetch one day's dataset and classify what the host answered.
//!
//! [`Transport`] is the seam between the fetch pipeline and the network.
//! Which one is in use is the application's choice, made once at startup and
//! supplied as a [`TransportSource`]. Nothing here reads the process
//! environment.
//!
//! A 404 is [`FetchOutcome::Missing`], not a failure. Whether that means
//! "not published yet" or a gap in the record is
//! [`crate::calendar::awaiting_publication`]'s answer, not the transport's.

use std::time::Duration;

use chrono::NaiveDate;

/// Retries per fetch, for transient failures only.
const RETRIES: usize = 1;

/// Timeout per request. A day is about 300 KiB gzipped.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Minimum gap between requests to the same host.
///
/// The datasets are static files on someone else's server, and a backfill
/// walks hundreds of them.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

/// Sent so the host can attribute the traffic.
pub const CLIENT_ID_HEADER: (&str, &str) = ("X-Client-Id", "geotrace");

/// HTTP status for a day the host has no file for.
const HTTP_NOT_FOUND: u16 = 404;

/// One HTTP response: what classification needs from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Decoded body. The host gzip-encodes regardless of `Accept-Encoding`,
    /// so a transport that does not decode returns compressed bytes here.
    pub body: String,
}

/// A failure below the HTTP layer (connection, timeout, TLS, ...).
#[derive(Debug, thiserror::Error)]
#[error("request failed: {detail}")]
pub struct TransportError {
    pub detail: String,
}

/// The seam between the fetch pipeline and the network.
pub trait Transport {
    fn get(&self, url: &str) -> Result<HttpResponse, TransportError>;
}

/// Fails every request with [`gt_types::env::OFFLINE_DETAIL`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OfflineTransport;

impl Transport for OfflineTransport {
    fn get(&self, _url: &str) -> Result<HttpResponse, TransportError> {
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
    fn get(&self, url: &str) -> Result<HttpResponse, TransportError> {
        match self {
            Self::Http(transport) => transport.get(url),
            Self::Offline(transport) => transport.get(url),
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
    /// Open a transport. A changed host gets its own connection pool, so
    /// this is called again whenever the host changes.
    pub fn connect(self) -> Result<Connection, TransportError> {
        match self {
            Self::Network => Ok(Connection::Http(HttpTransport::new()?)),
            Self::Offline => Ok(Connection::Offline(OfflineTransport)),
        }
    }
}

/// What one fetch produced.
#[derive(Debug, Clone, PartialEq, Eq, strum::EnumCount, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum FetchOutcome {
    /// The dataset, as served. Whether it parses is
    /// [`crate::wire::parse_dataset`]'s answer.
    Served(String),
    /// The host has no file for this day.
    Missing,
    /// The fetch failed and retrying did not help.
    Failed(String),
}

/// Blocking transport over reqwest, with transparent gzip decoding.
pub struct HttpTransport {
    client: reqwest::blocking::Client,
}

impl HttpTransport {
    pub fn new() -> Result<Self, TransportError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| TransportError {
                detail: format!("{err:#}"),
            })?;
        Ok(Self { client })
    }
}

impl Transport for HttpTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, TransportError> {
        let (header_name, header_value) = CLIENT_ID_HEADER;
        let response = self
            .client
            .get(url)
            .header(header_name, header_value)
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

/// Fetch `day` from `base_url`, retrying transient failures once.
pub fn fetch_day(transport: &impl Transport, base_url: &str, day: NaiveDate) -> FetchOutcome {
    let url = crate::dataset_url(base_url, day);

    let mut last_failure = String::new();
    for _ in 0..=RETRIES {
        match transport.get(&url) {
            Ok(response) => match classify(response) {
                Classified::Outcome(outcome) => return outcome,
                Classified::Transient(detail) => last_failure = detail,
            },
            Err(err) => last_failure = format!("{err:#}"),
        }
    }
    FetchOutcome::Failed(last_failure)
}

/// A classified response: a final outcome, or a transient failure worth one
/// retry.
enum Classified {
    Outcome(FetchOutcome),
    Transient(String),
}

/// Classify one response.
///
/// 5xx retries; 4xx is deterministic and does not.
fn classify(response: HttpResponse) -> Classified {
    let HttpResponse { status, body } = response;
    let Ok(code) = reqwest::StatusCode::from_u16(status) else {
        return Classified::Outcome(FetchOutcome::Failed(format!(
            "invalid HTTP status {status}"
        )));
    };
    if code.is_success() {
        return Classified::Outcome(FetchOutcome::Served(body));
    }
    if status == HTTP_NOT_FOUND {
        return Classified::Outcome(FetchOutcome::Missing);
    }
    if code.is_server_error() {
        return Classified::Transient(format!("HTTP {code}"));
    }
    Classified::Outcome(FetchOutcome::Failed(format!("HTTP {code}")))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::EnumCount as _;

    use super::*;
    use crate::DEFAULT_BASE_URL;

    /// A day inside the coverage window.
    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()
    }

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

    /// Replays a scripted sequence and records the URLs it was asked for.
    struct CannedTransport {
        script: RefCell<Vec<Result<HttpResponse, TransportError>>>,
        urls: RefCell<Vec<String>>,
    }

    impl CannedTransport {
        fn new(script: Vec<Result<HttpResponse, TransportError>>) -> Self {
            Self {
                script: RefCell::new(script),
                urls: RefCell::new(Vec::new()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.borrow().clone()
        }
    }

    impl Transport for CannedTransport {
        fn get(&self, url: &str) -> Result<HttpResponse, TransportError> {
            self.urls.borrow_mut().push(url.to_owned());
            let mut script = self.script.borrow_mut();
            if script.is_empty() {
                return transport_error("the test under-declared its requests");
            }
            script.remove(0)
        }
    }

    fn fetch(script: Vec<Result<HttpResponse, TransportError>>) -> (FetchOutcome, Vec<String>) {
        let transport = CannedTransport::new(script);
        let outcome = fetch_day(&transport, DEFAULT_BASE_URL, day());
        (outcome, transport.urls())
    }

    #[test]
    fn a_served_day_returns_its_body() {
        let (outcome, urls) = fetch(vec![response(200, "hex,count_good_aircraft\n")]);
        assert_eq!(
            outcome,
            FetchOutcome::Served("hex,count_good_aircraft\n".to_owned())
        );
        assert_eq!(urls, ["https://gpsjam.org/data/2026-07-20-h3_4.csv"]);
    }

    #[test]
    fn a_404_is_missing_not_a_failure() {
        let (outcome, urls) = fetch(vec![response(404, r#"{"message":"File not found"}"#)]);
        assert_eq!(outcome, FetchOutcome::Missing);
        assert_eq!(urls.len(), 1, "a 404 is deterministic");
    }

    #[rstest]
    #[case::bad_request(400)]
    #[case::forbidden(403)]
    #[case::teapot(418)]
    #[case::too_many_requests(429)]
    fn a_4xx_fails_without_a_retry(#[case] status: u16) {
        let (outcome, urls) = fetch(vec![response(status, "")]);
        assert!(matches!(outcome, FetchOutcome::Failed(_)), "{outcome:?}");
        assert_eq!(urls.len(), 1);
    }

    #[rstest]
    #[case::internal(500)]
    #[case::bad_gateway(502)]
    #[case::unavailable(503)]
    fn a_5xx_is_retried_once(#[case] status: u16) {
        let (outcome, urls) = fetch(vec![response(status, ""), response(status, "")]);
        assert!(matches!(outcome, FetchOutcome::Failed(_)), "{outcome:?}");
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn a_retried_5xx_that_succeeds_returns_the_body() {
        let (outcome, urls) = fetch(vec![response(503, ""), response(200, "csv")]);
        assert_eq!(outcome, FetchOutcome::Served("csv".to_owned()));
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn a_transport_failure_is_retried_once() {
        let (outcome, urls) = fetch(vec![
            transport_error("connection reset"),
            response(200, "csv"),
        ]);
        assert_eq!(outcome, FetchOutcome::Served("csv".to_owned()));
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn a_failure_carries_the_last_detail() {
        let (outcome, _) = fetch(vec![
            transport_error("connection reset"),
            transport_error("timed out"),
        ]);
        assert_eq!(
            outcome,
            FetchOutcome::Failed("request failed: timed out".to_owned())
        );
    }

    #[test]
    fn a_status_outside_http_fails() {
        let (outcome, _) = fetch(vec![response(0, "")]);
        assert_eq!(
            outcome,
            FetchOutcome::Failed("invalid HTTP status 0".to_owned())
        );
    }

    /// The offline source connects, and then declines every request, so a
    /// caller sees a working transport that says no.
    #[test]
    fn the_offline_source_refuses_every_request() {
        let transport = TransportSource::Offline
            .connect()
            .expect("the offline source connects");
        let err = transport
            .get("https://example.invalid/day.csv")
            .expect_err("offline transport refuses");
        assert!(err.detail.contains(gt_types::env::OFFLINE_DETAIL));
    }

    /// An offline fetch is a failure, not a missing day: the host was never
    /// asked, so nothing is known about whether it has the dataset.
    #[test]
    fn an_offline_fetch_is_a_failure_not_a_missing_day() {
        let transport = TransportSource::Offline
            .connect()
            .expect("the offline source connects");
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        assert!(matches!(
            fetch_day(&transport, crate::DEFAULT_BASE_URL, day),
            FetchOutcome::Failed(_)
        ));
    }

    proptest::proptest! {
        /// Status alone decides the outcome, for any status and any body.
        #[test]
        fn any_response_classifies_by_status(
            status in proptest::prelude::any::<u16>(),
            body in ".{0,512}",
        ) {
            // Two scripted responses: a 5xx spends its retry on the second.
            let (outcome, _) = fetch(vec![response(status, &body), response(status, &body)]);
            let served = (200..300).contains(&status);
            match outcome {
                FetchOutcome::Served(returned) => {
                    proptest::prop_assert!(served);
                    proptest::prop_assert_eq!(returned, body);
                }
                FetchOutcome::Missing => proptest::prop_assert_eq!(status, HTTP_NOT_FOUND),
                FetchOutcome::Failed(_) => {
                    proptest::prop_assert!(!served && status != HTTP_NOT_FOUND);
                }
            }
        }
    }

    /// A variant cannot be added without a response that reaches it.
    #[test]
    fn every_outcome_is_reachable() {
        let reached: HashSet<&'static str> = [
            fetch(vec![response(200, "csv")]).0,
            fetch(vec![response(404, "")]).0,
            fetch(vec![response(400, "")]).0,
        ]
        .iter()
        .map(<&'static str>::from)
        .collect();
        assert_eq!(reached.len(), FetchOutcome::COUNT);
    }
}
