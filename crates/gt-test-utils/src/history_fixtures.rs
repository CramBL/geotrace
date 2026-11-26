use std::path::{Path, PathBuf};

use gt_history_types::{
    ATTR_IDENTITY, CURRENT_UI_STATE_VERSION, DatabaseRef, HIDDEN_TRACKS_DATASET, HistoryAccess,
    TRACK_END_DATASET, TRACK_START_DATASET, TRACK_STATE_DATASET, TRACKS_GROUP, TrackRange,
    UI_STATE_GROUP, UI_STATE_VERSION_ATTR,
};
use hdf5_pure::{AttrValue, File, FileBuilder, Group};
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

#[expect(
    clippy::expect_used,
    reason = "The test requires a newer UI state version"
)]
pub fn set_newer_visibility_version(path: &Path, recording: &DatabaseRef) {
    let _access = HistoryAccess::lock();
    let file = File::open_rw(path).expect("edit the History fixture");
    let version = AttrValue::I64(CURRENT_UI_STATE_VERSION + 1);
    self::stored_visibility_group(&file, recording)
        .set_attr(UI_STATE_VERSION_ATTR, version)
        .expect("set the UI state version");
    file.commit().expect("publish the UI state version");
}

#[expect(clippy::expect_used, reason = "The fixture has stored visibility")]
pub fn raw_visibility_version_and_tracks(path: &Path, recording: &DatabaseRef) -> (i64, Vec<u64>) {
    let _access = HistoryAccess::lock();
    let file = File::open(path).expect("read the History fixture");
    let group = self::stored_visibility_group(&file, recording);
    let version = group
        .attrs()
        .expect("UI state attributes")
        .remove(UI_STATE_VERSION_ATTR)
        .and_then(|value| match value {
            AttrValue::I64(version) => Some(version),
            _ => None,
        })
        .expect("an integer UI state version");
    let tracks = group
        .dataset(HIDDEN_TRACKS_DATASET)
        .and_then(|dataset| dataset.read_u64())
        .expect("read the raw hidden track numbers");
    (version, tracks)
}

#[expect(clippy::expect_used, reason = "The fixture has saved UI state")]
fn stored_visibility_group(file: &File, recording: &DatabaseRef) -> Group {
    file.group(&format!(
        "by_identity/{}/{}/{UI_STATE_GROUP}",
        gt_history_types::identity_group_name(&recording.identity),
        recording.group_name,
    ))
    .expect("the saved UI state group")
}
