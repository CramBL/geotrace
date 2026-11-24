use std::iter;

use gt_query::{Diagnostic, Span};
use gt_query_run::QueryChunk;

use super::QueryValidation;
use crate::framing::MAX_REPLY_BYTES;
use crate::protocol::ErrorCode;

#[test]
fn validation_stops_checking_at_the_diagnostic_budget() {
    let rejected = || QueryChunk {
        range: 0..1,
        result: Err(Diagnostic {
            span: Span::new(0, 1),
            message: "Invalid component".to_owned(),
            help: Some("x".repeat(MAX_REPLY_BYTES / 2)),
        }),
    };
    let chunks = iter::repeat_with(rejected).take(2).chain(iter::from_fn(|| {
        panic!("checked a chunk past the diagnostic budget");
    }));
    assert_eq!(
        QueryValidation::from_chunks("a", chunks),
        Err(ErrorCode::ReplyTooLarge)
    );
}
