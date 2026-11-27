use std::mem;
use std::time::{Duration, Instant};

use gt_session_bridge::{
    DeferredSessionReply, ErrorCode, HistoryOpenId, HistoryOpenState, HistoryOpenStatus,
    MAX_DEFERRED_OPERATION_REPLIES, OpenedFile, OpenedFileState, QueryJobId, QueryJobState,
    QueryJobStatus, Reply, SessionReply,
};

use super::App;
use super::association_batches::SessionOpenWait;

pub(super) enum SessionOperationStatus {
    Files {
        wait: SessionOpenWait,
        files: Vec<OpenedFile>,
    },
    History(HistoryOpenStatus),
    Query(QueryJobStatus),
}

impl SessionOperationStatus {
    fn into_pending_and_reply(self) -> (Option<PendingOperation>, Reply) {
        let (operation, poll) = match self {
            Self::Files { wait, files } => (
                PendingOperation::Files(wait),
                OperationPoll::from_files(files),
            ),
            Self::History(status) => (
                PendingOperation::History(status.operation),
                OperationPoll::from_history(status),
            ),
            Self::Query(status) => (
                PendingOperation::Query(status.job),
                OperationPoll::from_query(status),
            ),
        };
        match poll {
            OperationPoll::Pending(reply) => (Some(operation), reply),
            OperationPoll::Complete(reply) => (None, reply),
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

enum OperationPoll {
    Complete(Reply),
    Pending(Reply),
}

impl OperationPoll {
    fn from_files(files: Vec<OpenedFile>) -> Self {
        Self::of(
            files
                .iter()
                .any(|file| file.state == OpenedFileState::Pending),
            Reply::FilesOpened { files },
        )
    }

    fn from_history(operation: HistoryOpenStatus) -> Self {
        Self::of(
            matches!(
                operation.state,
                HistoryOpenState::Loading | HistoryOpenState::Reading
            ),
            Reply::HistoryOpen { operation },
        )
    }

    fn from_query(job: QueryJobStatus) -> Self {
        Self::of(
            matches!(
                job.state,
                QueryJobState::Preparing
                    | QueryJobState::Running { .. }
                    | QueryJobState::Cancelling
            ),
            Reply::QueryJob { job },
        )
    }

    fn of(pending: bool, reply: Reply) -> Self {
        if pending {
            Self::Pending(reply)
        } else {
            Self::Complete(reply)
        }
    }
}

enum PendingOperation {
    Files(SessionOpenWait),
    History(HistoryOpenId),
    Query(QueryJobId),
}

impl PendingOperation {
    fn poll_status(&self, app: &mut App) -> Result<OperationPoll, ErrorCode> {
        match self {
            Self::Files(wait) => Ok(OperationPoll::from_files(
                app.association_batches.session_open_files(wait),
            )),
            Self::History(operation) => app
                .current_history_open_status(*operation)
                .map(OperationPoll::from_history),
            Self::Query(job) => app
                .session_query_status(job.session, *job)
                .map(OperationPoll::from_query),
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
        let (operation, initial_reply) = status.into_pending_and_reply();
        let Some(operation) = operation.filter(|_| !wait.is_zero()) else {
            reply.send(initial_reply);
            return;
        };
        let deadline = admitted_at + wait;
        let pending = &mut self.session_operation_replies.pending;
        pending.retain(|waiter| waiter.reply.client_is_waiting());
        if pending.len() >= MAX_DEFERRED_OPERATION_REPLIES {
            reply.send(initial_reply);
        } else if let Some(reply) = reply.defer_until(initial_reply, deadline) {
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
                match waiter.operation.poll_status(self) {
                    Ok(OperationPoll::Pending(reply)) => {
                        waiter.reply.publish_status(reply);
                        return Some(waiter);
                    }
                    Ok(OperationPoll::Complete(reply)) => waiter.reply.send(reply),
                    Err(error) => waiter.reply.send(error),
                }
                None
            })
            .collect();
    }
}

pub(super) const INITIAL_OPERATION_WAIT: Duration = Duration::from_secs(1);
