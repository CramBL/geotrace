//! Fetch one index window and classify what the service returned.
//!
//! [`gt_fetch::Transport`] sends the request. For a window the service has no
//! values for it returns HTTP 200 and empty arrays, so there is no
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
/// once. Whether the body parses is determined by [`crate::wire`].
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
    use chrono::NaiveDate;
    use gt_fetch::test_util::{self as scripted_transport, ScriptedTransport, TransportResponse};
    use rstest::rstest;

    use super::*;
    use crate::DEFAULT_BASE_URL;

    fn window() -> TimeWindow {
        TimeWindow::covering_utc_day(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or(NaiveDate::MIN))
    }

    fn fetch(
        script: Vec<TransportResponse<String>>,
    ) -> (Result<String, FetchFailure>, Vec<String>) {
        let transport = ScriptedTransport::in_order(script);
        let outcome = fetch_index_window(
            &transport,
            DEFAULT_BASE_URL,
            GeomagneticIndex::Hp30,
            window(),
        );
        (outcome, transport.requested_urls())
    }

    #[test]
    fn a_served_window_returns_its_body_from_the_addressed_url() {
        let (outcome, urls) = fetch(vec![scripted_transport::response(200, r#"{"Hp30":[]}"#)]);
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
        let (outcome, urls) = fetch(vec![scripted_transport::response(status, "")]);
        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn a_5xx_is_retried_once_and_then_fails() {
        let (outcome, urls) = fetch(vec![
            scripted_transport::response(503, ""),
            scripted_transport::response(503, ""),
        ]);
        assert_eq!(
            outcome,
            Err(FetchFailure {
                detail: "HTTP 503 Service Unavailable".to_owned()
            })
        );
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn a_transport_failure_is_retried_once_and_carries_the_last_detail() {
        let (outcome, urls) = fetch(vec![
            scripted_transport::transport_error("connection reset"),
            scripted_transport::transport_error("timed out"),
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
        let (outcome, _) = fetch(vec![scripted_transport::response(0, "")]);
        assert_eq!(
            outcome,
            Err(FetchFailure {
                detail: "invalid HTTP status 0".to_owned()
            })
        );
    }
}
