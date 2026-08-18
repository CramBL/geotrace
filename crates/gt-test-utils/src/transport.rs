//! A [`Transport`] that answers from a script, so a test never reaches a host.

use std::collections::VecDeque;

use gt_fetch::{HttpRequest, HttpResponse, Transport, TransportError};
use parking_lot::Mutex;

/// What a scripted transport answers one request with: a response, or a
/// failure below the HTTP layer.
pub type TransportAnswer<B> = Result<HttpResponse<B>, TransportError>;

/// The two answers [`ScriptedTransport::by_url_prefix`] picks between, for a
/// pipeline that tries several hosts.
pub struct UrlPrefixAnswers<B> {
    pub prefix: String,
    pub matching: TransportAnswer<B>,
    pub other: TransportAnswer<B>,
}

enum ScriptedAnswers<B> {
    Always(TransportAnswer<B>),
    InOrder(VecDeque<TransportAnswer<B>>),
    ByUrlPrefix(UrlPrefixAnswers<B>),
}

/// Records every request it is sent, for tests asserting which URLs a pipeline
/// requested and how many times.
pub struct ScriptedTransport<B> {
    answers: Mutex<ScriptedAnswers<B>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl<B> ScriptedTransport<B> {
    /// Answers every request the same way, which covers a pipeline that sends
    /// several requests per call (a retry, a second product, another mirror).
    pub fn always(answer: TransportAnswer<B>) -> Self {
        Self::new(ScriptedAnswers::Always(answer))
    }

    /// Answers requests in order, one script entry each. A request past the end
    /// of the script fails, naming the test's under-declared script.
    pub fn in_order(script: Vec<TransportAnswer<B>>) -> Self {
        Self::new(ScriptedAnswers::InOrder(script.into()))
    }

    pub fn by_url_prefix(answers: UrlPrefixAnswers<B>) -> Self {
        Self::new(ScriptedAnswers::ByUrlPrefix(answers))
    }

    fn new(answers: ScriptedAnswers<B>) -> Self {
        Self {
            answers: Mutex::new(answers),
            requests: Mutex::new(Vec::new()),
        }
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
    fn send(&self, request: &HttpRequest) -> TransportAnswer<B> {
        self.requests.lock().push(request.clone());
        match &mut *self.answers.lock() {
            ScriptedAnswers::Always(answer) => answer.clone(),
            ScriptedAnswers::InOrder(script) => script.pop_front().unwrap_or_else(|| {
                Err(TransportError {
                    detail: "the test under-declared its script".to_owned(),
                })
            }),
            ScriptedAnswers::ByUrlPrefix(answers) => {
                if request.url().starts_with(&answers.prefix) {
                    answers.matching.clone()
                } else {
                    answers.other.clone()
                }
            }
        }
    }
}
