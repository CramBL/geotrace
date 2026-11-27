use std::mem;
use std::time::{Duration, Instant};

use gt_session_bridge::{
    DeferredSessionReply, ErrorCode, HistoryOpenId, HistoryOpenState, HistoryOpenStatus,
    MAX_DEFERRED_OPERATION_REPLIES, QueryJobId, QueryJobState, QueryJobStatus, Reply, SessionReply,
};

use super::App;

pub(super) enum SessionOperationStatus {
    History(HistoryOpenStatus),
    Query(QueryJobStatus),
}

impl SessionOperationStatus {
    fn pending_operation(&self) -> Option<PendingOperation> {
        match self {
            Self::History(status) => matches!(
                status.state,
                HistoryOpenState::Loading | HistoryOpenState::Reading
            )
            .then_some(PendingOperation::History(status.operation)),
            Self::Query(status) => matches!(
                status.state,
                QueryJobState::Preparing
                    | QueryJobState::Running { .. }
                    | QueryJobState::Cancelling
            )
            .then_some(PendingOperation::Query(status.job)),
        }
    }
}

impl From<SessionOperationStatus> for Reply {
    fn from(status: SessionOperationStatus) -> Self {
        match status {
            SessionOperationStatus::History(operation) => Self::HistoryOpen { operation },
            SessionOperationStatus::Query(job) => Self::QueryJob { job },
        }
    }
}

#[derive(Default)]
pub(super) struct SessionOperationReplies {
    pending: Vec<OperationWaiter>,
}

#[cfg(test)]
impl SessionOperationReplies {
    pub(super) fn waiting_count(&self) -> usize {
        self.pending.len()
    }
}

struct OperationWaiter {
    operation: PendingOperation,
    reply: DeferredSessionReply,
}

enum PendingOperation {
    History(HistoryOpenId),
    Query(QueryJobId),
}

impl PendingOperation {
    fn status(&self, app: &mut App) -> Result<SessionOperationStatus, ErrorCode> {
        match *self {
            Self::History(operation) => app
                .current_history_open_status(operation)
                .map(SessionOperationStatus::History),
            Self::Query(job) => app
                .session_query_status(job.session, job)
                .map(SessionOperationStatus::Query),
        }
    }
}

impl App {
    pub(super) fn await_session_operation(
        &mut self,
        status: SessionOperationStatus,
        wait: Duration,
        reply: SessionReply,
        admitted_at: Instant,
    ) {
        let Some(operation) = status.pending_operation().filter(|_| !wait.is_zero()) else {
            reply.send(status);
            return;
        };
        let deadline = admitted_at + wait;
        let pending = &mut self.session_operation_replies.pending;
        pending.retain(|waiter| waiter.reply.client_is_waiting());
        if pending.len() >= MAX_DEFERRED_OPERATION_REPLIES {
            reply.send(status);
        } else if let Some(reply) = reply.defer_until(status, deadline) {
            pending.push(OperationWaiter { operation, reply });
        }
    }

    pub(super) fn publish_session_operation_replies(&mut self) {
        let previous = mem::take(&mut self.session_operation_replies.pending);
        self.session_operation_replies.pending = previous
            .into_iter()
            .filter_map(|waiter| {
                if !waiter.reply.client_is_waiting() {
                    return None;
                }
                match waiter.operation.status(self) {
                    Ok(status) if status.pending_operation().is_some() => {
                        waiter.reply.publish_status(status);
                        return Some(waiter);
                    }
                    Ok(status) => waiter.reply.send(status),
                    Err(error) => waiter.reply.send(error),
                }
                None
            })
            .collect();
    }
}
