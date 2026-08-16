//! Fetch one index window and classify what the service answered.
//!
//! [`gt_fetch::Transport`] sends the request. A window the service has no
//! values for is answered with HTTP 200 and empty arrays, so there is no
//! missing-window status to classify: every response outside 2xx is a failure,
//! and a 5xx is retried once.

use std::time::Duration;

use gt_fetch::{Classified, HttpRequest, HttpResponse, Transport};

use crate::{GeomagneticIndex, TimeWindow, index_url};

/// Minimum gap between requests to the service, enforced by the transport the
/// fetch worker connects with.
///
/// One day costs one request per index, and a backfill walks hundreds of days.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

/// Why one window could not be fetched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct FetchFailure {
    pub detail: String,
}

/// Fetch `index` over `window` from `base_url`, retrying a transient failure
/// once. Whether the body parses is [`crate::wire`]'s answer.
pub fn fetch_index_window(
    transport: &impl Transport,
    base_url: &str,
    index: GeomagneticIndex,
    window: TimeWindow,
) -> Result<String, FetchFailure> {
    let request = HttpRequest::get(index_url(base_url, index, window));
    gt_fetch::send_classified(transport, &request, classify, |detail| {
        Err(FetchFailure { detail })
    })
}

/// A 5xx is retried once. A 4xx is deterministic and is not.
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
    use crate::DEFAULT_BASE_URL;

    fn window() -> TimeWindow {
        TimeWindow::covering_utc_day(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or(NaiveDate::MIN))
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
        let outcome = fetch_index_window(
            &transport,
            DEFAULT_BASE_URL,
            GeomagneticIndex::Hp30,
            window(),
        );
        let urls = transport.urls.borrow().clone();
        (outcome, urls)
    }

    #[test]
    fn a_served_window_returns_its_body_from_the_addressed_url() {
        let (outcome, urls) = fetch(vec![response(200, r#"{"Hp30":[]}"#)]);
        assert_eq!(outcome, Ok(r#"{"Hp30":[]}"#.to_owned()));
        assert_eq!(
            urls,
            [
                "https://kp.gfz.de/app/json/?start=2026-07-20T00:00:00Z&end=2026-07-20T23:59:59Z&index=Hp30"
            ]
        );
    }

    #[rstest]
    #[case::bad_request(400)]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::too_many_requests(429)]
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
        let (outcome, urls) = fetch(vec![response(503, ""), response(200, "{}")]);
        assert_eq!(outcome, Ok("{}".to_owned()));
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn a_transport_failure_is_retried_once_and_carries_the_last_detail() {
        let (outcome, urls) = fetch(vec![
            transport_error("connection reset"),
            transport_error("timed out"),
        ]);
        assert_eq!(
            outcome,
            Err(FetchFailure {
                detail: "request failed: timed out".to_owned()
            })
        );
        assert_eq!(urls.len(), 2);
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

    /// Offline, the service is never asked, so nothing is known about the
    /// window: that is a failure, not an empty one.
    #[test]
    fn an_offline_fetch_fails() {
        let transport = TransportSource::Offline
            .connect(None)
            .expect("the offline source connects");
        let outcome =
            fetch_index_window(&transport, DEFAULT_BASE_URL, GeomagneticIndex::Kp, window());
        assert!(outcome.is_err(), "{outcome:?}");
    }
}
