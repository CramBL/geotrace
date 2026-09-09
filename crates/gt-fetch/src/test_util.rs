//! A [`Transport`] that responds from a script, so a test never reaches a host.

use std::collections::VecDeque;

use parking_lot::Mutex;

use crate::{HttpRequest, HttpResponse, Transport, TransportError};

/// What a scripted transport returns for one request: a response, or a failure
/// below the HTTP layer.
pub type TransportResponse<B> = Result<HttpResponse<B>, TransportError>;

pub fn response<B>(status: u16, body: impl Into<B>) -> TransportResponse<B> {
    Ok(HttpResponse {
        status,
        body: body.into(),
    })
}

pub fn transport_error<B>(detail: &str) -> TransportResponse<B> {
    Err(TransportError {
        detail: detail.to_owned(),
    })
}

/// The two responses [`ScriptedTransport::by_url_prefix`] picks between, for a
/// pipeline that tries several hosts.
pub struct UrlPrefixResponses<B> {
    pub prefix: String,
    pub matching: TransportResponse<B>,
    pub other: TransportResponse<B>,
}

enum ScriptedResponses<B> {
    Always(TransportResponse<B>),
    InOrder(VecDeque<TransportResponse<B>>),
    ByUrlPrefix(UrlPrefixResponses<B>),
}

/// Records every request it is sent, for tests asserting which URLs a pipeline
/// requested and how many times.
pub struct ScriptedTransport<B> {
    responses: Mutex<ScriptedResponses<B>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl<B> ScriptedTransport<B> {
    /// Returns the same response for every request, which covers a pipeline
    /// that sends several requests per call (a retry, a second product,
    /// another mirror).
    pub fn always(response: TransportResponse<B>) -> Self {
        Self::new(ScriptedResponses::Always(response))
    }

    /// Returns one script entry per request, in order. A request past the end
    /// of the script fails, and the failure states the URL the script has no
    /// entry for.
    pub fn in_order(script: Vec<TransportResponse<B>>) -> Self {
        Self::new(ScriptedResponses::InOrder(script.into()))
    }

    pub fn by_url_prefix(responses: UrlPrefixResponses<B>) -> Self {
        Self::new(ScriptedResponses::ByUrlPrefix(responses))
    }

    fn new(responses: ScriptedResponses<B>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// The requests it was sent, in order, headers included.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().clone()
    }

    /// The URLs requested of it, in order.
    pub fn requested_urls(&self) -> Vec<String> {
        self.requests
            .lock()
            .iter()
            .map(|request| request.url().to_owned())
            .collect()
    }

    pub fn sends(&self) -> usize {
        self.requests.lock().len()
    }
}

impl<B: Clone> Transport<B> for ScriptedTransport<B> {
    fn send(&self, request: &HttpRequest) -> TransportResponse<B> {
        self.requests.lock().push(request.clone());
        match &mut *self.responses.lock() {
            ScriptedResponses::Always(response) => response.clone(),
            ScriptedResponses::InOrder(script) => script.pop_front().unwrap_or_else(|| {
                transport_error(&format!(
                    "the test under-declared its script: no response left for {}",
                    request.url()
                ))
            }),
            ScriptedResponses::ByUrlPrefix(responses) => {
                if request.url().starts_with(&responses.prefix) {
                    responses.matching.clone()
                } else {
                    responses.other.clone()
                }
            }
        }
    }
}
