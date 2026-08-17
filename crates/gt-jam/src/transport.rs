//! Fetch one day's dataset and classify what the host returned.
//!
//! [`gt_fetch::Transport`] sends requests over the network. `classify` below
//! maps an [`gt_fetch::HttpResponse`] to a [`FetchOutcome`] for one
//! interference day.
//!
//! A 404 is [`FetchOutcome::Missing`], not a failure. Whether that means
//! "not published yet" or a gap in the record is determined by
//! [`crate::calendar::awaiting_publication`], not by the transport.

use std::time::Duration;

use chrono::NaiveDate;

use gt_fetch::{Classified, HttpRequest, HttpResponse, Transport};

/// Minimum gap between requests to the same host, left to the fetch worker
/// to enforce between days.
///
/// The datasets are static files on a third-party server, and a backfill
/// walks hundreds of them.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

/// HTTP status for a day the host has no file for.
const HTTP_NOT_FOUND: u16 = 404;

/// What one fetch produced.
#[derive(Debug, Clone, PartialEq, Eq, strum::EnumCount, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum FetchOutcome {
    /// The dataset, as served. Whether it parses is determined by
    /// [`crate::wire::parse_dataset`].
    Served(String),
    /// The host has no file for this day.
    Missing,
    /// The fetch failed and retrying did not help.
    Failed(String),
}

/// Fetch `day` from `base_url`, retrying transient failures once.
pub fn fetch_day(transport: &impl Transport, base_url: &str, day: NaiveDate) -> FetchOutcome {
    let request = HttpRequest::get(crate::dataset_url(base_url, day));
    gt_fetch::send_classified(transport, &request, classify, FetchOutcome::Failed)
}

/// A 5xx is retried once. A 4xx is deterministic and is not.
fn classify(response: HttpResponse) -> Classified<FetchOutcome> {
    if !response.status_is_valid() {
        return Classified::Outcome(FetchOutcome::Failed(format!(
            "invalid HTTP status {}",
            response.status
        )));
    }
    if response.is_success() {
        return Classified::Outcome(FetchOutcome::Served(response.body));
    }
    if response.status == HTTP_NOT_FOUND {
        return Classified::Outcome(FetchOutcome::Missing);
    }
    if response.is_server_error() {
        return Classified::Transient(format!("HTTP {}", response.status_line()));
    }
    Classified::Outcome(FetchOutcome::Failed(format!(
        "HTTP {}",
        response.status_line()
    )))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::EnumCount as _;

    use gt_fetch::{TransportError, TransportSource};

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

    /// Replays a scripted sequence and records the requested URLs.
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
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
            self.urls.borrow_mut().push(request.url().to_owned());
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

    /// An offline fetch is a failure, not a missing day: the host was never
    /// contacted, so nothing is known about whether it has the dataset.
    #[test]
    fn an_offline_fetch_is_a_failure_not_a_missing_day() {
        let transport = TransportSource::Offline
            .connect(None)
            .expect("the offline source connects");
        assert!(matches!(
            fetch_day(&transport, DEFAULT_BASE_URL, day()),
            FetchOutcome::Failed(_)
        ));
    }

    proptest::proptest! {
        /// Status alone determines the outcome, for any status and any body.
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
