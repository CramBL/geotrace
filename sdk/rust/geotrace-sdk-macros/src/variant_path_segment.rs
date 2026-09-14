use std::fmt;
use std::fmt::{Display, Formatter};

use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;
use syn::Ident;
use syn::ext::IdentExt as _;

/// One segment of an event marker variant path.
///
/// `validate_variant_path` in `geotrace-sdk` has a copy of this rule for a whole path, and the
/// tests of this module compare the two on a table of segments.
pub(crate) struct VariantPathSegment(String);

impl VariantPathSegment {
    pub(crate) fn from_variant_name(name: &Ident) -> Result<Self, VariantPathSegmentError> {
        Self::try_from(to_snake_case(&name.unraw().to_string()))
    }
}

/// Emits the segment as a string literal.
impl ToTokens for VariantPathSegment {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        self.0.to_tokens(tokens);
    }
}

impl TryFrom<String> for VariantPathSegment {
    type Error = VariantPathSegmentError;

    fn try_from(segment: String) -> Result<Self, Self::Error> {
        if segment.is_empty() {
            return Err(VariantPathSegmentError::Empty);
        }
        if segment.len() > VARIANT_PATH_CAPACITY_BYTES {
            return Err(VariantPathSegmentError::TooLong { len: segment.len() });
        }
        if let Some(invalid_char) = segment
            .chars()
            .find(|&c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err(VariantPathSegmentError::InvalidChar {
                segment,
                invalid_char,
            });
        }
        Ok(Self(segment))
    }
}

impl AsRef<str> for VariantPathSegment {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

pub(crate) enum VariantPathSegmentError {
    Empty,
    InvalidChar { segment: String, invalid_char: char },
    TooLong { len: usize },
}

impl Display for VariantPathSegmentError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("variant path segment is empty"),
            Self::InvalidChar {
                segment,
                invalid_char,
            } => write!(
                f,
                "variant path segment {segment:?} contains {invalid_char:?}, outside ASCII letters, digits, '-' and '_'"
            ),
            Self::TooLong { len } => write!(
                f,
                "variant path segment is {len} bytes, past the {VARIANT_PATH_CAPACITY_BYTES} bytes a variant path holds"
            ),
        }
    }
}

/// Starts a word at a capital after a lower-case letter or a digit, and at the last capital of a
/// run followed by a lower-case letter: `HTTPError` gives `http_error`, `GPS3Lock` gives
/// `gps3_lock`. It copies every character outside ASCII unchanged.
///
/// `_to_snake_case` in the Python SDK has the same rule. The tests of both SDKs read the names in
/// `tests/fixtures/event_kind_variant_path_segments.toml`.
fn to_snake_case(name: &str) -> String {
    let mut snake_case = String::with_capacity(name.len());
    let mut previous: Option<char> = None;
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_uppercase() {
            let next_is_lowercase = chars.peek().is_some_and(char::is_ascii_lowercase);
            let starts_a_word = previous.is_some_and(|previous| {
                previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase() && next_is_lowercase)
            });
            if starts_a_word {
                snake_case.push('_');
            }
            snake_case.push(c.to_ascii_lowercase());
        } else {
            snake_case.push(c);
        }
        previous = Some(c);
    }
    snake_case
}

const VARIANT_PATH_CAPACITY_BYTES: usize = 255;

#[cfg(test)]
mod tests {
    use geotrace_sdk::{DateTime, EventMarker, Utc};
    use rstest::rstest;

    use super::{VARIANT_PATH_CAPACITY_BYTES, VariantPathSegment};

    #[rstest]
    #[case::letters_and_digits("gps3".to_owned())]
    #[case::an_underscore("gps_lock".to_owned())]
    #[case::a_hyphen("legacy-path".to_owned())]
    #[case::at_the_capacity("a".repeat(VARIANT_PATH_CAPACITY_BYTES))]
    #[case::past_the_capacity("a".repeat(VARIANT_PATH_CAPACITY_BYTES + 1))]
    #[case::empty(String::new())]
    #[case::a_slash("power/boot".to_owned())]
    #[case::a_space("gps lock".to_owned())]
    #[case::a_non_ascii_letter("größe".to_owned())]
    fn a_segment_is_valid_exactly_when_event_marker_builder_accepts_it_without_a_slash(
        #[case] segment: String,
    ) {
        let accepted_by_event_marker_builder = EventMarker::builder()
            .variant_path(segment.clone())
            .sys_time(DateTime::<Utc>::UNIX_EPOCH)
            .build()
            .is_ok();
        let expected_valid = accepted_by_event_marker_builder && !segment.contains('/');
        assert_eq!(
            VariantPathSegment::try_from(segment).is_ok(),
            expected_valid
        );
    }
}
