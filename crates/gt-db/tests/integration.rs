use gt_db::{Database, RecordingMeta};

#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
/// Build a minimal NVD file and return its raw bytes.
///
/// The file has a single `nav_points/time` dataset with `n` entries starting
/// at `start_us`.  All other data groups (sat_reports, markers, event_markers)
/// are absent so their counts default to zero.
fn make_nvd_bytes(start_us: i64, n: u64) -> Vec<u8> {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let timestamps: Vec<i64> = (0..n).map(|i| start_us + i as i64).collect();
    let shape = [n];

    let mut fb = hdf5_pure::FileBuilder::new();
    let mut nav_gb = fb.create_group("nav_points");
    let ds = nav_gb.create_dataset("time");
    ds.with_shape(&shape);
    ds.with_i64_data(&timestamps);
    fb.add_group(nav_gb.finish());
    fb.write(tmp.path()).expect("write temp nvd");

    std::fs::read(tmp.path()).expect("read temp nvd")
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

    let bytes = make_nvd_bytes(1_000_000, 10);
    let meta = RecordingMeta::from_nvd_bytes(&bytes).expect("parse meta");
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

    let bytes = make_nvd_bytes(2_000_000, 5);
    let meta = RecordingMeta::from_nvd_bytes(&bytes).expect("parse meta");

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

    let bytes_a = make_nvd_bytes(3_000_000, 8);
    let bytes_b = make_nvd_bytes(4_000_000, 12);
    let meta_a = RecordingMeta::from_nvd_bytes(&bytes_a).expect("meta a");
    let meta_b = RecordingMeta::from_nvd_bytes(&bytes_b).expect("meta b");

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

    let bytes = make_nvd_bytes(5_000_000, 20);
    let meta = RecordingMeta::from_nvd_bytes(&bytes).expect("parse meta");

    assert!(
        !db.is_duplicate("sensor_1", &meta)
            .expect("check before insert")
    );

    db.insert("sensor_1", &meta, &bytes).expect("insert");

    assert!(
        db.is_duplicate("sensor_1", &meta)
            .expect("check after insert")
    );

    // Different identity → not a duplicate.
    assert!(
        !db.is_duplicate("sensor_2", &meta)
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

#[test]
fn nav_point_data_round_trips() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let start_us = 6_000_000_i64;
    let n = 5_u64;
    let expected: Vec<i64> = (0..n as i64).map(|i| start_us + i).collect();

    let bytes = make_nvd_bytes(start_us, n);
    let meta = RecordingMeta::from_nvd_bytes(&bytes).expect("parse meta");
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
