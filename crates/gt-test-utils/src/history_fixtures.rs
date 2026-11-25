use std::path::PathBuf;

use gt_history_types::{
    ATTR_IDENTITY, DatabaseRef, TRACK_END_DATASET, TRACK_START_DATASET, TRACK_STATE_DATASET,
    TRACKS_GROUP, TrackRange,
};
use hdf5_pure::{AttrValue, FileBuilder};
use tempfile::TempDir;

pub struct StoredTrackTableFixture {
    directory: TempDir,
}

impl StoredTrackTableFixture {
    #[expect(
        clippy::expect_used,
        reason = "Fixture generation fails the test on IO errors"
    )]
    pub fn new(recording: &DatabaseRef, rows: &[TrackRange]) -> Self {
        let directory = tempfile::tempdir().expect("history fixture directory");
        let fixture = Self { directory };
        let mut file = FileBuilder::new();
        let mut identities = file.create_group("by_identity");
        let mut identity =
            identities.create_group(&gt_history_types::identity_group_name(&recording.identity));
        identity.set_attr(ATTR_IDENTITY, AttrValue::String(recording.identity.clone()));
        let mut recording = identity.create_group(&recording.group_name);
        let mut tracks = recording.create_group(TRACKS_GROUP);
        let (starts, ends, states) = gt_history_types::track_columns(rows);
        for (name, column) in [
            (TRACK_START_DATASET, starts),
            (TRACK_END_DATASET, ends),
            (TRACK_STATE_DATASET, states),
        ] {
            tracks.create_dataset(name).with_u64_data(&column);
        }
        recording.add_group(tracks.finish());
        identity.add_group(recording.finish());
        identities.add_group(identity.finish());
        file.add_group(identities.finish());
        file.write(fixture.path())
            .expect("stored track table fixture");
        fixture
    }

    pub fn path(&self) -> PathBuf {
        self.directory.path().join("history.h5")
    }
}
