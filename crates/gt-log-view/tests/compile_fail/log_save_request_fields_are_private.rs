use std::sync::Arc;

use gt_history_types::{DatabaseRef, StoredLogSettings};
use gt_log_view::{LogSaveCompletion, LogSaveRequest};
use gt_ui_types::LoadedLogId;

fn forge_save(
    log: LoadedLogId,
    recording: DatabaseRef,
    name: String,
    text: Arc<str>,
    settings: StoredLogSettings,
) {
    let request = LogSaveRequest::new(log, recording, name, text, settings);
    let _completion = LogSaveCompletion {
        operation: request.operation,
        result: Err(String::new()),
    };
}

fn main() {}
