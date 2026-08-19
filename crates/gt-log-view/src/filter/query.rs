//! The cancellable parallel scan that fills one filter's match bitset.
//!
//! Every edit of a filter starts a new generation of its scan over the line
//! index. Chunks of a superseded generation stop where they are, and only the
//! newest generation ever lands: readers keep the one before it until then.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use gt_logfile::{LogEntry, ParsedLog};
use parking_lot::{Condvar, Mutex};
use rayon::prelude::*;

use crate::filter::{
    matches::{BITS_PER_WORD, EntryMatches, MatchChunk},
    pattern::CompiledFilter,
};

/// Bitset words one chunk of a scan fills. At 64 entries per word this is
/// 65,536 entries per chunk: enough for a worker to earn its dispatch, small
/// enough for the pool to keep stealing work on a log of a few hundred thousand
/// lines.
const WORDS_PER_CHUNK: NonZeroUsize = match NonZeroUsize::new(1024) {
    Some(words) => words,
    None => NonZeroUsize::MIN,
};

/// One filter's matches, and the scan keeping them up to date with the text the
/// user is typing.
#[derive(Debug)]
pub(crate) struct FilterQuery {
    shared: Arc<QueryState>,

    /// The newest landed generation, copied out of `shared`: reads take no
    /// lock.
    landed: LandedScan,
}

impl FilterQuery {
    /// A query of an unwritten filter, which matches no entry of a log of
    /// `entry_count` entries.
    pub(crate) fn matching_nothing(entry_count: usize) -> Self {
        let landed = LandedScan {
            generation: 0,
            matches: Arc::new(EntryMatches::none(entry_count)),
            matches_nothing: true,
        };
        Self {
            shared: Arc::new(QueryState {
                requested_generation: AtomicU64::new(0),
                landed: Mutex::new(landed.clone()),
                landing: Condvar::new(),
            }),
            landed,
        }
    }

    /// Starts a generation of the scan for `compiled`, superseding whichever
    /// generation is still running.
    pub(crate) fn restart(&mut self, log: &Arc<ParsedLog>, compiled: Arc<CompiledFilter>) {
        let generation = self
            .shared
            .requested_generation
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);

        if compiled.matches_nothing() {
            self.shared.land(LandedScan {
                generation,
                matches: Arc::new(EntryMatches::none(log.entries().len())),
                matches_nothing: true,
            });
            self.take_landed();
            return;
        }

        let shared = Arc::clone(&self.shared);
        let log = Arc::clone(log);
        let scan = move || {
            let is_current = || shared.requested_generation.load(Ordering::SeqCst) == generation;
            if let Some(matches) = scan_entries(&log, &compiled, &is_current) {
                shared.land(LandedScan {
                    generation,
                    matches: Arc::new(matches),
                    matches_nothing: false,
                });
            }
        };
        match gt_logfile::log_worker_pool() {
            Some(pool) => pool.spawn(scan),
            None => scan(),
        }
    }

    /// Whether a generation newer than the landed one is still running.
    pub(crate) fn is_pending(&self) -> bool {
        self.landed.generation < self.shared.requested_generation.load(Ordering::SeqCst)
    }

    /// Reads the newest landed generation, reporting whether it replaced the
    /// one read before it.
    pub(crate) fn take_landed(&mut self) -> bool {
        let landed = self.shared.landed.lock();
        if landed.generation == self.landed.generation {
            return false;
        }
        self.landed = landed.clone();
        true
    }

    /// Blocks until the newest generation has landed.
    pub(crate) fn wait_for_landing(&mut self) {
        let mut landed = self.shared.landed.lock();
        while landed.generation < self.shared.requested_generation.load(Ordering::SeqCst) {
            self.shared.landing.wait(&mut landed);
        }
        drop(landed);
        self.take_landed();
    }

    pub(crate) fn matches(&self) -> &EntryMatches {
        &self.landed.matches
    }

    /// Whether the landed generation came from a filter that selects nothing:
    /// an empty field, or a regex that failed to compile.
    pub(crate) fn landed_matches_nothing(&self) -> bool {
        self.landed.matches_nothing
    }
}

/// What a filter's scan and its readers share: the generation last requested,
/// and the newest one that finished.
#[derive(Debug)]
struct QueryState {
    requested_generation: AtomicU64,
    landed: Mutex<LandedScan>,
    landing: Condvar,
}

impl QueryState {
    /// Publishes what a generation matched, unless a newer one already landed.
    fn land(&self, scan: LandedScan) {
        let mut landed = self.landed.lock();
        if scan.generation <= landed.generation {
            return;
        }
        *landed = scan;
        self.landing.notify_all();
    }
}

#[derive(Debug, Clone)]
struct LandedScan {
    generation: u64,
    matches: Arc<EntryMatches>,
    matches_nothing: bool,
}

/// Marks every entry of `log` whose message `compiled` matches, over
/// [`gt_logfile::log_worker_pool`] in work-stealing chunks.
///
/// `is_current` is checked once per chunk: `None` means a newer generation
/// superseded this scan, which then leaves its remaining chunks unread.
fn scan_entries(
    log: &ParsedLog,
    compiled: &CompiledFilter,
    is_current: &(impl Fn() -> bool + Sync),
) -> Option<EntryMatches> {
    scan_entries_in_chunks_of(log, compiled, is_current, WORDS_PER_CHUNK)
}

fn scan_entries_in_chunks_of(
    log: &ParsedLog,
    compiled: &CompiledFilter,
    is_current: &(impl Fn() -> bool + Sync),
    words_per_chunk: NonZeroUsize,
) -> Option<EntryMatches> {
    let entries = log.entries();
    let text = log.text().as_ref();
    let entries_per_chunk = words_per_chunk.get().saturating_mul(BITS_PER_WORD);
    let scan_chunk = |chunk: &[LogEntry]| {
        is_current().then(|| {
            MatchChunk::of(
                chunk
                    .iter()
                    .map(|entry| compiled.matches(entry.message.in_text(text))),
            )
        })
    };

    let chunks: Option<Vec<MatchChunk>> = match gt_logfile::log_worker_pool() {
        Some(pool) => pool.install(|| {
            entries
                .par_chunks(entries_per_chunk)
                .map(scan_chunk)
                .collect()
        }),
        None => entries.chunks(entries_per_chunk).map(scan_chunk).collect(),
    };
    Some(EntryMatches::from_chunks(chunks?, entries.len()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use proptest::{prelude::*, proptest};

    use super::*;
    use crate::{
        filter::pattern::FilterPattern,
        test_fixtures::{self, parsed_log},
    };

    fn compiled(text: &str) -> CompiledFilter {
        FilterPattern::plain(text)
            .compile()
            .expect("a plain pattern always compiles")
    }

    fn words(count: usize) -> NonZeroUsize {
        NonZeroUsize::new(count).expect("a positive chunk width")
    }

    fn scanned(log: &ParsedLog, text: &str, words_per_chunk: NonZeroUsize) -> EntryMatches {
        scan_entries_in_chunks_of(log, &compiled(text), &|| true, words_per_chunk)
            .expect("nothing supersedes this scan")
    }

    #[test]
    fn a_scan_marks_the_entries_whose_message_matches() {
        let log = parsed_log(10);
        let matches = scanned(&log, "entry 7", WORDS_PER_CHUNK);

        assert_eq!(matches.match_count(), 1);
        assert_eq!(matches.matched_entry_indices().collect::<Vec<_>>(), [7]);
        assert_eq!(matches.entry_count(), 10);
    }

    /// The digits a timestamp is written with are not there to be matched: a
    /// timestamp is not part of the message.
    #[test]
    fn a_scan_never_matches_a_timestamp() {
        let log = parsed_log(3);
        assert_eq!(scanned(&log, "14:02", WORDS_PER_CHUNK).match_count(), 0);
        assert_eq!(scanned(&log, "2026", WORDS_PER_CHUNK).match_count(), 0);
    }

    /// No bit stands for a structural line and no filter selects it onto the
    /// map: a structural line is not an entry.
    #[test]
    fn a_scan_never_matches_a_structural_line() {
        let log = test_fixtures::parsed_log_of_text(
            "2026-01-01 14:02:11 navsyncd: starting\n\
             --- Device reboot ---\n\
             2026-01-01 14:02:20 navsyncd: reboot done\n",
        );
        assert_eq!(log.entries().len(), 2);
        assert_eq!(
            scanned(&log, "Device reboot", WORDS_PER_CHUNK).match_count(),
            0
        );
        assert_eq!(scanned(&log, "reboot", WORDS_PER_CHUNK).match_count(), 1);
    }

    /// An entry the parser timestamped from its neighbours filters like any
    /// other entry.
    #[test]
    fn a_scan_matches_an_interpolated_entry() {
        let log = test_fixtures::parsed_log_of_text(
            "2026-01-01 14:02:11 navsyncd: starting\n  at 0x0000c3f4 in gnss_task+0x54\n",
        );
        assert_eq!(log.interpolated_entry_count(), 1);
        assert_eq!(
            scanned(&log, "gnss_task", WORDS_PER_CHUNK)
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            [1]
        );
    }

    #[test]
    fn a_superseded_scan_leaves_its_remaining_chunks_unread() {
        let log = parsed_log(20_000);
        let words_per_chunk = words(1);
        let chunks_started = AtomicUsize::new(0);

        let scan = scan_entries_in_chunks_of(
            &log,
            &compiled("entry"),
            &|| {
                chunks_started.fetch_add(1, Ordering::SeqCst);
                false
            },
            words_per_chunk,
        );

        assert_eq!(scan, None, "a superseded scan publishes nothing");
        let chunk_count = log
            .entries()
            .len()
            .div_ceil(words_per_chunk.get() * BITS_PER_WORD);
        assert!(
            chunks_started.load(Ordering::SeqCst) < chunk_count,
            "the scan read all {chunk_count} chunks of a superseded generation"
        );
    }

    #[test]
    fn a_generation_older_than_the_landed_one_never_replaces_it() {
        let mut query = FilterQuery::matching_nothing(4);

        query.shared.land(LandedScan {
            generation: 2,
            matches: Arc::new(EntryMatches::none(4)),
            matches_nothing: false,
        });
        query.shared.land(LandedScan {
            generation: 1,
            matches: Arc::new(EntryMatches::none(8)),
            matches_nothing: true,
        });
        query.take_landed();

        assert_eq!(query.matches().entry_count(), 4);
        assert!(!query.landed_matches_nothing());
    }

    #[test]
    fn an_edited_filter_lands_its_matches_and_stops_pending() {
        let log = Arc::new(parsed_log(1_000));
        let mut query = FilterQuery::matching_nothing(log.entries().len());

        query.restart(&log, Arc::new(compiled("entry 512")));
        query.wait_for_landing();

        assert!(!query.is_pending());
        assert_eq!(query.matches().match_count(), 1);
        assert!(!query.landed_matches_nothing());
    }

    /// The last edit decides, whichever order the generations finish in.
    #[test]
    fn the_newest_generation_is_the_one_that_lands() {
        let log = Arc::new(parsed_log(5_000));
        let mut query = FilterQuery::matching_nothing(log.entries().len());

        query.restart(&log, Arc::new(compiled("entry")));
        query.restart(&log, Arc::new(compiled("entry 3")));
        query.wait_for_landing();

        assert_eq!(
            query.matches().match_count(),
            (0..5_000)
                .filter(|entry: &usize| entry.to_string().contains('3'))
                .count(),
        );
    }

    proptest! {
        /// However the chunk bounds fall, the bitset holds exactly the entries
        /// a walk of the log matches.
        #[test]
        fn a_chunked_scan_marks_what_a_walk_of_the_entries_marks(
            entry_count in 1usize..500,
            words_per_chunk in 1usize..8,
        ) {
            let log = parsed_log(entry_count);
            let filter = compiled("entry 1");
            let matches = scanned(&log, "entry 1", words(words_per_chunk));

            let expected: Vec<usize> = log
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, entry)| filter.matches(log.message(entry)))
                .map(|(index, _)| index)
                .collect();
            prop_assert_eq!(matches.match_count(), expected.len());
            prop_assert_eq!(matches.matched_entry_indices().collect::<Vec<_>>(), expected);
            prop_assert_eq!(matches.entry_count(), entry_count);
        }
    }
}
