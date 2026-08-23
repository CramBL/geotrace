//! The per-track values the environment schedulers resolve from their day
//! archives.
//!
//! A track's values are rebuilt exactly when the set of archived days it
//! spans changes, and an unchanged archive hands the `Arc` it already
//! resolved back. Downstream caches key on that identity: the plot's mipmap
//! cache, the query run's fingerprint.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::NaiveDate;
use gt_types::TrackRef;

use super::environment_storage::PrunedDays;

struct ResolvedTrackValues<T> {
    archived_days: Vec<NaiveDate>,
    values: Arc<T>,
}

/// One scheduler's per-track values, each keyed by the archived days it was
/// resolved from.
pub struct TrackValuesByArchivedDays<T> {
    by_track: HashMap<TrackRef, ResolvedTrackValues<T>>,
}

impl<T> Default for TrackValuesByArchivedDays<T> {
    fn default() -> Self {
        Self {
            by_track: HashMap::new(),
        }
    }
}

impl<T> TrackValuesByArchivedDays<T> {
    /// The track's values as resolved from `archived_days`, calling `resolve`
    /// only when the cache holds none for that set of days.
    ///
    /// The returned `Arc` keeps its identity for as long as the track's
    /// archived days stay the same.
    pub fn resolve(
        &mut self,
        track: TrackRef,
        archived_days: Vec<NaiveDate>,
        resolve: impl FnOnce() -> T,
    ) -> Arc<T> {
        if let Some(resolved) = self
            .by_track
            .get(&track)
            .filter(|resolved| resolved.archived_days == archived_days)
        {
            return Arc::clone(&resolved.values);
        }
        let values = Arc::new(resolve());
        self.by_track.insert(
            track,
            ResolvedTrackValues {
                archived_days,
                values: Arc::clone(&values),
            },
        );
        values
    }

    pub fn retain_loaded_tracks(&mut self, loaded: &HashSet<TrackRef>) {
        self.by_track.retain(|track, _| loaded.contains(track));
    }

    /// Drop every track that read a day a delete removed from the archive:
    /// the next resolve reads what the archive still holds.
    pub fn forget_pruned_days(&mut self, pruned: PrunedDays) {
        self.by_track
            .retain(|_, resolved| !resolved.archived_days.iter().any(|day| pruned.covers(*day)));
    }

    pub fn iter_unsorted(&self) -> impl Iterator<Item = (TrackRef, &Arc<T>)> {
        self.by_track
            .iter()
            .map(|(&track, resolved)| (track, &resolved.values))
    }
}

#[cfg(test)]
mod tests {
    use gt_types::{FileIdx, TrackIdx};

    use super::*;

    fn day(day_of_month: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, day_of_month).unwrap_or_default()
    }

    fn track(index: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(index))
    }

    /// An unchanged set of archived days hands out the `Arc` already
    /// resolved, and a changed one resolves fresh values under a new
    /// identity.
    #[test]
    fn values_keep_their_identity_until_the_archived_days_change() {
        let mut cache = TrackValuesByArchivedDays::default();

        let first = cache.resolve(track(0), vec![day(20)], || 1);
        let second = cache.resolve(track(0), vec![day(20)], || 2);
        let after_archiving = cache.resolve(track(0), vec![day(20), day(21)], || 3);

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*second, 1);
        assert!(!Arc::ptr_eq(&second, &after_archiving));
        assert_eq!(*after_archiving, 3);
    }

    #[test]
    fn each_track_is_resolved_on_its_own_days() {
        let mut cache = TrackValuesByArchivedDays::default();

        cache.resolve(track(0), vec![day(20)], || 1);
        cache.resolve(track(1), vec![day(21)], || 2);
        let revisited = cache.resolve(track(0), vec![day(20)], || 3);

        assert_eq!(*revisited, 1);
        let mut held: Vec<(TrackRef, i32)> = cache
            .iter_unsorted()
            .map(|(track, values)| (track, **values))
            .collect();
        held.sort_by_key(|&(track, _)| track);
        assert_eq!(held, vec![(track(0), 1), (track(1), 2)]);
    }

    #[test]
    fn a_track_that_is_no_longer_loaded_is_dropped() {
        let mut cache = TrackValuesByArchivedDays::default();
        cache.resolve(track(0), vec![day(20)], || 1);
        cache.resolve(track(1), vec![day(20)], || 2);

        cache.retain_loaded_tracks(&HashSet::from([track(1)]));

        let held: Vec<TrackRef> = cache.iter_unsorted().map(|(track, _)| track).collect();
        assert_eq!(held, vec![track(1)]);
    }

    /// Only the tracks that read a deleted day are dropped: the others keep
    /// the identity the plot and the query fingerprint hold.
    #[test]
    fn a_track_resolved_from_a_deleted_day_is_dropped() {
        let mut cache = TrackValuesByArchivedDays::default();
        let kept = cache.resolve(track(0), vec![day(21)], || 1);
        cache.resolve(track(1), vec![day(19), day(21)], || 2);

        cache.forget_pruned_days(PrunedDays::Before(day(20)));

        let held: Vec<TrackRef> = cache.iter_unsorted().map(|(track, _)| track).collect();
        assert_eq!(held, vec![track(0)]);
        assert!(Arc::ptr_eq(
            &kept,
            &cache.resolve(track(0), vec![day(21)], || 3)
        ));
    }
}
