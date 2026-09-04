//! Fetch one window of flare events and classify what the endpoint returned.
//!
//! [`gt_fetch::Transport`] sends the request. A window the catalog lists
//! nothing for comes back as HTTP 200 with an empty array, so there is no
//! missing-window status to classify: every response outside 2xx is a failure,
//! and a 5xx is retried once.
//!
//! The URL holds the user's key, so every failure detail is redacted before it
//! leaves this module.

use std::time::Duration;

use gt_fetch::{Classified, HttpRequest, HttpResponse, Transport};

use crate::{ApiKey, DateWindow, flare_url};

/// Minimum gap between requests, enforced by the transport the fetch worker
/// connects with.
///
/// api.nasa.gov allows a registered key 1000 requests an hour, and one day of
/// flares costs one request, so this keeps even a five-year backfill under
/// that ceiling.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(4);

/// Why one window could not be fetched.
///
/// The detail is redacted: it can quote the URL that was tried, which holds
/// the key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct FetchFailure {
    pub detail: String,
}

/// Fetch the flares of `window` from `base_url`, retrying a transient failure
/// once. Whether the body parses is determined by [`crate::wire`].
pub fn fetch_flare_window(
    transport: &impl Transport,
    base_url: &str,
    key: &ApiKey,
    window: DateWindow,
) -> Result<String, FetchFailure> {
    let request = HttpRequest::get(flare_url(base_url, window, key));
    gt_fetch::send_classified(transport, &request, classify, |detail| {
        Err(FetchFailure { detail })
    })
    .map_err(|failure| FetchFailure {
        detail: key.redact(&failure.detail),
    })
}

/// A 5xx is retried once. A 4xx is deterministic and is not: a rejected key
/// returns 403 and returns it again.
fn classify(response: HttpResponse) -> Classified<Result<String, FetchFailure>> {
    if !response.status_is_valid() {
        return Classified::Outcome(Err(FetchFailure {
            detail: format!("invalid HTTP status {}", response.status),
        }));
    }
    if response.is_success() {
        return Classified::Outcome(Ok(response.body));
    }
    if response.is_server_error() {
        return Classified::Transient(format!("HTTP {}", response.status_line()));
    }
    Classified::Outcome(Err(FetchFailure {
        detail: format!("HTTP {}", response.status_line()),
    }))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use chrono::NaiveDate;
    use gt_fetch::{TransportError, TransportSource};
    use rstest::rstest;

    use super::*;
    use crate::{DEFAULT_BASE_URL, REDACTED_KEY};

    /// The key the tests fetch with. Never a real one: a fixture holding a
    /// working key would publish it.
    const TEST_KEY: &str = "test-key";

    fn key() -> ApiKey {
        ApiKey::new(TEST_KEY).expect("a key")
    }

    fn window() -> DateWindow {
        DateWindow::covering_utc_day(NaiveDate::from_ymd_opt(2024, 5, 9).unwrap_or(NaiveDate::MIN))
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

    /// Replays a scripted sequence and records the URL of every request.
    struct CannedTransport {
        script: RefCell<Vec<Result<HttpResponse, TransportError>>>,
        urls: RefCell<Vec<String>>,
    }

    impl Transport for CannedTransport {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
            self.urls.borrow_mut().push(request.url().to_owned());
            let mut script = self.script.borrow_mut();
            if script.is_empty() {
                return transport_error("the test under-declared its requests");
            }
            script.remove(0)
        }
    }

    fn fetch(
        script: Vec<Result<HttpResponse, TransportError>>,
    ) -> (Result<String, FetchFailure>, Vec<String>) {
        let transport = CannedTransport {
            script: RefCell::new(script),
            urls: RefCell::new(Vec::new()),
        };
        let outcome = fetch_flare_window(&transport, DEFAULT_BASE_URL, &key(), window());
        let urls = transport.urls.borrow().clone();
        (outcome, urls)
    }

    #[test]
    fn a_served_window_returns_its_body_from_the_addressed_url() {
        let (outcome, urls) = fetch(vec![response(200, "[]")]);
        assert_eq!(outcome, Ok("[]".to_owned()));
        assert_eq!(
            urls,
            [format!(
                "https://api.nasa.gov/DONKI/FLR?startDate=2024-05-09&endDate=2024-05-09\
                 &api_key={TEST_KEY}"
            )]
        );
    }

    #[rstest]
    #[case::bad_request(400)]
    #[case::rejected_key(403)]
    #[case::not_found(404)]
    #[case::over_the_rate_limit(429)]
    fn a_4xx_fails_without_a_retry(#[case] status: u16) {
        let (outcome, urls) = fetch(vec![response(status, "")]);
        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn a_5xx_is_retried_once_and_then_fails() {
        let (outcome, urls) = fetch(vec![response(503, ""), response(503, "")]);
        assert_eq!(
            outcome,
            Err(FetchFailure {
                detail: "HTTP 503 Service Unavailable".to_owned()
            })
        );
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn a_retried_5xx_that_succeeds_returns_the_body() {
        let (outcome, urls) = fetch(vec![response(503, ""), response(200, "[]")]);
        assert_eq!(outcome, Ok("[]".to_owned()));
        assert_eq!(urls.len(), 2);
    }

    /// The HTTP client quotes the URL it tried, and the URL holds the key.
    #[test]
    fn a_transport_failure_reports_its_detail_without_the_key() {
        let quoted = format!(
            "error sending request for url ({})",
            flare_url(DEFAULT_BASE_URL, window(), &key())
        );
        let (outcome, _) = fetch(vec![transport_error(&quoted), transport_error(&quoted)]);

        let detail = outcome.expect_err("both attempts failed").detail;
        assert!(!detail.contains(TEST_KEY), "{detail}");
        assert!(detail.contains(REDACTED_KEY), "{detail}");
    }

    #[test]
    fn a_status_outside_http_fails() {
        let (outcome, _) = fetch(vec![response(0, "")]);
        assert_eq!(
            outcome,
            Err(FetchFailure {
                detail: "invalid HTTP status 0".to_owned()
            })
        );
    }

    /// Offline, the endpoint is never contacted, so nothing is known about the
    /// window: that is a failure, not an empty one.
    #[test]
    fn an_offline_fetch_fails() {
        let transport = TransportSource::Offline
            .connect(None)
            .expect("the offline source connects");
        let outcome = fetch_flare_window(&transport, DEFAULT_BASE_URL, &key(), window());
        assert!(outcome.is_err(), "{outcome:?}");
    }
}
