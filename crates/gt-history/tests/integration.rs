#[cfg(feature = "backend-sys")]
use geotrace_sdk::NavFile;
use gt_history::{
    Database, DatabaseRef, DbError, HistoryDatabase, RecordingMeta, StoredRecording,
    StoredSegmentation, TrackRange, extract_meta,
};
#[cfg(feature = "backend-pure")]
use gt_history_types::{
    ATTR_GTD_SIZE_BYTES, ATTR_NAV_POINT_COUNT, ATTR_START_US, TRACK_END_DATASET,
    TRACK_HIDDEN_DATASET, TRACK_START_DATASET, TRACKS_GROUP,
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

/// Like [`make_gtd_bytes`] but sets the SDK metadata root attributes the History
/// listing reads (`meta_title`/`meta_device`/`meta_notes`/`meta_travel_mode`),
/// each only when `Some`, mirroring how `geotrace_sdk` writes them.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
fn make_gtd_bytes_with_meta(
    start_us: i64,
    n: u64,
    title: Option<&str>,
    device: Option<&str>,
    notes: Option<&str>,
    travel_mode: Option<&str>,
) -> Vec<u8> {
    use hdf5_pure::AttrValue;

    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let timestamps: Vec<i64> = (0..n).map(|i| start_us + i as i64).collect();
    let shape = [n];

    let mut fb = hdf5_pure::FileBuilder::new();
    if let Some(title) = title {
        fb.set_attr("meta_title", AttrValue::String(title.to_owned()));
    }
    if let Some(device) = device {
        fb.set_attr("meta_device", AttrValue::String(device.to_owned()));
    }
    if let Some(notes) = notes {
        fb.set_attr("meta_notes", AttrValue::String(notes.to_owned()));
    }
    if let Some(travel_mode) = travel_mode {
        fb.set_attr(
            "meta_travel_mode",
            AttrValue::String(travel_mode.to_owned()),
        );
    }
    let mut nav_gb = fb.create_group("nav_points");
    let ds = nav_gb.create_dataset("time");
    ds.with_shape(&shape);
    ds.with_i64_data(&timestamps);
    fb.add_group(nav_gb.finish());
    fb.write(tmp.path()).expect("write temp gtd");

    std::fs::read(tmp.path()).expect("read temp gtd")
}

/// Like [`make_gtd_bytes`] but adds the two shapes of ad-hoc sensor channel the
/// SDK writes, under `channels/{name}/{time,value}`: a scalar `temperature` and
/// a three-component vector `accel`.
///
/// These sit two levels below the recording group once stored, which is deeper
/// than the rest of the GTD tree - so this fixture is also what proves the
/// backends preserve nested groups rather than flattening them away.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
fn make_gtd_bytes_with_channels(start_us: i64, n: u64, samples: u64) -> Vec<u8> {
    use hdf5_pure::AttrValue;

    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let timestamps: Vec<i64> = (0..n).map(|i| start_us + i as i64).collect();
    let sample_times: Vec<i64> = (0..samples).map(|i| start_us + i as i64).collect();

    let mut fb = hdf5_pure::FileBuilder::new();
    let mut nav_gb = fb.create_group("nav_points");
    let nav_ds = nav_gb.create_dataset("time");
    nav_ds.with_shape(&[n]);
    nav_ds.with_i64_data(&timestamps);
    fb.add_group(nav_gb.finish());

    let mut channels_gb = fb.create_group("channels");

    let mut accel_gb = channels_gb.create_group("accel");
    accel_gb.set_attr("unit", AttrValue::String("g".to_owned()));
    accel_gb.set_attr("description", AttrValue::String("Frame IMU".to_owned()));
    accel_gb.set_attr(
        "components",
        AttrValue::StringArray(vec!["x".to_owned(), "y".to_owned(), "z".to_owned()]),
    );
    let accel_time = accel_gb.create_dataset("time");
    accel_time.with_shape(&[samples]);
    accel_time.with_i64_data(&sample_times);
    let accel_values: Vec<f64> = (0..samples * 3).map(|i| i as f64 * 0.5).collect();
    let accel_value = accel_gb.create_dataset("value");
    accel_value.with_shape(&[samples, 3]);
    accel_value.with_f64_data(&accel_values);
    channels_gb.add_group(accel_gb.finish());

    let mut temp_gb = channels_gb.create_group("temperature");
    temp_gb.set_attr("unit", AttrValue::String("degC".to_owned()));
    let temp_time = temp_gb.create_dataset("time");
    temp_time.with_shape(&[samples]);
    temp_time.with_i64_data(&sample_times);
    let temp_values: Vec<f64> = (0..samples).map(|i| 20.0 + i as f64).collect();
    let temp_value = temp_gb.create_dataset("value");
    temp_value.with_shape(&[samples]);
    temp_value.with_f64_data(&temp_values);
    channels_gb.add_group(temp_gb.finish());

    fb.add_group(channels_gb.finish());
    fb.write(tmp.path()).expect("write temp gtd");

    std::fs::read(tmp.path()).expect("read temp gtd")
}

/// Like [`make_gtd_bytes`] but stores **chunked, deflate-compressed** datasets
/// (time + lat + lon), matching how real recordings are encoded - so the
/// free-space tests exercise reclaiming chunked/filtered storage, not just
/// contiguous data.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on I/O failure is the right behaviour"
)]
fn make_chunked_gtd_bytes(start_us: i64, n: u64) -> Vec<u8> {
    let timestamps: Vec<i64> = (0..n as i64).map(|i| start_us + i).collect();
    let lat: Vec<f64> = (0..n).map(|i| 55.0 + i as f64 * 1e-5).collect();
    let lon: Vec<f64> = (0..n).map(|i| 12.0 + i as f64 * 1e-5).collect();
    let shape = [n];
    let chunk = [n.clamp(1, 64)];

    let mut fb = hdf5_pure::FileBuilder::new();
    let mut nav = fb.create_group("nav_points");
    {
        let ds = nav.create_dataset("time");
        ds.with_shape(&shape);
        ds.with_i64_data(&timestamps);
        ds.with_chunks(&chunk);
        ds.with_deflate(6);
    }
    {
        let ds = nav.create_dataset("lat");
        ds.with_shape(&shape);
        ds.with_f64_data(&lat);
        ds.with_chunks(&chunk);
        ds.with_deflate(6);
    }
    {
        let ds = nav.create_dataset("lon");
        ds.with_shape(&shape);
        ds.with_f64_data(&lon);
        ds.with_chunks(&chunk);
        ds.with_deflate(6);
    }
    fb.add_group(nav.finish());
    fb.finish().expect("serialize chunked gtd")
}

/// The active backend, used to suffix free-space snapshots: the two backends
/// encode differently and so produce different file sizes.
fn backend_name() -> &'static str {
    if cfg!(feature = "backend-sys") {
        "sys"
    } else {
        "pure"
    }
}

/// Snapshot the exact baseline/after database file sizes (and their delta) with
/// `insta`, so any change in the space behaviour is caught precisely and shown for
/// review. The size is deterministic for a given backend toolchain (static
/// libhdf5 / pure Rust, fixed-length group names), so the snapshot is stable.
fn assert_size_snapshot(name: &str, baseline: u64, after: u64) {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_suffix(backend_name());
    settings.bind(|| {
        insta::assert_snapshot!(
            name,
            format!(
                "baseline: {baseline}\nafter:    {after}\ndelta:    {}",
                after as i64 - baseline as i64
            )
        );
    });
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

    db.insert_simple("test_device", &meta, &bytes)
        .expect("insert 1");

    // Insert again, should be deduplicated.
    db.insert_simple("test_device", &meta, &bytes)
        .expect("insert 2");

    let recordings = db.list_recordings().expect("list");

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
    let id_grp = by_id
        .group(&gt_history::identity_group_name("test_device"))
        .expect("identity group");
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
        .and_then(|b| b.group(&gt_history::identity_group_name("device_a")))
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

    assert!(identity_names.contains(&gt_history::identity_group_name("alpha")));
    assert!(identity_names.contains(&gt_history::identity_group_name("beta")));
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
        .and_then(|b| b.group(&gt_history::identity_group_name(&db_ref.identity)))
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
fn list_recordings_surfaces_sdk_metadata() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let with_meta = make_gtd_bytes_with_meta(
        1_000,
        3,
        Some("Morning ride"),
        Some("uBlox F9P"),
        Some("cross-town commute"),
        Some("bicycle"),
    );
    let meta = extract_meta(&with_meta).expect("meta");
    db.insert_simple("auto:ride", &meta, &with_meta)
        .expect("insert with metadata");

    // A recording with no SDK metadata attributes.
    let plain = make_gtd_bytes(2_000, 3);
    let plain_meta = extract_meta(&plain).expect("plain meta");
    db.insert_simple("auto:plain", &plain_meta, &plain)
        .expect("insert plain");

    let entries = db.list_recordings().expect("list");
    let labelled = entries
        .iter()
        .find(|e| e.db_ref.identity == "auto:ride")
        .expect("labelled entry present");
    assert_eq!(labelled.title.as_deref(), Some("Morning ride"));
    assert_eq!(labelled.device.as_deref(), Some("uBlox F9P"));
    assert_eq!(labelled.notes.as_deref(), Some("cross-town commute"));
    assert_eq!(labelled.travel_mode.as_deref(), Some("bicycle"));

    let plain_entry = entries
        .iter()
        .find(|e| e.db_ref.identity == "auto:plain")
        .expect("plain entry present");
    assert_eq!(plain_entry.title, None);
    assert_eq!(plain_entry.device, None);
    assert_eq!(plain_entry.notes, None);
    assert_eq!(plain_entry.travel_mode, None);
}

/// The listing describes each recording's ad-hoc sensor channels without
/// loading their samples, so the History window can show what custom data a
/// recording carries. A recording with no channels summarizes to none.
#[test]
fn list_recordings_summarizes_custom_channels() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let with_channels = make_gtd_bytes_with_channels(1_000, 3, 16);
    let meta = extract_meta(&with_channels).expect("meta");
    db.insert_simple("auto:sensors", &meta, &with_channels)
        .expect("insert with channels");

    let plain = make_gtd_bytes(2_000, 3);
    let plain_meta = extract_meta(&plain).expect("plain meta");
    db.insert_simple("auto:plain", &plain_meta, &plain)
        .expect("insert plain");

    let entries = db.list_recordings().expect("list");
    let sensors = entries
        .iter()
        .find(|e| e.db_ref.identity == "auto:sensors")
        .expect("channel-carrying entry present");

    // Sorted by name, so `accel` precedes `temperature`.
    let summarized: Vec<(&str, Option<&str>, Vec<&str>, u64)> = sensors
        .channels
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.unit.as_deref(),
                c.components.iter().map(String::as_str).collect(),
                c.sample_count,
            )
        })
        .collect();
    assert_eq!(
        summarized,
        vec![
            ("accel", Some("g"), vec!["x", "y", "z"], 16),
            ("temperature", Some("degC"), vec![], 16),
        ],
    );
    assert_eq!(
        sensors
            .channels
            .first()
            .and_then(|c| c.description.as_deref()),
        Some("Frame IMU"),
    );

    let plain_entry = entries
        .iter()
        .find(|e| e.db_ref.identity == "auto:plain")
        .expect("plain entry present");
    assert!(
        plain_entry.channels.is_empty(),
        "a recording without channels must summarize to none",
    );
}

/// Channel groups sit deeper in the GTD tree than anything else, and a backend
/// that rewrites the database (the pure one rewrites it on every mutation) must
/// carry that depth across. Delete a sibling recording to force the rewrite,
/// then check the survivor still lists and loads its channels intact.
#[test]
fn channels_survive_a_database_rewrite() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let keeper_bytes = make_gtd_bytes_with_channels(1_000, 3, 16);
    let keeper_meta = extract_meta(&keeper_bytes).expect("keeper meta");
    let keeper = db
        .insert_simple("auto:keeper", &keeper_meta, &keeper_bytes)
        .expect("insert keeper");

    let doomed_bytes = make_gtd_bytes_with_channels(50_000, 3, 4);
    let doomed_meta = extract_meta(&doomed_bytes).expect("doomed meta");
    let doomed = db
        .insert_simple("auto:doomed", &doomed_meta, &doomed_bytes)
        .expect("insert doomed");

    db.delete_batch(std::slice::from_ref(&doomed))
        .expect("delete sibling");

    let entries = db.list_recordings().expect("list after delete");
    let keeper_entry = entries
        .iter()
        .find(|e| e.db_ref == keeper)
        .expect("keeper still listed");
    let names: Vec<&str> = keeper_entry
        .channels
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["accel", "temperature"],
        "the rewrite dropped the recording's channels",
    );

    // The samples themselves must come back too, not just the group shells.
    let loaded = db.load_bytes(&keeper).expect("load keeper");
    let file = hdf5_pure::File::from_bytes(loaded).expect("parse loaded bytes");
    let values = file
        .group("channels")
        .and_then(|g| g.group("accel"))
        .and_then(|g| g.dataset("value"))
        .and_then(|ds| ds.read_f64())
        .expect("read accel values");
    let expected: Vec<f64> = (0..16 * 3).map(|i| f64::from(i) * 0.5).collect();
    assert_eq!(values, expected, "channel samples did not round-trip");
}

#[test]
fn rename_identity_moves_recordings_to_a_fresh_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes_a = make_gtd_bytes(1_000, 3);
    let bytes_b = make_gtd_bytes(2_000, 5);
    db.insert_simple(
        "auto:old",
        &extract_meta(&bytes_a).expect("meta a"),
        &bytes_a,
    )
    .expect("insert a");
    db.insert_simple(
        "auto:old",
        &extract_meta(&bytes_b).expect("meta b"),
        &bytes_b,
    )
    .expect("insert b");

    db.rename_identity("auto:old", "Trip 2025").expect("rename");

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), 2, "both recordings survive the rename");
    assert!(
        entries.iter().all(|e| e.db_ref.identity == "Trip 2025"),
        "every recording now reports the new identity"
    );
    // The renamed recordings still load (their group data moved intact).
    for entry in &entries {
        db.load_bytes(&entry.db_ref)
            .expect("load renamed recording");
    }
}

#[test]
fn rename_identity_merges_into_an_existing_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes_a = make_gtd_bytes(1_000, 3);
    let bytes_b = make_gtd_bytes(2_000, 5);
    db.insert_simple("auto:a", &extract_meta(&bytes_a).expect("meta a"), &bytes_a)
        .expect("insert a");
    db.insert_simple("keep", &extract_meta(&bytes_b).expect("meta b"), &bytes_b)
        .expect("insert b");

    db.rename_identity("auto:a", "keep").expect("rename-merge");

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), 2);
    assert!(
        entries.iter().all(|e| e.db_ref.identity == "keep"),
        "both recordings share the merged identity"
    );
}

#[test]
fn rename_identity_no_op_when_absent_or_unchanged() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000, 3);
    db.insert_simple("auto:only", &extract_meta(&bytes).expect("meta"), &bytes)
        .expect("insert");

    db.rename_identity("does-not-exist", "whatever")
        .expect("absent old is a no-op");
    db.rename_identity("auto:only", "auto:only")
        .expect("same name is a no-op");
    db.rename_identity("auto:only", "   ")
        .expect("whitespace-only new is a no-op");

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].db_ref.identity, "auto:only");
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

    // Hide the first track, the recording stays, with one hidden track.
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

#[test_log::test]
fn path_like_identity_is_stored_as_one_listed_identity() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("history.h5");
    let mut db = Database::open_or_create(&db_path).expect("open");
    let bytes = make_gtd_bytes(1_000, 5);
    let meta = extract_meta(&bytes).expect("meta");
    let identity = "/example.invalid/history/identity/with/slashes/";

    let db_ref = db
        .insert_simple(identity, &meta, &bytes)
        .expect("insert path-like identity");
    assert_eq!(db_ref.identity, identity);

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].db_ref.identity, identity);
    assert_eq!(entries[0].db_ref.group_name, db_ref.group_name);
    let loaded_bytes = db.load_bytes(&db_ref).expect("load");
    let loaded_meta = extract_meta(&loaded_bytes).expect("loaded meta");
    assert!(meta.same_recording(&loaded_meta));

    db.insert_simple(identity, &meta, &bytes)
        .expect("deduplicated reinsert");
    assert_eq!(db.list_recordings().expect("list after reinsert").len(), 1);

    #[cfg(feature = "backend-sys")]
    {
        let file = hdf5::File::open(&db_path).expect("open hdf5");
        let by_id = file.group("by_identity").expect("by_identity");
        by_id
            .group(&gt_history::identity_group_name(identity))
            .expect("encoded identity group");
        assert!(
            file.group("/home").is_err(),
            "path-like identities must not create root-level groups"
        );
    }
}

#[cfg(feature = "backend-sys")]
#[test_log::test]
fn open_repairs_absolute_identity_recording_stored_outside_by_identity() {
    use hdf5::types::VarLenUnicode;
    use std::str::FromStr as _;

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("history.h5");
    let identity = "/example.invalid/history/identity/with/slashes/";
    let group_name = "2026-01-02T03:04:05Z_00000000-1111-2222-3333-444444444444";
    let bytes = make_gtd_bytes(1_000, 5);
    let meta = extract_meta(&bytes).expect("meta");

    Database::open_or_create(&db_path).expect("create");
    {
        let file = hdf5::File::open_rw(&db_path).expect("open raw");
        let by_id = file.group("by_identity").expect("by_identity");
        let id_grp = by_id.create_group(identity).expect("buggy identity group");
        let rec_grp = id_grp.create_group(group_name).expect("recording group");
        let identity_attr = VarLenUnicode::from_str(identity).expect("identity string");
        rec_grp
            .new_attr::<VarLenUnicode>()
            .create("identity")
            .expect("identity attr")
            .write_scalar(&identity_attr)
            .expect("write identity");
        rec_grp
            .new_attr::<i64>()
            .create("start_us")
            .expect("start_us attr")
            .write_scalar(&meta.start_us)
            .expect("write start_us");
        rec_grp
            .new_attr::<i64>()
            .create("end_us")
            .expect("end_us attr")
            .write_scalar(&meta.end_us)
            .expect("write end_us");
        for (name, value) in [
            ("nav_point_count", meta.nav_point_count),
            ("sat_report_count", meta.sat_report_count),
            ("marker_count", meta.marker_count),
            ("event_marker_count", meta.event_marker_count),
            ("gtd_size_bytes", meta.gtd_size_bytes),
        ] {
            rec_grp
                .new_attr::<u64>()
                .create(name)
                .expect("count attr")
                .write_scalar(&value)
                .expect("write count");
        }
    }

    let db = Database::open_or_create(&db_path).expect("repair on open");
    let entries = db.list_recordings().expect("list repaired");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].db_ref.identity, identity);
    assert_eq!(entries[0].db_ref.group_name, group_name);

    let file = hdf5::File::open(&db_path).expect("open repaired hdf5");
    let by_id = file.group("by_identity").expect("by_identity");
    by_id
        .group(&gt_history::identity_group_name(identity))
        .and_then(|id| id.group(group_name))
        .expect("moved recording");
}

/// Renaming onto an identity that exists only as a legacy raw-named group must
/// merge into it, not spawn a second (encoded-name) group for the same identity.
#[cfg(feature = "backend-sys")]
#[test_log::test]
fn rename_identity_merges_into_a_legacy_raw_named_target() {
    use hdf5::types::VarLenUnicode;
    use std::str::FromStr as _;

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("history.h5");
    Database::open_or_create(&db_path).expect("create");

    // Hand-build a legacy raw-named (pre-encoding) identity group "keep".
    {
        let file = hdf5::File::open_rw(&db_path).expect("open raw");
        let by_id = file.group("by_identity").expect("by_identity");
        let id_grp = by_id.create_group("keep").expect("legacy identity group");
        let attr = VarLenUnicode::from_str("keep").expect("identity string");
        id_grp
            .new_attr::<VarLenUnicode>()
            .create("identity")
            .expect("identity attr")
            .write_scalar(&attr)
            .expect("write identity");
    }

    // A normally-indexed recording under a different identity, then rename it
    // onto the legacy "keep".
    let mut db = Database::open_or_create(&db_path).expect("reopen");
    let bytes = make_gtd_bytes(1_000, 3);
    db.insert_simple("auto:a", &extract_meta(&bytes).expect("meta"), &bytes)
        .expect("insert");
    db.rename_identity("auto:a", "keep")
        .expect("rename onto legacy");

    let entries = db.list_recordings().expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].db_ref.identity, "keep");

    // The merge must reuse the legacy group, not create an encoded twin.
    let file = hdf5::File::open(&db_path).expect("open hdf5");
    let by_id = file.group("by_identity").expect("by_identity");
    assert!(by_id.group("keep").is_ok(), "legacy target group kept");
    assert!(
        by_id
            .group(&gt_history::identity_group_name("keep"))
            .is_err(),
        "no duplicate encoded group for the same identity"
    );
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
        let id_grp = by_id
            .group(&gt_history::identity_group_name("dev"))
            .expect("id_grp");
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

    let identity = "auto:snapshot.gtd";
    db.insert_simple(identity, &meta, &bytes)
        .expect("first insert");

    // Re-inserting the same identity must be detected as a duplicate.
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

    let _db_ref = db.insert_simple("device", &meta, &bytes).expect("insert");

    // Verify the sys-backend structure with the hdf5-pure reader (as the pure
    // backend does): if the sys backend is parity-compatible, this works.
    let file = hdf5_pure::File::open(&db_path).expect("open");
    let root = file.root();

    let by_id = root.group("by_identity").expect("by_identity missing");
    let id_grp = by_id
        .group(&gt_history::identity_group_name("device"))
        .expect("identity group missing");

    let rec_names = id_grp.groups().expect("recording names missing");
    assert!(!rec_names.is_empty(), "No recordings found under 'device'");

    let rec_name = rec_names.first().expect("recording name");
    let rec_grp = id_grp.group(rec_name).expect("recording group missing");
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

/// The mirror of [`sys_backend_structural_parity_repro`]: a database written by
/// the pure backend must be readable by the reference C library, since the sys
/// backend (the default) opens the very same file. Reads the whole shape a
/// recording occupies - the identity tree, the recording attributes, and the
/// track table datasets - so an `hdf5-pure` upgrade that changes the emitted
/// bytes is caught here rather than by a user whose history file stops opening.
#[test_log::test]
#[cfg(feature = "backend-pure")]
fn pure_backend_database_is_readable_by_metno() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace_pure.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    let bytes = make_gtd_bytes(1_000_000, 9);
    let meta = extract_meta(&bytes).expect("parse meta");
    let tracks = [
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
    db.insert("device", &meta, &tracks, test_settings(), &bytes)
        .expect("insert");

    let file = hdf5::File::open(&db_path).expect("open with the reference C library");
    let by_id = file.group("by_identity").expect("by_identity missing");
    let id_grp = by_id
        .group(&gt_history::identity_group_name("device"))
        .expect("identity group missing");

    let rec_name = id_grp
        .member_names()
        .expect("member names")
        .into_iter()
        .next()
        .expect("a recording group");
    let rec_grp = id_grp.group(&rec_name).expect("recording group missing");

    let read_u64 = |name: &str| {
        rec_grp
            .attr(name)
            .and_then(|a| a.read_scalar::<u64>())
            .unwrap_or_else(|e| panic!("attribute {name}: {e}"))
    };
    let read_i64 = |name: &str| {
        rec_grp
            .attr(name)
            .and_then(|a| a.read_scalar::<i64>())
            .unwrap_or_else(|e| panic!("attribute {name}: {e}"))
    };
    assert_eq!(read_u64(ATTR_NAV_POINT_COUNT), meta.nav_point_count);
    assert_eq!(read_i64(ATTR_START_US), meta.start_us);
    assert_eq!(read_u64(ATTR_GTD_SIZE_BYTES), bytes.len() as u64);

    let tracks_grp = rec_grp.group(TRACKS_GROUP).expect("track table missing");
    let read_column = |name: &str| {
        tracks_grp
            .dataset(name)
            .and_then(|d| d.read_1d::<u64>())
            .unwrap_or_else(|e| panic!("track column {name}: {e}"))
            .to_vec()
    };
    assert_eq!(read_column(TRACK_START_DATASET), vec![0, 4]);
    assert_eq!(read_column(TRACK_END_DATASET), vec![4, 9]);
    assert_eq!(read_column(TRACK_HIDDEN_DATASET), vec![0, 1]);
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
        // they are not deduplicated).  Group-name uniqueness is handled by the UUID
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
/// sizes are equal. Without any reuse the file grows by one fill per cycle.
#[expect(
    clippy::expect_used,
    reason = "test helper; panicking on failure is the right behaviour"
)]
fn delete_reinsert_size_trajectory(db_path: &std::path::Path) -> (u64, u64) {
    const N: usize = 40;
    let fill = |db: &mut Database| {
        for i in 0..N {
            // Distinct start timestamps give distinct content fingerprints (so the
            // inserts are not deduplicated).  Group-name uniqueness comes from the
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
/// flat. A 2x ceiling leaves generous slack while still catching the unbounded
/// (one-fill-per-cycle) growth that variable-length strings used to cause.
#[test_log::test]
fn file_size_stays_bounded_across_delete_reinsert_cycles() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (baseline, after) = delete_reinsert_size_trajectory(&dir.path().join("geotrace.h5"));
    assert!(baseline > 0, "the filled database has a non-zero size");
    // Reuse keeps `after` at `baseline`. A regression that stops reclaiming space
    // would grow it by ~one fill per cycle, which the snapshot shows exactly.
    assert_size_snapshot("delete_all_refill", baseline, after);
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

    // Delete the older half (indices 0..N/2). The newer half stays physically
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

    // Reuse keeps the file near `full` (the deleted half's space is reused by the
    // new inserts). The snapshot pins the exact sizes so any regression shows up.
    assert_size_snapshot("interior_delete_older_half", full, after);
}

/// Strictly-interior churn: delete a middle range of recordings (recordings
/// remain at *both* ends, so the freed regions are bracketed by live data and
/// cannot be truncated), then insert the same number of fresh recordings. The
/// file must barely change - the inserts reuse the interior holes rather than
/// growing it linearly.
#[test_log::test]
fn interior_range_delete_keeps_file_bounded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    const N: usize = 40;
    let file_size = |p: &std::path::Path| std::fs::metadata(p).expect("metadata").len();
    let start_for = |i: usize| 1_000_000_000_i64 + i as i64 * 1_000_000;

    let refs: Vec<DatabaseRef> = (0..N)
        .map(|i| insert_two_track(&mut db, "dev", start_for(i), 50))
        .collect();
    let full = file_size(&db_path);

    // Delete the strictly interior range [9, 29): 0..9 and 29..40 stay live.
    db.delete_batch(&refs[9..29])
        .expect("delete interior range");
    assert_eq!(db.list_recordings().expect("list").len(), N - 20);

    // Insert 20 fresh recordings. Reuse returns the count to 40 with little growth.
    for i in N..(N + 20) {
        insert_two_track(&mut db, "dev", start_for(i), 50);
    }
    assert_eq!(db.list_recordings().expect("list").len(), N);
    let after = file_size(&db_path);

    // The 20 inserts reuse the 20 interior holes, so the file barely changes;
    // linear growth would push it to ~1.5x. The snapshot pins the exact sizes.
    assert_size_snapshot("interior_range_delete", full, after);
}

/// Chunked/filtered storage variant: recordings are stored chunked +
/// deflate-compressed (as real recordings are). After a strictly-interior delete
/// and reinsert, the freed chunk blocks must be reused so the file does not grow
/// linearly with the inserts.
#[test_log::test]
fn chunked_recordings_reuse_freed_space_on_interior_delete() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open_or_create");

    const N: usize = 40;
    let file_size = |p: &std::path::Path| std::fs::metadata(p).expect("metadata").len();
    let start_for = |i: usize| 1_000_000_000_i64 + i as i64 * 1_000_000;
    let insert = |db: &mut Database, i: usize| -> DatabaseRef {
        let bytes = make_chunked_gtd_bytes(start_for(i), 500);
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
        db.insert("dev", &meta, &tracks, test_settings(), &bytes)
            .expect("insert chunked")
    };

    let refs: Vec<DatabaseRef> = (0..N).map(|i| insert(&mut db, i)).collect();
    let full = file_size(&db_path);

    db.delete_batch(&refs[9..29])
        .expect("delete interior range");
    assert_eq!(db.list_recordings().expect("list").len(), N - 20);

    for i in N..(N + 20) {
        insert(&mut db, i);
    }
    assert_eq!(db.list_recordings().expect("list").len(), N);
    let after = file_size(&db_path);

    // libhdf5's free-space manager reclaims chunked block storage too, so the
    // file barely changes (vs ~1.5x with no reuse). The snapshot pins the sizes.
    assert_size_snapshot("chunked_interior_delete", full, after);
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

/// The snap blob round trip: absent until stored (older recordings simply
/// have none), replaced wholesale on rewrite - shrinking included - and an
/// empty blob is a value, not absence.
#[test_log::test]
fn snap_blob_roundtrips_and_overwrites() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut db = Database::open_or_create(&dir.path().join("geotrace.h5")).expect("open");
    let db_ref = insert_two_track(&mut db, "dev", 1_000_000_000, 50);

    assert_eq!(
        db.snap_blob(&db_ref).expect("read"),
        None,
        "a recording without a stored run carries no blob"
    );

    let first: Vec<u8> = (0..200u16).map(|i| (i % 251) as u8).collect();
    db.set_snap_blob(&db_ref, &first).expect("store");
    assert_eq!(db.snap_blob(&db_ref).expect("read"), Some(first));

    let shorter = vec![7u8; 32];
    db.set_snap_blob(&db_ref, &shorter).expect("overwrite");
    assert_eq!(
        db.snap_blob(&db_ref).expect("read"),
        Some(shorter),
        "a rewrite replaces the blob wholesale"
    );

    db.set_snap_blob(&db_ref, &[]).expect("store empty");
    assert_eq!(
        db.snap_blob(&db_ref).expect("read"),
        Some(Vec::new()),
        "an empty blob is stored, not treated as absence"
    );
}

/// The snap subgroup is DB bookkeeping: storing a blob must not change what
/// the reconstructed GTD contains, and each recording keeps its own blob.
///
/// Compared by content rather than byte-for-byte: HDF5 stamps every object
/// header with access/modification/change times, so two reconstructions taken
/// either side of a second boundary differ in those bytes (and the header
/// checksums over them) no matter what the blob did. A byte comparison here
/// failed on slower CI runners for exactly that reason.
#[test_log::test]
fn snap_blob_stays_out_of_the_reconstructed_gtd() {
    const BLOB: &[u8] = b"snap run bytes";

    let dir = tempfile::tempdir().expect("temp dir");
    let mut db = Database::open_or_create(&dir.path().join("geotrace.h5")).expect("open");
    let a = insert_two_track(&mut db, "dev", 1_000_000_000, 50);
    let b = insert_two_track(&mut db, "dev", 2_000_000_000, 50);

    let baseline = db.load_bytes(&a).expect("load");
    db.set_snap_blob(&a, BLOB).expect("store");
    let after = db.load_bytes(&a).expect("load");

    assert!(
        !after.windows(BLOB.len()).any(|window| window == BLOB),
        "the blob's bytes must never appear in the reconstructed GTD file"
    );
    assert_eq!(
        after.len(),
        baseline.len(),
        "storing a blob must not add anything to the reconstructed GTD file"
    );
    assert_eq!(
        extract_meta(&after).expect("meta"),
        extract_meta(&baseline).expect("meta"),
        "the reconstructed recording must be unchanged"
    );
    assert_eq!(
        db.snap_blob(&b).expect("read"),
        None,
        "blobs are per recording"
    );
}

/// Deleting a recording drops its blob with the group; a same-content
/// reinsert starts blank.
#[test_log::test]
fn snap_blob_is_dropped_with_its_recording() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut db = Database::open_or_create(&dir.path().join("geotrace.h5")).expect("open");
    let db_ref = insert_two_track(&mut db, "dev", 1_000_000_000, 50);
    db.set_snap_blob(&db_ref, &[1, 2, 3]).expect("store");

    db.delete_batch(std::slice::from_ref(&db_ref))
        .expect("delete");
    let db_ref = insert_two_track(&mut db, "dev", 1_000_000_000, 50);

    assert_eq!(
        db.snap_blob(&db_ref).expect("read"),
        None,
        "a reinserted recording starts without a blob"
    );
}

/// Rewriting a recording's snap blob many times must not grow the database
/// file without bound - the fixed-shape byte dataset is reclaimed by the
/// free-space manager on every unlink-and-recreate (unlike variable-length
/// values, which leak in libhdf5's global heap; see
/// `file_size_stays_bounded_across_delete_reinsert_cycles`).
#[test_log::test]
fn file_size_stays_bounded_across_snap_blob_rewrites() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Database::open_or_create(&db_path).expect("open");
    let db_ref = insert_two_track(&mut db, "dev", 1_000_000_000, 50);
    let file_size = |path: &std::path::Path| std::fs::metadata(path).expect("metadata").len();

    let blob_of = |fill: u8| vec![fill; 16 * 1024];
    db.set_snap_blob(&db_ref, &blob_of(0)).expect("store");
    let baseline = file_size(&db_path);

    for cycle in 0..40u8 {
        db.set_snap_blob(&db_ref, &blob_of(cycle)).expect("rewrite");
    }
    assert_size_snapshot("snap_blob_rewrites", baseline, file_size(&db_path));
}
