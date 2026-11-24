use gt_query::Diagnostic;
use gt_query_run::{MIXED_CHANNEL_MESSAGE, QueryChunk, RunKind};
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
    pub(crate) fn check(text: &str, files: &[LoadedFile]) -> Result<Self, ErrorCode> {
        let schema = gt_query_run::schema_from_files(files);
        Self::from_chunks(text, gt_query_run::check_chunks(text, &schema))
    }

    fn from_chunks(
        text: &str,
        chunks: impl Iterator<Item = QueryChunk>,
    ) -> Result<Self, ErrorCode> {
        let mut diagnostics = BoundedDiagnostics::new()?;
        let mut checked = Vec::new();
        for chunk in chunks {
            match chunk.result {
                Ok(_) => checked.push(chunk),
                Err(mut diagnostic) => {
                    diagnostic.span = diagnostic.span.offset_by(chunk.range.start);
                    diagnostics.push(diagnostic)?;
                }
            }
        }
        if !diagnostics.items.is_empty() {
            return Ok(Self::Invalid {
                diagnostics: diagnostics.items,
            });
        }
        if checked.is_empty() {
            return Ok(Self::Empty);
        }
        if RunKind::of_checked_chunks(&checked) == RunKind::MixedChannel {
            for span in checked
                .iter()
                .filter_map(|chunk| chunk.channel_source_span(text))
            {
                diagnostics.push(Diagnostic {
                    span,
                    message: MIXED_CHANNEL_MESSAGE.to_owned(),
                    help: None,
                })?;
            }
            return Ok(Self::Invalid {
                diagnostics: diagnostics.items,
            });
        }
        Ok(Self::Valid)
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
