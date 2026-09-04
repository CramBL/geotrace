//! The text a log is parsed from, decoded from bytes that need not be UTF-8.

use std::{str, sync::Arc};

/// A log's text together with what decoding it to UTF-8 cost.
///
/// The parse model is `str`: bytes that are not UTF-8 are replaced here, at the
/// load boundary, and the raw bytes are not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogText {
    text: Arc<str>,
    replaced_byte_count: usize,
}

impl LogText {
    /// Decodes `bytes` as UTF-8, writing one replacement character per
    /// malformed sequence and counting the bytes those sequences held.
    pub fn decode_lossy(bytes: &[u8]) -> Self {
        let mut rest = bytes;
        let mut decoded: Option<String> = None;
        let mut replaced_byte_count = 0;
        loop {
            let error = match str::from_utf8(rest) {
                Ok(tail) => {
                    return match decoded {
                        Some(mut decoded) => {
                            decoded.push_str(tail);
                            Self {
                                text: Arc::from(decoded),
                                replaced_byte_count,
                            }
                        }
                        None => Self {
                            text: Arc::from(tail),
                            replaced_byte_count,
                        },
                    };
                }
                Err(error) => error,
            };
            let decoded = decoded.get_or_insert_with(|| String::with_capacity(bytes.len()));
            let valid_up_to = error.valid_up_to();
            if let Some(valid) = rest
                .get(..valid_up_to)
                .and_then(|head| str::from_utf8(head).ok())
            {
                decoded.push_str(valid);
            }
            // A sequence cut short by the end of the input has no length:
            // everything after the last valid character is what it holds.
            let malformed_byte_count = error
                .error_len()
                .unwrap_or_else(|| rest.len().saturating_sub(valid_up_to));
            decoded.push(char::REPLACEMENT_CHARACTER);
            replaced_byte_count = replaced_byte_count.saturating_add(malformed_byte_count);
            rest = rest
                .get(valid_up_to.saturating_add(malformed_byte_count)..)
                .unwrap_or_default();
        }
    }

    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    /// Bytes a lossy decode replaced, zero for text that arrived as UTF-8.
    pub fn replaced_byte_count(&self) -> usize {
        self.replaced_byte_count
    }

    pub(crate) fn into_parts(self) -> (Arc<str>, usize) {
        (self.text, self.replaced_byte_count)
    }
}

impl From<Arc<str>> for LogText {
    fn from(text: Arc<str>) -> Self {
        Self {
            text,
            replaced_byte_count: 0,
        }
    }
}

impl From<String> for LogText {
    fn from(text: String) -> Self {
        Self::from(Arc::<str>::from(text))
    }
}

impl From<&str> for LogText {
    fn from(text: &str) -> Self {
        Self::from(Arc::<str>::from(text))
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, proptest};
    use rstest::rstest;

    use super::*;

    const REPLACEMENT: char = char::REPLACEMENT_CHARACTER;

    #[test]
    fn utf8_input_is_taken_as_it_stands() {
        let decoded = LogText::decode_lossy("navsyncd: fix acquired ±2 m\n".as_bytes());
        assert_eq!(&**decoded.text(), "navsyncd: fix acquired ±2 m\n");
        assert_eq!(decoded.replaced_byte_count(), 0);
    }

    /// One replacement character per malformed sequence, counting the bytes it
    /// replaced.
    #[rstest]
    #[case::latin1_byte_mid_line(b"caf\xe9 open", format!("caf{REPLACEMENT} open"), 1)]
    #[case::sequence_cut_short_by_the_end(b"end \xe2\x82", format!("end {REPLACEMENT}"), 2)]
    #[case::two_sequences(b"\xf0 mid \xff", format!("{REPLACEMENT} mid {REPLACEMENT}"), 2)]
    #[case::lone_continuation_byte(b"a\x80b", format!("a{REPLACEMENT}b"), 1)]
    fn a_malformed_sequence_is_replaced_and_counted(
        #[case] bytes: &[u8],
        #[case] expected_text: String,
        #[case] expected_replaced_bytes: usize,
    ) {
        let decoded = LogText::decode_lossy(bytes);
        assert_eq!(&**decoded.text(), expected_text);
        assert_eq!(decoded.replaced_byte_count(), expected_replaced_bytes);
    }

    #[test]
    fn empty_input_decodes_to_empty_text() {
        let decoded = LogText::decode_lossy(b"");
        assert_eq!(&**decoded.text(), "");
        assert_eq!(decoded.replaced_byte_count(), 0);
    }

    proptest! {
        /// Whatever bytes a drop carries, the decode agrees with the standard
        /// lossy conversion, and only input that is not UTF-8 costs bytes.
        #[test]
        fn decoding_any_bytes_matches_the_standard_lossy_conversion(
            bytes in prop::collection::vec(any::<u8>(), 0..64),
        ) {
            let decoded = LogText::decode_lossy(&bytes);
            prop_assert_eq!(decoded.text().as_ref(), String::from_utf8_lossy(&bytes));
            prop_assert_eq!(
                decoded.replaced_byte_count() == 0,
                str::from_utf8(&bytes).is_ok()
            );
        }
    }
}
