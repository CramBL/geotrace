use rstest::rstest;

use crate::parse::tests::fixtures;
use crate::parse::{self, ERROR_LINE_EXCERPT_CHARS, LogParseError};

#[test]
fn a_log_without_any_recognised_timestamp_names_its_first_line() {
    let error = parse::parse_log("nothing here\nnor here\n".into(), fixtures::now())
        .expect_err("fails to parse");
    assert_eq!(
        error.to_string(),
        "Not a recognised log: no line has a timestamp in a known format \
         (first line: \"nothing here\")"
    );
}

#[test]
fn a_very_long_first_line_is_quoted_up_to_an_excerpt() {
    let text = "x".repeat(ERROR_LINE_EXCERPT_CHARS.get() + 50);
    let error =
        parse::parse_log(text.as_str().into(), fixtures::now()).expect_err("fails to parse");
    assert_eq!(
        error,
        LogParseError::NoRecognisedFormat {
            first_line: format!(
                "{}{}",
                "x".repeat(ERROR_LINE_EXCERPT_CHARS.get()),
                gt_fmt::ELLIPSIS
            ),
        }
    );
}

#[rstest]
#[case::no_bytes("")]
#[case::only_blank_lines("\n\n   \n")]
fn a_log_without_any_line_is_empty(#[case] text: &str) {
    assert_eq!(
        parse::parse_log(text.into(), fixtures::now()),
        Err(LogParseError::Empty)
    );
}
