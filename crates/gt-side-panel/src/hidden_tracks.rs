use std::collections::BTreeSet;

use gt_history_types::DatabaseRef;

/// The hidden tracks of one recording in the history database.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct HiddenTracksInRecording {
    #[serde(flatten)]
    db_ref: DatabaseRef,
    /// The [`gt_types::track::TrackMetadata::index`] of each hidden track,
    /// ascending.
    track_numbers: Vec<usize>,
}

/// The tracks the user hid, by the recording in the history database they
/// belong to.
///
/// A settings file holds only the recordings the user curated: a recording
/// with no hidden track has no entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct HiddenTracksByRecording {
    recordings: Vec<HiddenTracksInRecording>,
}

impl HiddenTracksByRecording {
    /// The hidden track numbers of `db_ref`, ascending. A recording with no
    /// entry has an empty slice.
    pub fn track_numbers(&self, db_ref: &DatabaseRef) -> &[usize] {
        self.recordings
            .iter()
            .find(|recording| &recording.db_ref == db_ref)
            .map_or(&[], |recording| recording.track_numbers.as_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.recordings.is_empty()
    }

    /// Records `track_numbers` as the hidden tracks of `db_ref` and returns
    /// whether that changed the entry. An empty `track_numbers` drops the
    /// entry.
    pub fn record(&mut self, db_ref: &DatabaseRef, track_numbers: BTreeSet<usize>) -> bool {
        if self.track_numbers(db_ref).iter().eq(&track_numbers) {
            return false;
        }
        self.recordings
            .retain(|recording| &recording.db_ref != db_ref);
        if !track_numbers.is_empty() {
            self.recordings.push(HiddenTracksInRecording {
                db_ref: db_ref.clone(),
                track_numbers: track_numbers.into_iter().collect(),
            });
        }
        true
    }

    /// Drops the entry of `db_ref` and returns whether there was one.
    pub fn forget(&mut self, db_ref: &DatabaseRef) -> bool {
        let before = self.recordings.len();
        self.recordings
            .retain(|recording| &recording.db_ref != db_ref);
        self.recordings.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_ref(group_name: &str) -> DatabaseRef {
        DatabaseRef {
            identity: "dev".to_owned(),
            group_name: group_name.to_owned(),
        }
    }

    #[test]
    fn a_recording_with_no_hidden_track_has_no_entry() {
        let mut hidden = HiddenTracksByRecording::default();
        assert!(hidden.record(&db_ref("ride"), BTreeSet::from([2])));
        assert!(hidden.record(&db_ref("ride"), BTreeSet::new()));
        assert!(hidden.is_empty());
    }

    #[test]
    fn recording_the_same_track_numbers_again_reports_no_change() {
        let mut hidden = HiddenTracksByRecording::default();
        assert!(hidden.record(&db_ref("ride"), BTreeSet::from([2, 4])));
        assert!(!hidden.record(&db_ref("ride"), BTreeSet::from([4, 2])));
    }
}
