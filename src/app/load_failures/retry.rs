use super::{LoadReadRequest, LoadSource};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::app) struct LoadFailureId(pub(super) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LoadRetryAttempt {
    failure: LoadFailureId,
    generation: u64,
}

impl LoadRetryAttempt {
    pub(super) fn new(failure: LoadFailureId, generation: u64) -> Self {
        Self {
            failure,
            generation,
        }
    }
}

pub(in crate::app) struct LoadRetry {
    pub attempt: LoadRetryAttempt,
    pub request: LoadReadRequest,
    pub source: LoadSource,
}

#[derive(Clone, Copy)]
pub(in crate::app) enum LoadProblemRequest {
    ChooseFile(LoadRetryAttempt),
    LoadText(LoadRetryAttempt),
    Retry(LoadFailureId),
}
