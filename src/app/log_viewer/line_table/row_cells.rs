use std::borrow::Cow;

use super::TimestampText;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct CellIndex(pub(super) usize);

pub(super) struct RowCells<'a>(pub(super) [Option<RowCell<'a>>; 3]);

pub(super) enum RowCell<'a> {
    Divider { text: String, strong: bool },
    Message(&'a str),
    Number(String),
    Source { text: &'a str, boot: bool },
    Timestamp(TimestampText),
}

impl RowCells<'_> {
    pub(super) fn indexed(&self) -> impl DoubleEndedIterator<Item = (CellIndex, &RowCell<'_>)> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| Some((CellIndex(index), cell.as_ref()?)))
    }
}

impl RowCell<'_> {
    pub(super) fn clipboard_text(&self) -> Cow<'_, str> {
        match self {
            Self::Divider { text, .. } | Self::Number(text) => Cow::Borrowed(text),
            Self::Message(text) | Self::Source { text, .. } => Cow::Borrowed(text),
            Self::Timestamp(text) => Cow::Owned(text.clipboard_text()),
        }
    }
}

/// Each displayed character maps to one clipboard character at the same index.
#[derive(Clone, Copy)]
pub(super) enum CopyProjection {
    ReplaceFirst(char),
    Verbatim,
}

impl CopyProjection {
    pub(super) fn characters(self, display: &str) -> impl Iterator<Item = char> + '_ {
        display
            .chars()
            .enumerate()
            .map(move |(index, character)| match (index, self) {
                (0, Self::ReplaceFirst(replacement)) => replacement,
                _ => character,
            })
    }
}
