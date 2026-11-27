use std::io;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, watch};
use tokio::time;

use crate::Reply;

pub struct SessionReply {
    sender: oneshot::Sender<ExchangeReply>,
    defer_before: Instant,
}

impl SessionReply {
    pub(super) fn channel(exchange_deadline: Instant) -> (Self, oneshot::Receiver<ExchangeReply>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                sender,
                defer_before: exchange_deadline
                    .checked_sub(REPLY_DELIVERY_RESERVE)
                    .unwrap_or_else(Instant::now),
            },
            receiver,
        )
    }

    pub fn client_is_waiting(&self) -> bool {
        !self.sender.is_closed()
    }

    pub fn send(self, reply: impl Into<Reply>) {
        drop(self.sender.send(ExchangeReply::Immediate(reply.into())));
    }

    /// The bridge reserves time to send the reply before the exchange deadline.
    pub fn defer_until(
        self,
        status: impl Into<Reply>,
        deadline: Instant,
    ) -> Option<DeferredSessionReply> {
        let deadline = deadline.min(self.defer_before);
        let status = status.into();
        if Instant::now() >= deadline {
            self.send(status);
            return None;
        }
        let (sender, latest) = watch::channel(PublishedReply::Pending(status));
        self.sender
            .send(ExchangeReply::Deferred(DeferredResponse {
                latest,
                deadline,
            }))
            .ok()?;
        Some(DeferredSessionReply {
            latest: sender,
            deadline,
        })
    }
}

pub struct DeferredSessionReply {
    latest: watch::Sender<PublishedReply>,
    deadline: Instant,
}

impl DeferredSessionReply {
    pub fn client_is_waiting(&self) -> bool {
        !self.latest.is_closed()
    }

    pub fn publish_status(&self, status: impl Into<Reply>) {
        self.publish(PublishedReply::Pending(status.into()));
    }

    pub fn send(self, reply: impl Into<Reply>) {
        self.publish(PublishedReply::Complete(reply.into()));
    }

    fn publish(&self, reply: PublishedReply) {
        self.latest.send_if_modified(|previous| {
            if Instant::now() >= self.deadline || *previous == reply {
                return false;
            }
            *previous = reply;
            true
        });
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "The oneshot allocates space for the immediate reply"
)]
pub(super) enum ExchangeReply {
    Deferred(DeferredResponse),
    Immediate(Reply),
}

impl ExchangeReply {
    pub(super) async fn resolve(self) -> io::Result<Reply> {
        match self {
            Self::Deferred(response) => response.resolve().await,
            Self::Immediate(reply) => Ok(reply),
        }
    }
}

pub(super) struct DeferredResponse {
    latest: watch::Receiver<PublishedReply>,
    deadline: Instant,
}

impl DeferredResponse {
    async fn resolve(mut self) -> io::Result<Reply> {
        let expiry = time::sleep_until(self.deadline.into());
        tokio::pin!(expiry);
        loop {
            if let Some(reply) = self.ready_reply() {
                return Ok(reply);
            }
            tokio::select! {
                biased;
                () = &mut expiry => return Ok(self.latest.borrow().reply().clone()),
                updated = self.latest.changed() => {
                    if let Err(error) = updated {
                        return self.ready_reply().ok_or_else(|| io::Error::other(error));
                    }
                }
            }
        }
    }

    pub(super) fn ready_reply(&mut self) -> Option<Reply> {
        let status = self.latest.borrow_and_update();
        (matches!(*status, PublishedReply::Complete(_)) || Instant::now() >= self.deadline)
            .then(|| status.reply().clone())
    }
}

#[derive(PartialEq)]
enum PublishedReply {
    Complete(Reply),
    Pending(Reply),
}

impl PublishedReply {
    fn reply(&self) -> &Reply {
        match self {
            Self::Complete(reply) | Self::Pending(reply) => reply,
        }
    }
}

const REPLY_DELIVERY_RESERVE: Duration = Duration::from_millis(500);
