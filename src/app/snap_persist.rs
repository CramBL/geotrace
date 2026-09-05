//! The stored form of a recording's snap runs.
//!
//! The recording history database keeps one opaque blob per recording (see
//! `gt_history_types::SNAP_GROUP`). This module owns what is inside it: a
//! versioned serde-JSON envelope holding the latest run of every track that
//! has one, each keyed by the track's content fingerprint. A content key
//! means re-segmentation or index shifts can never attach a run to the wrong
//! track: a non-matching entry is never restored.
//!
//! Decoding is tolerant: an unreadable or newer-versioned blob is treated
//! as absent (with a warning) and the recording still opens. The inner
//! types' schema is pinned by `gt-snap`'s `stored_result_schema` snapshot.
//! Fields added later decode absent from older blobs via serde defaults.

use serde::{Deserialize, Serialize};

use gt_snap::merge::{SnapResult, SnapWarning};
use gt_types::LoadedTrack;

use super::snap::{SnapRun, TrackContentKey};

/// Version of the envelope layout itself (not the inner result schema).
/// Bumped only on incompatible layout changes. A blob written by a newer
/// version is treated as absent.
const STORED_SNAP_FORMAT_VERSION: u32 = 1;

/// The envelope stored as the recording's snap blob.
#[derive(Debug, Serialize, Deserialize)]
struct StoredSnapRuns {
    format_version: u32,
    /// One entry per track with a completed run.
    #[serde(default)]
    runs: Vec<StoredTrackRun>,
}

/// One track's stored run, keyed by the track's content fingerprint
/// (mirroring [`TrackContentKey`], spelled out so the storage schema stays
/// independent of the in-memory key type).
#[derive(Debug, Serialize, Deserialize)]
pub struct StoredTrackRun {
    start_us: i64,
    end_us: i64,
    tpv_count: usize,
    #[serde(default)]
    server_host: Option<String>,
    #[serde(default)]
    warnings: Vec<SnapWarning>,
    result: SnapResult,
}

impl StoredTrackRun {
    /// Whether this stored run belongs to `track`.
    pub fn matches(&self, track: &LoadedTrack) -> bool {
        let key = TrackContentKey::new(track);
        key.start_us == self.start_us
            && key.end_us == self.end_us
            && key.tpv_count == self.tpv_count
    }

    /// Rebuild the session-store run (re-projecting the map geometry).
    pub fn into_run(self) -> SnapRun {
        SnapRun::new(self.result, self.warnings, self.server_host)
    }
}

/// Serialize `runs` into the blob stored with a recording.
pub fn encode<'a>(runs: impl IntoIterator<Item = (&'a LoadedTrack, &'a SnapRun)>) -> Vec<u8> {
    let envelope = StoredSnapRuns {
        format_version: STORED_SNAP_FORMAT_VERSION,
        runs: runs
            .into_iter()
            .map(|(track, run)| {
                let key = TrackContentKey::new(track);
                StoredTrackRun {
                    start_us: key.start_us,
                    end_us: key.end_us,
                    tpv_count: key.tpv_count,
                    server_host: run.server_host.clone(),
                    warnings: run.warnings.clone(),
                    result: run.result.clone(),
                }
            })
            .collect(),
    };
    // Serializing owned serde types cannot fail. An empty blob decodes as
    // absent, so even a hypothetical failure only costs the cache entry.
    serde_json::to_vec(&envelope).unwrap_or_default()
}

/// Decode a recording's snap blob. `None` (with a warning) for undecodable
/// or newer-versioned blobs - a broken cache entry must never fail the
/// recording it belongs to.
pub fn decode(blob: &[u8]) -> Option<Vec<StoredTrackRun>> {
    let envelope: StoredSnapRuns = match serde_json::from_slice(blob) {
        Ok(envelope) => envelope,
        Err(err) => {
            log::warn!("Ignoring undecodable stored snap runs: {err}");
            return None;
        }
    };
    if envelope.format_version > STORED_SNAP_FORMAT_VERSION {
        log::warn!(
            "Ignoring stored snap runs with format version {} (this build reads up to {})",
            envelope.format_version,
            STORED_SNAP_FORMAT_VERSION
        );
        return None;
    }
    Some(envelope.runs)
}

#[cfg(test)]
mod tests {
    use gt_snap::merge::{self, SnapWarningReporter};
    use gt_snap::request_plan::SnapParams;
    use gt_snap::wire::Costing;
    use gt_snap::{DEFAULT_SERVER_URL, request_plan, server_host};
    use gt_test_utils::fixtures;

    use super::*;

    fn track(points: usize, start_secs: i64) -> LoadedTrack {
        fixtures::loaded_track_from(
            chrono::DateTime::from_timestamp(start_secs, 0).unwrap_or_default(),
            points,
            1,
        )
    }

    fn run(costing: Costing) -> SnapRun {
        SnapRun::new(
            merge::merge(
                &request_plan::plan(gt_types::PlacedPoints::default()),
                SnapParams::new(costing),
                &[],
                &SnapWarningReporter::default(),
            ),
            Vec::new(),
            server_host(DEFAULT_SERVER_URL),
        )
    }

    /// Encode-decode round trip: each stored run matches exactly its own
    /// track and restores with its parameters and host intact.
    #[test]
    fn roundtrip_matches_runs_to_their_tracks() {
        let (a, b) = (track(10, 1_000), track(20, 2_000));
        let (run_a, run_b) = (run(Costing::Auto), run(Costing::Bicycle));
        let blob = encode([(&a, &run_a), (&b, &run_b)]);

        let stored = decode(&blob).expect("blob decodes");
        assert_eq!(stored.len(), 2);
        let for_track = |track: &LoadedTrack| -> Vec<&StoredTrackRun> {
            stored.iter().filter(|r| r.matches(track)).collect()
        };
        assert_eq!(for_track(&a).len(), 1);
        assert_eq!(for_track(&b).len(), 1);

        let restored = stored
            .into_iter()
            .find(|r| r.matches(&b))
            .expect("b's run")
            .into_run();
        assert_eq!(restored.result.params.costing, Costing::Bicycle);
        assert_eq!(restored.server_host, server_host(DEFAULT_SERVER_URL));
    }

    /// Garbage and newer-versioned blobs decode as absent, never as an
    /// error that could fail the recording.
    #[test]
    fn undecodable_and_newer_blobs_are_absent() {
        assert!(decode(b"not json").is_none());
        assert!(decode(b"").is_none());
        let newer = format!(
            "{{\"format_version\": {}, \"runs\": []}}",
            STORED_SNAP_FORMAT_VERSION + 1
        );
        assert!(decode(newer.as_bytes()).is_none());
    }

    /// The full persistence round trip against a real temporary database:
    /// encode, store through the history API, fetch, decode, restore.
    #[test]
    fn roundtrips_through_a_history_database() {
        use geotrace_sdk::{
            Angle, DateTime, Duration as SdkDuration, NavFileBuilder, NavFix, NavFixTime,
        };
        use gt_store::{
            HistoryDatabase, ReadOnlyHistoryDatabase, Recordings, StoredSegmentation, TrackRange,
            TrackState,
        };

        let t0 = DateTime::from_timestamp(1_000, 0).expect("valid timestamp");
        let mut recorder = NavFileBuilder::new().open();
        for i in 0..10i64 {
            recorder.add_nav_fix(
                NavFix::builder()
                    .time(NavFixTime::Receiver(t0 + SdkDuration::seconds(i)))
                    .lat(Angle::degrees(55.68))
                    .lon(Angle::degrees(12.56))
                    .heading(Angle::degrees(0.0))
                    .build(),
            );
        }
        let nav_file = recorder.finish().expect("valid nav file");
        let mut bytes = Vec::new();
        nav_file.write(&mut bytes).expect("write bytes");

        let dir = tempfile::tempdir().expect("temp dir");
        let mut db = Recordings::open_or_create(&dir.path().join("geotrace.h5")).expect("open");
        let meta = gt_store::extract_meta(&bytes).expect("meta");
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            state: TrackState::Live,
        }];
        let settings = StoredSegmentation {
            track_split_gap_us: 300_000_000,
            track_split_rule: gt_store::StoredTrackSplitRule::StepInEitherDirection,
            fix_placement_rule: gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 5.0,
        };
        let db_ref = db
            .insert("dev", &meta, &tracks, settings, &bytes)
            .expect("insert");

        let loaded = track(10, 1_000);
        db.set_snap_blob(&db_ref, &encode([(&loaded, &run(Costing::Auto))]))
            .expect("store");

        let blob = db.snap_blob(&db_ref).expect("read").expect("blob present");
        let stored = decode(&blob).expect("decodes");
        let restored = stored
            .into_iter()
            .find(|r| r.matches(&loaded))
            .expect("run matches its track")
            .into_run();
        assert_eq!(restored.result.params.costing, Costing::Auto);
    }

    /// A minimal current-version envelope decodes. Unknown extra fields are
    /// tolerated - the forward-compatibility contract for stored blobs.
    #[test]
    fn minimal_and_extended_envelopes_decode() {
        let minimal = format!("{{\"format_version\": {STORED_SNAP_FORMAT_VERSION}}}");
        assert_eq!(decode(minimal.as_bytes()).expect("decodes").len(), 0);
        let extended = format!(
            "{{\"format_version\": {STORED_SNAP_FORMAT_VERSION}, \"runs\": [], \
             \"added_later\": true}}"
        );
        assert!(decode(extended.as_bytes()).is_some());
    }
}
