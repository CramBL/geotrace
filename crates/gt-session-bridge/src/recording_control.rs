use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::protocol::RecordingReference;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManageRecordings {
    Open {
        paths: RecordingBatch<PathBuf>,
    },
    Unload {
        recordings: RecordingBatch<RecordingReference>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(extend("minItems" = 1, "maxItems" = MAX_RECORDING_BATCH))
)]
#[serde(try_from = "Vec<T>")]
pub struct RecordingBatch<T>(Vec<T>);

impl<T> RecordingBatch<T> {
    pub fn into_items(self) -> Vec<T> {
        self.0
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("Provide 1 to {} items", MAX_RECORDING_BATCH)]
pub struct RecordingBatchSizeError;

impl<T> TryFrom<Vec<T>> for RecordingBatch<T> {
    type Error = RecordingBatchSizeError;

    fn try_from(items: Vec<T>) -> Result<Self, Self::Error> {
        if (1..=MAX_RECORDING_BATCH).contains(&items.len()) {
            Ok(Self(items))
        } else {
            Err(RecordingBatchSizeError)
        }
    }
}

pub const MAX_RECORDING_BATCH: usize = 16;
