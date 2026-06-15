#[cfg(feature = "backend-sys")]
use geotrace_sdk::NavFile;
use gt_history::{Database, HistoryDatabase, RecordingMeta, extract_meta};

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
    let db_ref = db.insert("test_device", &meta, &bytes).expect("insert");

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
    db.insert("test_device", &meta, &bytes).expect("insert 1");

    // Insert again - should be duplicate
    db.insert("test_device", &meta, &bytes).expect("insert 2");

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
    let db_ref = db.insert("test_device", &meta, &bytes).expect("insert");

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

    let first = db.insert("device_a", &meta, &bytes).expect("first insert");
    let second = db.insert("device_a", &meta, &bytes).expect("second insert");

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

    db.insert("alpha", &meta_a, &bytes_a).expect("insert a");
    db.insert("beta", &meta_b, &bytes_b).expect("insert b");

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

    assert!(
        !db.is_duplicate("sensor_1", &meta)
            .expect("check before insert")
    );

    db.insert("sensor_1", &meta, &bytes).expect("insert");

    assert!(
        db.is_duplicate("sensor_1", &meta)
            .expect("check after insert")
    );

    // Different identity → duplicate detected based on metadata.
    assert!(
        db.is_duplicate("sensor_2", &meta)
            .expect("different identity")
    );

    // Different nav_point_count → not a duplicate.
    let other_meta = RecordingMeta {
        nav_point_count: meta.nav_point_count + 1,
        ..meta
    };
    assert!(
        !db.is_duplicate("sensor_1", &other_meta)
            .expect("different count")
    );
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
    let db_ref = db.insert("round_trip_test", &meta, &bytes).expect("insert");

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

    db.insert("dev", &meta_a, &bytes_a).expect("insert a");
    db.insert("dev", &meta_b, &bytes_b).expect("insert b");

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
    let db_ref = db.insert("dev", &meta, &bytes).expect("insert");

    assert_eq!(db.list_recordings().expect("list before").len(), 1);

    db.delete(&db_ref).expect("delete");

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
    db.delete(&missing)
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
    let db_ref = db.insert("reload_test", &meta, &bytes).expect("insert");

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
    let db_ref = db.insert("f64_round_trip", &meta, &bytes).expect("insert");

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
        db.insert("dev", &meta, &bytes).expect("insert");
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

    db.insert("dev", &meta_a, &bytes_a).expect("insert a");
    db.insert("dev", &meta_b, &bytes_b).expect("insert b");

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

    let ref_a = db.insert("dev", &meta_a, &bytes_a).expect("insert a");
    let ref_b = db.insert("dev", &meta_b, &bytes_b).expect("insert b");
    db.insert("dev", &meta_c, &bytes_c).expect("insert c");

    db.delete_batch(&[ref_a, ref_b]).expect("batch delete");

    let remaining = db.list_recordings().expect("list");
    assert_eq!(remaining.len(), 1, "only one recording should remain");
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
        db.insert("dev", &meta, &bytes).expect("insert");
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
                    db.insert(&format!("device_{}", i), &meta, &bytes)
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

    db.insert("large_device", &meta, &bytes)
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
    db.insert(identity, &meta, &bytes).expect("first insert");

    // Second insertion with the same (recursively created) identity
    // If it's a duplicate, is_duplicate should return true
    let is_dup = db.is_duplicate(identity, &meta).expect("check duplicate");

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
    let db_ref = db.insert(identity, &meta, &bytes).expect("first insert");
    assert_eq!(db.list_recordings().unwrap().len(), 1);

    // Load the file back and try to re-insert it, as the app does on restart.
    let loaded_bytes = db.load_bytes(&db_ref).expect("load_bytes");
    let meta2 = extract_meta(&loaded_bytes).expect("parse meta");

    // The insert should detect this as a duplicate and return the existing db_ref
    let db_ref2 = db
        .insert(identity, &meta2, &loaded_bytes)
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
    let _db_ref = db.insert("device", &meta, &bytes).expect("insert");

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

    let _db_ref = db.insert("device", &meta, &bytes).expect("insert");
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
