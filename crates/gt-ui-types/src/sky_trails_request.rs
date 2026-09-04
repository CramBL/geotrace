//! A pending request to open the whole-track sky trails window.

use gt_types::{GpsTime, TrackRef};

/// A request to open the sky trails window on a track.
///
/// Opening from a clicked track point lands on that fix. Whole-track entry
/// points (the side panel and map context menus) leave the instant unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyTrailsRequest {
    pub track: TrackRef,
    /// The moment to scrub to. `None` opens at the start of the track.
    pub at: Option<GpsTime>,
}

impl SkyTrailsRequest {
    /// Open `track` from its beginning.
    pub const fn whole_track(track: TrackRef) -> Self {
        Self { track, at: None }
    }

    /// Open `track` scrubbed to `at`.
    pub const fn at_instant(track: TrackRef, at: GpsTime) -> Self {
        Self {
            track,
            at: Some(at),
        }
    }
}
