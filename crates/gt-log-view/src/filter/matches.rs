//! The bitset one filter's scan fills: one bit per entry, in entry order.

use std::iter;

/// The entries of one log a single filter matched.
///
/// The table, the gutter bars and the map all read this, and only a newer
/// generation of the filter's scan replaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMatches {
    words: Vec<u64>,
    entry_count: usize,
    match_count: usize,
}

impl EntryMatches {
    pub(crate) fn intersection(sets: &[&Self]) -> Self {
        let entry_count = sets.iter().map(|set| set.entry_count).min().unwrap_or(0);
        let words: Vec<_> = (0..entry_count.div_ceil(BITS_PER_WORD))
            .map(|index| {
                sets.iter()
                    .fold(u64::MAX, |word, set| word & set.word(index))
            })
            .collect();
        let match_count = words.iter().map(|word| word.count_ones() as usize).sum();
        Self {
            words,
            entry_count,
            match_count,
        }
    }

    pub(crate) fn union(sets: &[&Self]) -> Self {
        let entry_count = sets.iter().map(|set| set.entry_count).max().unwrap_or(0);
        let words: Vec<_> = (0..entry_count.div_ceil(BITS_PER_WORD))
            .map(|index| sets.iter().fold(0, |word, set| word | set.word(index)))
            .collect();
        let match_count = words.iter().map(|word| word.count_ones() as usize).sum();
        Self {
            words,
            entry_count,
            match_count,
        }
    }

    pub fn none(entry_count: usize) -> Self {
        Self {
            words: vec![0; entry_count.div_ceil(BITS_PER_WORD)],
            entry_count,
            match_count: 0,
        }
    }

    /// Whether the entry at `entry_index` of
    /// [`ParsedLog::entries`](gt_logfile::ParsedLog::entries) matched.
    pub fn contains(&self, entry_index: usize) -> bool {
        let Some(word) = self.words.get(entry_index / BITS_PER_WORD) else {
            return false;
        };
        word & (1 << (entry_index % BITS_PER_WORD)) != 0
    }

    pub fn match_count(&self) -> usize {
        self.match_count
    }

    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// The matched entries' indices, ascending.
    pub fn matched_entry_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.words
            .iter()
            .enumerate()
            .flat_map(|(word_index, word)| set_bits(*word, word_index * BITS_PER_WORD))
    }

    fn word(&self, word_index: usize) -> u64 {
        self.words.get(word_index).copied().unwrap_or(0)
    }

    /// Joins the spans of the bitset the chunks of one scan filled, in the
    /// order the chunks cover the log.
    pub(crate) fn from_chunks(chunks: Vec<MatchChunk>, entry_count: usize) -> Self {
        let mut words = Vec::with_capacity(entry_count.div_ceil(BITS_PER_WORD));
        let mut match_count = 0;
        for chunk in chunks {
            words.extend_from_slice(&chunk.words);
            match_count += chunk.match_count;
        }
        Self {
            words,
            entry_count,
            match_count,
        }
    }
}

/// The words one chunk of a scan filled, covering that chunk's entries alone.
///
/// The chunks concatenate into the bitset of the whole log without shifting:
/// each of them covers a whole number of words.
pub(crate) struct MatchChunk {
    words: Vec<u64>,
    match_count: usize,
}

impl MatchChunk {
    pub(crate) fn of(matched: impl Iterator<Item = bool>) -> Self {
        let mut words = Vec::with_capacity(matched.size_hint().0.div_ceil(BITS_PER_WORD));
        let mut match_count = 0;
        let mut word = 0u64;
        let mut bit = 0;
        for is_match in matched {
            if is_match {
                word |= 1 << bit;
                match_count += 1;
            }
            bit += 1;
            if bit == BITS_PER_WORD {
                words.push(word);
                word = 0;
                bit = 0;
            }
        }
        if bit > 0 {
            words.push(word);
        }
        Self { words, match_count }
    }
}

/// The entry indices the set bits of `word` stand for, offset by
/// `first_entry_index`.
fn set_bits(word: u64, first_entry_index: usize) -> impl Iterator<Item = usize> {
    let mut remaining = word;
    iter::from_fn(move || {
        if remaining == 0 {
            return None;
        }
        let bit = remaining.trailing_zeros() as usize;
        remaining &= remaining.wrapping_sub(1);
        Some(first_entry_index.saturating_add(bit))
    })
}

pub(crate) const BITS_PER_WORD: usize = u64::BITS as usize;

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    fn matches_of(entry_count: usize, matched: &[usize]) -> EntryMatches {
        EntryMatches::from_chunks(
            vec![MatchChunk::of(
                (0..entry_count).map(|entry_index| matched.contains(&entry_index)),
            )],
            entry_count,
        )
    }

    #[test]
    fn a_bitset_holds_the_entries_the_scan_set_and_no_others() {
        let matched = [0, 63, 64, 129];
        let matches = matches_of(200, &matched);

        assert_eq!(matches.match_count(), matched.len());
        assert_eq!(matches.entry_count(), 200);
        assert_eq!(
            matches.matched_entry_indices().collect::<Vec<_>>(),
            matched.to_vec()
        );
        assert!((0..200).all(|entry| matches.contains(entry) == matched.contains(&entry)));
    }

    #[test]
    fn an_intersection_holds_the_entries_every_set_matched() {
        let first = matches_of(200, &[1, 2, 64, 130]);
        let second = matches_of(200, &[2, 64, 131]);
        let third = matches_of(200, &[2, 3, 64]);

        assert_eq!(
            EntryMatches::intersection(&[&first, &second, &third])
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            [2, 64]
        );
        assert_eq!(
            EntryMatches::intersection(&[&first])
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            [1, 2, 64, 130]
        );
        assert_eq!(
            EntryMatches::intersection(&[])
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            Vec::<usize>::new()
        );
    }

    /// No intersection invents an entry past the last one: a log whose entry
    /// count is not a multiple of the word width leaves those bits clear.
    #[test]
    fn an_intersection_stops_at_the_last_entry() {
        let first = matches_of(65, &[64]);
        let second = matches_of(65, &[64]);
        assert_eq!(
            EntryMatches::intersection(&[&first, &second])
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            [64]
        );
    }

    #[rstest]
    #[case::overlap(3, vec![0, 1], vec![1, 2], vec![0, 1, 2])]
    #[case::word_and_tail_edges(131, vec![0, 63, 64, 130], vec![63, 65, 129], vec![0, 63, 64, 65, 129, 130])]
    #[case::empty(0, vec![], vec![], vec![])]
    fn union_composes_matches_in_entry_order(
        #[case] count: usize,
        #[case] first: Vec<usize>,
        #[case] second: Vec<usize>,
        #[case] expected: Vec<usize>,
    ) {
        let first = matches_of(count, &first);
        let second = matches_of(count, &second);
        assert_eq!(
            EntryMatches::union(&[&first, &second])
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            EntryMatches::union(&[&first])
                .matched_entry_indices()
                .collect::<Vec<_>>(),
            first.matched_entry_indices().collect::<Vec<_>>()
        );
        assert_eq!(EntryMatches::union(&[]).match_count(), 0);
    }

    proptest! {
        #[test]
        fn bitset_composition_matches_boolean_operations(
            first in proptest::collection::vec(any::<bool>(), 0..260),
            second in proptest::collection::vec(any::<bool>(), 0..260),
        ) {
            let first_set = EntryMatches::from_chunks(vec![MatchChunk::of(first.iter().copied())], first.len());
            let second_set = EntryMatches::from_chunks(vec![MatchChunk::of(second.iter().copied())], second.len());
            let expected_union: Vec<_> = (0..first.len().max(second.len()))
                .filter(|index| first.get(*index).copied().unwrap_or(false) || second.get(*index).copied().unwrap_or(false))
                .collect();
            let expected_intersection: Vec<_> = (0..first.len().min(second.len()))
                .filter(|index| first.get(*index).copied().unwrap_or(false) && second.get(*index).copied().unwrap_or(false))
                .collect();
            prop_assert_eq!(EntryMatches::union(&[&first_set, &second_set]).matched_entry_indices().collect::<Vec<_>>(), expected_union);
            prop_assert_eq!(EntryMatches::intersection(&[&first_set, &second_set]).matched_entry_indices().collect::<Vec<_>>(), expected_intersection);
        }
    }

    #[test]
    fn a_bitset_reports_no_match_for_an_entry_past_the_log() {
        let matches = matches_of(3, &[2]);
        assert!(!matches.contains(3));
        assert!(!matches.contains(usize::MAX));
    }

    #[test]
    fn an_empty_bitset_matches_nothing() {
        let matches = EntryMatches::none(100);
        assert_eq!(matches.match_count(), 0);
        assert_eq!(matches.matched_entry_indices().count(), 0);
        assert!(!matches.contains(0));
    }

    /// A chunk's span lands where the entries it covers are: every chunk fills
    /// whole words.
    #[test]
    fn chunks_concatenate_into_the_bitset_of_the_whole_log() {
        let chunks = vec![
            MatchChunk::of((0..BITS_PER_WORD).map(|entry| entry == 5)),
            MatchChunk::of((0..BITS_PER_WORD).map(|entry| entry == 5)),
        ];
        let joined = EntryMatches::from_chunks(chunks, 2 * BITS_PER_WORD);

        assert_eq!(
            joined.matched_entry_indices().collect::<Vec<_>>(),
            [5, BITS_PER_WORD + 5]
        );
        assert_eq!(joined.match_count(), 2);
    }
}
