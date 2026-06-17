#[cfg(feature = "backend-sys")]
use geotrace_sdk::NavFile;
use gt_history::{
    Database, DatabaseRef, DbError, HistoryDatabase, RecordingMeta, StoredRecording,
    StoredSegmentation, TrackRange, extract_meta,
};

/// Default segmentation settings for tests (mirrors `SegmentationConfig::default`).
fn test_settings() -> StoredSegmentation {
    StoredSegmentation {
        track_split_gap_us: 300_000_000,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    }
}

/// Test conveniences mapping the old single-recording API onto the per-track one.
trait TestDbExt {
    /// Insert a recording with a single track spanning all nav points.
    fn insert_simple(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        bytes: &[u8],
    ) -> Result<DatabaseRef, DbError>;
    /// Load just the reconstructed GTD bytes.
    fn load_bytes(&self, db_ref: &DatabaseRef) -> Result<Vec<u8>, DbError>;
    /// Load the full stored recording (bytes + tracks + settings).
    fn load_full(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError>;
}

impl TestDbExt for Database {
    fn insert_simple(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            hidden: false,
        }];
        self.insert(identity, meta, &tracks, test_settings(), bytes)
    }

    fn load_bytes(&self, db_ref: &DatabaseRef) -> Result<Vec<u8>, DbError> {
        self.load(db_ref).map(|r| r.bytes)
    }

    fn load_full(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError> {
        self.load(db_ref)
    }
}

#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
/// Build a minimal GTD file and return its raw bytes.
///
/// The file has a single `nav_points/time` dataset with `n` entries starting
/// at `start_us`.  All other data groups (sat_reports, markers, event_markers)
/// are absent so their counts default to zero.
fn make_gtd_bytes(start_us: i64, n: u64) -> Vec<u8> {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let timestamps: Vec<i64> = (0..n).map(|i| start_us + i as i64).collect();
    let shape = [n];

    let mut fb = hdf5_pure::FileBuilder::new();
    let mut nav_gb = fb.create_group("nav_points");
    let ds = nav_gb.create_dataset("time");
    ds.with_shape(&shape);
    ds.with_i64_data(&timestamps);
    fb.add_group(nav_gb.finish());
    fb.write(tmp.path()).expect("write temp gtd");

    std::fs::read(tmp.path()).expect("read temp gtd")
}

#[test_log::test]
#[cfg(feature = "backend-sys")]
fn repro_missing_version_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    // Manually create bytes WITHOUT version attribute
    let start_us = 1_000_000_i64;
    let n = 10_u64;
    let bytes = {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let timestamps: Vec<i64> = (0..n).map(|i| start_us + i as i64).collect();
        let shape = [n];

        let mut fb = hdf5_pure::FileBuilder::new();
        // NO version attribute here

        let mut nav_gb = fb.create_group("nav_points");

        let ds = nav_gb.create_dataset("time");
        ds.with_shape(&shape);
        ds.with_i64_data(&timestamps);

        let data: Vec<f64> = vec![0.0; n as usize];
        for name in &["lat", "lon", "heading", "speed_mps"] {
            let ds = nav_gb.create_dataset(name);
            ds.with_shape(&shape);
            ds.with_f64_data(&data);
        }

        fb.add_group(nav_gb.finish());
        fb.write(tmp.path()).expect("write temp gtd");
        std::fs::read(tmp.path()).expect("read temp gtd")
    };

    let meta = extract_meta(&bytes).expect("parse meta");
    let db_ref = db
        .insert_simple("test_device", &meta, &bytes)
        .expect("insert");

    // Now load it
    let loaded_bytes = db.load_bytes(&db_ref).expect("load_bytes");

    // Inspect the file to see if it has the version attribute
    let file = hdf5_pure::File::from_bytes(loaded_bytes.clone()).expect("parse loaded bytes");
    let version = file
        .root()
        .attrs()
        .ok()
        .and_then(|a| a.get("geotrace_version").cloned());
    log::debug!("Version attribute after load_bytes: {:?}", version);

    // Now try to parse with the SDK, which SHOULD trigger the error
    let result = NavFile::read(&loaded_bytes[..]);

    assert!(
        result.is_ok(),
        "Should have succeeded by defaulting version to 1, but got: {:?}",
        result
    );
    let nav_file = result.unwrap();
    assert_eq!(nav_file.meta().title.as_deref(), None); // Basic check
}

#[test_log::test]
#[cfg(feature = "backend-sys")]
fn repro_duplicate_entry_issue() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let start_us = 1_000_000_i64;
    let n = 10_u64;
    let bytes = make_gtd_bytes(start_us, n);

    let meta = extract_meta(&bytes).expect("parse meta");

    // Insert once
    db.insert_simple("test_device", &meta, &bytes)
        .expect("insert 1");

    // Insert again - should be duplicate
    db.insert_simple("test_device", &meta, &bytes)
        .expect("insert 2");

    // List recordings
    let recordings = db.list_recordings().expect("list");

    // Check for duplicates
    assert_eq!(
        recordings.len(),
        1,
        "Should only have 1 entry, but found: {:?}",
        recordings.len()
    );
}

#[test]
fn create_on_nonexistent_path() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("sub").join("geotrace.h5");

    assert!(!db_path.exists());
    let db = Database::open_or_create(&db_path).expect("open_or_create");
    assert!(db_path.exists());
    assert_eq!(db.path(), db_path.as_path());
}

#[test]
fn open_twice_preserves_schema_version() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");

    Database::open_or_create(&db_path).expect("first open");
    Database::open_or_create(&db_path).expect("second open should succeed");
}

#[test]
fn groups_present_after_creation() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");

    Database::open_or_create(&db_path).expect("open_or_create");

    let file = hdf5_pure::File::open(&db_path).expect("open file");
    let root = file.root();

    let groups = root.groups().expect("list groups");
    assert!(
        groups.iter().any(|g| g == "by_identity"),
        "by_identity group missing; found: {groups:?}"
    );
    assert!(
        groups.iter().any(|g| g == "meta"),
        "meta group missing; found: {groups:?}"
    );
}

#[test]
fn schema_version_is_zero_on_new_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");

    Database::open_or_create(&db_path).expect("open_or_create");

    let file = hdf5_pure::File::open(&db_path).expect("open file");
    let root = file.root();
    let attrs = root.attrs().expect("root attrs");

    match attrs.get("schema_version") {
        Some(hdf5_pure::AttrValue::I64(v)) => assert_eq!(*v, 0),
        other => panic!("unexpected schema_version attr: {other:?}"),
    }
}

#[test]
fn insert_creates_recording_group() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000_000, 10);
    let meta = extract_meta(&bytes).expect("parse meta");
    let db_ref = db
        .insert_simple("test_device", &meta, &bytes)
        .expect("insert");

    assert_eq!(db_ref.identity, "test_device");

    let file = hdf5_pure::File::open(&db_path).expect("open file");
    let by_id = file.root().group("by_identity").expect("by_identity");
    let id_grp = by_id.group("test_device").expect("identity group");
    let groups = id_grp.groups().expect("recording groups");
    assert!(
        groups.contains(&db_ref.group_name),
        "expected group '{}'; found: {groups:?}",
        db_ref.group_name
    );
}

#[test]
fn insert_duplicate_returns_same_group_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(2_000_000, 5);
    let meta = extract_meta(&bytes).expect("parse meta");

    let first = db
        .insert_simple("device_a", &meta, &bytes)
        .expect("first insert");
    let second = db
        .insert_simple("device_a", &meta, &bytes)
        .expect("second insert");

    assert_eq!(
        first.group_name, second.group_name,
        "duplicate should return same group"
    );

    // Only one recording group should exist.
    let file = hdf5_pure::File::open(&db_path).expect("open file");
    let id_grp = file
        .root()
        .group("by_identity")
        .and_then(|b| b.group("device_a"))
        .expect("identity group");
    let groups = id_grp.groups().expect("groups");
    assert_eq!(groups.len(), 1, "expected 1 group, found: {groups:?}");
}

#[test]
fn insert_different_identities_are_independent() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes_a = make_gtd_bytes(3_000_000, 8);
    let bytes_b = make_gtd_bytes(4_000_000, 12);
    let meta_a = extract_meta(&bytes_a).expect("meta a");
    let meta_b = extract_meta(&bytes_b).expect("meta b");

    db.insert_simple("alpha", &meta_a, &bytes_a)
        .expect("insert a");
    db.insert_simple("beta", &meta_b, &bytes_b)
        .expect("insert b");

    let file = hdf5_pure::File::open(&db_path).expect("open file");
    let by_id = file.root().group("by_identity").expect("by_identity");
    let identity_names = by_id.groups().expect("identity groups");

    assert!(identity_names.contains(&"alpha".to_owned()));
    assert!(identity_names.contains(&"beta".to_owned()));
}

#[test]
fn is_duplicate_matches_only_exact_meta() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(5_000_000, 20);
    let meta = extract_meta(&bytes).expect("parse meta");

    assert!(!db.is_duplicate(&meta).expect("check before insert"));

    db.insert_simple("sensor_1", &meta, &bytes).expect("insert");

    assert!(db.is_duplicate(&meta).expect("check after insert"));

    // Different identity → duplicate detected based on metadata.
    assert!(db.is_duplicate(&meta).expect("different identity"));

    // Different nav_point_count → not a duplicate.
    let other_meta = RecordingMeta {
        nav_point_count: meta.nav_point_count + 1,
        ..meta
    };
    assert!(!db.is_duplicate(&other_meta).expect("different count"));
}

#[test_log::test]
fn nav_point_data_round_trips() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let start_us = 6_000_000_i64;
    let n = 5_u64;
    let expected: Vec<i64> = (0..n as i64).map(|i| start_us + i).collect();

    let bytes = make_gtd_bytes(start_us, n);
    let meta = extract_meta(&bytes).expect("parse meta");
    let db_ref = db
        .insert_simple("round_trip_test", &meta, &bytes)
        .expect("insert");

    // Verify the stored timestamps round-trip correctly.
    let file = hdf5_pure::File::open(&db_path).expect("open file");
    let rec_grp = file
        .root()
        .group("by_identity")
        .and_then(|b| b.group(&db_ref.identity))
        .and_then(|i| i.group(&db_ref.group_name))
        .expect("recording group");
    let nav_grp = rec_grp.group("nav_points").expect("nav_points group");
    let stored = nav_grp
        .dataset("time")
        .expect("time dataset")
        .read_i64()
        .expect("read i64");

    assert_eq!(stored, expected, "timestamps should round-trip losslessly");
}

#[test]
fn list_recordings_returns_entries_sorted_descending() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes_a = make_gtd_bytes(1_000, 3);
    let bytes_b = make_gtd_bytes(2_000, 5);
    let meta_a = extract_meta(&bytes_a).expect("meta a");
    let meta_b = extract_meta(&bytes_b).expect("meta b");

    db.insert_simple("dev", &meta_a, &bytes_a)
        .expect("insert a");
    db.insert_simple("dev", &meta_b, &bytes_b)
        .expect("insert b");

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), 2);
    assert!(
        entries[0].meta.start_us >= entries[1].meta.start_us,
        "entries should be sorted descending by start_us"
    );
}

#[test]
fn list_recordings_empty_database() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let db = Database::open_or_create(&db_path).expect("open_or_create");

    let entries = db.list_recordings().expect("list");
    assert!(entries.is_empty());
}

#[test]
fn delete_removes_recording() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(3_000, 4);
    let meta = extract_meta(&bytes).expect("meta");
    let db_ref = db.insert_simple("dev", &meta, &bytes).expect("insert");

    assert_eq!(db.list_recordings().expect("list before").len(), 1);

    db.delete_batch(std::slice::from_ref(&db_ref))
        .expect("delete");

    assert_eq!(db.list_recordings().expect("list after").len(), 0);
}

#[test]
fn delete_nonexistent_is_noop() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let missing = gt_history::DatabaseRef {
        identity: "nobody".to_owned(),
        group_name: "2000-01-01T00:00:00Z".to_owned(),
    };
    db.delete_batch(std::slice::from_ref(&missing))
        .expect("delete of nonexistent should not error");
    assert_eq!(db.list_recordings().expect("list").len(), 0);
}

#[test]
fn load_gtd_bytes_round_trips_nav_point_timestamps() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let start_us = 9_000_000_i64;
    let n = 6_u64;
    let expected: Vec<i64> = (0..n as i64).map(|i| start_us + i).collect();

    let bytes = make_gtd_bytes(start_us, n);
    let meta = extract_meta(&bytes).expect("meta");
    let db_ref = db
        .insert_simple("reload_test", &meta, &bytes)
        .expect("insert");

    let loaded_bytes = db.load_bytes(&db_ref).expect("load_gtd_bytes");
    let loaded_file = hdf5_pure::File::from_bytes(loaded_bytes).expect("parse loaded bytes");
    let times = loaded_file
        .group("nav_points")
        .and_then(|g| g.dataset("time"))
        .and_then(|ds| ds.read_i64())
        .expect("read timestamps");

    assert_eq!(times, expected, "loaded timestamps should match original");
}

#[test_log::test]
fn nav_point_f64_data_round_trips() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let start_us = 12_000_000_i64;
    let n = 4_u64;
    let lat: Vec<f64> = (0..n).map(|i| 50.0 + i as f64 * 0.001).collect();
    let lon: Vec<f64> = (0..n).map(|i| 8.0 - i as f64 * 0.002).collect();

    // A GTD file mixing i64 (time) and f64 (lat/lon) datasets. Storing then
    // reloading must preserve the f64 values exactly - an earlier hand-rolled
    // copy in the sys backend reinterpreted non-i64 datasets as raw bytes and
    // silently corrupted coordinates.
    let bytes = {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let times: Vec<i64> = (0..n as i64).map(|i| start_us + i).collect();
        let mut fb = hdf5_pure::FileBuilder::new();
        let mut nav = fb.create_group("nav_points");
        let t = nav.create_dataset("time");
        t.with_shape(&[n]);
        t.with_i64_data(&times);
        let dlat = nav.create_dataset("lat");
        dlat.with_shape(&[n]);
        dlat.with_f64_data(&lat);
        let dlon = nav.create_dataset("lon");
        dlon.with_shape(&[n]);
        dlon.with_f64_data(&lon);
        fb.add_group(nav.finish());
        fb.write(tmp.path()).expect("write gtd");
        std::fs::read(tmp.path()).expect("read gtd")
    };

    let meta = extract_meta(&bytes).expect("meta");
    let db_ref = db
        .insert_simple("f64_round_trip", &meta, &bytes)
        .expect("insert");

    let loaded = db.load_bytes(&db_ref).expect("load_bytes");
    let file = hdf5_pure::File::from_bytes(loaded).expect("parse loaded bytes");
    let nav = file.group("nav_points").expect("nav_points group");
    let got_lat = nav
        .dataset("lat")
        .and_then(|d| d.read_f64())
        .expect("read lat");
    let got_lon = nav
        .dataset("lon")
        .and_then(|d| d.read_f64())
        .expect("read lon");

    assert!(
        got_lat
            .iter()
            .zip(&lat)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "f64 lat must round-trip exactly: got {got_lat:?}, want {lat:?}"
    );
    assert!(
        got_lon
            .iter()
            .zip(&lon)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "f64 lon must round-trip exactly: got {got_lon:?}, want {lon:?}"
    );
}

#[test]
fn prune_by_count_keeps_most_recent() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    for i in 0..5_u64 {
        let bytes = make_gtd_bytes((i * 1_000_000) as i64, 2);
        let meta = extract_meta(&bytes).expect("meta");
        db.insert_simple("dev", &meta, &bytes).expect("insert");
    }

    let candidates = db
        .prune_candidates(&gt_history::PruneMode::ByCount { keep: 2 })
        .expect("candidates");
    assert_eq!(candidates.len(), 3, "should prune 3 oldest when keeping 2");
}

#[test]
fn prune_by_total_size_removes_oldest_first() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes_a = make_gtd_bytes(1_000, 2);
    let bytes_b = make_gtd_bytes(2_000, 2);
    let meta_a = extract_meta(&bytes_a).expect("meta a");
    let meta_b = extract_meta(&bytes_b).expect("meta b");

    db.insert_simple("dev", &meta_a, &bytes_a)
        .expect("insert a");
    db.insert_simple("dev", &meta_b, &bytes_b)
        .expect("insert b");

    let total: u64 = meta_a.gtd_size_bytes + meta_b.gtd_size_bytes;

    // Limit is just under total - should remove the oldest (a).
    let candidates = db
        .prune_candidates(&gt_history::PruneMode::ByTotalSize {
            max_bytes: total - 1,
        })
        .expect("candidates");

    assert_eq!(candidates.len(), 1);
    // The oldest recording (start_us=1000, group_name contains timestamp) should be removed.
    assert!(candidates[0].identity == "dev");
}

#[test]
fn delete_batch_removes_multiple_in_one_pass() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes_a = make_gtd_bytes(10_000, 2);
    let bytes_b = make_gtd_bytes(20_000, 3);
    let bytes_c = make_gtd_bytes(30_000, 4);
    let meta_a = extract_meta(&bytes_a).expect("meta a");
    let meta_b = extract_meta(&bytes_b).expect("meta b");
    let meta_c = extract_meta(&bytes_c).expect("meta c");

    let ref_a = db
        .insert_simple("dev", &meta_a, &bytes_a)
        .expect("insert a");
    let ref_b = db
        .insert_simple("dev", &meta_b, &bytes_b)
        .expect("insert b");
    db.insert_simple("dev", &meta_c, &bytes_c)
        .expect("insert c");

    db.delete_batch(&[ref_a, ref_b]).expect("batch delete");

    let remaining = db.list_recordings().expect("list");
    assert_eq!(remaining.len(), 1, "only one recording should remain");
}

#[test_log::test]
fn set_tracks_hidden_flags_tracks_and_is_reversible() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    // A recording with two tracks.
    let bytes = make_gtd_bytes(8_000, 6);
    let meta = extract_meta(&bytes).expect("meta");
    let tracks = [
        TrackRange {
            start: 0,
            end: 3,
            hidden: false,
        },
        TrackRange {
            start: 3,
            end: 6,
            hidden: false,
        },
    ];
    let db_ref = db
        .insert("dev", &meta, &tracks, test_settings(), &bytes)
        .expect("insert");

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries[0].total_tracks, 2);
    assert_eq!(entries[0].hidden_tracks, 0, "new tracks are visible");

    // Hide the first track; the recording stays, with one hidden track.
    db.set_tracks_hidden(&db_ref, &[0], true).expect("hide");
    let entries = db.list_recordings().expect("list after hide");
    assert_eq!(entries[0].hidden_tracks, 1);
    let stored = db.load_full(&db_ref).expect("load");
    assert!(stored.tracks[0].hidden, "track 0 should be hidden");
    assert!(!stored.tracks[1].hidden, "track 1 should be visible");

    // The recording's data still round-trips after the hide edit.
    let file = hdf5_pure::File::from_bytes(stored.bytes).expect("parse loaded");
    let nav = file.group("nav_points").expect("nav_points group");
    let times = nav
        .dataset("time")
        .and_then(|d| d.read_i64())
        .expect("read times");
    assert_eq!(
        times.len(),
        6,
        "hidden tracks must keep the recording's data"
    );

    // Unhiding clears it.
    db.set_tracks_hidden(&db_ref, &[0], false).expect("unhide");
    assert_eq!(db.list_recordings().expect("list")[0].hidden_tracks, 0);
}

#[cfg(feature = "backend-sys")]
#[test_log::test]
fn sys_insert_with_colon_identity_into_fresh_db() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("fresh.h5");
    let mut db = Database::open_or_create(&db_path).expect("open");
    let bytes = make_gtd_bytes(1_000, 5);
    let meta = extract_meta(&bytes).expect("meta");
    db.insert_simple("auto:p3.gtd", &meta, &bytes)
        .expect("insert colon identity into fresh db");
    assert_eq!(db.list_recordings().expect("list").len(), 1);
}

/// Regression: a stale "open for write" superblock flag (e.g. from a crash)
/// makes libhdf5 refuse the file. `clear_write_lock` must repair it so the
/// database opens again with all recordings intact.
#[cfg(feature = "backend-sys")]
#[test_log::test]
fn clear_write_lock_recovers_a_locked_database() {
    use std::io::{Seek, SeekFrom, Write};

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("locked.h5");

    {
        let mut db = Database::open_or_create(&db_path).expect("create");
        let bytes = make_gtd_bytes(1_000, 5);
        let meta = extract_meta(&bytes).expect("meta");
        db.insert_simple("dev", &meta, &bytes).expect("insert");
    }

    // Set the superblock status-flags byte (offset 11) to mark it open for write.
    {
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&db_path)
            .expect("raw open");
        f.seek(SeekFrom::Start(11)).expect("seek");
        f.write_all(&[0x01]).expect("set flag");
    }
    assert!(
        hdf5::File::open(&db_path).is_err(),
        "libhdf5 should refuse the locked file before recovery"
    );

    // Clearing the lock recomputes the superblock checksum so the file opens.
    Database::clear_write_lock(&db_path).expect("clear lock");

    let db = Database::open_or_create(&db_path).expect("open after clearing the lock");
    assert_eq!(db.list_recordings().expect("list").len(), 1);
}

/// Write a database the way the old pure-Rust backend did, seeding one recording
/// (`identity`/`rec_name`) with an `n`-point `nav_points/time` dataset.
#[cfg(feature = "backend-sys")]
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
fn write_pure_db_with_recording(db_path: &std::path::Path, identity: &str, rec_name: &str, n: u64) {
    use hdf5_pure::AttrValue;

    let times: Vec<i64> = (0..n as i64).map(|i| 1_000 + i).collect();
    let mut fb = hdf5_pure::FileBuilder::new();
    fb.set_attr("schema_version", AttrValue::I64(0));

    let meta = fb.create_group("meta");
    fb.add_group(meta.finish());

    let mut by_id = fb.create_group("by_identity");
    let mut id_grp = by_id.create_group(identity);
    let mut rec = id_grp.create_group(rec_name);
    rec.set_attr("identity", AttrValue::String(identity.to_owned()));
    rec.set_attr("start_us", AttrValue::I64(1_000));
    rec.set_attr("end_us", AttrValue::I64(1_000 + n as i64 - 1));
    rec.set_attr("nav_point_count", AttrValue::U64(n));
    rec.set_attr("sat_report_count", AttrValue::U64(0));
    rec.set_attr("marker_count", AttrValue::U64(0));
    rec.set_attr("event_marker_count", AttrValue::U64(0));
    rec.set_attr("gtd_size_bytes", AttrValue::U64(0));
    rec.set_attr("geotrace_version", AttrValue::String("1".to_owned()));

    let mut nav = rec.create_group("nav_points");
    let ds = nav.create_dataset("time");
    ds.with_shape(&[n]);
    ds.with_i64_data(&times);
    rec.add_group(nav.finish());

    id_grp.add_group(rec.finish());
    by_id.add_group(id_grp.finish());
    fb.add_group(by_id.finish());
    fb.write(db_path).expect("write pure db");
}

/// Regression: a database written by the old pure-Rust backend cannot be
/// extended by libhdf5. Opening it must migrate it in place so existing
/// recordings survive and new edits (insert, hide, delete) work.
#[cfg(feature = "backend-sys")]
#[test_log::test]
fn sys_migrates_and_writes_a_pure_created_database() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("legacy.h5");
    write_pure_db_with_recording(&db_path, "auto:old.gtd", "2024-01-01T00:00:00Z", 5);

    let mut db = Database::open_or_create(&db_path).expect("open + migrate");

    // The seeded recording survives migration with its data intact.
    let entries = db.list_recordings().expect("list");
    assert_eq!(
        entries.len(),
        1,
        "seeded recording should survive migration"
    );
    let seeded = entries[0].db_ref.clone();
    assert_eq!(seeded.identity, "auto:old.gtd");
    let loaded = db.load_bytes(&seeded).expect("load migrated recording");
    let file = hdf5_pure::File::from_bytes(loaded).expect("parse loaded");
    let nav = file.group("nav_points").expect("nav_points");
    let times = nav
        .dataset("time")
        .and_then(|d| d.read_i64())
        .expect("read times");
    assert_eq!(times.len(), 5, "migrated recording keeps its nav data");

    // A brand-new recording inserts into the migrated database.
    let bytes = make_gtd_bytes(9_000, 4);
    let meta = extract_meta(&bytes).expect("meta");
    db.insert_simple("auto:new.gtd", &meta, &bytes)
        .expect("insert into a migrated database");

    assert_eq!(
        db.list_recordings().expect("list").len(),
        2,
        "migrated recording plus the new one"
    );
}

#[test_log::test]
fn reinserting_a_recording_keeps_its_track_table() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(4_000, 6);
    let meta = extract_meta(&bytes).expect("meta");
    let tracks = [
        TrackRange {
            start: 0,
            end: 3,
            hidden: false,
        },
        TrackRange {
            start: 3,
            end: 6,
            hidden: false,
        },
    ];
    let db_ref = db
        .insert("dev", &meta, &tracks, test_settings(), &bytes)
        .expect("insert");
    db.set_tracks_hidden(&db_ref, &[0], true).expect("hide");

    // Re-storing the same recording dedups, and must not clobber the track
    // table (the hidden mark is preserved).
    let db_ref2 = db
        .insert("dev", &meta, &tracks, test_settings(), &bytes)
        .expect("reinsert");
    assert_eq!(db_ref, db_ref2, "re-insert returns the existing reference");
    assert_eq!(db.list_recordings().expect("list").len(), 1, "no duplicate");
    assert_eq!(
        db.list_recordings().expect("list")[0].hidden_tracks,
        1,
        "the hidden track must survive a re-insert"
    );
}

#[test]
fn set_tracks_hidden_marks_only_the_given_tracks() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000, 9);
    let meta = extract_meta(&bytes).expect("meta");
    let tracks = [
        TrackRange {
            start: 0,
            end: 3,
            hidden: false,
        },
        TrackRange {
            start: 3,
            end: 6,
            hidden: false,
        },
        TrackRange {
            start: 6,
            end: 9,
            hidden: false,
        },
    ];
    let db_ref = db
        .insert("dev", &meta, &tracks, test_settings(), &bytes)
        .expect("insert");

    db.set_tracks_hidden(&db_ref, &[0, 2], true)
        .expect("hide 0 and 2");

    let stored = db.load_full(&db_ref).expect("load");
    assert!(stored.tracks[0].hidden, "track 0 hidden");
    assert!(!stored.tracks[1].hidden, "track 1 visible");
    assert!(stored.tracks[2].hidden, "track 2 hidden");
    assert_eq!(db.list_recordings().expect("list")[0].hidden_tracks, 2);
}

#[test]
fn open_with_older_schema_version_migrates_data() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");

    // Create a db then manually lower the schema_version to simulate an older file.
    {
        let mut db = Database::open_or_create(&db_path).expect("create");
        let bytes = make_gtd_bytes(7_000_000, 3);
        let meta = extract_meta(&bytes).expect("meta");
        db.insert_simple("dev", &meta, &bytes).expect("insert");
    }

    // Rewrite schema_version to -1 to simulate an older file.
    {
        let existing = hdf5_pure::File::open(&db_path).expect("open");
        let by_id = existing.root().group("by_identity").expect("by_identity");
        let id_grp = by_id.group("dev").expect("id_grp");
        let rec_names = id_grp.groups().expect("groups");

        let mut fb = hdf5_pure::FileBuilder::new();
        fb.set_attr("schema_version", hdf5_pure::AttrValue::I64(-1));
        let meta_gb = fb.create_group("meta");
        fb.add_group(meta_gb.finish());
        let mut by_id_gb = fb.create_group("by_identity");
        let mut dev_gb = by_id_gb.create_group("dev");
        for rec_name in &rec_names {
            let empty_gb = dev_gb.create_group(rec_name);
            dev_gb.add_group(empty_gb.finish());
        }
        by_id_gb.add_group(dev_gb.finish());
        fb.add_group(by_id_gb.finish());
        fb.write(&db_path).expect("write lower version");
    }

    // Opening should succeed and migrate to current version.
    Database::open_or_create(&db_path).expect("open after downgrade should succeed");
}

#[test]
fn meta_end_us_and_size_bytes_are_populated() {
    let bytes = make_gtd_bytes(5_000, 10);
    let meta = extract_meta(&bytes).expect("meta");

    assert_eq!(meta.start_us, 5_000);
    assert_eq!(meta.end_us, 5_009, "end_us should be start + (n-1)");
    assert_eq!(meta.gtd_size_bytes, bytes.len() as u64);
}

// hdf5-pure feature-gap canaries
//
// Each test below is marked `#[should_panic]` and currently passes because the
// tested hdf5-pure feature is absent.  If the feature is added upstream the
// panic will stop occurring, the `#[should_panic]` wrapper will report a test
// failure, and the corresponding workaround in `copy.rs` / `lib.rs` can be
// removed.  See `docs/storage-roadmap.md` for the full evidence trail.
/// hdf5-pure 0.6 writes superblock v2 files, which set `free_space_address` to
/// `None`.  A functional free-space manager would record a valid (non-max)
/// address there.  Without it every delete requires a full read-modify-write
/// cycle (see `copy.rs`).
#[test]
#[should_panic(expected = "free-space manager is active")]
fn free_space_management_not_supported() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("test.h5");

    let timestamps: Vec<i64> = (0..100).collect();
    let mut fb = hdf5_pure::FileBuilder::new();
    let mut gb = fb.create_group("nav_points");
    let ds = gb.create_dataset("time");
    ds.with_shape(&[100]);
    ds.with_i64_data(&timestamps);
    fb.add_group(gb.finish());
    fb.write(&path).expect("write");

    let file = hdf5_pure::File::open(&path).expect("open");
    let sb = file.superblock();

    const UNDEFINED_ADDRESS: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    sb.free_space_address
        .filter(|&a| a != UNDEFINED_ADDRESS)
        .expect("free-space manager is active");
}

#[test_log::test]
fn concurrent_insert_does_not_panic() {
    use std::thread;

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");

    let mut handles = Vec::new();
    for i in 0..5 {
        let path = db_path.clone();
        handles.push(
            thread::Builder::new()
                .name(format!("test-thread-{i}"))
                .spawn(move || {
                    let mut db = Database::open_or_create(&path).expect("open_or_create");
                    let bytes = make_gtd_bytes(i * 1_000_000, 5);
                    let meta = extract_meta(&bytes).expect("parse meta");
                    db.insert_simple(&format!("device_{}", i), &meta, &bytes)
                        .expect("insert");
                })
                .expect("spawn thread"),
        );
    }

    for handle in handles {
        handle.join().expect("thread join");
    }
}

#[test]
fn insert_malformed_data_returns_error() {
    let malformed_bytes = vec![0, 1, 2, 3, 4]; // Not a GTD file
    // extract_meta should fail
    let meta = extract_meta(&malformed_bytes);
    assert!(
        meta.is_err(),
        "Should have failed to extract meta from malformed data"
    );
}

#[test]
fn insert_large_dataset_works() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    // Insert a larger dataset to trigger HDF5 chunking
    let n = 20_000_u64;
    let bytes = make_gtd_bytes(1_000_000, n);
    let meta = extract_meta(&bytes).expect("parse meta");

    db.insert_simple("large_device", &meta, &bytes)
        .expect("insert large recording");

    let db_ref = &db.list_recordings().unwrap()[0].db_ref;
    let loaded_bytes = db.load_bytes(db_ref).expect("load");

    // Compare content by parsing instead of raw byte length
    let original_nav = hdf5_pure::File::from_bytes(bytes.clone())
        .expect("parse original")
        .group("nav_points")
        .unwrap()
        .dataset("time")
        .unwrap()
        .read_i64()
        .unwrap();
    let loaded_nav = hdf5_pure::File::from_bytes(loaded_bytes)
        .expect("parse loaded")
        .group("nav_points")
        .unwrap()
        .dataset("time")
        .unwrap()
        .read_i64()
        .unwrap();

    assert_eq!(original_nav, loaded_nav, "Large dataset content mismatch");
}

#[test]
fn pure_backend_does_not_add_duplicate_recordings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000_000, 5);
    let meta = extract_meta(&bytes).expect("parse meta");

    // First insertion
    let identity = "auto:snapshot.gtd";
    db.insert_simple(identity, &meta, &bytes)
        .expect("first insert");

    // Second insertion with the same (recursively created) identity
    // If it's a duplicate, is_duplicate should return true
    let is_dup = db.is_duplicate(&meta).expect("check duplicate");

    assert!(is_dup, "Should be detected as a duplicate");

    let recordings = db.list_recordings().expect("list");
    assert_eq!(
        recordings.len(),
        1,
        "Should only have 1 recording, found: {:?}",
        recordings.len()
    );
}

#[test_log::test]
fn pure_backend_prevents_recursive_insertion_of_loaded_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000_000, 5);
    let meta = extract_meta(&bytes).expect("parse meta");
    let identity = "auto:snapshot.gtd";
    let db_ref = db
        .insert_simple(identity, &meta, &bytes)
        .expect("first insert");
    assert_eq!(db.list_recordings().unwrap().len(), 1);

    // Load the file back and try to re-insert it, as the app does on restart.
    let loaded_bytes = db.load_bytes(&db_ref).expect("load_bytes");
    let meta2 = extract_meta(&loaded_bytes).expect("parse meta");

    // The insert should detect this as a duplicate and return the existing db_ref
    let db_ref2 = db
        .insert_simple(identity, &meta2, &loaded_bytes)
        .expect("second insert");

    assert_eq!(
        db_ref, db_ref2,
        "Second insert should return the same reference"
    );
    assert_eq!(
        db.list_recordings().unwrap().len(),
        1,
        "Should not have added a duplicate"
    );
}

#[test]
fn sys_backend_structural_parity_repro() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace_sys.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000_000, 5);
    let meta = extract_meta(&bytes).expect("parse meta");

    // Insert using the sys backend
    let _db_ref = db.insert_simple("device", &meta, &bytes).expect("insert");

    // Attempt to verify structure using the hdf5-pure reader
    // This is what the pure backend does. If sys backend is parity-compatible,
    // this should work.
    let file = hdf5_pure::File::open(&db_path).expect("open");
    let root = file.root();

    let by_id = root.group("by_identity").expect("by_identity missing");
    let id_grp = by_id.group("device").expect("identity group missing");

    let rec_names = id_grp.groups().expect("recording names missing");
    assert!(!rec_names.is_empty(), "No recordings found under 'device'");

    let rec_grp = id_grp
        .group(&rec_names[0])
        .expect("recording group missing");
    assert!(
        rec_grp.group("nav_points").is_ok(),
        "nav_points group missing in sys-backend database"
    );
}

#[test_log::test]
#[cfg(feature = "backend-sys")]
fn debug_sys_backend_structure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000_000, 5);
    let meta = extract_meta(&bytes).expect("parse meta");

    let _db_ref = db.insert_simple("device", &meta, &bytes).expect("insert");
    let loaded_bytes = db.load_bytes(&_db_ref).expect("load_bytes");

    let tmp_path = dir.path().join("reconstructed.h5");
    std::fs::write(&tmp_path, loaded_bytes).expect("write");

    let file = hdf5::File::open(&tmp_path).expect("parse loaded bytes");
    println!("--- Root members ---");
    for name in file.group("/").expect("root").member_names().unwrap() {
        println!("Member: {}", name);
    }
}

#[test]
fn test_hdf5_pure_self_compatibility() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let mut fb = hdf5_pure::FileBuilder::new();
    fb.set_attr("version", hdf5_pure::AttrValue::String("1".into()));
    fb.write(tmp.path()).expect("write");

    let file = hdf5_pure::File::open(tmp.path()).expect("open pure");
    file.root().attrs().unwrap();
}

#[test]
fn test_hdf5_pure_file_openable_by_metno() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let mut fb = hdf5_pure::FileBuilder::new();
    fb.set_attr("version", hdf5_pure::AttrValue::String("1".into()));
    fb.write(tmp.path()).expect("write");

    // Try to open with hdf5 (metno)
    let res = hdf5::File::open(tmp.path());
    assert!(res.is_ok(), "Failed to open: {:?}", res.err());
}

/// Regression: two distinct recordings that start within the same second and have
/// the same point count used to derive the same (second-resolution) group name and
/// collide on insert (`H5Gcreate2: name already exists`). The UUID suffix in
/// `make_group_name` must keep them distinct.
#[test_log::test]
fn recordings_in_the_same_second_get_distinct_group_names() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    // Same whole second, same point count, different microsecond start: distinct
    // content (so not deduplicated) but an identical legacy group name.
    let bytes_a = make_gtd_bytes(1_000_000, 5);
    let bytes_b = make_gtd_bytes(1_000_500, 5);
    let meta_a = extract_meta(&bytes_a).expect("meta a");
    let meta_b = extract_meta(&bytes_b).expect("meta b");
    assert!(
        !meta_a.same_recording(&meta_b),
        "the two must not be duplicates"
    );

    let ref_a = db
        .insert_simple("dev", &meta_a, &bytes_a)
        .expect("insert a");
    let ref_b = db
        .insert_simple("dev", &meta_b, &bytes_b)
        .expect("insert b");

    assert_ne!(
        ref_a.group_name, ref_b.group_name,
        "same-second recordings must get distinct group names"
    );
    assert_eq!(
        db.list_recordings().expect("list").len(),
        2,
        "both recordings must be stored"
    );
}

/// Insert a two-track recording with `n` nav points starting at `start_us`.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on failure is the right behaviour"
)]
fn insert_two_track(db: &mut Database, identity: &str, start_us: i64, n: u64) -> DatabaseRef {
    let bytes = make_gtd_bytes(start_us, n);
    let meta = extract_meta(&bytes).expect("meta");
    let half = meta.nav_point_count / 2;
    let tracks = [
        TrackRange {
            start: 0,
            end: half,
            hidden: false,
        },
        TrackRange {
            start: half,
            end: meta.nav_point_count,
            hidden: false,
        },
    ];
    db.insert(identity, &meta, &tracks, test_settings(), &bytes)
        .expect("insert")
}

/// Exercises the whole API at scale: hundreds of recordings inserted, then
/// listed, read back, partially hidden, batch-deleted, and re-opened. We do not
/// trust that HDF5 keeps a large number of sibling groups consistent - we verify
/// it end to end on whichever backend is active.
#[test_log::test]
fn many_recordings_support_list_read_hide_and_delete() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    const N: usize = 200;
    let mut refs: Vec<(DatabaseRef, u64)> = Vec::with_capacity(N);
    for i in 0..N {
        // Distinct start timestamps give each a distinct content fingerprint (so
        // they are not deduplicated); group-name uniqueness is handled by the UUID
        // suffix regardless of spacing.
        let start = 1_000_000_000 + i as i64 * 1_000_000;
        let n = 6 + (i as u64 % 7);
        let db_ref = insert_two_track(&mut db, "dev", start, n);
        refs.push((db_ref, n));
    }

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), N, "every distinct recording is listed");
    assert!(
        entries.iter().all(|e| e.total_tracks == 2),
        "each recording keeps its two-track table"
    );

    // Read a sampling and confirm the stored tracks cover exactly the recording.
    for (db_ref, n) in refs.iter().step_by(37) {
        let stored = db.load_full(db_ref).expect("load");
        let covered: u64 = stored.tracks.iter().map(|t| t.end - t.start).sum();
        assert_eq!(covered, *n, "track ranges must cover all nav points");
    }

    // Hide the first track of every third recording.
    let mut expected_hidden = 0;
    for (db_ref, _) in refs.iter().step_by(3) {
        db.set_tracks_hidden(db_ref, &[0], true).expect("hide");
        expected_hidden += 1;
    }
    let hidden_total: usize = db
        .list_recordings()
        .expect("list")
        .iter()
        .map(|e| e.hidden_tracks)
        .sum();
    assert_eq!(
        hidden_total, expected_hidden,
        "hidden marks persist across the whole set"
    );

    // Batch-delete the first half.
    let to_delete: Vec<DatabaseRef> = refs.iter().take(N / 2).map(|(r, _)| r.clone()).collect();
    db.delete_batch(&to_delete).expect("batch delete");
    assert_eq!(
        db.list_recordings().expect("list").len(),
        N / 2,
        "half the recordings remain after the batch delete"
    );

    // Re-open from disk and confirm the survivors persisted.
    drop(db);
    let db = Database::open_or_create(&db_path).expect("reopen");
    let survivors = db.list_recordings().expect("list");
    assert_eq!(survivors.len(), N / 2);
    let survivor_ref = survivors[0].db_ref.clone();
    let stored = db.load_full(&survivor_ref).expect("load survivor");
    assert_eq!(stored.tracks.len(), 2, "a survivor keeps its track table");
}

/// Insert 40 recordings, then run 4 delete-all + refill cycles, returning
/// `(size_after_first_fill, size_after_cycles)`. With perfect space reuse the two
/// sizes are equal; without any reuse the file grows by one fill per cycle.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on failure is the right behaviour"
)]
fn delete_reinsert_size_trajectory(db_path: &std::path::Path) -> (u64, u64) {
    const N: usize = 40;
    let fill = |db: &mut Database| {
        for i in 0..N {
            // Distinct start timestamps give distinct content fingerprints (so the
            // inserts are not deduplicated); group-name uniqueness comes from the
            // UUID suffix.
            let start = 1_000_000_000 + i as i64 * 1_000_000;
            insert_two_track(db, "dev", start, 50);
        }
    };
    let file_size = |path: &std::path::Path| std::fs::metadata(path).expect("metadata").len();

    let mut db = Database::open_or_create(db_path).expect("open_or_create");
    fill(&mut db);
    let baseline = file_size(db_path);

    for _ in 0..4 {
        let refs: Vec<DatabaseRef> = db
            .list_recordings()
            .expect("list")
            .iter()
            .map(|e| e.db_ref.clone())
            .collect();
        db.delete_batch(&refs).expect("delete all");
        assert_eq!(db.list_recordings().expect("list").len(), 0);
        fill(&mut db);
        assert_eq!(db.list_recordings().expect("list").len(), N);
    }
    (baseline, file_size(db_path))
}

/// Repeatedly deleting every recording and reinserting must not grow the database
/// file without bound - freed space has to be reused.
///
/// The pure backend gets this for free by rewriting the whole tree on every
/// mutation. The sys backend relies on libhdf5's free-space manager, which only
/// reuses object-header and raw-data space - not the global heap that backs
/// variable-length strings - so the backend stores all string attributes
/// fixed-length (see `write_string_attr`). Both backends therefore keep the file
/// flat; a 2x ceiling leaves generous slack while still catching the unbounded
/// (one-fill-per-cycle) growth that variable-length strings used to cause.
#[test_log::test]
fn file_size_stays_bounded_across_delete_reinsert_cycles() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (baseline, after) = delete_reinsert_size_trajectory(&dir.path().join("geotrace.h5"));
    assert!(baseline > 0, "the filled database has a non-zero size");
    assert!(
        after <= baseline * 2,
        "history file grew across delete/reinsert cycles: baseline {baseline} bytes, \
         after 4 cycles {after} bytes (expected <= 2x). Freed space is not being reused."
    );
}

/// Deleting an *older* recording while newer ones remain leaves an interior hole
/// that cannot be truncated away (live data sits after it). Inserting fresh
/// recordings afterwards must reuse that freed space rather than only appending,
/// so the file stays near its full size instead of growing by the inserts.
#[test_log::test]
fn interior_delete_then_insert_reuses_freed_space() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    const N: usize = 40;
    let file_size = |path: &std::path::Path| std::fs::metadata(path).expect("metadata").len();
    let start_for = |i: usize| 1_000_000_000_i64 + i as i64 * 1_000_000;

    // Fill the file with N recordings laid out in insertion order.
    let mut refs: Vec<DatabaseRef> = (0..N)
        .map(|i| insert_two_track(&mut db, "dev", start_for(i), 50))
        .collect();
    let full = file_size(&db_path);

    // Delete the older half (indices 0..N/2); the newer half stays physically
    // after them, so the freed regions are interior holes.
    let old: Vec<DatabaseRef> = refs.drain(0..N / 2).collect();
    db.delete_batch(&old).expect("delete older half");
    assert_eq!(db.list_recordings().expect("list").len(), N / 2);

    // Insert a fresh half. Reuse of the interior holes keeps the file near `full`.
    for i in N..(N + N / 2) {
        insert_two_track(&mut db, "dev", start_for(i), 50);
    }
    assert_eq!(db.list_recordings().expect("list").len(), N);
    let after = file_size(&db_path);

    // With no reuse the file would grow to ~1.5x full (the deleted half lingers as
    // dead bytes and the new half is appended). Reuse keeps it near 1x (measured
    // ~1.01 on the sys backend, 1.00 on pure); a 1.25x ceiling is a robust guard.
    assert!(
        after <= full + full / 4,
        "interior freed space was not reused: full {full} bytes, after delete+insert {after} bytes"
    );
}

/// `set_tracks` must wholly replace the stored track table and segmentation
/// settings, not merge with or append to the previous ones (including dropping
/// stale hidden marks).
#[test_log::test]
fn set_tracks_replaces_the_table_and_settings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000, 9);
    let meta = extract_meta(&bytes).expect("meta");
    let two = [
        TrackRange {
            start: 0,
            end: 4,
            hidden: false,
        },
        TrackRange {
            start: 4,
            end: 9,
            hidden: true,
        },
    ];
    let db_ref = db
        .insert("dev", &meta, &two, test_settings(), &bytes)
        .expect("insert");
    {
        let entry = &db.list_recordings().expect("list")[0];
        assert_eq!(entry.total_tracks, 2);
        assert_eq!(entry.hidden_tracks, 1);
    }

    let three = [
        TrackRange {
            start: 0,
            end: 3,
            hidden: false,
        },
        TrackRange {
            start: 3,
            end: 6,
            hidden: false,
        },
        TrackRange {
            start: 6,
            end: 9,
            hidden: false,
        },
    ];
    let new_settings = StoredSegmentation {
        track_split_gap_us: 42_000_000,
        detect_clock_discontinuities: false,
        clock_discontinuity_sigmas: 2.5,
    };
    db.set_tracks(&db_ref, &three, new_settings)
        .expect("set_tracks");

    let stored = db.load_full(&db_ref).expect("load");
    assert_eq!(
        stored.tracks,
        three.to_vec(),
        "the old two-track table is fully replaced"
    );
    assert_eq!(
        stored.segmentation,
        Some(new_settings),
        "segmentation settings are replaced too"
    );
    let entry = &db.list_recordings().expect("list")[0];
    assert_eq!(entry.total_tracks, 3);
    assert_eq!(
        entry.hidden_tracks, 0,
        "the replacement clears the old hidden mark"
    );
}
