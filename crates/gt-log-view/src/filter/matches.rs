//! The bitset one filter's scan fills: one bit per entry, in entry order.

use std::iter;

pub(crate) const BITS_PER_WORD: usize = u64::BITS as usize;

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

/// The entries every one of `sets` matched, ascending. An empty `sets` matches
/// no entry: an intersection of nothing narrows nothing, and the caller decides
/// what that means before the call.
pub(crate) fn intersecting_entry_indices(sets: &[&EntryMatches]) -> Vec<usize> {
    let word_count = sets.iter().map(|set| set.words.len()).min().unwrap_or(0);
    let mut entry_indices = Vec::new();
    for word_index in 0..word_count {
        let word = sets
            .iter()
            .fold(u64::MAX, |shared, set| shared & set.word(word_index));
        entry_indices.extend(set_bits(word, word_index.saturating_mul(BITS_PER_WORD)));
    }
    entry_indices
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

#[cfg(test)]
mod tests {
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
            intersecting_entry_indices(&[&first, &second, &third]),
            [2, 64]
        );
        assert_eq!(intersecting_entry_indices(&[&first]), [1, 2, 64, 130]);
        assert_eq!(intersecting_entry_indices(&[]), Vec::<usize>::new());
    }

    /// No intersection invents an entry past the last one: a log whose entry
    /// count is not a multiple of the word width leaves those bits clear.
    #[test]
    fn an_intersection_stops_at_the_last_entry() {
        let first = matches_of(65, &[64]);
        let second = matches_of(65, &[64]);
        assert_eq!(intersecting_entry_indices(&[&first, &second]), [64]);
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
