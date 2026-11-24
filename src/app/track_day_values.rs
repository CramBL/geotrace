//! The per-track values the environment schedulers resolve from their day
//! archives.
//!
//! The cache reuses values for the same recording data and archived days.

use std::collections::hash_map::Entry;

use chrono::NaiveDate;
use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_types::{Generation, TrackRef, Versionable, Versioned};
use rustc_hash::{FxHashMap, FxHashSet};

use super::environment_storage::PrunedDays;

struct ResolvedTrackValues<T> {
    archived_days: Vec<NaiveDate>,
    values: T,
}

pub(super) struct ArchivedTrackValues;

impl Versionable for ArchivedTrackValues {}

/// One scheduler's per-track values, each keyed by the archived days it was
/// resolved from.
pub struct TrackValuesByArchivedDays<T> {
    by_track: FxHashMap<TrackRef, ResolvedTrackValues<T>>,
    data_generation: Option<Generation<RecordingData>>,
    revision: Versioned<ArchivedTrackValues>,
}

impl<T> Default for TrackValuesByArchivedDays<T> {
    fn default() -> Self {
        Self {
            by_track: FxHashMap::default(),
            data_generation: None,
            revision: Versioned::new(ArchivedTrackValues),
        }
    }
}

impl<T> TrackValuesByArchivedDays<T> {
    pub fn sync_recordings(&mut self, files: LoadedFilesView<'_>) {
        let generation = Some(files.data_generation());
        if self.data_generation != generation {
            self.by_track.clear();
            self.data_generation = generation;
            self.revision.get_mut();
        }
    }

    pub fn resolve(
        &mut self,
        track: TrackRef,
        archived_days: Vec<NaiveDate>,
        resolve: impl FnOnce() -> T,
    ) -> &T {
        let resolved = match self.by_track.entry(track) {
            Entry::Occupied(entry) if entry.get().archived_days == archived_days => entry,
            entry => {
                self.revision.get_mut();
                entry.insert_entry(ResolvedTrackValues {
                    archived_days,
                    values: resolve(),
                })
            }
        };
        &resolved.into_mut().values
    }

    pub fn retain_loaded_tracks(&mut self, loaded: &FxHashSet<TrackRef>) {
        self.retain(|track, _| loaded.contains(track));
    }

    pub fn forget_day(&mut self, day: NaiveDate) {
        self.retain(|_, resolved| !resolved.archived_days.contains(&day));
    }

    /// Drop every track that read a day a delete removed from the archive:
    /// the next resolve reads what the archive still holds.
    pub fn forget_pruned_days(&mut self, pruned: PrunedDays) {
        self.retain(|_, resolved| !resolved.archived_days.iter().any(|day| pruned.covers(*day)));
    }

    fn retain(&mut self, mut keep: impl FnMut(&TrackRef, &ResolvedTrackValues<T>) -> bool) {
        let previous_len = self.by_track.len();
        self.by_track
            .retain(|track, resolved| keep(track, resolved));
        if self.by_track.len() != previous_len {
            self.revision.get_mut();
        }
    }

    pub(super) fn generation(&self) -> Generation<ArchivedTrackValues> {
        self.revision.generation()
    }

    pub fn iter_unsorted(&self) -> impl Iterator<Item = (TrackRef, &T)> {
        self.by_track
            .iter()
            .map(|(&track, resolved)| (track, &resolved.values))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_types::{FileIdx, TrackIdx};

    use super::*;

    fn day(day_of_month: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, day_of_month).unwrap_or_default()
    }

    fn track(index: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(index))
    }

    #[test]
    fn changing_archived_days_replaces_the_resolved_values() {
        let mut cache = TrackValuesByArchivedDays::default();

        let first = Arc::clone(cache.resolve(track(0), vec![day(20)], || Arc::new(1)));
        let second = Arc::clone(cache.resolve(track(0), vec![day(20)], || Arc::new(2)));
        let after_archiving = cache.resolve(track(0), vec![day(20), day(21)], || Arc::new(3));

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*second, 1);
        assert!(!Arc::ptr_eq(&second, after_archiving));
        assert_eq!(**after_archiving, 3);
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
            .map(|(track, values)| (track, *values))
            .collect();
        held.sort_by_key(|&(track, _)| track);
        assert_eq!(held, vec![(track(0), 1), (track(1), 2)]);

        let before_replacement = cache.generation();
        cache.forget_day(day(21));
        assert_ne!(cache.generation(), before_replacement);
        assert_eq!(cache.iter_unsorted().collect::<Vec<_>>(), [(track(0), &1)]);
    }

    #[test]
    fn a_track_that_is_no_longer_loaded_is_dropped() {
        let mut cache = TrackValuesByArchivedDays::default();
        cache.resolve(track(0), vec![day(20)], || 1);
        cache.resolve(track(1), vec![day(20)], || 2);

        let before_unload = cache.generation();
        cache.retain_loaded_tracks(&FxHashSet::from_iter([track(1)]));
        assert_ne!(cache.generation(), before_unload);

        let held: Vec<TrackRef> = cache.iter_unsorted().map(|(track, _)| track).collect();
        assert_eq!(held, vec![track(1)]);
    }

    /// Only the tracks that read a deleted day are dropped: the others keep
    /// the identity the plot and the query fingerprint hold.
    #[test]
    fn a_track_resolved_from_a_deleted_day_is_dropped() {
        let mut cache = TrackValuesByArchivedDays::default();
        let kept = Arc::clone(cache.resolve(track(0), vec![day(21)], || Arc::new(1)));
        cache.resolve(track(1), vec![day(19), day(21)], || Arc::new(2));

        let before_prune = cache.generation();
        cache.forget_pruned_days(PrunedDays::Before(day(20)));
        assert_ne!(cache.generation(), before_prune);

        let held: Vec<TrackRef> = cache.iter_unsorted().map(|(track, _)| track).collect();
        assert_eq!(held, vec![track(0)]);
        assert!(Arc::ptr_eq(
            &kept,
            cache.resolve(track(0), vec![day(21)], || Arc::new(3))
        ));
    }

    #[test]
    fn missing_values_stay_cached_until_the_archived_days_change() {
        let mut cache = TrackValuesByArchivedDays::default();
        let archived = day(20);
        let resolve = || Some(Arc::new(vec![5]));

        assert_eq!(cache.resolve(track(0), vec![archived], || None), &None);
        assert_eq!(cache.resolve(track(0), vec![archived], resolve), &None);

        assert_eq!(
            cache.resolve(track(0), vec![archived, day(21)], resolve),
            &Some(Arc::new(vec![5]))
        );
    }
}
