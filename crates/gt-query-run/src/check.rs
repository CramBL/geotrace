use std::ops::Range;

use gt_query::lexer::{self, TokenClass};
use gt_query::{ChannelSchema, CheckedQuery, Diagnostic};

/// One query in the editor text: its byte range and its check outcome.
pub struct QueryChunk {
    pub range: Range<usize>,
    pub result: Result<CheckedQuery, Diagnostic>,
}

/// Parse and check one query against the loaded channels.
pub fn check_text(text: &str, schema: &ChannelSchema) -> Result<CheckedQuery, Diagnostic> {
    gt_query::check(&gt_query::parse(text)?, schema)
}

/// Parse and check every query in `text` against the loaded channels.
/// Queries are separated by a blank line. Each chunk keeps its byte range so
/// diagnostics and the caret map back to editor coordinates.
///
/// A chunk holding no code, a standalone comment paragraph, is skipped: a block
/// comment between queries neither errors nor blocks a run.
pub fn check_all(text: &str, schema: &ChannelSchema) -> Vec<QueryChunk> {
    split_queries(text)
        .into_iter()
        .filter_map(|range| {
            let src = text.get(range.clone()).unwrap_or("");
            // Comment-only via the highlighter's classes: the parsing
            // tokenizer drops rejected characters, which must still be checked
            // and reported.
            let comment_only = lexer::highlight_classes(src)
                .iter()
                .all(|(_, class)| *class == TokenClass::Comment);
            if comment_only {
                return None;
            }
            Some(QueryChunk {
                result: check_text(src, schema),
                range,
            })
        })
        .collect()
}

/// The byte range of the text the caret's completions analyze.
///
/// The query chunk containing the caret. When the caret sits on the line
/// directly after a chunk (an Enter pressed to continue it with `| …`), that
/// chunk extended to the caret. Otherwise an empty context at the caret, for a
/// fresh query.
pub fn analysis_context(text: &str, caret: usize) -> Range<usize> {
    let mut preceding: Option<Range<usize>> = None;
    for range in split_queries(text) {
        if range.start <= caret && caret <= range.end {
            return range;
        }
        if range.end < caret {
            preceding = Some(range);
        }
    }
    // Directly after a chunk means only whitespace with at most one newline in
    // between: a second newline is the blank-line separator, so the caret
    // starts a fresh query.
    if let Some(range) = preceding
        && let Some(gap) = text.get(range.end..caret)
        && gap.chars().all(char::is_whitespace)
        && gap.matches('\n').count() <= 1
    {
        return range.start..caret;
    }
    caret..caret
}

/// Byte ranges of the blank-line-separated queries in `text`. Each range spans
/// from a query's first non-blank line to the end of its last non-blank line.
pub fn split_queries(text: &str) -> Vec<Range<usize>> {
    let mut chunks = Vec::new();
    let mut current: Option<Range<usize>> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if line.trim().is_empty() {
            if let Some(range) = current.take() {
                chunks.push(range);
            }
        } else {
            let content_end = start + line.trim_end().len();
            match &mut current {
                Some(range) => range.end = content_end,
                None => current = Some(start..content_end),
            }
        }
    }
    if let Some(range) = current {
        chunks.push(range);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    // A blank line separates two queries.
    #[case("a\n\nb", &[(0, 1), (3, 4)])]
    // Leading and trailing blank lines are dropped.
    #[case("\n\na\n\n", &[(2, 3)])]
    // A whitespace-only line still separates.
    #[case("a\n   \nb", &[(0, 1), (6, 7)])]
    // A single chunk without a trailing newline.
    #[case("a", &[(0, 1)])]
    // Adjacent non-blank lines are one multi-line query, and the range ends at
    // the last line's trimmed content.
    #[case("points\n| draw", &[(0, 13)])]
    fn split_queries_handles_blank_line_edge_cases(
        #[case] text: &str,
        #[case] want: &[(usize, usize)],
    ) {
        let ranges: Vec<(usize, usize)> = split_queries(text)
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect();
        assert_eq!(ranges, want, "input: {text:?}");
    }

    #[test]
    fn analysis_context_ties_the_next_line_to_its_chunk() {
        // Caret inside a chunk: the chunk itself.
        assert_eq!(analysis_context("points | draw", 6), 0..13);
        // Caret on the line directly after a chunk (Enter pressed to continue
        // it): the chunk extends to the caret, so `| where` is analyzed in
        // context.
        let text = "points\n";
        assert_eq!(analysis_context(text, text.len()), 0..text.len());
        // A blank line in between is the query separator: fresh context.
        let separated = "points\n\n";
        assert_eq!(
            analysis_context(separated, separated.len()),
            separated.len()..separated.len()
        );
        // On the (would-be separator) line right after a chunk, typing still
        // continues that chunk - a character typed there joins the two lines
        // into one query, so the analysis matches what an edit would produce.
        let two = "points | draw\n\npoints | hide";
        assert_eq!(analysis_context(two, 14), 0..14);
    }

    #[test]
    fn comment_only_chunks_are_skipped_not_checked() {
        // A standalone comment paragraph between queries is documentation,
        // not a query: it must not error (or block a run).
        let text = "# block comment\n\npoints | draw";
        let chunks = check_all(text, &ChannelSchema::new());
        assert_eq!(chunks.len(), 1, "only the real query is a chunk");
        assert!(chunks[0].result.is_ok(), "the real query checks");
        // A chunk with a lexer-rejected character is code, not comment - it
        // still surfaces its error.
        let bad = check_all("Points", &ChannelSchema::new());
        assert_eq!(bad.len(), 1);
        assert!(bad[0].result.is_err(), "the rejected character errors");
    }
}
