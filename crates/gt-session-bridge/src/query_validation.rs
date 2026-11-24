use gt_query::{ChannelSchema, Diagnostic};
use gt_query_run::{CheckedQueryText, QueryTextCheck};
use gt_types::LoadedFile;
use serde::{Deserialize, Serialize};

use crate::framing::{Frame, MAX_REPLY_BYTES};
use crate::protocol::{ErrorCode, Reply};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QueryValidation {
    Empty,
    Invalid { diagnostics: Vec<Diagnostic> },
    Valid,
}

impl QueryValidation {
    pub(crate) fn check(text: String, files: &[LoadedFile]) -> Result<Self, ErrorCode> {
        let schema = gt_query_run::schema_from_files(files);
        match QueryPreparation::check(text, schema)? {
            QueryPreparation::Rejected(validation) => Ok(validation),
            QueryPreparation::Ready(_) => Ok(Self::Valid),
        }
    }
}

pub enum QueryPreparation {
    Rejected(QueryValidation),
    Ready(CheckedQueryText),
}

impl QueryPreparation {
    pub fn check(text: String, schema: ChannelSchema) -> Result<Self, ErrorCode> {
        let mut diagnostics = BoundedDiagnostics::new()?;
        let checked =
            CheckedQueryText::check(text, schema, |diagnostic| diagnostics.push(diagnostic))?;
        Ok(match checked {
            QueryTextCheck::Empty => Self::Rejected(QueryValidation::Empty),
            QueryTextCheck::Invalid => Self::Rejected(QueryValidation::Invalid {
                diagnostics: diagnostics.items,
            }),
            QueryTextCheck::Valid(checked) => Self::Ready(checked),
        })
    }
}

struct BoundedDiagnostics {
    items: Vec<Diagnostic>,
    remaining: usize,
}

impl BoundedDiagnostics {
    fn new() -> Result<Self, ErrorCode> {
        let envelope = Reply::QueryCheck {
            validation: QueryValidation::Invalid {
                diagnostics: Vec::new(),
            },
        };
        let frame = Frame::encode(&envelope, MAX_REPLY_BYTES)
            .map_err(|_size_limit| ErrorCode::ReplyTooLarge)?;
        Ok(Self {
            items: Vec::new(),
            remaining: MAX_REPLY_BYTES - frame.payload_len(),
        })
    }

    fn push(&mut self, diagnostic: Diagnostic) -> Result<(), ErrorCode> {
        let separator_bytes = usize::from(!self.items.is_empty());
        let remaining = self.remaining.saturating_sub(separator_bytes);
        let frame = Frame::encode(&diagnostic, remaining)
            .map_err(|_size_limit| ErrorCode::ReplyTooLarge)?;
        self.remaining = remaining - frame.payload_len();
        self.items.push(diagnostic);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
