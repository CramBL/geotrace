use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use gt_history_types::StoredLayoutRevision;

#[derive(Debug)]
pub struct ObservedVisibilityWrite {
    pub layout: StoredLayoutRevision,
    pub hidden: BTreeSet<usize>,
    pub receipt: Option<VisibilityStoreReceipt>,
}

#[derive(Clone, Debug)]
pub struct VisibilityStoreReceipt(Arc<AtomicU8>);

#[repr(u8)]
enum VisibilityStoreStatus {
    Expired,
    InFlight,
    Settled,
}

impl VisibilityStoreReceipt {
    pub(crate) fn in_flight() -> Self {
        Self(Arc::new(AtomicU8::new(
            VisibilityStoreStatus::InFlight as u8,
        )))
    }

    pub fn settle(&self, layout_expired: bool) {
        let status = if layout_expired {
            VisibilityStoreStatus::Expired
        } else {
            VisibilityStoreStatus::Settled
        };
        self.0.store(status as u8, Ordering::Release);
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.0.load(Ordering::Acquire) == VisibilityStoreStatus::Settled as u8
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.0.load(Ordering::Acquire) == VisibilityStoreStatus::Expired as u8
    }
}
