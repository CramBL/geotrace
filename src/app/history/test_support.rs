//! Listing entries for the tests of the History window and of its
//! delete-shelved confirmation.

use gt_store::{DatabaseRef, RecordingEntry, RecordingMeta};

/// How many tracks a listing entry states in all.
pub(super) struct TotalTracks(pub usize);

/// How many of a listing entry's tracks are shelved.
pub(super) struct ShelvedTracks(pub usize);

/// A listing entry for `identity` with no tracks and no SDK metadata.
pub(super) fn entry_with_identity(identity: &str) -> RecordingEntry {
    RecordingEntry {
        db_ref: DatabaseRef {
            identity: identity.to_owned(),
            group_name: "rec0".to_owned(),
        },
        meta: RecordingMeta {
            time_range: None,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        total_tracks: 0,
        shelved_tracks: 0,
        title: None,
        device: None,
        notes: None,
        travel_mode: None,
        channels: Vec::new(),
        log_attachments: Vec::new(),
    }
}

/// A listing entry for `identity` with the track counts the delete of shelved
/// data reads.
pub(super) fn entry_with_shelved_tracks(
    identity: &str,
    TotalTracks(total_tracks): TotalTracks,
    ShelvedTracks(shelved_tracks): ShelvedTracks,
) -> RecordingEntry {
    let mut entry = entry_with_identity(identity);
    entry.total_tracks = total_tracks;
    entry.shelved_tracks = shelved_tracks;
    entry
}
