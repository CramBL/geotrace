use gt_store::{
    DatabaseRef, DbError, HistoryDatabase, PruneMode, ReadOnlyHistoryDatabase, Recordings,
};

pub enum AutoPruneOutcome {
    /// Total stored size is within the limit, nothing to delete.
    NotNeeded,
    /// Pruning was performed, this many recordings were deleted.
    PrunedSilently(usize),
    /// Caller must present the user with a confirmation dialog before deleting.
    NeedsConfirmation(Vec<DatabaseRef>),
}

/// Check whether the database exceeds `max_bytes` and prune the oldest
/// recordings if it does.
///
/// When `confirm` is `true` the database is not touched - candidates are
/// returned so the caller can prompt for confirmation first.
pub fn run(
    db: &mut Recordings,
    max_bytes: u64,
    confirm: bool,
) -> Result<AutoPruneOutcome, DbError> {
    let candidates = db.prune_candidates(&PruneMode::ByTotalSize { max_bytes })?;
    if candidates.is_empty() {
        return Ok(AutoPruneOutcome::NotNeeded);
    }
    if confirm {
        return Ok(AutoPruneOutcome::NeedsConfirmation(candidates));
    }
    let n = candidates.len();
    db.delete_batch(&candidates)?;
    Ok(AutoPruneOutcome::PrunedSilently(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geotrace_sdk::{Angle, DateTime, Duration as SdkDuration, NavFileBuilder, NavFix};

    fn make_gtd(start_secs: i64, n: u32) -> Vec<u8> {
        let t0 = DateTime::from_timestamp(start_secs, 0).expect("valid timestamp");
        let mut recorder = NavFileBuilder::new().open();
        for i in 0..n {
            recorder.add_nav_fix(
                NavFix::builder()
                    .gps_time(t0 + SdkDuration::seconds(i as i64))
                    .lat(Angle::degrees(51.5))
                    .lon(Angle::degrees(-0.1))
                    .heading(Angle::degrees(0.0))
                    .build(),
            );
        }
        let nav_file = recorder.finish().expect("valid nav file");
        let mut bytes = Vec::new();
        nav_file.write(&mut bytes).expect("write bytes");
        bytes
    }

    fn insert(db: &mut Recordings, identity: &str, bytes: &[u8]) {
        use gt_store::{StoredSegmentation, TrackRange};
        let meta = gt_store::extract_meta(bytes).expect("parse meta");
        // One track spanning the whole recording is enough for prune tests.
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            hidden: false,
        }];
        let settings = StoredSegmentation {
            track_split_gap_us: 300_000_000,
            track_split_rule: gt_store::StoredTrackSplitRule::StepInEitherDirection,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 5.0,
        };
        db.insert(identity, &meta, &tracks, settings, bytes)
            .expect("insert");
    }

    #[test]
    fn not_needed_when_under_limit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut db = Recordings::open_or_create(&dir.path().join("h.h5")).expect("db");
        let bytes = make_gtd(1_000_000, 2);
        insert(&mut db, "dev", &bytes);

        let outcome = run(&mut db, bytes.len() as u64 * 10, false).expect("run");
        assert!(matches!(outcome, AutoPruneOutcome::NotNeeded));
        assert_eq!(db.list_recordings().expect("list").len(), 1);
    }

    #[test]
    fn not_needed_on_empty_db() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut db = Recordings::open_or_create(&dir.path().join("h.h5")).expect("db");

        let outcome = run(&mut db, 1, false).expect("run");
        assert!(matches!(outcome, AutoPruneOutcome::NotNeeded));
    }

    #[test]
    fn prunes_silently_when_over_limit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut db = Recordings::open_or_create(&dir.path().join("h.h5")).expect("db");
        let bytes_a = make_gtd(1_000_000, 2);
        let bytes_b = make_gtd(2_000_000, 2);
        insert(&mut db, "dev", &bytes_a);
        insert(&mut db, "dev", &bytes_b);
        let total = bytes_a.len() as u64 + bytes_b.len() as u64;

        // Limit just under total - should remove the oldest recording.
        let outcome = run(&mut db, total - 1, false).expect("run");
        assert!(
            matches!(outcome, AutoPruneOutcome::PrunedSilently(1)),
            "expected PrunedSilently(1)"
        );
        assert_eq!(
            db.list_recordings().expect("list").len(),
            1,
            "one recording should remain"
        );
    }

    #[test]
    fn returns_candidates_when_confirm_true() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut db = Recordings::open_or_create(&dir.path().join("h.h5")).expect("db");
        let bytes_a = make_gtd(1_000_000, 2);
        let bytes_b = make_gtd(2_000_000, 2);
        insert(&mut db, "dev", &bytes_a);
        insert(&mut db, "dev", &bytes_b);
        let total = bytes_a.len() as u64 + bytes_b.len() as u64;

        let outcome = run(&mut db, total - 1, true).expect("run");
        let AutoPruneOutcome::NeedsConfirmation(refs) = outcome else {
            panic!("expected NeedsConfirmation");
        };
        assert_eq!(refs.len(), 1, "one candidate expected");
        // Nothing deleted - caller must confirm.
        assert_eq!(
            db.list_recordings().expect("list").len(),
            2,
            "no recordings should be deleted before confirmation"
        );
    }

    #[test]
    fn prunes_all_when_limit_is_zero() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut db = Recordings::open_or_create(&dir.path().join("h.h5")).expect("db");
        let bytes_a = make_gtd(1_000_000, 2);
        let bytes_b = make_gtd(2_000_000, 2);
        insert(&mut db, "dev", &bytes_a);
        insert(&mut db, "dev", &bytes_b);

        let outcome = run(&mut db, 0, false).expect("run");
        assert!(
            matches!(outcome, AutoPruneOutcome::PrunedSilently(2)),
            "expected PrunedSilently(2)"
        );
        assert_eq!(db.list_recordings().expect("list").len(), 0);
    }
}
