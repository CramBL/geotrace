//! The fixed-width string fields of the `.gtd` format.

use std::fmt;
use std::str::Utf8Error;

/// A UTF-8 string stored in a `.gtd` field of `ROW_BYTES` bytes: the content,
/// a nul terminator, then nul padding out to the row width.
///
/// [`FixedWidthString::new`] is the only way in, and it refuses a value the
/// row cannot hold and a value containing a nul byte. Every value that
/// constructs therefore survives [`FixedWidthString::encode_row`] followed by
/// [`FixedWidthString::decode_row`] unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedWidthString<const ROW_BYTES: usize>(String);

/// The `variant_path` of `event_markers` and of `event_marker_styles`.
pub type VariantPathField = FixedWidthString<256>;

/// The `annotation` of `event_markers`.
pub type AnnotationField = FixedWidthString<512>;

/// The `label` of `markers`.
pub type MarkerLabelField = FixedWidthString<256>;

/// The `icon_name` of `event_marker_styles`.
pub type IconNameField = FixedWidthString<32>;

/// The `color_hex` of `event_marker_styles`.
pub type ColorHexField = FixedWidthString<8>;

impl<const ROW_BYTES: usize> FixedWidthString<ROW_BYTES> {
    /// The longest UTF-8 encoding a value may have: the row width less the nul
    /// terminator. A `ROW_BYTES` of zero leaves no room for one, and using the
    /// const generic at zero fails to compile.
    pub const CONTENT_CAPACITY: usize = ROW_BYTES - 1;

    /// Errors when the value contains a nul byte, and when its UTF-8 encoding
    /// is longer than [`Self::CONTENT_CAPACITY`].
    pub fn new(value: impl Into<String>) -> Result<Self, FixedWidthStringError> {
        let value = value.into();
        if let Some(offset) = value.bytes().position(|byte| byte == 0) {
            return Err(FixedWidthStringError::InteriorNul { value, offset });
        }
        if value.len() > Self::CONTENT_CAPACITY {
            return Err(FixedWidthStringError::TooLong {
                len: value.len(),
                capacity: Self::CONTENT_CAPACITY,
                value,
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `None` where the field is empty, the `.gtd` encoding of an absent value.
    pub(crate) fn into_string_unless_empty(self) -> Option<String> {
        if self.0.is_empty() {
            None
        } else {
            Some(self.0)
        }
    }

    pub fn encode_row(&self) -> [u8; ROW_BYTES] {
        let mut row = [0u8; ROW_BYTES];
        if let Some(destination) = row.get_mut(..self.0.len()) {
            destination.copy_from_slice(self.0.as_bytes());
        }
        row
    }

    /// Errors on a row of the wrong width, on a row with no nul terminator, on
    /// a non-nul padding byte, and on content that is not UTF-8.
    pub fn decode_row(row: &[u8]) -> Result<Self, FixedWidthStringError> {
        if row.len() != ROW_BYTES {
            return Err(FixedWidthStringError::RowWidth {
                expected: ROW_BYTES,
                actual: row.len(),
            });
        }
        let Some(terminator) = row.iter().position(|&byte| byte == 0) else {
            return Err(FixedWidthStringError::MissingTerminator { width: ROW_BYTES });
        };
        let padding_start = terminator.saturating_add(1);
        let padding = row.get(padding_start..).unwrap_or_default();
        if let Some(offset) = padding.iter().position(|&byte| byte != 0) {
            return Err(FixedWidthStringError::PaddingNotNul {
                offset: padding_start.saturating_add(offset),
            });
        }
        let content = row.get(..terminator).unwrap_or_default();
        let content = std::str::from_utf8(content)
            .map_err(|source| FixedWidthStringError::NotUtf8 { source })?;
        Self::new(content)
    }
}

impl<const ROW_BYTES: usize> fmt::Display for FixedWidthString<ROW_BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const ROW_BYTES: usize> AsRef<str> for FixedWidthString<ROW_BYTES> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<const ROW_BYTES: usize> From<FixedWidthString<ROW_BYTES>> for String {
    fn from(value: FixedWidthString<ROW_BYTES>) -> Self {
        value.0
    }
}

/// Why a value cannot be held in a [`FixedWidthString`], or why a row does not
/// decode to one.
#[derive(Debug, Clone, thiserror::Error)]
pub enum FixedWidthStringError {
    #[error("{value:?} is {len} bytes, past the {capacity} bytes the field holds")]
    TooLong {
        value: String,
        len: usize,
        capacity: usize,
    },

    #[error("{value:?} has a nul byte at offset {offset}")]
    InteriorNul { value: String, offset: usize },

    #[error("a field row of {actual} bytes, where the field is {expected} bytes wide")]
    RowWidth { expected: usize, actual: usize },

    #[error("the field row has no nul terminator in its {width} bytes")]
    MissingTerminator { width: usize },

    #[error("the field row is not UTF-8: {source}")]
    NotUtf8 { source: Utf8Error },

    #[error("the field row has a non-nul byte at offset {offset}, past its nul terminator")]
    PaddingNotNul { offset: usize },
}
