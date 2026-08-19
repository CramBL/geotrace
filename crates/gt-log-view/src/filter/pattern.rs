//! What a filter matches a log entry's message against.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// A filter as the user wrote it: the text of the field, and whether the `.*`
/// toggle was on while it was written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterPattern {
    pub text: String,
    pub regex: bool,
}

impl FilterPattern {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            regex: false,
        }
    }

    pub fn regex(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            regex: true,
        }
    }

    /// Prepares the pattern for the scan that applies it to every entry.
    ///
    /// What counts as an empty filter differs with the mode: whitespace
    /// separates the terms of a plain filter, and a regex takes it literally.
    pub(crate) fn compile(&self) -> Result<CompiledFilter, InvalidFilterPattern> {
        if self.regex {
            if self.text.is_empty() {
                return Ok(CompiledFilter::matching_nothing());
            }
            return case_insensitive_regex(&self.text)
                .map(|regex| CompiledFilter(Matcher::Regex(Box::new(regex))))
                .map_err(|err| InvalidFilterPattern(err.to_string()));
        }

        let terms: Vec<PlainTerm> = self.text.split_whitespace().map(PlainTerm::new).collect();
        Ok(match terms.is_empty() {
            true => CompiledFilter::matching_nothing(),
            false => CompiledFilter(Matcher::AllTerms(terms)),
        })
    }
}

/// The regex the user wrote could not be compiled. Carries what the regex
/// engine said about it, for the viewer to show under the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidFilterPattern(String);

impl InvalidFilterPattern {
    pub fn message(&self) -> &str {
        &self.0
    }
}

/// A [`FilterPattern`] compiled once per edit and shared by every chunk of the
/// scan that applies it.
#[derive(Debug)]
pub(crate) struct CompiledFilter(Matcher);

impl CompiledFilter {
    /// The filter of an empty field, which no entry passes.
    pub(crate) fn matching_nothing() -> Self {
        Self(Matcher::MatchesNothing)
    }

    /// Whether `message` passes the filter. No filter ever matches a
    /// timestamp: an entry's timestamp is not part of its message.
    pub(crate) fn matches(&self, message: &str) -> bool {
        match &self.0 {
            Matcher::MatchesNothing => false,
            Matcher::AllTerms(terms) => terms.iter().all(|term| term.matches(message)),
            Matcher::Regex(regex) => regex.is_match(message),
        }
    }

    /// An empty field matches no entry: it puts nothing on the map, and leaves
    /// the table showing every line.
    pub(crate) fn matches_nothing(&self) -> bool {
        matches!(self.0, Matcher::MatchesNothing)
    }

    /// Where in `message` the filter matched, as byte ranges the viewer paints
    /// the match colour over: every occurrence of every term of a plain filter,
    /// and every match of a regex. Ascending, non-overlapping, and empty for a
    /// message the filter does not match.
    pub(crate) fn match_spans(&self, message: &str) -> Vec<Range<usize>> {
        let mut spans = Vec::new();
        match &self.0 {
            Matcher::MatchesNothing => return spans,
            Matcher::AllTerms(terms) => {
                for term in terms {
                    let term_spans = term.spans(message);
                    if term_spans.is_empty() {
                        return Vec::new();
                    }
                    spans.extend(term_spans);
                }
            }
            Matcher::Regex(regex) => {
                spans.extend(regex.find_iter(message).map(|found| found.range()));
            }
        }
        merged_spans(spans, message)
    }
}

/// `spans` sorted and joined where they overlap or touch, dropping the empty
/// ones and any that a byte-wise search put inside a character of `message`.
fn merged_spans(mut spans: Vec<Range<usize>>, message: &str) -> Vec<Range<usize>> {
    spans.retain(|span| {
        span.start < span.end
            && message.is_char_boundary(span.start)
            && message.is_char_boundary(span.end)
    });
    spans.sort_unstable_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

#[derive(Debug)]
enum Matcher {
    MatchesNothing,

    /// Every term must occur in the message.
    AllTerms(Vec<PlainTerm>),

    /// Case-insensitive unless the pattern turns that off with `(?-i)`.
    Regex(Box<Regex>),
}

/// One whitespace-separated term of a plain filter, matched as a
/// case-insensitive substring of the message.
#[derive(Debug)]
enum PlainTerm {
    /// An all-ASCII term, the overwhelmingly common case: `memchr` finds the
    /// candidate positions and the bytes there are compared in place.
    Ascii(AsciiTerm),

    /// A term with characters outside ASCII, whose case folding the regex
    /// engine knows.
    Unicode(Box<Regex>),
}

impl PlainTerm {
    fn new(term: &str) -> Self {
        if term.is_ascii() {
            return Self::Ascii(AsciiTerm::new(term));
        }
        match case_insensitive_regex(&regex::escape(term)) {
            Ok(regex) => Self::Unicode(Box::new(regex)),
            Err(err) => {
                log::warn!("Matching the case of {term:?} exactly: {err:#}");
                Self::Ascii(AsciiTerm::new(term))
            }
        }
    }

    fn matches(&self, message: &str) -> bool {
        match self {
            Self::Ascii(term) => term.find_from(message, 0).is_some(),
            Self::Unicode(regex) => regex.is_match(message),
        }
    }

    /// Every occurrence of this term in `message`, ascending and
    /// non-overlapping.
    fn spans(&self, message: &str) -> Vec<Range<usize>> {
        match self {
            Self::Ascii(term) => term.spans(message),
            Self::Unicode(regex) => regex
                .find_iter(message)
                .map(|found| found.range())
                .collect(),
        }
    }
}

/// A term compared byte for byte, folding the case of ASCII letters alone.
#[derive(Debug)]
struct AsciiTerm {
    lowercase: Box<[u8]>,
}

impl AsciiTerm {
    fn new(term: &str) -> Self {
        Self {
            lowercase: term.to_ascii_lowercase().into_bytes().into_boxed_slice(),
        }
    }

    /// Where this term next occurs in `message` at or after the byte offset
    /// `from`. An empty term occurs at `from`.
    fn find_from(&self, message: &str, from: usize) -> Option<Range<usize>> {
        let Some(&first_lowercase) = self.lowercase.first() else {
            return Some(from..from);
        };
        let first_uppercase = first_lowercase.to_ascii_uppercase();
        let haystack = message.as_bytes();
        let mut searched_to = from;
        while let Some(rest) = haystack.get(searched_to..) {
            let candidate_start = memchr::memchr2(first_lowercase, first_uppercase, rest)
                .map(|hit| searched_to.saturating_add(hit))?;
            let candidate_end = candidate_start.saturating_add(self.lowercase.len());
            match haystack.get(candidate_start..candidate_end) {
                Some(candidate) if candidate.eq_ignore_ascii_case(&self.lowercase) => {
                    return Some(candidate_start..candidate_end);
                }
                Some(_) => searched_to = candidate_start.saturating_add(1),
                None => return None,
            }
        }
        None
    }

    fn spans(&self, message: &str) -> Vec<Range<usize>> {
        let mut spans = Vec::new();
        if self.lowercase.is_empty() {
            return spans;
        }
        let mut searched_to = 0;
        while let Some(span) = self.find_from(message, searched_to) {
            searched_to = span.end;
            spans.push(span);
        }
        spans
    }
}

fn case_insensitive_regex(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern).case_insensitive(true).build()
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, prop_oneof, proptest};
    use rstest::rstest;

    use super::*;

    fn matches(pattern: &FilterPattern, message: &str) -> bool {
        pattern
            .compile()
            .expect("the fixture pattern compiles")
            .matches(message)
    }

    #[rstest]
    #[case::one_term("gnss", "navsyncd: gnss fix acquired", true)]
    #[case::every_term_matches("gnss fix", "navsyncd: gnss fix acquired", true)]
    #[case::terms_match_in_any_order("fix gnss", "navsyncd: gnss fix acquired", true)]
    #[case::one_term_missing("gnss modem", "navsyncd: gnss fix acquired", false)]
    #[case::term_written_in_another_case("GNSS FIX", "navsyncd: gnss fix acquired", true)]
    #[case::message_written_in_another_case("gnss", "navsyncd: GNSS fix acquired", true)]
    #[case::candidates_before_the_match("gg", "gagbggc", true)]
    #[case::candidate_running_past_the_message("acquired!", "navsyncd: gnss fix acquired", false)]
    #[case::term_longer_than_the_message("navsyncd", "nav", false)]
    #[case::non_ascii_case_folding("größe", "Größe: 4 KiB", true)]
    fn a_plain_filter_matches_a_message_holding_every_term(
        #[case] text: &str,
        #[case] message: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(matches(&FilterPattern::plain(text), message), expected);
    }

    #[rstest]
    #[case::plain_and_empty(FilterPattern::plain(""))]
    #[case::plain_and_blank(FilterPattern::plain("   "))]
    #[case::regex_and_empty(FilterPattern::regex(""))]
    fn a_filter_without_a_pattern_matches_nothing(#[case] pattern: FilterPattern) {
        let compiled = pattern.compile().expect("compiles");
        assert!(compiled.matches_nothing());
        assert!(!compiled.matches("navsyncd: gnss fix acquired"));
    }

    /// A blank regex is a pattern like any other: whitespace is part of a
    /// regex.
    #[test]
    fn a_regex_of_whitespace_matches_the_whitespace_it_holds() {
        let compiled = FilterPattern::regex("  ").compile().expect("compiles");
        assert!(!compiled.matches_nothing());
        assert!(compiled.matches("navsyncd:  gnss"));
        assert!(!compiled.matches("navsyncd: gnss"));
    }

    #[rstest]
    #[case::case_insensitive_by_default("GNSS", "navsyncd: gnss fix acquired", true)]
    #[case::case_sensitivity_turned_back_on("(?-i)GNSS", "navsyncd: gnss fix acquired", false)]
    #[case::alternation("modem|gnss", "navsyncd: gnss fix acquired", true)]
    #[case::anchored("^navsyncd", "navsyncd: gnss fix acquired", true)]
    #[case::anchored_past_the_start("^gnss", "navsyncd: gnss fix acquired", false)]
    #[case::whitespace_is_part_of_the_pattern("gnss fix", "navsyncd: gnss fix acquired", true)]
    #[case::whitespace_is_not_a_term_separator("fix gnss", "navsyncd: gnss fix acquired", false)]
    fn a_regex_filter_matches_the_message_as_one_pattern(
        #[case] text: &str,
        #[case] message: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(matches(&FilterPattern::regex(text), message), expected);
    }

    #[test]
    fn an_unclosed_group_is_an_error_naming_what_the_regex_engine_read() {
        let error = FilterPattern::regex("navsyncd(")
            .compile()
            .expect_err("fails to compile");
        assert!(
            error.message().contains("unclosed group"),
            "the viewer shows this under the field: {}",
            error.message()
        );
    }

    /// A term pasted out of a log line needs no escaping: a regex
    /// metacharacter is a literal to a plain filter.
    #[test]
    fn a_plain_term_matches_regex_metacharacters_literally() {
        let pattern = FilterPattern::plain("gnss_task+0x54");
        assert!(matches(&pattern, "  at 0x0000c3f4 in gnss_task+0x54"));
        assert!(!matches(&pattern, "  at 0x0000c3f4 in gnss_taskk0x54"));
    }

    fn spans(pattern: &FilterPattern, message: &str) -> Vec<Range<usize>> {
        pattern
            .compile()
            .expect("the fixture pattern compiles")
            .match_spans(message)
    }

    #[rstest]
    #[case::one_term(FilterPattern::plain("gnss"), vec![10..14])]
    #[case::every_occurrence_of_a_term(FilterPattern::plain("fix"), vec![15..18, 29..32])]
    #[case::one_span_per_term(FilterPattern::plain("gnss lost"), vec![10..14, 33..37])]
    #[case::a_term_in_another_case(FilterPattern::plain("GNSS"), vec![10..14])]
    #[case::a_term_the_message_misses(FilterPattern::plain("gnss modem"), Vec::new())]
    #[case::the_whole_regex_match(FilterPattern::regex("gnss.*acquired"), vec![10..27])]
    #[case::every_regex_match(FilterPattern::regex("fix"), vec![15..18, 29..32])]
    #[case::an_empty_field(FilterPattern::plain(""), Vec::new())]
    fn the_spans_are_where_the_table_paints_the_match_colour(
        #[case] pattern: FilterPattern,
        #[case] expected: Vec<Range<usize>>,
    ) {
        let message = "navsyncd: gnss fix acquired, fix lost again";
        assert_eq!(spans(&pattern, message), expected);
    }

    /// Two terms sharing a stretch of the message paint one span, not two
    /// overlapping ones.
    #[test]
    fn overlapping_terms_paint_one_span() {
        let message = "navsyncd: gnss fix acquired";
        assert_eq!(
            spans(&FilterPattern::plain("gnss ss"), message),
            vec![10..14]
        );
    }

    /// A term whose bytes could be found inside a character never yields a span
    /// the table would have to slice a character in half to paint.
    #[test]
    fn a_span_always_starts_and_ends_on_a_character() {
        let message = "größe: 4 KiB";
        for span in spans(&FilterPattern::plain("größe"), message) {
            assert!(message.get(span.clone()).is_some(), "{span:?}");
        }
    }

    fn any_filter_text() -> impl Strategy<Value = String> {
        prop_oneof![
            // Metacharacter soup: the regex mode has to meet patterns it rejects.
            r"[a-z (){}\[\]\\*+?|^$.-]{0,12}",
            any::<String>(),
        ]
    }

    proptest! {
        /// Whatever the user wrote and whatever the message holds, the table
        /// can paint every span it is handed: they are ascending, apart, and
        /// slices of the message.
        #[test]
        fn every_span_is_a_slice_of_the_message_after_the_one_before_it(
            text in any_filter_text(),
            regex in any::<bool>(),
            message in any::<String>(),
        ) {
            let pattern = FilterPattern { text, regex };
            let Ok(compiled) = pattern.compile() else {
                return Ok(());
            };

            let mut painted_to: Option<usize> = None;
            for span in compiled.match_spans(&message) {
                prop_assert!(span.start < span.end, "{span:?} paints nothing");
                if let Some(previous_end) = painted_to {
                    prop_assert!(span.start > previous_end, "{span:?} is not apart");
                }
                prop_assert!(message.get(span.clone()).is_some(), "{span:?} is no slice");
                painted_to = Some(span.end);
            }
            prop_assert!(
                painted_to.is_none() || compiled.matches(&message),
                "a message the filter misses is painted nowhere"
            );
        }

        /// Whatever a user writes into the field, it compiles into a matcher or
        /// into the error the viewer shows under the field.
        #[test]
        fn any_text_compiles_into_a_matcher_or_the_error_to_show(
            text in any_filter_text(),
            regex in any::<bool>(),
            message in any::<String>(),
        ) {
            let pattern = FilterPattern { text: text.clone(), regex };
            match pattern.compile() {
                Ok(compiled) => {
                    let matches_nothing = match regex {
                        true => text.is_empty(),
                        false => text.split_whitespace().next().is_none(),
                    };
                    prop_assert_eq!(compiled.matches_nothing(), matches_nothing);
                    prop_assert!(!(compiled.matches(&message) && matches_nothing));
                }
                Err(error) => {
                    prop_assert!(regex, "a plain filter compiles whatever it holds");
                    prop_assert!(!error.message().is_empty());
                }
            }
        }
    }
}
