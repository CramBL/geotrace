use gt_query::{Diagnostic, Span};

use super::{BoundedDiagnostics, QueryValidation};
use crate::framing::MAX_REPLY_BYTES;
use crate::protocol::{ErrorCode, Reply};

#[test]
fn diagnostic_budget_includes_the_serialized_reply_overhead() {
    let diagnostic = |help_len| Diagnostic {
        span: Span::new(0, 1),
        message: "Invalid component".to_owned(),
        help: Some("x".repeat(help_len)),
    };
    let reply = Reply::QueryCheck {
        validation: QueryValidation::Invalid {
            diagnostics: vec![diagnostic(0)],
        },
    };
    let overhead = serde_json::to_vec(&reply).unwrap().len();
    let help_len = MAX_REPLY_BYTES - overhead;
    assert_eq!(
        BoundedDiagnostics::new()
            .unwrap()
            .push(diagnostic(help_len)),
        Ok(())
    );
    assert_eq!(
        BoundedDiagnostics::new()
            .unwrap()
            .push(diagnostic(help_len + 1)),
        Err(ErrorCode::ReplyTooLarge)
    );
}
