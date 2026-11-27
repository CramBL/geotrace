use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("maximum" = MAX_STATUS_WAIT_MS)))]
#[serde(try_from = "u16")]
pub struct StatusWait(u16);

impl StatusWait {
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl From<StatusWait> for Duration {
    fn from(wait: StatusWait) -> Self {
        Self::from_millis(u64::from(wait.0))
    }
}

impl TryFrom<u16> for StatusWait {
    type Error = StatusWaitOutOfRange;

    fn try_from(milliseconds: u16) -> Result<Self, Self::Error> {
        if milliseconds <= MAX_STATUS_WAIT_MS {
            Ok(Self(milliseconds))
        } else {
            Err(StatusWaitOutOfRange)
        }
    }
}

#[derive(Clone, Copy, Debug, Error)]
#[error("wait_ms must be between 0 and {MAX_STATUS_WAIT_MS}")]
pub struct StatusWaitOutOfRange;

const MAX_STATUS_WAIT_MS: u16 = 2_000;
