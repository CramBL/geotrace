use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;

/// In-memory capture for a single session bridge server.
/// Capture starts disabled and evicts the oldest frames as retention limits are reached.
#[derive(Clone)]
pub struct TrafficCapture(Arc<Mutex<CaptureState>>);

impl TrafficCapture {
    pub(crate) fn new() -> Self {
        Self(Arc::default())
    }

    /// Disabling capture preserves the retained frames for inspection.
    pub fn set_enabled(&self, enabled: bool) {
        let mut state = self.0.lock();
        if enabled != state.epoch.is_some() {
            state.epoch = enabled.then(Arc::default);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.lock().epoch.is_some()
    }

    /// Returns frames in capture order, oldest first, with shared payload storage.
    /// Retention limits apply to the server's copy of the frame list.
    /// Existing snapshots and cloned frames keep their payloads after clear or eviction.
    pub fn snapshot(&self) -> Vec<TrafficFrame> {
        self.0.lock().frames.iter().cloned().collect()
    }

    /// Removes retained frames and excludes later frames from exchanges already admitted.
    pub fn clear(&self) {
        let mut state = self.0.lock();
        state.frames.clear();
        state.retained_bytes = 0;
        if state.epoch.is_some() {
            state.epoch = Some(Arc::default());
        }
    }

    pub(crate) fn start_exchange(&self) -> Option<ExchangeCapture<'_>> {
        let mut state = self.0.lock();
        let epoch = Arc::clone(state.epoch.as_ref()?);
        state.exchange_id = state.exchange_id.checked_add(1)?;
        Some(ExchangeCapture {
            capture: self,
            epoch,
            exchange_id: state.exchange_id,
        })
    }
}

impl fmt::Debug for TrafficCapture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrafficCapture")
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct TrafficFrame {
    /// Identifies the exchange within its server, even across capture resets.
    pub exchange_id: u64,
    pub direction: TrafficDirection,
    pub observed_at: DateTime<Utc>,
    payload: Arc<Vec<u8>>,
}

impl TrafficFrame {
    /// Exact payload bytes after the length prefix, including malformed input.
    pub fn payload(&self) -> &[u8] {
        self.payload.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrafficDirection {
    Reply,
    Request,
}

pub(crate) struct ExchangeCapture<'a> {
    capture: &'a TrafficCapture,
    epoch: Arc<()>,
    exchange_id: u64,
}

impl ExchangeCapture<'_> {
    pub(crate) fn record(&self, direction: TrafficDirection, payload: Vec<u8>) {
        let mut state = self.capture.0.lock();
        if state
            .epoch
            .as_ref()
            .is_none_or(|epoch| !Arc::ptr_eq(epoch, &self.epoch))
        {
            return;
        }
        let bytes = payload.capacity();
        if bytes > MAX_CAPTURE_BYTES {
            return;
        }
        while state.frames.len() >= MAX_CAPTURE_FRAMES
            || state.retained_bytes > MAX_CAPTURE_BYTES - bytes
        {
            let Some(previous) = state.frames.pop_front() else {
                break;
            };
            state.retained_bytes -= previous.payload.capacity();
        }
        state.retained_bytes += bytes;
        state.frames.push_back(TrafficFrame {
            exchange_id: self.exchange_id,
            direction,
            observed_at: Utc::now(),
            payload: Arc::new(payload),
        });
    }
}

#[derive(Default)]
struct CaptureState {
    epoch: Option<Arc<()>>,
    exchange_id: u64,
    frames: VecDeque<TrafficFrame>,
    retained_bytes: usize,
}

pub(crate) const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CAPTURE_FRAMES: usize = 128;
